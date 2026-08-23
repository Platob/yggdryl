//! Natural YAML values, comments, and document streams.

use std::io::{Read, Write};

use base64::Engine as _;

mod parser;

use crate::text::wire::{RawValue, from_raw};
use crate::text::{
    Formatting, Limits, Value, ValueIter, apply_field, check_encode_depth, check_input_size,
};
use crate::{Error, Field, Result};

use self::parser::YamlParser;

/// Maximum nesting accepted by Saphyr's YAML flow-collection grammar.
///
/// Block collections may use a larger explicit limit up to
/// [`MAX_PARSER_DEPTH`], but `[`/`{` flow syntax is bounded by Saphyr first.
pub const MAX_FLOW_DEPTH: usize = 255;

/// Maximum structural nesting accepted by YAML parsing and value conversion.
///
/// Caller limits may choose any smaller depth. This implementation ceiling
/// keeps deeply nested block syntax from exhausting the conversion stack.
pub const MAX_PARSER_DEPTH: usize = 384;

/// Decode exactly one YAML document from borrowed UTF-8 text.
///
/// This delegates through the string's borrowed bytes without an intermediate
/// UTF-8/byte input buffer. The returned owned value still allocates or shares
/// storage for strings and collections.
pub fn from_utf8(input: &str) -> Result<Value> {
    from_utf8_with_limits(input, Limits::default())
}

/// Decode exactly one YAML document from borrowed UTF-8 text with explicit limits.
pub fn from_utf8_with_limits(input: &str, limits: Limits) -> Result<Value> {
    from_bytes_with_limits(input.as_bytes(), limits)
}

/// Decode exactly one YAML document from bytes.
pub fn from_bytes(input: &[u8]) -> Result<Value> {
    from_bytes_with_limits(input, Limits::default())
}

/// Decode exactly one YAML document from bytes with explicit limits.
pub fn from_bytes_with_limits(input: &[u8], limits: Limits) -> Result<Value> {
    check_input_size(input, limits, "yaml")?;
    from_reader_with_limits(std::io::Cursor::new(input), limits)
}

/// Decode YAML UTF-8 under `field`.
pub fn from_utf8_with_field(input: &str, field: &Field) -> Result<Value> {
    from_utf8_with_field_and_limits(input, field, Limits::default())
}

/// Decode schema-directed YAML UTF-8 with limits.
pub fn from_utf8_with_field_and_limits(
    input: &str,
    field: &Field,
    limits: Limits,
) -> Result<Value> {
    from_bytes_with_field_and_limits(input.as_bytes(), field, limits)
}

/// Decode YAML bytes under `field`.
pub fn from_bytes_with_field(input: &[u8], field: &Field) -> Result<Value> {
    from_bytes_with_field_and_limits(input, field, Limits::default())
}

/// Decode schema-directed YAML bytes with limits.
pub fn from_bytes_with_field_and_limits(
    input: &[u8],
    field: &Field,
    limits: Limits,
) -> Result<Value> {
    field.from_natural_value(from_bytes_with_limits(input, limits)?)
}

/// Decode exactly one YAML document from a streaming reader.
pub fn from_reader<R: Read>(reader: R) -> Result<Value> {
    from_reader_with_limits(reader, Limits::default())
}

/// Decode exactly one YAML document from a streaming reader with explicit limits.
pub fn from_reader_with_limits<R: Read>(reader: R, limits: Limits) -> Result<Value> {
    let mut reader = Reader::with_limits(reader, limits);
    let value = reader.next().transpose()?.ok_or_else(|| Error::Codec {
        format: "yaml",
        position: 0,
        reason: "expected one YAML document".into(),
    })?;
    if let Some(trailing) = reader.next() {
        trailing?;
        return Err(Error::Codec {
            format: "yaml",
            position: reader.document_start(),
            reason: "expected one YAML document but found trailing data".into(),
        });
    }
    Ok(value)
}

/// Decode YAML from a reader under `field`.
pub fn from_reader_with_field<R: Read>(reader: R, field: &Field) -> Result<Value> {
    from_reader_with_field_and_limits(reader, field, Limits::default())
}

/// Decode schema-directed YAML from a reader with limits.
pub fn from_reader_with_field_and_limits<R: Read>(
    reader: R,
    field: &Field,
    limits: Limits,
) -> Result<Value> {
    field.from_natural_value(from_reader_with_limits(reader, limits)?)
}

/// Decode every YAML document from bytes.
pub fn from_bytes_all(input: &[u8]) -> Result<Vec<Value>> {
    from_bytes_all_with_limits(input, Limits::default())
}

/// Decode every YAML document from borrowed UTF-8 text.
pub fn from_utf8_all(input: &str) -> Result<Vec<Value>> {
    from_utf8_all_with_limits(input, Limits::default())
}

/// Decode every YAML document from borrowed UTF-8 text with explicit limits.
pub fn from_utf8_all_with_limits(input: &str, limits: Limits) -> Result<Vec<Value>> {
    from_bytes_all_with_limits(input.as_bytes(), limits)
}

/// Decode every YAML document from bytes with explicit limits.
pub fn from_bytes_all_with_limits(input: &[u8], limits: Limits) -> Result<Vec<Value>> {
    check_input_size(input, limits, "yaml")?;
    Reader::with_limits(std::io::Cursor::new(input), limits).collect()
}

/// Decode every YAML document from UTF-8 under `field`.
pub fn from_utf8_all_with_field(input: &str, field: &Field) -> Result<Vec<Value>> {
    from_utf8_all_with_field_and_limits(input, field, Limits::default())
}

/// Decode every schema-directed YAML document from UTF-8 with limits.
pub fn from_utf8_all_with_field_and_limits(
    input: &str,
    field: &Field,
    limits: Limits,
) -> Result<Vec<Value>> {
    from_bytes_all_with_field_and_limits(input.as_bytes(), field, limits)
}

/// Decode every YAML document from bytes under `field`.
pub fn from_bytes_all_with_field(input: &[u8], field: &Field) -> Result<Vec<Value>> {
    from_bytes_all_with_field_and_limits(input, field, Limits::default())
}

/// Decode every schema-directed YAML document from bytes with limits.
pub fn from_bytes_all_with_field_and_limits(
    input: &[u8],
    field: &Field,
    limits: Limits,
) -> Result<Vec<Value>> {
    apply_field(from_bytes_all_with_limits(input, limits)?, field)
}

/// Decode every YAML document from a reader.
pub fn from_reader_all<R: Read>(reader: R) -> Result<Vec<Value>> {
    Reader::new(reader).collect()
}

/// Decode every YAML document from a reader with explicit limits.
pub fn from_reader_all_with_limits<R: Read>(reader: R, limits: Limits) -> Result<Vec<Value>> {
    Reader::with_limits(reader, limits).collect()
}

/// Decode every YAML document from a reader under `field`.
pub fn from_reader_all_with_field<R: Read>(reader: R, field: &Field) -> Result<Vec<Value>> {
    from_reader_all_with_field_and_limits(reader, field, Limits::default())
}

/// Decode every schema-directed YAML document from a reader with limits.
pub fn from_reader_all_with_field_and_limits<R: Read>(
    reader: R,
    field: &Field,
    limits: Limits,
) -> Result<Vec<Value>> {
    apply_field(from_reader_all_with_limits(reader, limits)?, field)
}

/// Lazily decode YAML documents from a borrowed reader.
pub fn from_reader_iter<'a, R: Read + 'a>(reader: &'a mut R) -> ValueIter<'a> {
    from_reader_iter_with_limits(reader, Limits::default())
}

/// Lazily decode YAML documents from a borrowed reader with explicit limits.
pub fn from_reader_iter_with_limits<'a, R: Read + 'a>(
    reader: &'a mut R,
    limits: Limits,
) -> ValueIter<'a> {
    ValueIter::new(Reader::with_limits(reader, limits))
}

/// Lazily decode YAML documents from a reader under `field`.
pub fn from_reader_iter_with_field<'a, R: Read + 'a>(
    reader: &'a mut R,
    field: &'a Field,
) -> ValueIter<'a> {
    from_reader_iter_with_field_and_limits(reader, field, Limits::default())
}

/// Lazily decode schema-directed YAML documents with limits.
pub fn from_reader_iter_with_field_and_limits<'a, R: Read + 'a>(
    reader: &'a mut R,
    field: &'a Field,
    limits: Limits,
) -> ValueIter<'a> {
    ValueIter::new(Reader::with_limits(reader, limits)).with_field(field)
}

/// An owning, lazy iterator over YAML documents.
pub struct Reader<'a> {
    inner: YamlParser<'a>,
    limits: Limits,
    failed: bool,
}

impl<'a> Reader<'a> {
    /// Construct a document reader with default resource limits.
    pub fn new<R: Read + 'a>(reader: R) -> Self {
        Self::with_limits(reader, Limits::default())
    }

    /// Construct a document reader with explicit resource limits.
    pub fn with_limits<R: Read + 'a>(reader: R, limits: Limits) -> Self {
        Self {
            inner: YamlParser::new(reader, limits),
            limits,
            failed: false,
        }
    }

    /// Return the number of source bytes pulled from the reader.
    pub fn byte_offset(&self) -> usize {
        self.inner.byte_offset()
    }

    fn document_start(&self) -> usize {
        self.inner.document_start()
    }
}

impl Iterator for Reader<'_> {
    type Item = Result<Value>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        let value = self.inner.next()?;
        let value = value.and_then(|raw| from_raw(raw, self.limits, "yaml"));
        if value.is_err() {
            self.failed = true;
        }
        Some(value)
    }
}

/// Encode one value to YAML bytes.
pub fn into_bytes(value: &Value) -> Result<Vec<u8>> {
    into_bytes_with_formatting(value, Formatting::default())
}

/// Encode one value to YAML bytes laid out as `formatting` asks.
///
/// [`Indent::Default`](crate::text::Indent::Default) is today's output: block
/// style, two spaces per level. [`Indent::Spaces`](crate::text::Indent::Spaces)
/// keeps block style at that width. [`Indent::None`](crate::text::Indent::None)
/// means **flow style** - `{a: 1, b: 2}` on one line - which is valid YAML and
/// round-trips, and is an explicitly requested opt-in: a schema dump's block
/// style is the default precisely so nobody has to ask for it.
///
/// ```
/// use yggdryl::generic::Value;
/// use yggdryl::text::Formatting;
///
/// # fn main() -> yggdryl::Result<()> {
/// let value = Value::from_record([("id", Value::I64(1))])?;
/// assert_eq!(yggdryl::yaml::into_bytes(&value)?, b"id: 1\n");
///
/// let flow = yggdryl::yaml::into_bytes_with_formatting(&value, Formatting::compact())?;
/// assert_eq!(flow, b"{id: 1}\n");
/// // Formatting changes bytes, never meaning.
/// assert_eq!(yggdryl::yaml::from_utf8("{id: 1}")?, value);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns the encoder's failure, including the published depth cap.
pub fn into_bytes_with_formatting(value: &Value, formatting: Formatting) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    into_writer_with_formatting(value, &mut output, formatting)?;
    Ok(output)
}

/// Encode one value to YAML UTF-8.
pub fn into_utf8(value: &Value) -> Result<String> {
    into_utf8_with_formatting(value, Formatting::default())
}

/// Encode one value to YAML UTF-8 with explicit formatting.
pub fn into_utf8_with_formatting(value: &Value, formatting: Formatting) -> Result<String> {
    String::from_utf8(into_bytes_with_formatting(value, formatting)?).map_err(|error| {
        Error::Codec {
            format: "yaml",
            position: error.utf8_error().valid_up_to(),
            reason: "encoded YAML is not valid UTF-8".into(),
        }
    })
}

/// Encode one value to a byte writer.
pub fn into_writer<W: Write>(value: &Value, writer: W) -> Result<()> {
    into_writer_with_formatting(value, writer, Formatting::default())
}

/// Encode one value to a byte writer, laid out as `formatting` asks.
///
/// # Errors
///
/// Returns the encoder's or the sink's failure.
pub fn into_writer_with_formatting<W: Write>(
    value: &Value,
    mut writer: W,
    formatting: Formatting,
) -> Result<()> {
    check_encode_depth(value, "yaml")?;
    // `write_value` terminates the document, so nothing is added here.
    write_value(&mut writer, value, Layout::from(formatting))
}

/// Encode YAML documents to a byte vector.
pub fn into_bytes_all(values: &[Value]) -> Result<Vec<u8>> {
    into_bytes_all_with_formatting(values, Formatting::default())
}

/// Encode YAML documents to a byte vector, laid out as `formatting` asks.
///
/// # Errors
///
/// Returns the encoder's failure, including the published depth cap.
pub fn into_bytes_all_with_formatting(values: &[Value], formatting: Formatting) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    into_writer_all_with_formatting(values, &mut output, formatting)?;
    Ok(output)
}

/// Encode YAML documents to UTF-8.
pub fn into_utf8_all(values: &[Value]) -> Result<String> {
    into_utf8_all_with_formatting(values, Formatting::default())
}

/// Encode YAML documents to UTF-8 with explicit formatting.
pub fn into_utf8_all_with_formatting(values: &[Value], formatting: Formatting) -> Result<String> {
    String::from_utf8(into_bytes_all_with_formatting(values, formatting)?).map_err(|error| {
        Error::Codec {
            format: "yaml",
            position: error.utf8_error().valid_up_to(),
            reason: "encoded YAML is not valid UTF-8".into(),
        }
    })
}

/// Encode multiple values as a YAML document stream.
pub fn into_writer_all<W, I, V>(values: I, writer: W) -> Result<()>
where
    W: Write,
    I: IntoIterator<Item = V>,
    V: std::borrow::Borrow<Value>,
{
    into_writer_all_with_formatting(values, writer, Formatting::default())
}

/// Encode a YAML document stream, laid out as `formatting` asks.
///
/// # Errors
///
/// Returns the encoder's or the sink's failure.
pub fn into_writer_all_with_formatting<W, I, V>(
    values: I,
    mut writer: W,
    formatting: Formatting,
) -> Result<()>
where
    W: Write,
    I: IntoIterator<Item = V>,
    V: std::borrow::Borrow<Value>,
{
    let layout = Layout::from(formatting);
    for (index, value) in values.into_iter().enumerate() {
        if index != 0 {
            writer.write_all(b"---\n")?;
        }
        let value = value.borrow();
        check_encode_depth(value, "yaml")?;
        write_value(&mut writer, value, layout)?;
    }
    Ok(())
}

/// The resolved layout one dump runs under.
///
/// `Formatting` is the caller's request; this is what the writer actually
/// applies, with YAML's own default already substituted for
/// `Indent::Default` - so the writing code never re-decides the default.
#[derive(Clone, Copy)]
struct Layout {
    /// How many columns one nesting level costs, or `None` for flow style.
    ///
    /// Columns rather than a byte string, because YAML's indentation is
    /// positional: a mapping nested under a key indents by this width, while
    /// one nested under a `- ` indents by the *dash marker's* two columns so
    /// its keys line up with the first one. A level count cannot express both.
    width: Option<usize>,
}

/// YAML's own default: block style, two columns per level.
const DEFAULT_WIDTH: usize = 2;

/// The width the `- ` marker itself occupies, which continuation lines match.
const DASH_WIDTH: usize = 2;

impl From<Formatting> for Layout {
    fn from(value: Formatting) -> Self {
        let indent = value.indent();
        if indent.is_none() {
            // Flow style is the only thing "no indent" can mean in YAML, and
            // it is reached only by asking for it explicitly.
            return Self { width: None };
        }
        Self {
            width: Some(match indent.unit() {
                // YAML forbids a tab as indentation outright, so a tab request
                // falls back to the default width rather than emitting a
                // document that will not parse. A zero width would too.
                Some(b"\t") | Some(b"") | None => DEFAULT_WIDTH,
                Some(unit) => unit.len(),
            }),
        }
    }
}

/// Write one document, block or flow as the layout asks.
fn write_value<W: Write>(writer: &mut W, value: &Value, layout: Layout) -> Result<()> {
    match layout.width {
        Some(width) => write_node(writer, value, 0, Position::Root, width)?,
        // Flow style, spelled the way the block writer spells its scalars so
        // the two differ only in layout: `{id: 1}` rather than `{? "id" : 1}`.
        None => write_flow(writer, value)?,
    }
    writer.write_all(b"\n")?;
    Ok(())
}

/// Where a node sits, which decides how it opens.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Position {
    /// The document root.
    Root,
    /// Directly after `key:`, so a collection starts on the next line.
    AfterKey,
    /// Directly after `-`, so a collection continues on the dash line.
    AfterDash,
}

/// Write one node at `indent`, honouring where it sits.
fn write_node<W: Write>(
    writer: &mut W,
    value: &Value,
    columns: usize,
    position: Position,
    width: usize,
) -> Result<()> {
    // After a dash the line is already open, so the collection continues it;
    // after a key the collection starts on the next line.
    let skip_first_indent = position == Position::AfterDash;
    match value {
        Value::Sequence(values) if !values.is_empty() => {
            if position == Position::AfterKey {
                writer.write_all(b"\n")?;
            }
            write_sequence(writer, values, columns, skip_first_indent, width)
        }
        Value::Mapping(entries) if !entries.is_empty() => {
            if position == Position::AfterKey {
                writer.write_all(b"\n")?;
            }
            write_mapping(writer, entries, columns, skip_first_indent, width)
        }
        Value::Record(entries) if !entries.is_empty() => {
            if position == Position::AfterKey {
                writer.write_all(b"\n")?;
            }
            write_record(writer, entries, columns, skip_first_indent, width)
        }
        // Everything else is one scalar-shaped token and stays on this line.
        other => write_inline(writer, other),
    }
}

/// Write a block sequence, one `- item` per line.
///
/// `skip_first_indent` is set when the sequence itself opens on a line another
/// marker already started, so its first entry continues that line.
fn write_sequence<W: Write>(
    writer: &mut W,
    values: &[Value],
    columns: usize,
    skip_first_indent: bool,
    width: usize,
) -> Result<()> {
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            writer.write_all(b"\n")?;
        }
        if index != 0 || !skip_first_indent {
            write_indent(writer, columns)?;
        }
        writer.write_all(b"- ")?;
        // A nested collection under a dash keeps its first line on the dash
        // and aligns its later lines with the text after the marker, which is
        // what YAML requires whatever the level width is.
        write_node(
            writer,
            value,
            columns + DASH_WIDTH,
            Position::AfterDash,
            width,
        )?;
    }
    Ok(())
}

/// Write a block mapping, one `key: value` per line.
fn write_mapping<W: Write>(
    writer: &mut W,
    entries: &[(Value, Value)],
    columns: usize,
    skip_first_indent: bool,
    width: usize,
) -> Result<()> {
    for (index, (key, value)) in entries.iter().enumerate() {
        if index != 0 {
            writer.write_all(b"\n")?;
        }
        if index != 0 || !skip_first_indent {
            write_indent(writer, columns)?;
        }
        if is_plain_key(key) {
            write_inline(writer, key)?;
            writer.write_all(b":")?;
        } else {
            // A key that is not a scalar needs YAML's explicit-key form.
            writer.write_all(b"? ")?;
            write_node(
                writer,
                key,
                columns + DASH_WIDTH,
                Position::AfterDash,
                width,
            )?;
            writer.write_all(b"\n")?;
            write_indent(writer, columns)?;
            writer.write_all(b":")?;
        }
        if is_block(value) {
            write_node(writer, value, columns + width, Position::AfterKey, width)?;
        } else {
            writer.write_all(b" ")?;
            write_inline(writer, value)?;
        }
    }
    Ok(())
}

fn write_record<W: Write>(
    writer: &mut W,
    entries: &std::collections::BTreeMap<smol_str::SmolStr, Value>,
    columns: usize,
    skip_first_indent: bool,
    width: usize,
) -> Result<()> {
    for (index, (name, value)) in entries.iter().enumerate() {
        if index != 0 {
            writer.write_all(b"\n")?;
        }
        if index != 0 || !skip_first_indent {
            write_indent(writer, columns)?;
        }
        write_scalar_string(writer, name)?;
        writer.write_all(b":")?;
        if is_block(value) {
            write_node(writer, value, columns + width, Position::AfterKey, width)?;
        } else {
            writer.write_all(b" ")?;
            write_inline(writer, value)?;
        }
    }
    Ok(())
}

/// Return whether a value is written as an indented block rather than inline.
fn is_block(value: &Value) -> bool {
    match value {
        Value::Sequence(values) => !values.is_empty(),
        Value::Mapping(entries) => !entries.is_empty(),
        Value::Record(entries) => !entries.is_empty(),
        _ => false,
    }
}

/// Return whether a key can be written plainly before the colon.
fn is_plain_key(key: &Value) -> bool {
    matches!(
        key,
        Value::Null
            | Value::Bool(_)
            | Value::I64(_)
            | Value::U64(_)
            | Value::I128(_)
            | Value::U128(_)
            | Value::F16(_)
            | Value::F32(_)
            | Value::F64(_)
            | Value::String(_)
    )
}

fn write_indent<W: Write>(writer: &mut W, columns: usize) -> Result<()> {
    /// One chunk of spaces, so a deep indent costs a handful of writes.
    const SPACES: &[u8; 32] = b"                                ";
    let mut left = columns;
    while left > 0 {
        let step = left.min(SPACES.len());
        writer.write_all(&SPACES[..step])?;
        left -= step;
    }
    Ok(())
}

/// Write one scalar or empty collection as one token.
fn write_inline<W: Write>(writer: &mut W, value: &Value) -> Result<()> {
    match value {
        Value::Null => writer.write_all(b"null")?,
        Value::Bool(true) => writer.write_all(b"true")?,
        Value::Bool(false) => writer.write_all(b"false")?,
        Value::I8(value) => write!(writer, "{value}")?,
        Value::I16(value) => write!(writer, "{value}")?,
        Value::I32(value) => write!(writer, "{value}")?,
        Value::I64(value) => write!(writer, "{value}")?,
        Value::U8(value) => write!(writer, "{value}")?,
        Value::U16(value) => write!(writer, "{value}")?,
        Value::U32(value) => write!(writer, "{value}")?,
        Value::U64(value) => write!(writer, "{value}")?,
        Value::I128(value) => write!(writer, "{value}")?,
        Value::U128(value) => write!(writer, "{value}")?,
        Value::F16(value) => write_float(writer, value.as_f64())?,
        Value::F32(value) => write_float(writer, value.as_f64())?,
        Value::F64(value) => write_float(writer, value.as_f64())?,
        Value::D128(unscaled, scale) => write_quoted(
            writer,
            &crate::generic::decimal::decimal_text(crate::I256::from_i128(*unscaled), *scale),
        )?,
        Value::D256(unscaled, scale) => write_quoted(
            writer,
            &crate::generic::decimal::decimal_text(*unscaled, *scale),
        )?,
        Value::String(value) => write_scalar_string(writer, value)?,
        Value::Bytes(value) | Value::Geospatial(value) => {
            // `!!binary` is YAML's standard tag, understood outside Yggdryl.
            writer.write_all(b"!!binary ")?;
            write_quoted(
                writer,
                &base64::engine::general_purpose::STANDARD.encode(value),
            )?;
        }
        Value::Date32(count, unit, zone) => {
            if !zone.is_naive() {
                return Err(codec_error(0, "Date32 cannot carry a timezone"));
            }
            if *unit == crate::TimeUnit::Day && zone.is_naive() {
                if let Some(text) = crate::generic::iso::format_date(*count) {
                    return write_scalar_string(writer, &text);
                }
            }
            write!(writer, "{count}")?;
        }
        Value::Date64(count, unit, zone) => {
            if !zone.is_naive() {
                return Err(codec_error(0, "Date64 cannot carry a timezone"));
            }
            const DAY_MILLISECONDS: i64 = 86_400_000;
            let days = count.div_euclid(DAY_MILLISECONDS);
            if *unit == crate::TimeUnit::Millisecond
                && zone.is_naive()
                && count.rem_euclid(DAY_MILLISECONDS) == 0
            {
                if let Ok(days) = i32::try_from(days) {
                    if let Some(text) = crate::generic::iso::format_date(days) {
                        return write_scalar_string(writer, &text);
                    }
                }
            }
            write!(writer, "{count}")?;
        }
        Value::Time32(count, unit, zone) => {
            write_time(writer, i64::from(*count), *unit, zone)?;
        }
        Value::Time64(count, unit, zone) => {
            write_time(writer, *count, *unit, zone)?;
        }
        Value::DateTime64(count, unit, zone) => {
            let text = if zone.is_naive() {
                crate::generic::iso::format_datetime(*count, *unit)
            } else {
                crate::generic::iso::format_timestamp(*count, *unit, zone)
            };
            match text {
                Some(text) => write_scalar_string(writer, &text)?,
                None => write!(writer, "{count}")?,
            }
        }
        Value::Duration32(count, unit, zone) => {
            write_duration(writer, i64::from(*count), *unit, zone)?;
        }
        Value::Duration64(count, unit, zone) => {
            write_duration(writer, *count, *unit, zone)?;
        }
        Value::Sequence(values) => {
            // Only an empty sequence reaches here.
            debug_assert!(values.is_empty());
            writer.write_all(b"[]")?;
        }
        Value::Mapping(entries) => {
            debug_assert!(entries.is_empty());
            writer.write_all(b"{}")?;
        }
        Value::Record(entries) => {
            debug_assert!(entries.is_empty());
            writer.write_all(b"{}")?;
        }
    }
    Ok(())
}

fn write_time<W: Write>(
    writer: &mut W,
    count: i64,
    unit: crate::TimeUnit,
    zone: &crate::Timezone,
) -> Result<()> {
    if !zone.is_naive() {
        return Err(codec_error(
            0,
            "time-of-day cannot carry a timezone; use DateTime64 for a zoned instant",
        ));
    }
    let Some(text) = crate::generic::iso::format_time(count, unit) else {
        write!(writer, "{count}")?;
        return Ok(());
    };
    write_scalar_string(writer, &text)
}

fn write_duration<W: Write>(
    writer: &mut W,
    count: i64,
    unit: crate::TimeUnit,
    zone: &crate::Timezone,
) -> Result<()> {
    if zone.is_naive() {
        if let Some(text) = crate::generic::iso::format_duration(count, unit) {
            return write_scalar_string(writer, &text);
        }
    } else {
        return Err(codec_error(0, "duration cannot carry a timezone"));
    }
    write!(writer, "{count}")?;
    Ok(())
}

/// Write one float in YAML's core schema, non-finite spellings included.
///
/// YAML and its scanner both define these non-finite spellings.
fn write_float<W: Write>(writer: &mut W, value: f64) -> Result<()> {
    if value.is_finite() {
        serde_json::to_writer(&mut *writer, &value).map_err(|error| Error::Codec {
            format: "yaml",
            position: 0,
            reason: error.to_string().into(),
        })?;
    } else if value.is_nan() {
        writer.write_all(b".nan")?;
    } else if value.is_sign_positive() {
        writer.write_all(b".inf")?;
    } else {
        writer.write_all(b"-.inf")?;
    }
    Ok(())
}

/// Write one value in flow style, at any depth.
///
/// The scalar spelling is the block writer's - a plain key stays plain, a
/// string is quoted only when a plain scalar would read back as something
/// else - so flow and block differ in layout and nothing else. A key the flow
/// grammar cannot spell plainly falls back to YAML's explicit-key form.
fn write_flow<W: Write>(writer: &mut W, value: &Value) -> Result<()> {
    match value {
        Value::Sequence(values) if !values.is_empty() => {
            writer.write_all(b"[")?;
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    writer.write_all(b", ")?;
                }
                write_flow(writer, value)?;
            }
            writer.write_all(b"]")?;
            Ok(())
        }
        Value::Mapping(entries) if !entries.is_empty() => {
            writer.write_all(b"{")?;
            for (index, (key, value)) in entries.iter().enumerate() {
                if index != 0 {
                    writer.write_all(b", ")?;
                }
                if is_plain_key(key) {
                    write_inline(writer, key)?;
                    writer.write_all(b": ")?;
                } else {
                    writer.write_all(b"? ")?;
                    write_flow(writer, key)?;
                    writer.write_all(b" : ")?;
                }
                write_flow(writer, value)?;
            }
            writer.write_all(b"}")?;
            Ok(())
        }
        Value::Record(entries) if !entries.is_empty() => {
            writer.write_all(b"{")?;
            for (index, (name, value)) in entries.iter().enumerate() {
                if index != 0 {
                    writer.write_all(b", ")?;
                }
                write_scalar_string(writer, name)?;
                writer.write_all(b": ")?;
                write_flow(writer, value)?;
            }
            writer.write_all(b"}")?;
            Ok(())
        }
        other => write_inline(writer, other),
    }
}

/// Write a string plainly when YAML would read it back unchanged.
///
/// Quoting is the safe default: a plain scalar is used only when it cannot be
/// mistaken for a number, a boolean, null, or YAML syntax.
fn write_scalar_string<W: Write>(writer: &mut W, value: &str) -> Result<()> {
    if is_plain_safe(value) {
        writer.write_all(value.as_bytes())?;
        Ok(())
    } else {
        write_quoted(writer, value)
    }
}

/// Return whether a string round-trips as a plain YAML scalar.
fn is_plain_safe(value: &str) -> bool {
    if value.is_empty() || value.trim() != value {
        return false;
    }
    // A plain scalar must not start with an indicator character.
    const INDICATORS: [char; 21] = [
        '-', '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%', '@',
        '`', '\t', ' ',
    ];
    if value.starts_with(INDICATORS) {
        return false;
    }
    if value.contains([':', '#', '\n', '\r', '\t']) {
        return false;
    }
    // These spellings are quoted for readers other than this one: `<<` is a
    // merge key, `---` and `...` are document markers, and `nan` and `inf`
    // resolve as floats in implementations looser than YAML's core schema.
    // The list is deliberately wider than what our own scanner resolves, so it
    // only ever adds quoting and never decides that a string is safe plain.
    let lowered = value.to_ascii_lowercase();
    if matches!(
        lowered.as_str(),
        "null"
            | "~"
            | "true"
            | "false"
            | "yes"
            | "no"
            | "on"
            | "off"
            | "nan"
            | "inf"
            | "-inf"
            | "<<"
            | "---"
            | "..."
    ) {
        return false;
    }
    if value.chars().any(char::is_control) {
        return false;
    }
    scanner_reads_back_the_same_string(value)
}

/// Return whether the YAML scanner resolves this text to the same string.
///
/// The emitter cannot keep its own idea of which spellings are numbers. The
/// scanner resolves `.inf`, `.nan`, underscored digits such as `1_000.5`, and
/// the radix prefixes `0x1F`, `0o17` and `0b101`, none of which Rust's own
/// `FromStr` accepts, so a second hand-written list here would drift from the
/// scanner and silently retype string data on the way back. Asking the scanner
/// is one parse of one short scalar, reached only by strings that already
/// passed every cheaper check.
fn scanner_reads_back_the_same_string(value: &str) -> bool {
    let mut parser = YamlParser::new(value.as_bytes(), Limits::default());
    let Some(Ok(RawValue::String(scanned))) = parser.next() else {
        return false;
    };
    // More than one document means the text carries a marker the scanner acts
    // on rather than a scalar it hands back.
    scanned == value && parser.next().is_none()
}

fn write_quoted<W: Write>(writer: &mut W, value: &str) -> Result<()> {
    serde_json::to_writer(writer, value).map_err(|error| Error::Codec {
        format: "yaml",
        position: 0,
        reason: error.to_string().into(),
    })
}

fn codec_error(position: usize, reason: &'static str) -> Error {
    Error::Codec {
        format: "yaml",
        position,
        reason: reason.into(),
    }
}
