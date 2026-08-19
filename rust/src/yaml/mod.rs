//! YAML value encoding, machine-readable tags, comments, and document streams.

use std::io::{Read, Write};

use base64::Engine as _;

mod parser;

use crate::text::wire::{RawValue, from_raw};
use crate::text::{Formatting, Limits, Value, ValueIter, check_encode_depth, check_input_size};
use crate::{Error, Result};

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
pub fn from_str(input: &str) -> Result<Value> {
    from_str_with_limits(input, Limits::default())
}

/// Decode exactly one YAML document from borrowed UTF-8 text with explicit limits.
pub fn from_str_with_limits(input: &str, limits: Limits) -> Result<Value> {
    from_slice_with_limits(input.as_bytes(), limits)
}

/// Decode exactly one YAML document from bytes.
pub fn from_slice(input: &[u8]) -> Result<Value> {
    from_slice_with_limits(input, Limits::default())
}

/// Decode exactly one YAML document from bytes with explicit limits.
pub fn from_slice_with_limits(input: &[u8], limits: Limits) -> Result<Value> {
    check_input_size(input, limits, "yaml")?;
    from_reader_with_limits(std::io::Cursor::new(input), limits)
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

/// Decode every YAML document from bytes.
pub fn from_slice_all(input: &[u8]) -> Result<Vec<Value>> {
    from_slice_all_with_limits(input, Limits::default())
}

/// Decode every YAML document from borrowed UTF-8 text.
pub fn from_str_all(input: &str) -> Result<Vec<Value>> {
    from_str_all_with_limits(input, Limits::default())
}

/// Decode every YAML document from borrowed UTF-8 text with explicit limits.
pub fn from_str_all_with_limits(input: &str, limits: Limits) -> Result<Vec<Value>> {
    from_slice_all_with_limits(input.as_bytes(), limits)
}

/// Decode every YAML document from bytes with explicit limits.
pub fn from_slice_all_with_limits(input: &[u8], limits: Limits) -> Result<Vec<Value>> {
    check_input_size(input, limits, "yaml")?;
    Reader::with_limits(std::io::Cursor::new(input), limits).collect()
}

/// Decode every YAML document from a reader.
pub fn from_reader_all<R: Read>(reader: R) -> Result<Vec<Value>> {
    Reader::new(reader).collect()
}

/// Decode every YAML document from a reader with explicit limits.
pub fn from_reader_all_with_limits<R: Read>(reader: R, limits: Limits) -> Result<Vec<Value>> {
    Reader::with_limits(reader, limits).collect()
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
pub fn to_vec(value: &Value) -> Result<Vec<u8>> {
    to_vec_with_formatting(value, Formatting::default())
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
/// let value = Value::from_mapping([(Value::String("id".into()), Value::I64(1))])?;
/// assert_eq!(yggdryl::yaml::to_vec(&value)?, b"id: 1\n");
///
/// let flow = yggdryl::yaml::to_vec_with_formatting(&value, Formatting::compact())?;
/// assert_eq!(flow, b"{id: 1}\n");
/// // Formatting changes bytes, never meaning.
/// assert_eq!(yggdryl::yaml::from_str("{id: 1}")?, value);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns the encoder's failure, including the published depth cap.
pub fn to_vec_with_formatting(value: &Value, formatting: Formatting) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    to_writer_with_formatting(&mut output, value, formatting)?;
    Ok(output)
}

/// Consume and encode one value to YAML bytes.
///
/// The encoded bytes require a new buffer; consuming avoids retaining or
/// cloning the input value but cannot reuse its typed backing allocations.
pub fn into_vec(value: Value) -> Result<Vec<u8>> {
    to_vec(&value)
}

/// Consume and encode one value to YAML bytes laid out as `formatting` asks.
///
/// # Errors
///
/// Returns the encoder's failure, including the published depth cap.
pub fn into_vec_with_formatting(value: Value, formatting: Formatting) -> Result<Vec<u8>> {
    to_vec_with_formatting(&value, formatting)
}

/// Encode one value to a byte writer.
pub fn to_writer<W: Write>(writer: W, value: &Value) -> Result<()> {
    to_writer_with_formatting(writer, value, Formatting::default())
}

/// Encode one value to a byte writer, laid out as `formatting` asks.
///
/// # Errors
///
/// Returns the encoder's or the sink's failure.
pub fn to_writer_with_formatting<W: Write>(
    mut writer: W,
    value: &Value,
    formatting: Formatting,
) -> Result<()> {
    check_encode_depth(value, "yaml")?;
    // `write_value` terminates the document, so nothing is added here.
    write_value(&mut writer, value, Layout::from(formatting))
}

/// Encode YAML documents to a byte vector.
pub fn to_vec_all(values: &[Value]) -> Result<Vec<u8>> {
    to_vec_all_with_formatting(values, Formatting::default())
}

/// Encode YAML documents to a byte vector, laid out as `formatting` asks.
///
/// # Errors
///
/// Returns the encoder's failure, including the published depth cap.
pub fn to_vec_all_with_formatting(values: &[Value], formatting: Formatting) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    to_writer_all_with_formatting(&mut output, values, formatting)?;
    Ok(output)
}

/// Encode multiple values as a YAML document stream.
pub fn to_writer_all<W, I, V>(writer: W, values: I) -> Result<()>
where
    W: Write,
    I: IntoIterator<Item = V>,
    V: std::borrow::Borrow<Value>,
{
    to_writer_all_with_formatting(writer, values, Formatting::default())
}

/// Encode a YAML document stream, laid out as `formatting` asks.
///
/// # Errors
///
/// Returns the encoder's or the sink's failure.
pub fn to_writer_all_with_formatting<W, I, V>(
    mut writer: W,
    values: I,
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
        // A record is its named-mapping spelling, block or inline alike.
        Value::Record(..) => {
            write_node(writer, &value.record_to_mapping(), columns, position, width)
        }
        Value::Sequence(values) if !values.is_empty() => {
            if position == Position::AfterKey {
                writer.write_all(b"\n")?;
            }
            write_sequence(writer, values, columns, skip_first_indent, width)
        }
        Value::Mapping(entries) if !entries.is_empty() && !is_yaml_envelope_collision(entries) => {
            if position == Position::AfterKey {
                writer.write_all(b"\n")?;
            }
            write_mapping(writer, entries, columns, skip_first_indent, width)
        }
        // Everything else is one scalar-shaped token: it stays on the line the
        // marker opened, including the empty collections and the envelopes.
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

/// Return whether a value is written as an indented block rather than inline.
fn is_block(value: &Value) -> bool {
    match value {
        Value::Sequence(values) => !values.is_empty(),
        Value::Mapping(entries) => !entries.is_empty() && !is_yaml_envelope_collision(entries),
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

/// Write one value as a single token, for scalars and the lossless envelopes.
fn write_inline<W: Write>(writer: &mut W, value: &Value) -> Result<()> {
    match value {
        // A record inlines as the mapping its field names spell.
        Value::Record(..) => write_inline_recursive(writer, &value.record_to_mapping())?,
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
        Value::I128(value) => {
            writer.write_all(b"{\"$yggdryl\": \"i128\", \"value\": ")?;
            write!(writer, "\"{value}\"")?;
            writer.write_all(b"}")?;
        }
        Value::U128(value) => {
            writer.write_all(b"{\"$yggdryl\": \"u128\", \"value\": ")?;
            write!(writer, "\"{value}\"")?;
            writer.write_all(b"}")?;
        }
        Value::F32(value) => write_float(writer, value.as_f64())?,
        Value::F64(value) => write_float(writer, value.as_f64())?,
        Value::Decimal(unscaled, scale) => {
            // The coefficient is quoted for the same reason `i128` is: it is
            // wider than the integer every YAML reader agrees on.
            writer.write_all(b"{\"$yggdryl\": \"decimal\", \"value\": [")?;
            write!(writer, "\"{unscaled}\", {scale}")?;
            writer.write_all(b"]}")?;
        }
        Value::String(value) => write_scalar_string(writer, value)?,
        Value::Bytes(value) => {
            writer.write_all(b"{\"$yggdryl\": \"bytes\", \"value\": ")?;
            write_quoted(
                writer,
                &base64::engine::general_purpose::STANDARD.encode(value),
            )?;
            writer.write_all(b"}")?;
        }
        // A geometry is the bytes envelope under its own kind, so the WKB
        // reads back as the geospatial value it left as.
        Value::Geospatial(value) => {
            writer.write_all(b"{\"$yggdryl\": \"geospatial\", \"value\": ")?;
            write_quoted(
                writer,
                &base64::engine::general_purpose::STANDARD.encode(value),
            )?;
            writer.write_all(b"}")?;
        }
        Value::Date(days) => match crate::generic::iso::format_date(*days) {
            Some(spelled) => write_scalar_string(writer, &spelled)?,
            None => {
                writer.write_all(b"{\"$yggdryl\": \"date\", \"value\": ")?;
                write!(writer, "{days}")?;
                writer.write_all(b"}")?;
            }
        },
        Value::Time(count, unit) => match crate::generic::iso::format_time(*count, *unit) {
            Some(spelled) => write_scalar_string(writer, &spelled)?,
            None => write_temporal(writer, "time", *count, *unit, None)?,
        },
        Value::Duration(count, unit) => match crate::generic::iso::format_duration(*count, *unit) {
            Some(spelled) => write_scalar_string(writer, &spelled)?,
            None => write_temporal(writer, "duration", *count, *unit, None)?,
        },
        Value::Timestamp(count, unit, zone) => {
            match crate::generic::iso::format_timestamp(*count, *unit, zone) {
                Some(spelled) => write_scalar_string(writer, &spelled)?,
                None => write_temporal(writer, "timestamp", *count, *unit, Some(zone))?,
            }
        }
        Value::DateTime(count, unit) => match crate::generic::iso::format_datetime(*count, *unit) {
            Some(spelled) => write_scalar_string(writer, &spelled)?,
            None => write_temporal(writer, "timestamp", *count, *unit, None)?,
        },
        Value::Sequence(values) => {
            // Only an empty sequence reaches here.
            debug_assert!(values.is_empty());
            writer.write_all(b"[]")?;
        }
        Value::Mapping(entries) => {
            if is_yaml_envelope_collision(entries) {
                writer.write_all(b"{\"$yggdryl\": \"mapping\", \"value\": [")?;
                for (index, (key, value)) in entries.iter().enumerate() {
                    if index != 0 {
                        writer.write_all(b", ")?;
                    }
                    writer.write_all(b"[")?;
                    write_inline_recursive(writer, key)?;
                    writer.write_all(b", ")?;
                    write_inline_recursive(writer, value)?;
                    writer.write_all(b"]")?;
                }
                writer.write_all(b"]}")?;
            } else {
                debug_assert!(entries.is_empty());
                writer.write_all(b"{}")?;
            }
        }
    }
    Ok(())
}

/// Write one float in YAML's core schema, non-finite spellings included.
///
/// YAML spells the non-finite floats itself and the scanner reads those
/// spellings back, so an envelope would only cost every other implementation
/// the value it can already read.
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

/// Write one temporal as the flat `$yggdryl` envelope every reader can read.
fn write_temporal<W: Write>(
    writer: &mut W,
    kind: &str,
    count: i64,
    unit: crate::TimeUnit,
    zone: Option<&crate::Timezone>,
) -> Result<()> {
    write!(writer, "{{\"$yggdryl\": \"{kind}\", \"value\": [")?;
    write_quoted(writer, unit.as_str())?;
    write!(writer, ", {count}")?;
    if let Some(zone) = zone {
        writer.write_all(b", ")?;
        write_quoted(writer, zone.as_str())?;
    }
    writer.write_all(b"]}")?;
    Ok(())
}

/// Write a value in flow style, for the inside of a lossless envelope.
fn write_inline_recursive<W: Write>(writer: &mut W, value: &Value) -> Result<()> {
    match value {
        Value::Sequence(values) => {
            writer.write_all(b"[")?;
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    writer.write_all(b", ")?;
                }
                write_inline_recursive(writer, value)?;
            }
            writer.write_all(b"]")?;
            Ok(())
        }
        Value::Mapping(entries) if !is_yaml_envelope_collision(entries) => {
            writer.write_all(b"{")?;
            for (index, (key, value)) in entries.iter().enumerate() {
                if index != 0 {
                    writer.write_all(b", ")?;
                }
                writer.write_all(b"? ")?;
                write_inline_recursive(writer, key)?;
                writer.write_all(b" : ")?;
                write_inline_recursive(writer, value)?;
            }
            writer.write_all(b"}")?;
            Ok(())
        }
        Value::String(value) => write_quoted(writer, value),
        other => write_inline(writer, other),
    }
}

/// Write one value in flow style, at any depth.
///
/// The scalar spelling is the block writer's - a plain key stays plain, a
/// string is quoted only when a plain scalar would read back as something
/// else - so flow and block differ in layout and nothing else. A key the flow
/// grammar cannot spell plainly falls back to YAML's explicit-key form.
fn write_flow<W: Write>(writer: &mut W, value: &Value) -> Result<()> {
    match value {
        Value::Record(..) => write_flow(writer, &value.record_to_mapping()),
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
        Value::Mapping(entries) if !entries.is_empty() && !is_yaml_envelope_collision(entries) => {
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

fn is_yaml_envelope_collision(entries: &[(Value, Value)]) -> bool {
    let marker = entries.iter().find_map(|(key, value)| {
        (key.as_str() == Some("$yggdryl"))
            .then(|| value.as_str())
            .flatten()
    });
    match marker {
        Some("bytes" | "geospatial" | "i128" | "u128" | "float" | "mapping") => {
            exact_value_keys(entries, &["$yggdryl", "value"])
        }
        _ => false,
    }
}

fn exact_value_keys(entries: &[(Value, Value)], names: &[&str]) -> bool {
    entries.len() == names.len()
        && entries
            .iter()
            .all(|(key, _)| key.as_str().is_some_and(|key| names.contains(&key)))
}

fn write_quoted<W: Write>(writer: &mut W, value: &str) -> Result<()> {
    serde_json::to_writer(writer, value).map_err(|error| Error::Codec {
        format: "yaml",
        position: 0,
        reason: error.to_string().into(),
    })
}
