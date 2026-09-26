//! The natural XML projection: what a value writes as, and how a declared
//! field reads the text a document leaves behind.

use std::io::Write;

use base64::Engine as _;
use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Error, Field, Result, Scalar, Serie, TimeUnit};
use crate::{bytes_dtypes, bytes_scalars, code_scalars, string_dtypes, string_scalars};

use super::{ATTRIBUTE_PREFIX, MAX_PARSER_DEPTH, TEXT_KEY};

/// How a document is laid out: one line, or one element per line.
#[derive(Clone, Copy)]
pub(super) struct Layout {
    unit: Option<&'static [u8]>,
}

impl From<crate::text::Formatting> for Layout {
    fn from(value: crate::text::Formatting) -> Self {
        Self {
            unit: value.indent().unit(),
        }
    }
}

/// Where text is written, which decides what has to be escaped.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Escape {
    /// Character data: `<`, `&` and `>`; a carriage return, which end-of-line
    /// normalization would otherwise read as a line feed.
    Content,
    /// A quoted attribute value: content's set, the quote, and the tab and
    /// line feed that attribute-value normalization would read as spaces.
    Attribute,
}

/// Write one natural XML document.
///
/// The document element is at depth 1, as the parser counts it, so a
/// document the writer accepts under `max_depth` is one the reader accepts
/// under the same limit; [`MAX_PARSER_DEPTH`] holds whatever the limit says.
///
/// # Errors
///
/// Returns [`Error::Codec`] naming what has no XML spelling: a root that is
/// not a record with one entry, a sequence under the document element or
/// inside another, a sequence holding one null, a `#` key other than
/// `#text`, an attribute or `#text` that is a container, a key that is not an
/// XML name, a character XML 1.0 cannot carry, a temporal outside its ISO 8601
/// range, a zoned time or duration, an interval of several components where
/// one value is wanted, and nesting past `max_depth`.
pub(super) fn write_document<W: Write>(
    writer: &mut W,
    value: &Scalar,
    layout: Layout,
    max_depth: usize,
) -> Result<()> {
    match value {
        // A variant at the document boundary is the record its bytes hold.
        Scalar::Variant(held) => write_document(writer, &held.scalar()?, layout, max_depth),
        Scalar::Struct(entries) => {
            let entries = entries.as_map();
            let mut entries = entries.iter().map(|(name, value)| (name.as_str(), value));
            let root = one_root(entries.next(), entries.next().is_some())?;
            write_root(writer, root, layout, max_depth)
        }
        Scalar::Map(entries) | Scalar::SortedMap(entries) => {
            let entries = entries.as_slice();
            let mut entries = entries.iter().map(|(key, value)| {
                key.as_str().map(|key| (key, value)).ok_or_else(|| {
                    codec_error(format_smolstr!(
                        "XML element names must be strings, got a {} key",
                        key.kind()
                    ))
                })
            });
            let first = entries.next().transpose()?;
            let second = entries.next().transpose()?;
            let root = one_root(first, second.is_some())?;
            write_root(writer, root, layout, max_depth)
        }
        other => Err(codec_error(format_smolstr!(
            "XML document root must be a record with one entry naming the document element, got {}",
            other.kind()
        ))),
    }
}

/// The one entry a document root holds, or the refusal that names how many
/// it held.
fn one_root<'a>(first: Option<(&'a str, &'a Scalar)>, more: bool) -> Result<(&'a str, &'a Scalar)> {
    match (first, more) {
        (Some(root), false) => Ok(root),
        (None, _) => Err(codec_error(
            "expected one entry naming the document element, got none",
        )),
        (Some(_), true) => Err(codec_error(
            "expected one entry naming the document element, got several",
        )),
    }
}

fn write_root<W: Write>(
    writer: &mut W,
    (name, value): (&str, &Scalar),
    layout: Layout,
    max_depth: usize,
) -> Result<()> {
    if is_sequence(value) {
        return Err(codec_error(format_smolstr!(
            "a sequence under the document element `{name}` would repeat the root"
        )));
    }
    write_element(writer, name, value, layout, 1, false, max_depth)
}

/// Write `<name>` holding `value`, at `depth` (the document element is 1),
/// on its own line when `separated` and the layout indents.
fn write_element<W: Write>(
    writer: &mut W,
    name: &str,
    value: &Scalar,
    layout: Layout,
    depth: usize,
    separated: bool,
    max_depth: usize,
) -> Result<()> {
    if depth > MAX_PARSER_DEPTH {
        return Err(codec_error(format_smolstr!(
            "XML nesting exceeds the writer hard limit of {MAX_PARSER_DEPTH}"
        )));
    }
    if depth > max_depth {
        return Err(codec_error("nesting depth limit exceeded while encoding"));
    }
    check_name(name)?;
    if let Some(unit) = layout.unit.filter(|_| separated) {
        writer.write_all(b"\n")?;
        write_units(writer, unit, depth - 1)?;
    }
    match value {
        // A variant is the value its bytes hold, written as that value.
        Scalar::Variant(held) => {
            let held = held.scalar()?;
            return write_element(writer, name, &held, layout, depth, false, max_depth);
        }
        Scalar::Null => {
            write!(writer, "<{name}/>")?;
        }
        Scalar::Struct(entries) => {
            let entries = entries.as_map();
            let pairs = entries.iter().map(|(key, value)| (key.as_str(), value));
            write_record(writer, name, pairs, layout, depth, max_depth)?;
        }
        Scalar::Map(entries) | Scalar::SortedMap(entries) => {
            let entries = entries.as_slice();
            if let Some((key, _)) = entries.iter().find(|(key, _)| key.as_str().is_none()) {
                return Err(codec_error(format_smolstr!(
                    "XML element names must be strings, got a {} key under `{name}`",
                    key.kind()
                )));
            }
            let pairs = entries
                .iter()
                .map(|(key, value)| (key.as_str().unwrap_or_default(), value));
            write_record(writer, name, pairs, layout, depth, max_depth)?;
        }
        Scalar::Serie(_)
        | Scalar::SerieView(_)
        | Scalar::FixedSizeSerie(_)
        | Scalar::LargeSerie(_)
        | Scalar::LargeSerieView(_) => {
            return Err(codec_error(format_smolstr!(
                "a sequence inside a sequence has no element to repeat under `{name}`"
            )));
        }
        leaf => {
            write!(writer, "<{name}>")?;
            write_leaf(writer, leaf, Escape::Content, name)?;
            write!(writer, "</{name}>")?;
        }
    }
    Ok(())
}

/// Write an element whose value is a record: attributes, then its own text,
/// then one child element per entry - repeated for a sequence.
///
/// A record with no entry but attributes is the empty element `<a/>`, which
/// reads as null; one whose entries are all empty sequences is `<a></a>`,
/// which reads as present and empty, the way an empty text does.
fn write_record<'a, W, I>(
    writer: &mut W,
    name: &str,
    pairs: I,
    layout: Layout,
    depth: usize,
    max_depth: usize,
) -> Result<()>
where
    W: Write,
    I: Iterator<Item = (&'a str, &'a Scalar)> + Clone,
{
    write!(writer, "<{name}")?;
    let mut text = None;
    let mut has_entries = false;
    let mut has_children = false;
    for (key, value) in pairs.clone() {
        if let Some(attribute) = key.strip_prefix(ATTRIBUTE_PREFIX) {
            write_attribute(writer, attribute, value)?;
        } else if key == TEXT_KEY {
            text = Some(value).filter(|value| !value.is_null());
            has_entries |= text.is_some();
        } else if key.starts_with('#') {
            return Err(codec_error(format_smolstr!(
                "`{TEXT_KEY}` is the only `#` key XML writes, got `{key}` under `{name}`"
            )));
        } else {
            has_entries = true;
            has_children |= !is_empty_sequence(value)?;
        }
    }
    if !has_entries {
        writer.write_all(b"/>")?;
        return Ok(());
    }
    writer.write_all(b">")?;
    if let Some(text) = text {
        write_leaf(writer, text, Escape::Content, TEXT_KEY)?;
    }
    // Indentation between children would join the element's own text, so an
    // element with `#text` keeps its children on its line.
    let separated = layout.unit.is_some() && text.is_none();
    for (key, value) in pairs {
        if key.starts_with(ATTRIBUTE_PREFIX) || key.starts_with('#') {
            continue;
        }
        write_child(writer, key, value, layout, depth + 1, separated, max_depth)?;
    }
    if let Some(unit) = layout.unit.filter(|_| has_children && separated) {
        writer.write_all(b"\n")?;
        write_units(writer, unit, depth - 1)?;
    }
    write!(writer, "</{name}>")?;
    Ok(())
}

/// Write one entry of a record as its child element or elements.
fn write_child<W: Write>(
    writer: &mut W,
    key: &str,
    value: &Scalar,
    layout: Layout,
    depth: usize,
    separated: bool,
    max_depth: usize,
) -> Result<()> {
    match value {
        Scalar::Variant(held) => {
            let held = held.scalar()?;
            write_child(writer, key, &held, layout, depth, separated, max_depth)
        }
        Scalar::Serie(values)
        | Scalar::SerieView(values)
        | Scalar::FixedSizeSerie(values)
        | Scalar::LargeSerie(values)
        | Scalar::LargeSerieView(values) => {
            // One repeated element cannot say whether it is a null item or a
            // null sequence, so a sequence of one null is refused rather than
            // written as something that reads back as the other.
            if values.len() == 1 && values.scalar(0)?.is_null() {
                return Err(codec_error(format_smolstr!(
                    "a sequence holding one null has no XML spelling that reads back, under `{key}`"
                )));
            }
            for item in values.iter() {
                if interval_components(&item).is_some() {
                    return Err(codec_error(format_smolstr!(
                        "an interval of several components inside a sequence has no XML shape under `{key}`"
                    )));
                }
                write_element(writer, key, &item, layout, depth, separated, max_depth)?;
            }
            Ok(())
        }
        // An interval of several components is what JSON spells as an
        // array: the components, one repeated element each.
        interval if interval_components(interval).is_some() => {
            for component in interval_components(interval).unwrap_or_default() {
                write_element(
                    writer,
                    key,
                    &Scalar::from(component),
                    layout,
                    depth,
                    separated,
                    max_depth,
                )?;
            }
            Ok(())
        }
        other => write_element(writer, key, other, layout, depth, separated, max_depth),
    }
}

/// Write ` name="value"` for one attribute; an absent value writes nothing.
fn write_attribute<W: Write>(writer: &mut W, name: &str, value: &Scalar) -> Result<()> {
    check_name(name)?;
    match value {
        Scalar::Variant(held) => write_attribute(writer, name, &held.scalar()?),
        Scalar::Null => Ok(()),
        Scalar::Struct(_)
        | Scalar::Map(_)
        | Scalar::SortedMap(_)
        | Scalar::Serie(_)
        | Scalar::SerieView(_)
        | Scalar::FixedSizeSerie(_)
        | Scalar::LargeSerie(_)
        | Scalar::LargeSerieView(_) => Err(codec_error(format_smolstr!(
            "attribute `{name}` must be a scalar, got {}",
            value.kind()
        ))),
        leaf => {
            write!(writer, " {name}=\"")?;
            write_leaf(writer, leaf, Escape::Attribute, name)?;
            writer.write_all(b"\"")?;
            Ok(())
        }
    }
}

/// Write one leaf as text, escaped for where it lands.
///
/// The spellings are the ones the JSON writer uses where JSON has them -
/// decimal text, ISO 8601 temporals, base64 bytes - and XML Schema's where it
/// does not: `INF`, `-INF` and `NaN` for the floats JSON cannot carry. A
/// temporal outside its ISO 8601 range is refused rather than written as its
/// count: in a document of nothing but text, digits read as a compact date.
fn write_leaf<W: Write>(
    writer: &mut W,
    value: &Scalar,
    escape: Escape,
    context: &str,
) -> Result<()> {
    match value {
        Scalar::Variant(held) => write_leaf(writer, &held.scalar()?, escape, context),
        Scalar::Null => Ok(()),
        Scalar::Boolean(value) => {
            writer.write_all(if value.get() { b"true" } else { b"false" })?;
            Ok(())
        }
        Scalar::Int8(_)
        | Scalar::Int16(_)
        | Scalar::Int32(_)
        | Scalar::Int64(_)
        | Scalar::UInt8(_)
        | Scalar::UInt16(_)
        | Scalar::UInt32(_)
        | Scalar::UInt64(_)
        | Scalar::Int128(_)
        | Scalar::UInt128(_) => {
            match (value.as_i128(), value.as_u128()) {
                (Some(signed), _) => write!(writer, "{signed}")?,
                (None, Some(unsigned)) => write!(writer, "{unsigned}")?,
                (None, None) => unreachable!("an integer reads across widths"),
            }
            Ok(())
        }
        Scalar::Float16(value) => write_float(writer, value.as_f64()),
        Scalar::Float32(value) => write_float(writer, value.as_f64()),
        Scalar::Float64(value) => write_float(writer, value.as_f64()),
        // A decimal leaf displays exactly its canonical decimal text.
        Scalar::Decimal32(value) => write_display(writer, value),
        Scalar::Decimal64(value) => write_display(writer, value),
        Scalar::Decimal128(value) => write_display(writer, value),
        Scalar::Decimal256(value) => write_display(writer, value),
        string_scalars!(value) => write_escaped(writer, value.as_str(), escape),
        code_scalars!() => write_escaped(
            writer,
            value.as_str().expect("a code borrowed its text"),
            escape,
        ),
        Scalar::Version(value) => write_escaped(writer, &value.to_string(), escape),
        Scalar::Url(value) => write_escaped(writer, &value.to_string(), escape),
        Scalar::Urn(value) => write_escaped(writer, &value.to_string(), escape),
        Scalar::Timezone(value) => write_escaped(writer, value.as_str(), escape),
        Scalar::MimeType(value) => write_escaped(writer, value.as_str(), escape),
        Scalar::MediaType(value) => write_escaped(writer, &value.to_string(), escape),
        Scalar::Uuid(value) => {
            let mut slot = [0_u8; crate::Uuid::TEXT_LEN];
            writer.write_all(value.render(&mut slot).as_bytes())?;
            Ok(())
        }
        bytes_scalars!(value) => write_base64(writer, value.as_bytes()),
        Scalar::Geometry(value) => write_base64(writer, value.as_bytes()),
        Scalar::Geography(value) => write_base64(writer, value.as_bytes()),
        Scalar::Date32(value) => write_spelled(
            writer,
            crate::temporal::format_date(value.count()),
            "date32",
            i64::from(value.count()),
            context,
        ),
        Scalar::Date64(value) => {
            const DAY_MILLISECONDS: i64 = 86_400_000;
            let count = value.count();
            let text = (count.rem_euclid(DAY_MILLISECONDS) == 0)
                .then(|| i32::try_from(count.div_euclid(DAY_MILLISECONDS)).ok())
                .flatten()
                .and_then(crate::temporal::format_date);
            write_spelled(writer, text, "date64", count, context)
        }
        Scalar::Time32(value) => write_time(
            writer,
            i64::from(value.count()),
            value.unit(),
            value.timezone().is_naive(),
            context,
        ),
        Scalar::Time64(value) => write_time(
            writer,
            value.count(),
            value.unit(),
            value.timezone().is_naive(),
            context,
        ),
        Scalar::DateTime64(value) => {
            let text = if value.timezone().is_naive() {
                crate::temporal::format_datetime(value.count(), value.unit())
            } else {
                crate::temporal::format_timestamp(value.count(), value.unit(), &value.timezone())
            };
            write_spelled(writer, text, "datetime64", value.count(), context)
        }
        Scalar::Duration32(value) => write_duration(
            writer,
            i64::from(value.count()),
            value.unit(),
            value.timezone().is_naive(),
            context,
        ),
        Scalar::Duration64(value) => write_duration(
            writer,
            value.count(),
            value.unit(),
            value.timezone().is_naive(),
            context,
        ),
        Scalar::Interval(value) => {
            if value.unit() == TimeUnit::YearMonth {
                write!(writer, "{}", value.months())?;
                return Ok(());
            }
            Err(codec_error(format_smolstr!(
                "an interval of several components has no XML spelling as one value, for `{context}`"
            )))
        }
        Scalar::Struct(_)
        | Scalar::Map(_)
        | Scalar::SortedMap(_)
        | Scalar::Serie(_)
        | Scalar::SerieView(_)
        | Scalar::FixedSizeSerie(_)
        | Scalar::LargeSerie(_)
        | Scalar::LargeSerieView(_) => Err(codec_error(format_smolstr!(
            "expected a scalar for `{context}`, got {}",
            value.kind()
        ))),
    }
}

fn write_display<W: Write>(writer: &mut W, value: impl std::fmt::Display) -> Result<()> {
    write!(writer, "{value}")?;
    Ok(())
}

fn write_base64<W: Write>(writer: &mut W, bytes: &[u8]) -> Result<()> {
    writer.write_all(
        base64::engine::general_purpose::STANDARD
            .encode(bytes)
            .as_bytes(),
    )?;
    Ok(())
}

fn write_float<W: Write>(writer: &mut W, value: f64) -> Result<()> {
    if value.is_nan() {
        writer.write_all(b"NaN")?;
    } else if value == f64::INFINITY {
        writer.write_all(b"INF")?;
    } else if value == f64::NEG_INFINITY {
        writer.write_all(b"-INF")?;
    } else {
        let spelling = serde_json::Number::from_f64(value)
            .ok_or_else(|| codec_error("float has no XML spelling"))?
            .to_string();
        writer.write_all(spelling.as_bytes())?;
    }
    Ok(())
}

/// Write a temporal's ISO 8601 spelling, or refuse the count that has none.
fn write_spelled<W: Write>(
    writer: &mut W,
    text: Option<SmolStr>,
    kind: &str,
    count: i64,
    context: &str,
) -> Result<()> {
    match text {
        Some(text) => {
            writer.write_all(text.as_bytes())?;
            Ok(())
        }
        None => Err(codec_error(format_smolstr!(
            "the {kind} count {count} under `{context}` has no ISO 8601 spelling"
        ))),
    }
}

fn write_time<W: Write>(
    writer: &mut W,
    count: i64,
    unit: TimeUnit,
    naive: bool,
    context: &str,
) -> Result<()> {
    if !naive {
        return Err(codec_error(
            "time-of-day cannot carry a timezone; use DateTime64 for a zoned instant",
        ));
    }
    write_spelled(
        writer,
        crate::temporal::format_time(count, unit),
        "time",
        count,
        context,
    )
}

fn write_duration<W: Write>(
    writer: &mut W,
    count: i64,
    unit: TimeUnit,
    naive: bool,
    context: &str,
) -> Result<()> {
    if !naive {
        return Err(codec_error("duration cannot carry a timezone"));
    }
    write_spelled(
        writer,
        crate::temporal::format_duration(count, unit),
        "duration",
        count,
        context,
    )
}

/// Write text with what XML 1.0 cannot carry bare replaced by a reference,
/// refusing the characters it cannot carry at all.
fn write_escaped<W: Write>(writer: &mut W, text: &str, escape: Escape) -> Result<()> {
    if let Some(character) = illegal_character(text) {
        return Err(codec_error(format_smolstr!(
            "text carries U+{character:04X}, which XML 1.0 cannot spell"
        )));
    }
    let bytes = text.as_bytes();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        let replacement: &[u8] = match byte {
            b'<' => b"&lt;",
            b'>' => b"&gt;",
            b'&' => b"&amp;",
            b'\r' => b"&#13;",
            b'"' if escape == Escape::Attribute => b"&quot;",
            b'\n' if escape == Escape::Attribute => b"&#10;",
            b'\t' if escape == Escape::Attribute => b"&#9;",
            _ => continue,
        };
        writer.write_all(&bytes[start..index])?;
        writer.write_all(replacement)?;
        start = index + 1;
    }
    writer.write_all(&bytes[start..])?;
    Ok(())
}

/// The first character of `text` that XML 1.0 cannot carry in character data
/// or an attribute value, as its code point: a control other than tab, line
/// feed and carriage return, or the two noncharacters below U+10000.
pub(super) fn illegal_character(text: &str) -> Option<u32> {
    let bytes = text.as_bytes();
    if let Some(byte) = bytes
        .iter()
        .find(|byte| matches!(byte, 0x00..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F))
    {
        return Some(u32::from(*byte));
    }
    // The two non-characters are three bytes each and both open with 0xEF,
    // so a text with no such byte is not walked.
    memchr::memchr(0xEF, bytes)?;
    text.chars()
        .find(|c| matches!(c, '\u{FFFE}' | '\u{FFFF}'))
        .map(u32::from)
}

fn write_units<W: Write>(writer: &mut W, unit: &[u8], count: usize) -> Result<()> {
    for _ in 0..count {
        writer.write_all(unit)?;
    }
    Ok(())
}

/// Refuse a key that is not an XML 1.0 `Name`.
pub(super) fn check_name(name: &str) -> Result<()> {
    if is_name(name) {
        Ok(())
    } else {
        Err(codec_error(format_smolstr!(
            "expected an XML name, got {:?}",
            crate::text::elide_to(name, crate::text::ERROR_TEXT_LIMIT)
        )))
    }
}

/// Whether `name` is an XML 1.0 `Name`.
pub(super) fn is_name(name: &str) -> bool {
    let mut characters = name.chars();
    matches!(characters.next(), Some(first) if is_name_start(first)) && characters.all(is_name_char)
}

/// `NameStartChar` of XML 1.0, fifth edition.
const fn is_name_start(character: char) -> bool {
    matches!(
        character,
        ':' | 'A'..='Z'
            | '_'
            | 'a'..='z'
            | '\u{C0}'..='\u{D6}'
            | '\u{D8}'..='\u{F6}'
            | '\u{F8}'..='\u{2FF}'
            | '\u{370}'..='\u{37D}'
            | '\u{37F}'..='\u{1FFF}'
            | '\u{200C}'..='\u{200D}'
            | '\u{2070}'..='\u{218F}'
            | '\u{2C00}'..='\u{2FEF}'
            | '\u{3001}'..='\u{D7FF}'
            | '\u{F900}'..='\u{FDCF}'
            | '\u{FDF0}'..='\u{FFFD}'
            | '\u{10000}'..='\u{EFFFF}'
    )
}

/// `NameChar` of XML 1.0, fifth edition.
const fn is_name_char(character: char) -> bool {
    is_name_start(character)
        || matches!(
            character,
            '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}'
        )
}

const fn is_sequence_dtype(dtype: &DataType) -> bool {
    matches!(
        dtype,
        DataType::Serie(_)
            | DataType::SerieView(_)
            | DataType::FixedSizeSerie(_, _)
            | DataType::LargeSerie(_)
            | DataType::LargeSerieView(_)
    )
}

/// The item field of a sequence datatype, or `None` for any other.
fn sequence_item(dtype: &DataType) -> Option<&Field> {
    match dtype {
        DataType::Serie(item)
        | DataType::SerieView(item)
        | DataType::FixedSizeSerie(item, _)
        | DataType::LargeSerie(item)
        | DataType::LargeSerieView(item) => Some(item),
        _ => None,
    }
}

const fn is_sequence(value: &Scalar) -> bool {
    matches!(
        value,
        Scalar::Serie(_)
            | Scalar::SerieView(_)
            | Scalar::FixedSizeSerie(_)
            | Scalar::LargeSerie(_)
            | Scalar::LargeSerieView(_)
    )
}

/// Whether a value - or the value a variant holds - is a sequence of nothing.
fn is_empty_sequence(value: &Scalar) -> Result<bool> {
    match value {
        Scalar::Variant(held) => is_empty_sequence(&held.scalar()?),
        other => Ok(other.as_serie().is_some_and(Serie::is_empty)),
    }
}

/// The components of an interval that is not one number: `[days,
/// milliseconds]` or `[months, days, nanoseconds]`, as the JSON writer spells
/// them.
fn interval_components(value: &Scalar) -> Option<Vec<i64>> {
    let Scalar::Interval(interval) = value else {
        return None;
    };
    match interval.unit() {
        TimeUnit::DayTime => Some(vec![
            i64::from(interval.days()),
            interval.nanoseconds() / 1_000_000,
        ]),
        TimeUnit::MonthDayNano => Some(vec![
            i64::from(interval.months()),
            i64::from(interval.days()),
            interval.nanoseconds(),
        ]),
        _ => None,
    }
}

fn codec_error(reason: impl Into<SmolStr>) -> Error {
    Error::Codec {
        format: "xml",
        position: 0,
        reason: reason.into(),
    }
}

/// Where a value sits in the document, which decides what a sequence field
/// reads it as.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Position {
    /// An element's own content: the document element, or one item of a
    /// sequence. A sequence here is the repeated children the element holds,
    /// named after the item field, and an empty element is the empty
    /// sequence.
    Container,
    /// The entry a record holds under a name: the repeated elements of that
    /// name, one of which is one item.
    Child,
}

/// Restate a document's natural value in the shape `field`'s value contract
/// reads.
///
/// XML states less than a field does, and the gaps are the document's, not
/// the reader's: a child element read once is one item of a column declared
/// as a sequence; a repeated element occurring no time at all is the empty
/// sequence; a sequence inside a sequence is an element holding the inner
/// items as children named after the inner item field, and an empty such
/// element is the empty inner sequence; `<a></a>` - present and empty - is
/// the empty text, the empty sequence or the empty record its field says,
/// where `<a/>` is null; text is trimmed, and empty text null, under any leaf
/// that is not text, bytes or a code, as XML Schema's `collapse` facet reads
/// every type but `string`; a union's type id and an interval's components
/// are the numbers their digits spell. Nothing here types a value: the
/// field's contract does that next.
pub(crate) fn shaped(value: Scalar, field: &Field) -> Scalar {
    shaped_at(value, field, Position::Container)
}

fn shaped_at(value: Scalar, field: &Field, position: Position) -> Scalar {
    match field.dtype() {
        DataType::Serie(item)
        | DataType::SerieView(item)
        | DataType::FixedSizeSerie(item, _)
        | DataType::LargeSerie(item)
        | DataType::LargeSerieView(item) => shaped_sequence(value, item, position),
        DataType::Struct(fields) => match value {
            Scalar::Struct(entries) => {
                let entries = entries.as_map();
                let mut shaped_entries: Vec<(SmolStr, Scalar)> = entries
                    .iter()
                    .map(|(name, value)| {
                        let value = match fields.get_by_name(name) {
                            Some(child) => shaped_at(value.clone(), child, Position::Child),
                            None => value.clone(),
                        };
                        (name.clone(), value)
                    })
                    .collect();
                fill_absent_sequences(&mut shaped_entries, fields, |name| {
                    entries.contains_key(name)
                });
                record(shaped_entries)
            }
            // `<a></a>`: a record present and empty, every sequence child
            // occurring no time.
            string_scalars!(text) if text.as_str().is_empty() => {
                let mut shaped_entries = Vec::new();
                fill_absent_sequences(&mut shaped_entries, fields, |_| false);
                record(shaped_entries)
            }
            other => other,
        },
        map_dtype @ (DataType::Map(_) | DataType::SortedMap(_)) => {
            let Some(map) = map_dtype.as_mapping() else {
                return value;
            };
            let [_, value_field] = map.entries().fields() else {
                return value;
            };
            match value {
                Scalar::Struct(entries) => record(
                    entries
                        .as_map()
                        .iter()
                        .map(|(name, value)| {
                            (
                                name.clone(),
                                shaped_at(value.clone(), value_field, Position::Child),
                            )
                        })
                        .collect(),
                ),
                Scalar::Map(entries) | Scalar::SortedMap(entries) => {
                    Scalar::from_mapping(entries.as_slice().iter().map(|(key, value)| {
                        (
                            key.clone(),
                            shaped_at(value.clone(), value_field, Position::Child),
                        )
                    }))
                    .unwrap_or(Scalar::Null)
                }
                string_scalars!(text) if text.as_str().is_empty() => record(Vec::new()),
                other => other,
            }
        }
        DataType::Union(fields, _) => {
            let Some(pair) = value.sequence_rows() else {
                return value;
            };
            let [type_id, payload] = &*pair else {
                return value;
            };
            // The id crossed as digits, like every number in a document.
            let id = digits(type_id).and_then(|id| i8::try_from(id).ok());
            let branch = id.and_then(|id| {
                fields
                    .iter()
                    .find_map(|(candidate, branch)| (candidate == id).then_some(branch))
            });
            match (id, branch) {
                (Some(id), Some(branch)) => Scalar::from_sequence([
                    Scalar::from(id),
                    shaped_at(payload.clone(), branch, Position::Container),
                ]),
                _ => value,
            }
        }
        DataType::Dictionary(dictionary) => shaped_at(
            value,
            &Field::new(
                field.name(),
                dictionary.value().clone(),
                field.is_nullable(),
            ),
            position,
        ),
        DataType::RunEndEncoded(encoded) => shaped_at(value, encoded.values(), position),
        leaf => shaped_leaf(value, leaf),
    }
}

/// Add the empty sequence for every sequence child `present` does not hold:
/// a repeated element occurring zero times is the only way XML spells one.
fn fill_absent_sequences(
    entries: &mut Vec<(SmolStr, Scalar)>,
    fields: &crate::StructType,
    present: impl Fn(&str) -> bool,
) {
    for child in fields.iter() {
        if is_sequence_dtype(child.dtype()) && !present(child.name()) {
            entries.push((SmolStr::new(child.name()), Scalar::from_sequence([])));
        }
    }
}

/// The items a sequence field reads at `position`.
fn shaped_sequence(value: Scalar, item: &Field, position: Position) -> Scalar {
    match (position, value) {
        (_, Scalar::Null) => Scalar::Null,
        // The repeated elements of a record entry: each is one item.
        (
            Position::Child,
            Scalar::Serie(values)
            | Scalar::SerieView(values)
            | Scalar::FixedSizeSerie(values)
            | Scalar::LargeSerie(values)
            | Scalar::LargeSerieView(values),
        ) => Scalar::from_sequence(
            values
                .iter()
                .map(|value| shaped_at(value.into_owned(), item, Position::Container))
                .collect::<Vec<_>>(),
        ),
        (Position::Child, other) => {
            Scalar::from_sequence([shaped_at(other, item, Position::Container)])
        }
        // An element holding the items as children named after the item
        // field, `<matrix><item>1</item><item>2</item></matrix>`.
        (Position::Container, Scalar::Struct(entries))
            if entries.as_map().len() == 1 && entries.as_map().contains_key(item.name()) =>
        {
            let inner = entries.as_map()[item.name()].clone();
            shaped_sequence(inner, item, Position::Child)
        }
        // `<matrix></matrix>`: present, holding nothing.
        (Position::Container, string_scalars!(text)) if text.as_str().is_empty() => {
            Scalar::from_sequence([])
        }
        (Position::Container, other) => {
            Scalar::from_sequence([shaped_at(other, item, Position::Container)])
        }
    }
}

/// Trim the text a non-text leaf reads, empty text being absence; the
/// components of an interval, which no value contract reads as text, are the
/// numbers their digits spell.
fn shaped_leaf(value: Scalar, dtype: &DataType) -> Scalar {
    if matches!(
        dtype,
        string_dtypes!()
            | bytes_dtypes!()
            | DataType::Geometry(_)
            | DataType::Geography(_)
            | DataType::Variant
    ) || dtype.is_code()
    {
        return value;
    }
    match value {
        string_scalars!(text) => {
            let trimmed = text.as_str().trim_matches([' ', '\t', '\r', '\n']);
            if trimmed.is_empty() {
                Scalar::Null
            } else if let (DataType::Interval(_), Ok(count)) = (dtype, trimmed.parse::<i64>()) {
                Scalar::from(count)
            } else if trimmed.len() == text.as_str().len() {
                Scalar::from(text.as_str())
            } else {
                Scalar::from(trimmed)
            }
        }
        // The components of an interval, each one number.
        Scalar::Serie(values)
        | Scalar::SerieView(values)
        | Scalar::FixedSizeSerie(values)
        | Scalar::LargeSerie(values)
        | Scalar::LargeSerieView(values) => Scalar::from_sequence(
            values
                .iter()
                .map(|value| shaped_leaf(value.into_owned(), dtype))
                .collect::<Vec<_>>(),
        ),
        other => other,
    }
}

/// A number a document spells: an integer, or the digits of one.
fn digits(value: &Scalar) -> Option<i128> {
    value
        .as_i128()
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
}

/// Restate a row's natural value in the shape XML can write.
///
/// The one gap: a sequence inside a sequence has no element to repeat, so
/// each inner sequence becomes a record holding its items under the inner
/// item field's name - the element `<matrix><item>1</item><item>2</item></matrix>`,
/// which is what [`shaped`] reads back.
pub(crate) fn natural(value: Scalar, field: &Field) -> Scalar {
    match field.dtype() {
        DataType::Serie(item)
        | DataType::SerieView(item)
        | DataType::FixedSizeSerie(item, _)
        | DataType::LargeSerie(item)
        | DataType::LargeSerieView(item) => match value {
            Scalar::Serie(values)
            | Scalar::SerieView(values)
            | Scalar::FixedSizeSerie(values)
            | Scalar::LargeSerie(values)
            | Scalar::LargeSerieView(values) => Scalar::from_sequence(
                values
                    .iter()
                    .map(|value| {
                        let value = natural(value.into_owned(), item);
                        match sequence_item(item.dtype()) {
                            Some(inner) if is_sequence(&value) => {
                                record(vec![(SmolStr::new(inner.name()), value)])
                            }
                            _ => value,
                        }
                    })
                    .collect::<Vec<_>>(),
            ),
            other => other,
        },
        DataType::Struct(fields) => match value {
            Scalar::Struct(entries) => record(
                entries
                    .as_map()
                    .iter()
                    .map(|(name, value)| {
                        let value = match fields.get_by_name(name) {
                            Some(child) => natural(value.clone(), child),
                            None => value.clone(),
                        };
                        (name.clone(), value)
                    })
                    .collect(),
            ),
            other => other,
        },
        map_dtype @ (DataType::Map(_) | DataType::SortedMap(_)) => {
            let Some(map) = map_dtype.as_mapping() else {
                return value;
            };
            let [_, value_field] = map.entries().fields() else {
                return value;
            };
            match value {
                Scalar::Map(entries) | Scalar::SortedMap(entries) => Scalar::from_mapping(
                    entries
                        .as_slice()
                        .iter()
                        .map(|(key, value)| (key.clone(), natural(value.clone(), value_field))),
                )
                .unwrap_or(Scalar::Null),
                Scalar::Struct(entries) => record(
                    entries
                        .as_map()
                        .iter()
                        .map(|(name, value)| (name.clone(), natural(value.clone(), value_field)))
                        .collect(),
                ),
                other => other,
            }
        }
        DataType::Union(fields, _) => {
            let Some(pair) = value.sequence_rows() else {
                return value;
            };
            let [type_id, payload] = &*pair else {
                return value;
            };
            let branch = type_id
                .as_i128()
                .and_then(|id| i8::try_from(id).ok())
                .and_then(|id| {
                    fields
                        .iter()
                        .find_map(|(candidate, branch)| (candidate == id).then_some(branch))
                });
            match branch {
                Some(branch) => {
                    Scalar::from_sequence([type_id.clone(), natural(payload.clone(), branch)])
                }
                None => value,
            }
        }
        DataType::Dictionary(dictionary) => natural(
            value,
            &Field::new(
                field.name(),
                dictionary.value().clone(),
                field.is_nullable(),
            ),
        ),
        DataType::RunEndEncoded(encoded) => natural(value, encoded.values()),
        _ => value,
    }
}

/// A record rebuilt from entries that came out of one, so no name repeats.
fn record(entries: Vec<(SmolStr, Scalar)>) -> Scalar {
    Scalar::from_struct(entries).expect("the entries came out of one record")
}
