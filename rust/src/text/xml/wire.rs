//! The natural XML projection of [`Scalar`].
//!
//! XML has no private type envelopes and no type grammar at all: every value
//! is character data. Leaves are written in their interoperable spelling - the
//! same ISO text, decimal text and base64 the other structured formats use -
//! and a [`crate::Field`] restores exact types on a schema-directed read.

use std::io::Write;

use base64::Engine as _;
use smol_str::format_smolstr;

use crate::types::{Integer, Nested, Temporal};
use crate::{Error, Result, Scalar, TimeUnit, Timezone};

use super::{ATTRIBUTE_PREFIX, TEXT_KEY, codec_error};

/// How one document is laid out.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Layout {
    /// The bytes one nesting level costs, or `None` for one flat line.
    unit: Option<&'static [u8]>,
}

impl From<crate::text::Formatting> for Layout {
    fn from(value: crate::text::Formatting) -> Self {
        Self {
            unit: value.indent().unit(),
        }
    }
}

/// Write one value as one XML document.
///
/// The value is a one-entry record or mapping: its key names the root element,
/// because an XML document is exactly one root element.
pub(super) fn write_document<W: Write>(
    writer: &mut W,
    value: &Scalar,
    layout: Layout,
) -> Result<()> {
    let entries = entries_of(value).ok_or_else(|| {
        invalid(format_smolstr!(
            "an XML document is one root element, so it is written from a one-entry record, got {}",
            value.kind()
        ))
    })?;
    let [(name, root)] = entries.as_slice() else {
        return Err(invalid(format_smolstr!(
            "an XML document has exactly one root element, got {} entries",
            entries.len()
        )));
    };
    if name.starts_with(ATTRIBUTE_PREFIX) || *name == TEXT_KEY {
        return Err(invalid(format_smolstr!(
            "a document root names an element, got {name:?}"
        )));
    }
    write_element(writer, name, root, layout, 0)
}

/// Write one element, named by the key it was stored under.
fn write_element<W: Write>(
    writer: &mut W,
    name: &str,
    value: &Scalar,
    layout: Layout,
    depth: usize,
) -> Result<()> {
    check_name(name)?;
    if let Scalar::Nested(Nested::Sequence(_)) = value {
        return Err(invalid(format_smolstr!(
            "XML repeats an element rather than nesting a sequence inside one, so the sequence \
             under {name:?} cannot hold another"
        )));
    }
    if let Scalar::Temporal(Temporal::Interval(interval)) = value {
        // An interval is its ordered parts, and XML frames a sequence by
        // repeating the element rather than nesting one inside it.
        let parts = interval_parts(
            interval.unit(),
            interval.months(),
            interval.days(),
            interval.nanoseconds(),
        )?;
        if depth == 0 && parts.len() > 1 {
            return Err(invalid(format_smolstr!(
                "an XML document has exactly one root element, and an interval of {} needs {}",
                interval.unit(),
                parts.len()
            )));
        }
        let mut first = true;
        for part in parts {
            if !first {
                write_break(writer, layout, depth)?;
            }
            first = false;
            write!(writer, "<{name}>{part}</{name}>")?;
        }
        return Ok(());
    }
    let Some(entries) = entries_of(value) else {
        // A leaf is its character data, and absence is the empty element.
        if value.is_null() {
            write!(writer, "<{name}/>")?;
            return Ok(());
        }
        write!(writer, "<{name}>")?;
        write_text(writer, &leaf_text(value)?, Escaping::Text)?;
        write!(writer, "</{name}>")?;
        return Ok(());
    };

    let mut attributes = Vec::new();
    let mut text = None;
    let mut children = Vec::new();
    for (key, value) in entries {
        if let Some(attribute) = key.strip_prefix(ATTRIBUTE_PREFIX) {
            attributes.push((attribute, value));
        } else if key == TEXT_KEY {
            text = Some(value);
        } else if let Scalar::Nested(Nested::Sequence(values)) = value {
            // A sequence is how a document repeats one element, so each of its
            // values is written under the same name.
            children.extend(values.as_slice().iter().map(|value| (key, value)));
        } else {
            children.push((key, value));
        }
    }
    if text.is_some() && !children.is_empty() {
        return Err(invalid(format_smolstr!(
            "an element holds character data or child elements, not both, and {name:?} holds both"
        )));
    }

    write!(writer, "<{name}")?;
    for (attribute, value) in attributes {
        check_name(attribute)?;
        if value.is_null() || entries_of(value).is_some() {
            return Err(invalid(format_smolstr!(
                "an XML attribute holds one text value, and {attribute:?} holds {}",
                value.kind()
            )));
        }
        write!(writer, " {attribute}=\"")?;
        write_text(writer, &leaf_text(value)?, Escaping::Attribute)?;
        writer.write_all(b"\"")?;
    }

    if let Some(text) = text {
        writer.write_all(b">")?;
        if !text.is_null() {
            write_text(writer, &leaf_text(text)?, Escaping::Text)?;
        }
        write!(writer, "</{name}>")?;
        return Ok(());
    }
    if children.is_empty() {
        writer.write_all(b"/>")?;
        return Ok(());
    }

    writer.write_all(b">")?;
    let child_depth = depth.saturating_add(1);
    for (child, value) in children {
        write_break(writer, layout, child_depth)?;
        write_element(writer, child, value, layout, child_depth)?;
    }
    write_break(writer, layout, depth)?;
    write!(writer, "</{name}>")?;
    Ok(())
}

/// Start the line one nested element is written on, when laying one out.
fn write_break<W: Write>(writer: &mut W, layout: Layout, depth: usize) -> Result<()> {
    let Some(unit) = layout.unit else {
        return Ok(());
    };
    writer.write_all(b"\n")?;
    for _ in 0..depth {
        writer.write_all(unit)?;
    }
    Ok(())
}

/// The name-keyed entries of a record or a string-keyed mapping.
fn entries_of(value: &Scalar) -> Option<Vec<(&str, &Scalar)>> {
    match value {
        Scalar::Nested(Nested::Record(entries)) => Some(
            entries
                .as_map()
                .iter()
                .map(|(name, value)| (name.as_str(), value))
                .collect(),
        ),
        Scalar::Nested(Nested::Mapping(entries)) => entries
            .as_slice()
            .iter()
            .map(|(key, value)| key.as_str().map(|key| (key, value)))
            .collect(),
        _ => None,
    }
}

/// Which characters a position must spell as references.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Escaping {
    /// Character data, where only a carriage return needs a reference to
    /// survive the end-of-line normalization every reader applies.
    Text,
    /// An attribute value, where the reader also normalizes every tab and line
    /// break to a space unless it arrives as a reference.
    Attribute,
}

/// Write text, spelling as a reference everything that would not survive.
fn write_text<W: Write>(writer: &mut W, text: &str, escaping: Escaping) -> Result<()> {
    let mut start = 0;
    for (position, character) in text.char_indices() {
        let replacement = match character {
            '<' => "&lt;",
            '>' => "&gt;",
            '&' => "&amp;",
            '"' if escaping == Escaping::Attribute => "&quot;",
            '\r' => "&#13;",
            '\n' if escaping == Escaping::Attribute => "&#10;",
            '\t' if escaping == Escaping::Attribute => "&#9;",
            '\t' | '\n' => continue,
            // XML 1.0 has no spelling at all for the other control characters,
            // not even a character reference.
            control if control < '\u{20}' => {
                return Err(invalid(format_smolstr!(
                    "XML cannot represent the control character U+{:04X}",
                    u32::from(control)
                )));
            }
            _ => continue,
        };
        writer.write_all(&text.as_bytes()[start..position])?;
        writer.write_all(replacement.as_bytes())?;
        start = position + character.len_utf8();
    }
    writer.write_all(&text.as_bytes()[start..])?;
    Ok(())
}

/// Whether a key is a name XML can write as an element or an attribute.
fn check_name(name: &str) -> Result<()> {
    let mut characters = name.chars();
    let valid = characters
        .next()
        .is_some_and(is_name_start)
        .then(|| characters.all(is_name_char))
        .unwrap_or_default();
    if valid {
        return Ok(());
    }
    Err(invalid(format_smolstr!(
        "expected an XML name, got {:?}",
        crate::text::elide_to(name, crate::text::ERROR_TEXT_LIMIT)
    )))
}

/// The XML 1.0 `NameStartChar` production.
fn is_name_start(character: char) -> bool {
    matches!(character,
        ':' | '_'
        | 'A'..='Z'
        | 'a'..='z'
        | '\u{c0}'..='\u{d6}'
        | '\u{d8}'..='\u{f6}'
        | '\u{f8}'..='\u{2ff}'
        | '\u{370}'..='\u{37d}'
        | '\u{37f}'..='\u{1fff}'
        | '\u{200c}'..='\u{200d}'
        | '\u{2070}'..='\u{218f}'
        | '\u{2c00}'..='\u{2fef}'
        | '\u{3001}'..='\u{d7ff}'
        | '\u{f900}'..='\u{fdcf}'
        | '\u{fdf0}'..='\u{fffd}'
        | '\u{10000}'..='\u{effff}')
}

/// The XML 1.0 `NameChar` production.
fn is_name_char(character: char) -> bool {
    is_name_start(character)
        || matches!(character,
            '-' | '.'
            | '0'..='9'
            | '\u{b7}'
            | '\u{300}'..='\u{36f}'
            | '\u{203f}'..='\u{2040}')
}

/// One leaf's interoperable spelling, which is all XML carries.
fn leaf_text(value: &Scalar) -> Result<String> {
    Ok(match value {
        Scalar::Null => return Err(invalid("null has no XML character data".into())),
        Scalar::Boolean(value) => (if value.get() { "true" } else { "false" }).to_owned(),
        Scalar::Integer(value) => match value {
            Integer::I8(value) => value.get().to_string(),
            Integer::I16(value) => value.get().to_string(),
            Integer::I32(value) => value.get().to_string(),
            Integer::I64(value) => value.get().to_string(),
            Integer::U8(value) => value.get().to_string(),
            Integer::U16(value) => value.get().to_string(),
            Integer::U32(value) => value.get().to_string(),
            Integer::U64(value) => value.get().to_string(),
            Integer::I128(value) => value.get().to_string(),
            Integer::U128(value) => value.get().to_string(),
        },
        Scalar::Floating(value) => float_text(value.as_f64()),
        Scalar::Decimal(value) => {
            crate::types::decimal::scalars::decimal_text(value.coefficient(), value.scale())
        }
        Scalar::Text(value) => value.as_str().to_owned(),
        Scalar::Ascii(value) => value.as_str().to_owned(),
        Scalar::Version(value) => value.to_string(),
        Scalar::Url(value) => value.to_string(),
        Scalar::Uuid(value) => value.to_string(),
        Scalar::Enum(value) => value.as_str().to_owned(),
        Scalar::Bytes(value) => {
            base64::engine::general_purpose::STANDARD.encode(value.as_bytes())
        }
        Scalar::Geospatial(value) => {
            base64::engine::general_purpose::STANDARD.encode(value.as_bytes())
        }
        Scalar::Temporal(Temporal::Date32(value)) => {
            match (value.unit() == TimeUnit::Day)
                .then(|| crate::types::ascii::iso::format_date(value.count()))
                .flatten()
            {
                Some(text) => text.to_string(),
                None => value.count().to_string(),
            }
        }
        Scalar::Temporal(Temporal::Date64(value)) => {
            const DAY_MILLISECONDS: i64 = 86_400_000;
            match (value.unit() == TimeUnit::Millisecond
                && value.count().rem_euclid(DAY_MILLISECONDS) == 0)
                .then(|| i32::try_from(value.count().div_euclid(DAY_MILLISECONDS)).ok())
                .flatten()
                .and_then(crate::types::ascii::iso::format_date)
            {
                Some(text) => text.to_string(),
                None => value.count().to_string(),
            }
        }
        Scalar::Temporal(Temporal::Time32(value)) => time_text(
            i64::from(value.count()),
            value.unit(),
            &value.timezone(),
        )?,
        Scalar::Temporal(Temporal::Time64(value)) => {
            time_text(value.count(), value.unit(), &value.timezone())?
        }
        Scalar::Temporal(Temporal::DateTime64(value)) => {
            let text = if value.timezone().is_naive() {
                crate::types::ascii::iso::format_datetime(value.count(), value.unit())
            } else {
                crate::types::ascii::iso::format_timestamp(
                    value.count(),
                    value.unit(),
                    &value.timezone(),
                )
            };
            text.map_or_else(|| value.count().to_string(), |text| text.to_string())
        }
        Scalar::Temporal(Temporal::Duration32(value)) => duration_text(
            i64::from(value.count()),
            value.unit(),
            &value.timezone(),
        )?,
        Scalar::Temporal(Temporal::Duration64(value)) => {
            duration_text(value.count(), value.unit(), &value.timezone())?
        }
        // An interval is its ordered parts, which the element writer repeats
        // rather than spelling as one leaf, and a nested value is no leaf.
        Scalar::Temporal(Temporal::Interval(_)) | Scalar::Nested(_) => {
            return Err(invalid(format_smolstr!(
                "expected one leaf value for character data, got {}",
                value.kind()
            )));
        }
    })
}

/// The ordered parts an interval's layout is read back from.
fn interval_parts(
    unit: TimeUnit,
    months: i32,
    days: i32,
    nanoseconds: i64,
) -> Result<Vec<i64>> {
    Ok(match unit {
        TimeUnit::YearMonth => vec![i64::from(months)],
        TimeUnit::DayTime => vec![i64::from(days), nanoseconds / 1_000_000],
        TimeUnit::MonthDayNano => vec![i64::from(months), i64::from(days), nanoseconds],
        _ => return Err(invalid("invalid interval layout".into())),
    })
}

/// A float's spelling, in the XML Schema vocabulary readers expect.
fn float_text(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value == f64::INFINITY {
        return "INF".to_owned();
    }
    if value == f64::NEG_INFINITY {
        return "-INF".to_owned();
    }
    let spelling = value.to_string();
    if spelling.contains(['.', 'e', 'E']) {
        return spelling;
    }
    spelling + ".0"
}

fn time_text(count: i64, unit: TimeUnit, zone: &Timezone) -> Result<String> {
    if !zone.is_naive() {
        return Err(invalid(
            "time-of-day cannot carry a timezone; use DateTime64 for a zoned instant".into(),
        ));
    }
    Ok(crate::types::ascii::iso::format_time(count, unit)
        .map_or_else(|| count.to_string(), |text| text.to_string()))
}

fn duration_text(count: i64, unit: TimeUnit, zone: &Timezone) -> Result<String> {
    if !zone.is_naive() {
        return Err(invalid("duration cannot carry a timezone".into()));
    }
    Ok(crate::types::ascii::iso::format_duration(count, unit)
        .map_or_else(|| count.to_string(), |text| text.to_string()))
}

fn invalid(reason: smol_str::SmolStr) -> Error {
    codec_error(0, reason)
}
