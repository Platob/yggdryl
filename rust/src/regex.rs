//! Struct datatype inference from named regular-expression captures.

use ::regex::bytes::Regex;
use regex_syntax::ParserBuilder;
use regex_syntax::hir::{Class, Hir, HirKind};
use smol_str::format_smolstr;

use crate::{DataType, Error, Field, Result, Scalar, StructType, TimeUnit, Timezone};

impl DataType {
    /// Build a Struct datatype from a regex's named captures.
    ///
    /// Capture order is regex order and every capture field is nullable: the
    /// complete expression may miss a row, and a capture may sit in an
    /// optional branch. With `autotype`, a capture whose regex constrains its
    /// complete language to a supported scalar format becomes Boolean, Int64,
    /// Float64, Date32, Time32/Time64, or DateTime64. A clock's fraction is
    /// read at either decimal sign ISO 8601 names, and a capture admitting
    /// several widths takes the widest spelling it matches, the only
    /// resolution that holds every row it admits. Broad captures such as
    /// `\S+` remain `utf8`. Disabling `autotype` makes every capture `utf8`.
    ///
    /// Inference examines syntax only. It never reads a value, so callers can
    /// publish a result schema before opening the resource they will parse.
    /// Candidate formats are materialized through [`Scalar`] and its datatype
    /// identity rather than through a second datatype table.
    ///
    /// ```
    /// use yggdryl::{DataType, TimeUnit, Timezone};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let dtype = DataType::from_regex(
    ///     r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)",
    ///     true,
    /// )?;
    /// assert_eq!(dtype.field("level")?.dtype(), &DataType::utf8());
    /// assert_eq!(dtype.field("id")?.dtype(), &DataType::Int64);
    /// assert!(dtype.field("id")?.is_nullable());
    ///
    /// // A clock reads at either decimal sign, and a capture admitting
    /// // several fraction widths publishes the widest of them.
    /// let stamps = DataType::from_regex(
    ///     concat!(
    ///         r"(?<comma>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3}) ",
    ///         r"(?<wide>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{1,5})",
    ///     ),
    ///     true,
    /// )?;
    /// assert_eq!(
    ///     stamps.field("comma")?.dtype(),
    ///     &DataType::datetime64(TimeUnit::Millisecond, Timezone::NAIVE)?,
    /// );
    /// assert_eq!(
    ///     stamps.field("wide")?.dtype(),
    ///     &DataType::datetime64(TimeUnit::Microsecond, Timezone::NAIVE)?,
    /// );
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

        StructType::from_fields(
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
        .map(DataType::from)
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
        if let Some(dtype) = temporal_dtype(capture, &expression) {
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

/// Whether a capture's language can spell `byte`.
///
/// A capture emits a byte only where a literal or a class holds it, so a
/// `false` here is exact rather than a guess: no candidate carrying that byte
/// can match, and the probe never builds it. `\d+` spells neither `:` nor
/// `-`, so it is offered no clock and no calendar at all.
fn contains_byte(hir: &Hir, byte: u8) -> bool {
    let mut pending = vec![hir];
    while let Some(node) = pending.pop() {
        match node.kind() {
            HirKind::Literal(literal) if literal.0.contains(&byte) => return true,
            HirKind::Class(class) if class_contains(class, byte, byte) => return true,
            _ => pending.extend(node.kind().subs()),
        }
    }
    false
}

const fn numeric_byte(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.' | b'e' | b'E')
}

/// The bytes a temporal spelling is made of.
///
/// The comma is here as ISO 8601's other decimal sign, so a log4j clock is
/// offered to the ISO reader at all. It is deliberately absent from
/// [`numeric_byte`]: a comma outside a clock groups thousands or separates a
/// list, and nothing here should read `1,234` as a number.
const fn temporal_byte(byte: u8) -> bool {
    byte.is_ascii_digit()
        || matches!(
            byte,
            b'+' | b'-' | b':' | b'.' | b',' | b'_' | b' ' | b'T' | b't' | b'Z' | b'z'
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

/// The temporal datatype a capture's language names, or `None`.
///
/// Candidate spellings are offered to the compiled capture and the first one
/// it matches whole is read through [`Scalar`], so the unit and zone a capture
/// publishes are the ones this crate's own reader will answer with on the rows
/// that follow. Nothing here decides what a fraction width means; the reader
/// is asked, in [`resolved_dtype`].
///
/// The capture's own alphabet decides which candidates are built at all. A
/// datetime carries `-` and `:`, a date carries `-`, a clock carries `:`, and
/// every fraction opens with a decimal sign, so a capture that cannot spell
/// those bytes cannot match those candidates and is never offered them. That
/// is why `(?<id>\d+)` costs no probe at all, where it once cost every one.
fn temporal_dtype(capture: &Hir, expression: &Regex) -> Option<DataType> {
    /// The fraction spellings a clock is probed with, widest width first.
    ///
    /// A capture admitting several widths names the widest spelling it
    /// matches: every coarser unit restates exactly into a finer one, so only
    /// the widest width holds every count the capture can spell. `\.\d{1,5}`
    /// is therefore microseconds, and calling it milliseconds would publish a
    /// schema this crate's own reader refuses on a five-digit row. The empty
    /// fraction sorts last for the same reason, or `(?:\.\d{3})?` would read
    /// as seconds and drop the milliseconds it admits.
    ///
    /// ISO 8601 names both the full stop and the comma the decimal sign, so
    /// each width is spelled both ways; the full stop leads only because it is
    /// the one this crate writes back. `_` groups the digits a log emitter
    /// groups, which is microseconds and nanoseconds and nothing narrower.
    const FRACTIONS: &[&str] = &[
        ".123456789",
        ".123_456_789",
        ",123456789",
        ",123_456_789",
        ".12345678",
        ",12345678",
        ".1234567",
        ",1234567",
        ".123456",
        ".123_456",
        ",123456",
        ",123_456",
        ".12345",
        ",12345",
        ".1234",
        ",1234",
        ".123",
        ",123",
        ".12",
        ",12",
        ".1",
        ",1",
        "",
    ];
    const ZONES: &[&str] = &["Z", "+00:00", "+02:00", "-05:00"];

    let clock = contains_byte(capture, b':');
    let calendar = contains_byte(capture, b'-');
    let signed = contains_byte(capture, b'.') || contains_byte(capture, b',');
    let grouped = contains_byte(capture, b'_');
    let fractions = FRACTIONS
        .iter()
        .copied()
        .filter(|fraction| fraction.is_empty() || (signed && (grouped || !fraction.contains('_'))))
        .collect::<Vec<_>>();

    if clock && calendar {
        for fraction in &fractions {
            for separator in ['T', ' '] {
                for zone in ZONES {
                    let value = format!("2024-02-01{separator}12:34:56{fraction}{zone}");
                    if expression.is_match(value.as_bytes()) {
                        if let Some(dtype) = resolved_dtype(&value, |unit| {
                            DataType::datetime64(unit, Timezone::UTC).ok()
                        }) {
                            return Some(dtype);
                        }
                    }
                }
                let value = format!("2024-02-01{separator}12:34:56{fraction}");
                if expression.is_match(value.as_bytes()) {
                    if let Some(dtype) = resolved_dtype(&value, |unit| {
                        DataType::datetime64(unit, Timezone::NAIVE).ok()
                    }) {
                        return Some(dtype);
                    }
                }
            }
        }
    }

    if calendar {
        let date = "2024-02-01";
        if expression.is_match(date.as_bytes()) {
            if let Some(dtype) = temporal_scalar_dtype(date, DataType::date32()) {
                return Some(dtype);
            }
        }
    }

    if clock {
        for fraction in &fractions {
            let value = format!("12:34:56{fraction}");
            if expression.is_match(value.as_bytes()) {
                if let Some(dtype) = resolved_dtype(&value, |unit| DataType::time(unit).ok()) {
                    return Some(dtype);
                }
            }
        }
    }
    None
}

/// The datatype a matched clock candidate materializes into, at the coarsest
/// resolution that reads its spelling exactly.
///
/// What unit a fraction width names is the ISO reader's rule, not this
/// module's, so the width is never counted here. The candidate is offered at
/// each resolution from seconds down and [`Scalar::from_temporal_text`]
/// refuses every unit too coarse to hold the count it read - `12:34:56.123` is
/// no exact second - while every finer unit would accept. The first resolution
/// that reads is therefore the one the spelling names, and the width-to-unit
/// table stays where it belongs.
fn resolved_dtype(value: &str, dtype: impl Fn(TimeUnit) -> Option<DataType>) -> Option<DataType> {
    const RESOLUTIONS: &[TimeUnit] = &[
        TimeUnit::Second,
        TimeUnit::Millisecond,
        TimeUnit::Microsecond,
        TimeUnit::Nanosecond,
    ];
    RESOLUTIONS
        .iter()
        .copied()
        .filter_map(dtype)
        .find_map(|dtype| temporal_scalar_dtype(value, dtype))
}

/// The datatype `value` reads as when offered as `dtype`, or `None` where this
/// crate's own reader refuses the spelling.
fn temporal_scalar_dtype(value: &str, dtype: DataType) -> Option<DataType> {
    Scalar::from_temporal_text(&dtype, value).ok()?.dtype().ok()
}

fn text_dtype() -> Result<DataType> {
    Scalar::from("").dtype()
}
