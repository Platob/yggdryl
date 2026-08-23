//! Natural JSON values and lazy stream decoding.

use std::borrow::Borrow;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::{Arc, Mutex};

use serde_json::value::RawValue as JsonRawValue;

mod parser;
mod wire;

use crate::text::position::{LineOffsets, line_column_to_byte_offset};
use crate::text::wire::from_raw;
use crate::text::{
    Formatting, Limits, Value, ValueIter, apply_field, check_encode_depth, check_input_size,
};
use crate::{Error, Field, Result};

use self::wire::JsonRef;

const READER_BUFFER_CAPACITY: usize = 8 * 1024;
const POSITION_WINDOW: usize = READER_BUFFER_CAPACITY + 1;

fn is_json_whitespace(input: &[u8]) -> bool {
    input
        .iter()
        .all(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
}

/// Maximum structural nesting accepted by the recursive JSON parser.
///
/// Caller limits may choose any smaller depth. This implementation ceiling
/// keeps adversarial explicit limits from turning nesting into stack exhaustion.
pub const MAX_PARSER_DEPTH: usize = 384;

/// Decode exactly one JSON value from borrowed UTF-8 text.
///
/// This delegates through the string's borrowed bytes without an intermediate
/// UTF-8/byte input buffer. The returned owned value still allocates or shares
/// storage for strings and collections.
pub fn from_utf8(input: &str) -> Result<Value> {
    from_utf8_with_limits(input, Limits::default())
}

/// Decode exactly one JSON value from borrowed UTF-8 text with explicit limits.
pub fn from_utf8_with_limits(input: &str, limits: Limits) -> Result<Value> {
    from_bytes_with_limits(input.as_bytes(), limits)
}

/// Decode exactly one JSON value from bytes.
pub fn from_bytes(input: &[u8]) -> Result<Value> {
    from_bytes_with_limits(input, Limits::default())
}

/// Decode exactly one JSON value from bytes with explicit limits.
pub fn from_bytes_with_limits(input: &[u8], limits: Limits) -> Result<Value> {
    check_input_size(input, limits, "json")?;
    if limits.max_documents() == 0 {
        return Err(codec_error(0, "document limit exceeded"));
    }
    let raw = parser::parse(input, limits)?;
    from_raw(raw, limits, "json")
}

/// Decode JSON and interpret the natural value under `field`.
pub fn from_utf8_with_field(input: &str, field: &Field) -> Result<Value> {
    from_utf8_with_field_and_limits(input, field, Limits::default())
}

/// Decode schema-directed JSON with explicit limits.
pub fn from_utf8_with_field_and_limits(
    input: &str,
    field: &Field,
    limits: Limits,
) -> Result<Value> {
    from_bytes_with_field_and_limits(input.as_bytes(), field, limits)
}

/// Decode JSON bytes and interpret the natural value under `field`.
pub fn from_bytes_with_field(input: &[u8], field: &Field) -> Result<Value> {
    from_bytes_with_field_and_limits(input, field, Limits::default())
}

/// Decode schema-directed JSON bytes with explicit limits.
pub fn from_bytes_with_field_and_limits(
    input: &[u8],
    field: &Field,
    limits: Limits,
) -> Result<Value> {
    field.from_natural_value(from_bytes_with_limits(input, limits)?)
}

/// Decode exactly one JSON value from a streaming reader.
pub fn from_reader<R: Read>(reader: R) -> Result<Value> {
    from_reader_with_limits(reader, Limits::default())
}

/// Decode exactly one JSON value from a streaming reader with explicit limits.
pub fn from_reader_with_limits<R: Read>(reader: R, limits: Limits) -> Result<Value> {
    let mut iterator = Reader::with_limits(reader, limits);
    let Some(value) = iterator.next() else {
        return Err(codec_error(0, "expected one JSON value"));
    };
    let value = value?;
    if let Some(trailing) = iterator.next() {
        trailing?;
        return Err(codec_error(
            iterator.document_start,
            "expected one JSON value but found trailing data",
        ));
    }
    Ok(value)
}

/// Decode JSON from a reader under `field`.
pub fn from_reader_with_field<R: Read>(reader: R, field: &Field) -> Result<Value> {
    from_reader_with_field_and_limits(reader, field, Limits::default())
}

/// Decode schema-directed JSON from a reader with explicit limits.
pub fn from_reader_with_field_and_limits<R: Read>(
    reader: R,
    field: &Field,
    limits: Limits,
) -> Result<Value> {
    field.from_natural_value(from_reader_with_limits(reader, limits)?)
}

/// Decode every whitespace-separated JSON value from bytes.
pub fn from_bytes_all(input: &[u8]) -> Result<Vec<Value>> {
    from_bytes_all_with_limits(input, Limits::default())
}

/// Decode every whitespace-separated JSON value from borrowed UTF-8 text.
pub fn from_utf8_all(input: &str) -> Result<Vec<Value>> {
    from_utf8_all_with_limits(input, Limits::default())
}

/// Decode every whitespace-separated JSON value from borrowed UTF-8 text with
/// explicit limits.
pub fn from_utf8_all_with_limits(input: &str, limits: Limits) -> Result<Vec<Value>> {
    from_bytes_all_with_limits(input.as_bytes(), limits)
}

/// Decode every whitespace-separated JSON value from bytes with explicit limits.
pub fn from_bytes_all_with_limits(input: &[u8], limits: Limits) -> Result<Vec<Value>> {
    check_input_size(input, limits, "json")?;
    let mut deserializer = serde_json::Deserializer::from_slice(input);
    deserializer.disable_recursion_limit();
    let mut stream = deserializer.into_iter::<&JsonRawValue>();
    let mut values = Vec::new();
    while let Some(raw) = stream.next() {
        let raw = raw.map_err(|error| json_error(input, error))?;
        let end = stream.byte_offset();
        let start = end.saturating_sub(raw.get().len());
        if values.len() >= limits.max_documents() {
            return Err(codec_error(start, "document limit exceeded"));
        }
        values.push(parse_raw_document(raw.get().as_bytes(), limits, start)?);
    }
    Ok(values)
}

/// Decode every JSON value from UTF-8 under one field.
pub fn from_utf8_all_with_field(input: &str, field: &Field) -> Result<Vec<Value>> {
    from_utf8_all_with_field_and_limits(input, field, Limits::default())
}

/// Decode every schema-directed JSON value from UTF-8 with limits.
pub fn from_utf8_all_with_field_and_limits(
    input: &str,
    field: &Field,
    limits: Limits,
) -> Result<Vec<Value>> {
    from_bytes_all_with_field_and_limits(input.as_bytes(), field, limits)
}

/// Decode every JSON value from bytes under one field.
pub fn from_bytes_all_with_field(input: &[u8], field: &Field) -> Result<Vec<Value>> {
    from_bytes_all_with_field_and_limits(input, field, Limits::default())
}

/// Decode every schema-directed JSON value from bytes with limits.
pub fn from_bytes_all_with_field_and_limits(
    input: &[u8],
    field: &Field,
    limits: Limits,
) -> Result<Vec<Value>> {
    apply_field(from_bytes_all_with_limits(input, limits)?, field)
}

/// Decode every JSON value from a reader.
pub fn from_reader_all<R: Read>(reader: R) -> Result<Vec<Value>> {
    from_reader_all_with_limits(reader, Limits::default())
}

/// Decode every JSON value from a reader with explicit limits.
pub fn from_reader_all_with_limits<R: Read>(reader: R, limits: Limits) -> Result<Vec<Value>> {
    Reader::with_limits(reader, limits).collect()
}

/// Decode every JSON value from a reader under one field.
pub fn from_reader_all_with_field<R: Read>(reader: R, field: &Field) -> Result<Vec<Value>> {
    from_reader_all_with_field_and_limits(reader, field, Limits::default())
}

/// Decode every schema-directed JSON value from a reader with limits.
pub fn from_reader_all_with_field_and_limits<R: Read>(
    reader: R,
    field: &Field,
    limits: Limits,
) -> Result<Vec<Value>> {
    apply_field(from_reader_all_with_limits(reader, limits)?, field)
}

/// Lazily decode JSON values from a borrowed reader.
pub fn from_reader_iter<'a, R: Read + 'a>(reader: &'a mut R) -> ValueIter<'a> {
    from_reader_iter_with_limits(reader, Limits::default())
}

/// Lazily decode JSON values from a borrowed reader with explicit limits.
pub fn from_reader_iter_with_limits<'a, R: Read + 'a>(
    reader: &'a mut R,
    limits: Limits,
) -> ValueIter<'a> {
    ValueIter::new(Reader::with_limits(reader, limits))
}

/// Lazily decode schema-directed JSON values from a reader.
pub fn from_reader_iter_with_field<'a, R: Read + 'a>(
    reader: &'a mut R,
    field: &'a Field,
) -> ValueIter<'a> {
    from_reader_iter_with_field_and_limits(reader, field, Limits::default())
}

/// Lazily decode schema-directed JSON values with explicit limits.
pub fn from_reader_iter_with_field_and_limits<'a, R: Read + 'a>(
    reader: &'a mut R,
    field: &'a Field,
    limits: Limits,
) -> ValueIter<'a> {
    ValueIter::new(Reader::with_limits(reader, limits)).with_field(field)
}

/// An owning, lazy iterator over whitespace-separated JSON values.
pub struct Reader<R: Read> {
    inner: serde_json::StreamDeserializer<
        'static,
        serde_json::de::IoRead<BufReader<LimitedReader<R>>>,
        Box<JsonRawValue>,
    >,
    positions: Arc<Mutex<LineOffsets>>,
    limits: Limits,
    documents: usize,
    document_start: usize,
    finished: bool,
}

impl<R: Read> Reader<R> {
    /// Construct with default resource limits.
    pub fn new(reader: R) -> Self {
        Self::with_limits(reader, Limits::default())
    }

    /// Construct with explicit resource limits.
    pub fn with_limits(reader: R, limits: Limits) -> Self {
        let positions = Arc::new(Mutex::new(LineOffsets::new(POSITION_WINDOW)));
        let reader =
            LimitedReader::with_positions(reader, limits.max_input_bytes(), Arc::clone(&positions));
        let mut deserializer = serde_json::Deserializer::from_reader(BufReader::with_capacity(
            READER_BUFFER_CAPACITY,
            reader,
        ));
        deserializer.disable_recursion_limit();
        Self {
            inner: deserializer.into_iter(),
            positions,
            limits,
            documents: 0,
            document_start: 0,
            finished: false,
        }
    }

    /// Return the number of consumed encoded bytes.
    pub fn byte_offset(&self) -> usize {
        self.inner.byte_offset()
    }
}

impl<R: Read> Iterator for Reader<R> {
    type Item = Result<Value>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        let result = self.inner.next()?;
        let end = self.byte_offset();
        let document_start = result
            .as_ref()
            .ok()
            .map_or(end, |raw| end.saturating_sub(raw.get().len()));
        if self.documents >= self.limits.max_documents() {
            self.finished = true;
            return Some(Err(codec_error(document_start, "document limit exceeded")));
        }
        self.documents += 1;
        Some(match result {
            Ok(raw) => {
                self.document_start = document_start;
                parse_raw_document(raw.get().as_bytes(), self.limits, document_start)
            }
            Err(error) => {
                self.finished = true;
                let position = self
                    .positions
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .position(error.line(), error.column());
                Err(codec_error(position, &error.to_string()))
            }
        })
    }
}

/// Decode strict newline-delimited JSON bytes.
pub fn from_lines_bytes(input: &[u8]) -> Result<Vec<Value>> {
    from_lines_bytes_with_limits(input, Limits::default())
}

/// Decode strict newline-delimited JSON from borrowed UTF-8 text.
pub fn from_lines_utf8(input: &str) -> Result<Vec<Value>> {
    from_lines_utf8_with_limits(input, Limits::default())
}

/// Decode strict newline-delimited JSON from borrowed UTF-8 text with
/// explicit limits.
pub fn from_lines_utf8_with_limits(input: &str, limits: Limits) -> Result<Vec<Value>> {
    from_lines_bytes_with_limits(input.as_bytes(), limits)
}

/// Decode strict newline-delimited JSON bytes with explicit limits.
pub fn from_lines_bytes_with_limits(input: &[u8], limits: Limits) -> Result<Vec<Value>> {
    check_input_size(input, limits, "json")?;
    let mut values = Vec::new();
    let mut offset = 0_usize;
    for encoded_line in input.split_inclusive(|byte| *byte == b'\n') {
        let line = encoded_line.strip_suffix(b"\n").unwrap_or(encoded_line);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if !is_json_whitespace(line) {
            if values.len() >= limits.max_documents() {
                return Err(codec_error(offset, "document limit exceeded"));
            }
            values.push(
                from_bytes_with_limits(line, limits)
                    .map_err(|error| offset_error(error, offset))?,
            );
        }
        offset = offset.saturating_add(encoded_line.len());
    }
    Ok(values)
}

/// Decode strict newline-delimited JSON from a byte reader.
pub fn from_lines_reader<R: Read>(reader: R) -> Result<Vec<Value>> {
    from_lines_reader_with_limits(reader, Limits::default())
}

/// Decode strict newline-delimited JSON from a reader with explicit limits.
pub fn from_lines_reader_with_limits<R: Read>(reader: R, limits: Limits) -> Result<Vec<Value>> {
    LinesReader::with_limits(reader, limits).collect()
}

/// Lazily decode strict newline-delimited JSON from a borrowed reader.
pub fn from_lines_reader_iter<'a, R: Read + 'a>(reader: &'a mut R) -> ValueIter<'a> {
    from_lines_reader_iter_with_limits(reader, Limits::default())
}

/// Lazily decode strict newline-delimited JSON with explicit limits.
pub fn from_lines_reader_iter_with_limits<'a, R: Read + 'a>(
    reader: &'a mut R,
    limits: Limits,
) -> ValueIter<'a> {
    ValueIter::new(LinesReader::with_limits(reader, limits))
}

/// An owning, lazy iterator over strict newline-delimited JSON values.
pub struct LinesReader<R: Read> {
    reader: BufReader<LimitedReader<R>>,
    buffer: Vec<u8>,
    offset: usize,
    documents: usize,
    limits: Limits,
    finished: bool,
}

impl<R: Read> LinesReader<R> {
    /// Construct with default resource limits.
    pub fn new(reader: R) -> Self {
        Self::with_limits(reader, Limits::default())
    }

    /// Construct with explicit resource limits.
    pub fn with_limits(reader: R, limits: Limits) -> Self {
        Self {
            reader: BufReader::new(LimitedReader::new(reader, limits.max_input_bytes())),
            buffer: Vec::new(),
            offset: 0,
            documents: 0,
            limits,
            finished: false,
        }
    }

    /// Return the number of consumed encoded bytes.
    pub const fn byte_offset(&self) -> usize {
        self.offset
    }
}

impl<R: Read> Iterator for LinesReader<R> {
    type Item = Result<Value>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        loop {
            self.buffer.clear();
            let start = self.offset;
            let read = match self.reader.read_until(b'\n', &mut self.buffer) {
                Ok(0) => {
                    self.finished = true;
                    return None;
                }
                Ok(read) => read,
                Err(error) => {
                    self.finished = true;
                    return Some(Err(codec_error(self.offset, &error.to_string())));
                }
            };
            self.offset = self.offset.saturating_add(read);
            let line = self
                .buffer
                .strip_suffix(b"\n")
                .unwrap_or(&self.buffer)
                .strip_suffix(b"\r")
                .unwrap_or_else(|| self.buffer.strip_suffix(b"\n").unwrap_or(&self.buffer));
            if is_json_whitespace(line) {
                continue;
            }
            if self.documents >= self.limits.max_documents() {
                self.finished = true;
                return Some(Err(codec_error(start, "document limit exceeded")));
            }
            self.documents += 1;
            return Some(
                from_bytes_with_limits(line, self.limits)
                    .map_err(|error| offset_error(error, start)),
            );
        }
    }
}

/// Encode one value as compact JSON bytes.
pub fn into_bytes(value: &Value) -> Result<Vec<u8>> {
    into_bytes_with_formatting(value, Formatting::default())
}

/// Encode one value to JSON bytes laid out as `formatting` asks.
///
/// [`Indent::Default`](crate::text::Indent::Default) and [`Indent::None`](crate::text::Indent::None) are both today's compact output;
/// [`Indent::Spaces`](crate::text::Indent::Spaces) pretty-prints with that many spaces per nesting level,
/// exactly as another JSON formatter's `indent=n` option reads.
///
/// ```
/// use yggdryl::generic::Value;
/// use yggdryl::text::Formatting;
///
/// # fn main() -> yggdryl::Result<()> {
/// let value = Value::from_record([("id", Value::I64(1))])?;
/// assert_eq!(yggdryl::json::into_bytes(&value)?, br#"{"id":1}"#);
/// assert_eq!(
///     yggdryl::json::into_bytes_with_formatting(&value, Formatting::indented(2))?,
///     b"{\n  \"id\": 1\n}",
/// );
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

/// Encode one value as compact JSON UTF-8.
pub fn into_utf8(value: &Value) -> Result<String> {
    into_utf8_with_formatting(value, Formatting::default())
}

/// Encode one value as JSON UTF-8 with explicit formatting.
pub fn into_utf8_with_formatting(value: &Value, formatting: Formatting) -> Result<String> {
    String::from_utf8(into_bytes_with_formatting(value, formatting)?).map_err(|error| {
        Error::Codec {
            format: "json",
            position: error.utf8_error().valid_up_to(),
            reason: "encoded JSON is not valid UTF-8".into(),
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
    writer: W,
    formatting: Formatting,
) -> Result<()> {
    check_encode_depth(value, "json")?;
    write_one(writer, value, formatting)
}

/// Encode values as newline-delimited JSON bytes.
pub fn into_bytes_all(values: &[Value]) -> Result<Vec<u8>> {
    into_bytes_all_with_formatting(values, Formatting::default())
}

/// Encode values as newline-delimited JSON bytes, laid out as `formatting` asks.
///
/// An indented document still occupies one line-delimited slot, because the
/// stream separator is the newline *between* documents; an indented JSON Lines
/// stream is therefore no longer one document per line, which is why the
/// compact default is what a `.jsonl` reader expects.
///
/// # Errors
///
/// Returns the encoder's failure.
pub fn into_bytes_all_with_formatting(values: &[Value], formatting: Formatting) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    into_writer_all_with_formatting(values, &mut output, formatting)?;
    Ok(output)
}

/// Encode values as newline-delimited JSON UTF-8.
pub fn into_utf8_all(values: &[Value]) -> Result<String> {
    into_utf8_all_with_formatting(values, Formatting::default())
}

/// Encode values as newline-delimited JSON UTF-8 with explicit formatting.
pub fn into_utf8_all_with_formatting(values: &[Value], formatting: Formatting) -> Result<String> {
    String::from_utf8(into_bytes_all_with_formatting(values, formatting)?).map_err(|error| {
        Error::Codec {
            format: "json",
            position: error.utf8_error().valid_up_to(),
            reason: "encoded JSON is not valid UTF-8".into(),
        }
    })
}

/// Encode values as newline-delimited JSON to a byte writer.
pub fn into_writer_all<W, I, V>(values: I, writer: W) -> Result<()>
where
    W: Write,
    I: IntoIterator<Item = V>,
    V: Borrow<Value>,
{
    into_writer_all_with_formatting(values, writer, Formatting::default())
}

/// Encode values as newline-delimited JSON, laid out as `formatting` asks.
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
    V: Borrow<Value>,
{
    for value in values {
        let value = value.borrow();
        check_encode_depth(value, "json")?;
        write_one(&mut writer, value, formatting)?;
        writer.write_all(b"\n")?;
    }
    Ok(())
}

/// Emit one already-depth-checked value under `formatting`.
fn write_one<W: Write>(writer: W, value: &Value, formatting: Formatting) -> Result<()> {
    match formatting.indent().unit() {
        // Compact is the default and the only shape JSON Lines can carry.
        None => serde_json::to_writer(writer, &JsonRef(value))
            .map_err(|error| codec_error(0, &error.to_string())),
        Some(unit) => {
            let formatter = serde_json::ser::PrettyFormatter::with_indent(unit);
            let mut serializer = serde_json::Serializer::with_formatter(writer, formatter);
            serde::Serialize::serialize(&JsonRef(value), &mut serializer)
                .map_err(|error| codec_error(0, &error.to_string()))
        }
    }
}

fn parse_raw_document(input: &[u8], limits: Limits, base: usize) -> Result<Value> {
    from_bytes_with_limits(input, limits).map_err(|error| offset_error(error, base))
}

fn offset_error(error: Error, base: usize) -> Error {
    match error {
        Error::Codec {
            format,
            position,
            reason,
        } => Error::Codec {
            format,
            position: base.saturating_add(position),
            reason,
        },
        other => other,
    }
}

fn json_error(input: &[u8], error: serde_json::Error) -> Error {
    codec_error(
        line_column_to_byte_offset(input, error.line(), error.column()),
        &error.to_string(),
    )
}

fn codec_error(position: usize, reason: &str) -> Error {
    Error::Codec {
        format: "json",
        position,
        reason: reason.into(),
    }
}

struct LimitedReader<R> {
    inner: R,
    remaining: usize,
    checked_end: bool,
    positions: Option<Arc<Mutex<LineOffsets>>>,
}

impl<R> LimitedReader<R> {
    const fn new(inner: R, limit: usize) -> Self {
        Self {
            inner,
            remaining: limit,
            checked_end: false,
            positions: None,
        }
    }

    fn with_positions(inner: R, limit: usize, positions: Arc<Mutex<LineOffsets>>) -> Self {
        Self {
            inner,
            remaining: limit,
            checked_end: false,
            positions: Some(positions),
        }
    }
}

impl<R: Read> Read for LimitedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if buffer.is_empty() || self.checked_end {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut sentinel = [0_u8; 1];
            return match self.inner.read(&mut sentinel)? {
                0 => {
                    self.checked_end = true;
                    Ok(0)
                }
                _ => Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "input byte limit exceeded",
                )),
            };
        }
        let length = buffer.len().min(self.remaining);
        let read = self.inner.read(&mut buffer[..length])?;
        self.remaining -= read;
        if let Some(positions) = &self.positions {
            positions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .observe(&buffer[..read]);
        }
        Ok(read)
    }
}
