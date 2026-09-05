//! Struct datatype inference from named regular-expression captures.

use ::regex::bytes::Regex;
use regex_syntax::ParserBuilder;
use regex_syntax::hir::{Class, Hir, HirKind};
use smol_str::format_smolstr;

use crate::{DataType, Error, Field, Result, Scalar, TimeUnit, Timezone};

impl DataType {
    /// Build a Struct datatype from a regex's named captures.
    ///
    /// Capture order is regex order and every capture field is nullable: the
    /// complete expression may miss a row, and a capture may sit in an
    /// optional branch. With `autotype`, a capture whose regex constrains its
    /// complete language to a supported scalar format becomes Boolean, Int64,
    /// Float64, Date32, Time32/Time64, or DateTime64. Broad captures such as
    /// `\S+` remain Utf8. Disabling `autotype` makes every capture Utf8.
    ///
    /// Inference examines syntax only. It never reads a value, so callers can
    /// publish a result schema before opening the resource they will parse.
    /// Candidate formats are materialized through [`Scalar`] and its datatype
    /// identity rather than through a second datatype table.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let dtype = DataType::from_regex(
    ///     r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)",
    ///     true,
    /// )?;
    /// assert_eq!(dtype.field("level")?.dtype(), &DataType::Utf8);
    /// assert_eq!(dtype.field("id")?.dtype(), &DataType::Int64);
    /// assert!(dtype.field("id")?.is_nullable());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a typed datatype error when the regex is malformed or exceeds
    /// the shared schema recursion limit.
    pub fn from_regex(pattern: &str, autotype: bool) -> Result<Self> {
        let hir = parse(pattern)?;
        let unicode_digits = unicode_digits()?;
        let mut captures = Vec::new();
        let mut pending = vec![&hir];
        while let Some(node) = pending.pop() {
            if let HirKind::Capture(capture) = node.kind() {
                if let Some(name) = capture.name.as_deref() {
                    captures.push((capture.index, name, capture.sub.as_ref()));
                }
            }
            pending.extend(node.kind().subs().iter().rev());
        }
        captures.sort_by_key(|(index, _, _)| *index);

        DataType::from_fields(
            captures
                .into_iter()
                .map(|(_, name, capture)| {
                    let dtype = if autotype {
                        inferred_capture(capture, &unicode_digits)?
                    } else {
                        text_dtype()?
                    };
                    Ok(Field::new(name, dtype, true))
                })
                .collect::<Result<Vec<_>>>()?,
        )
    }
}

fn parse(pattern: &str) -> Result<Hir> {
    let mut builder = ParserBuilder::new();
    builder
        .utf8(false)
        .nest_limit(DataType::PARSE_RECURSION_LIMIT as u32);
    builder
        .build()
        .parse(pattern)
        .map_err(|error| regex_error(&error))
}

fn unicode_digits() -> Result<Class> {
    let hir = parse(r"\d")?;
    match hir.kind() {
        HirKind::Class(class) => Ok(class.clone()),
        _ => Err(regex_error(
            "the built-in decimal class did not compile to a class",
        )),
    }
}

fn regex_error(error: &(impl std::fmt::Display + ?Sized)) -> Error {
    Error::InvalidDataType {
        kind: "regex",
        reason: format_smolstr!(
            "expected a valid bounded regular expression: {}",
            crate::text::elide_to(&error.to_string(), crate::text::ERROR_TEXT_LIMIT)
        ),
    }
}

fn inferred_capture(capture: &Hir, unicode_digits: &Class) -> Result<DataType> {
    if let Some(dtype) = boolean_dtype(capture)? {
        return Ok(dtype);
    }

    let expression =
        Regex::new(&format!(r"\A(?:{capture})\z")).map_err(|error| regex_error(&error))?;
    if allowed(capture, unicode_digits, temporal_byte) && contains_digit(capture, unicode_digits) {
        if let Some(dtype) = temporal_dtype(&expression) {
            return Ok(dtype);
        }
    }
    if allowed(capture, unicode_digits, numeric_byte) && contains_digit(capture, unicode_digits) {
        if let Some(dtype) = numeric_dtype(&expression)? {
            return Ok(dtype);
        }
    }
    text_dtype()
}

fn boolean_dtype(capture: &Hir) -> Result<Option<DataType>> {
    let Some(words) = finite_literals(capture, 0) else {
        return Ok(None);
    };
    if words.is_empty()
        || words
            .iter()
            .any(|word| word.as_slice() != b"true" && word.as_slice() != b"false")
    {
        return Ok(None);
    }
    Scalar::from(true).dtype().map(Some)
}

fn finite_literals(hir: &Hir, depth: usize) -> Option<Vec<Vec<u8>>> {
    if depth >= DataType::PARSE_RECURSION_LIMIT {
        return None;
    }
    match hir.kind() {
        HirKind::Empty | HirKind::Look(_) => Some(vec![Vec::new()]),
        HirKind::Literal(literal) => Some(vec![literal.0.to_vec()]),
        HirKind::Capture(capture) => finite_literals(&capture.sub, depth + 1),
        HirKind::Concat(parts) => {
            let mut combined = vec![Vec::new()];
            for part in parts {
                let values = finite_literals(part, depth + 1)?;
                let mut next = Vec::new();
                for prefix in &combined {
                    for value in &values {
                        if next.len() == 8 || prefix.len().saturating_add(value.len()) > 16 {
                            return None;
                        }
                        let mut word = Vec::with_capacity(prefix.len() + value.len());
                        word.extend_from_slice(prefix);
                        word.extend_from_slice(value);
                        next.push(word);
                    }
                }
                combined = next;
            }
            Some(combined)
        }
        HirKind::Alternation(parts) => {
            let mut words = Vec::new();
            for part in parts {
                words.extend(finite_literals(part, depth + 1)?);
                if words.len() > 8 {
                    return None;
                }
            }
            Some(words)
        }
        HirKind::Repetition(_) | HirKind::Class(_) => None,
    }
}

fn allowed(hir: &Hir, unicode_digits: &Class, accepts: fn(u8) -> bool) -> bool {
    let mut pending = vec![hir];
    while let Some(node) = pending.pop() {
        match node.kind() {
            HirKind::Literal(literal) if !literal.0.iter().copied().all(accepts) => return false,
            HirKind::Class(class) if class != unicode_digits && !class_allowed(class, accepts) => {
                return false;
            }
            _ => pending.extend(node.kind().subs()),
        }
    }
    true
}

fn class_allowed(class: &Class, accepts: fn(u8) -> bool) -> bool {
    match class {
        Class::Bytes(class) => class
            .iter()
            .all(|range| (range.start()..=range.end()).all(accepts)),
        Class::Unicode(class) => class.iter().all(|range| {
            range.end().is_ascii()
                && (u32::from(range.start())..=u32::from(range.end()))
                    .all(|value| accepts(value as u8))
        }),
    }
}

fn contains_digit(hir: &Hir, unicode_digits: &Class) -> bool {
    let mut pending = vec![hir];
    while let Some(node) = pending.pop() {
        match node.kind() {
            HirKind::Literal(literal) if literal.0.iter().any(u8::is_ascii_digit) => return true,
            HirKind::Class(class)
                if class == unicode_digits || class_contains(class, b'0', b'9') =>
            {
                return true;
            }
            _ => pending.extend(node.kind().subs()),
        }
    }
    false
}

fn class_contains(class: &Class, start: u8, end: u8) -> bool {
    match class {
        Class::Bytes(class) => class
            .iter()
            .any(|range| range.start() <= end && range.end() >= start),
        Class::Unicode(class) => class.iter().any(|range| {
            u32::from(range.start()) <= u32::from(end) && u32::from(range.end()) >= u32::from(start)
        }),
    }
}

const fn numeric_byte(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.' | b'e' | b'E')
}

const fn temporal_byte(byte: u8) -> bool {
    byte.is_ascii_digit()
        || matches!(
            byte,
            b'+' | b'-' | b':' | b'.' | b'_' | b' ' | b'T' | b't' | b'Z' | b'z'
        )
}

fn numeric_dtype(expression: &Regex) -> Result<Option<DataType>> {
    const FLOATS: &[&str] = &["0.123456789", "-1.5", "+1.5", "1e3", "1.0e-3", "1234.56"];
    if let Some(value) = FLOATS
        .iter()
        .copied()
        .find(|value| expression.is_match(value.as_bytes()))
    {
        let value = value.parse::<f64>().map_err(|error| regex_error(&error))?;
        return Scalar::from(value).dtype().map(Some);
    }

    const INTEGERS: &[&str] = &[
        "0",
        "1",
        "-1",
        "+1",
        "12",
        "123",
        "1234",
        "12345678",
        "1234567890",
        "123456789012345678",
    ];
    let Some(value) = INTEGERS
        .iter()
        .copied()
        .find(|value| expression.is_match(value.as_bytes()))
    else {
        return Ok(None);
    };
    let value = value.parse::<i64>().map_err(|error| regex_error(&error))?;
    Scalar::from(value).dtype().map(Some)
}

fn temporal_dtype(expression: &Regex) -> Option<DataType> {
    const FRACTIONS: &[(&str, TimeUnit)] = &[
        (".123456789", TimeUnit::Nanosecond),
        (".123_456_789", TimeUnit::Nanosecond),
        (".123456", TimeUnit::Microsecond),
        (".123_456", TimeUnit::Microsecond),
        (".123", TimeUnit::Millisecond),
        ("", TimeUnit::Second),
    ];
    const ZONES: &[&str] = &["Z", "+00:00", "+02:00", "-05:00"];

    for (fraction, unit) in FRACTIONS {
        for separator in ['T', ' '] {
            for zone in ZONES {
                let value = format!("2024-02-01{separator}12:34:56{fraction}{zone}");
                if expression.is_match(value.as_bytes()) {
                    return temporal_scalar_dtype(
                        &value,
                        DataType::DateTime64 {
                            unit: *unit,
                            timezone: Timezone::UTC,
                        },
                    );
                }
            }
            let value = format!("2024-02-01{separator}12:34:56{fraction}");
            if expression.is_match(value.as_bytes()) {
                return temporal_scalar_dtype(
                    &value,
                    DataType::DateTime64 {
                        unit: *unit,
                        timezone: Timezone::NAIVE,
                    },
                );
            }
        }
    }

    let date = "2024-02-01";
    if expression.is_match(date.as_bytes()) {
        return temporal_scalar_dtype(date, DataType::Date32);
    }

    for (fraction, unit) in FRACTIONS {
        let value = format!("12:34:56{fraction}");
        if expression.is_match(value.as_bytes()) {
            return temporal_scalar_dtype(&value, DataType::time(*unit).ok()?);
        }
    }
    None
}

fn temporal_scalar_dtype(value: &str, dtype: DataType) -> Option<DataType> {
    Scalar::from_temporal_text(&dtype, value).ok()?.dtype().ok()
}

fn text_dtype() -> Result<DataType> {
    Scalar::from("").dtype()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_captures_keep_order_nullability_and_format_types() {
        let dtype = DataType::from_regex(
            concat!(
                r"(?<enabled>true|false) ",
                r"(?<id>[-+]?\d+) ",
                r"(?<price>[-+]?\d+\.\d+) ",
                r"(?<date>\d{4}-\d{2}-\d{2}) ",
                r"(?<clock>\d{2}:\d{2}:\d{2}\.\d{6}) ",
                r"(?<stamp>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z) ",
                r"(?<text>\S+)"
            ),
            true,
        )
        .unwrap();
        let fields = dtype.as_fields().unwrap();
        assert_eq!(
            fields.iter().map(Field::name).collect::<Vec<_>>(),
            ["enabled", "id", "price", "date", "clock", "stamp", "text"]
        );
        assert_eq!(dtype.field("enabled").unwrap().dtype(), &DataType::Boolean);
        assert_eq!(dtype.field("id").unwrap().dtype(), &DataType::Int64);
        assert_eq!(dtype.field("price").unwrap().dtype(), &DataType::Float64);
        assert_eq!(dtype.field("date").unwrap().dtype(), &DataType::Date32);
        assert_eq!(
            dtype.field("clock").unwrap().dtype(),
            &DataType::Time64(TimeUnit::Microsecond)
        );
        assert_eq!(
            dtype.field("stamp").unwrap().dtype(),
            &DataType::DateTime64 {
                unit: TimeUnit::Second,
                timezone: Timezone::UTC,
            }
        );
        assert_eq!(dtype.field("text").unwrap().dtype(), &DataType::Utf8);
        assert!(fields.iter().all(Field::is_nullable));
    }

    #[test]
    fn disabling_autotype_keeps_every_capture_utf8() {
        let dtype = DataType::from_regex(r"(?<id>\d+)-(?<flag>true|false)", false).unwrap();
        assert!(
            dtype
                .as_fields()
                .unwrap()
                .iter()
                .all(|field| field.dtype() == &DataType::Utf8)
        );
    }

    #[test]
    fn malformed_and_over_nested_regexes_are_typed_errors() {
        assert!(matches!(
            DataType::from_regex("(?<id>", true),
            Err(Error::InvalidDataType { kind: "regex", .. })
        ));
        let nested = format!(
            "{}x{}",
            "(?:".repeat(DataType::PARSE_RECURSION_LIMIT + 1),
            ")".repeat(DataType::PARSE_RECURSION_LIMIT + 1)
        );
        assert!(matches!(
            DataType::from_regex(&nested, true),
            Err(Error::InvalidDataType { kind: "regex", .. })
        ));
    }
}
