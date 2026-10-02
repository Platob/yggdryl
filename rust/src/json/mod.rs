//! Natural JSON values and lazy stream decoding.
//!
//! The explicit representation forms carry the implementation: `from_utf8`,
//! `from_bytes` and `from_reader` with their `_all`, `_with_field` and
//! `_with_limits` modifiers, and `into_utf8`, `into_bytes` and `into_writer`
//! with `_with_formatting`. [`from_json_scalar`], [`from_json_scalar_with_field`]
//! and [`into_json_scalar`] are the one inferring boundary over them, not
//! aliases: each names the `Scalar` it answers, coerces any byte-like input at
//! the boundary and redirects to its explicit form, holding no parsing,
//! rendering, validation or limits logic of its own. Deleting them as
//! duplicates would remove the only entry point that names the `Scalar`.

use std::borrow::Borrow;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::{Arc, Mutex};

use serde_json::value::RawValue as JsonRawValue;

pub(crate) mod column;
pub(crate) mod field;
mod parser;
mod wire;

use crate::text::position::{LineOffsets, line_column_to_byte_offset};
use crate::text::wire::from_raw;
use crate::text::{
    Formatting, Limits, Scalar, ScalarIter, apply_field, check_encode_depth, check_input_size,
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

/// Decode exactly one JSON value from byte-like content into the shared
/// `Scalar`.
///
/// This is the inferring entry point over [`from_bytes`]: `input` may be
/// `&str`, `String`, `&[u8]`, `Vec<u8>` or any other byte-like value, and its
/// bytes are decoded there under default [`Limits`]. Inference is
/// deterministic - the input is always content, never a path - so text that
/// happens to name an existing file is parsed as JSON rather than read. A
/// caller who needs explicit limits calls [`from_bytes_with_limits`].
///
/// ```
/// use yggdryl::{Scalar, from_json_scalar, into_json_scalar};
///
/// let value = from_json_scalar(r#"{"id":1}"#)?;
/// assert_eq!(value, Scalar::from_struct([("id", Scalar::from(1))])?);
/// assert_eq!(into_json_scalar(&value)?, r#"{"id":1}"#);
/// # Ok::<(), yggdryl::Error>(())
/// ```
pub fn from_json_scalar(input: impl AsRef<[u8]>) -> Result<Scalar> {
    from_bytes(input.as_ref())
}

/// Decode JSON content and interpret the natural value under `field`,
/// answering the shared `Scalar`.
///
/// This is the inferring entry point over [`from_bytes_with_field`]: it
/// coerces `input` exactly as [`from_json_scalar`] does - content, never a
/// path - and redirects, so `field` types natural strings, orders records and
/// validates there under default [`Limits`]. Explicit limits go through
/// [`from_bytes_with_field_and_limits`].
///
/// ```
/// use yggdryl::{DataType, Field, Scalar, from_json_scalar_with_field};
///
/// let field = Field::new("amount", DataType::decimal128(10, 2)?, false);
/// let value = from_json_scalar_with_field(r#""12.50""#, &field)?;
/// assert_eq!(value, Scalar::decimal128(1250, 2));
/// # Ok::<(), yggdryl::Error>(())
/// ```
pub fn from_json_scalar_with_field(input: impl AsRef<[u8]>, field: &Field) -> Result<Scalar> {
    from_bytes_with_field(input.as_ref(), field)
}

/// Encode one value as compact JSON UTF-8, naming the shared `Scalar` it
/// takes.
///
/// This is the named entry point over [`into_utf8`], which it redirects to
/// unchanged; a layout other than compact goes through
/// [`into_utf8_with_formatting`].
///
/// ```
/// use yggdryl::{Scalar, into_json_scalar};
///
/// let value = Scalar::from_struct([("id", Scalar::from(1))])?;
/// assert_eq!(into_json_scalar(&value)?, r#"{"id":1}"#);
/// # Ok::<(), yggdryl::Error>(())
/// ```
pub fn into_json_scalar(value: &Scalar) -> Result<String> {
    into_utf8(value)
}

/// Decode exactly one JSON value from borrowed UTF-8 text.
///
/// This delegates through the string's borrowed bytes without an intermediate
/// UTF-8/byte input buffer. The returned owned value still allocates or shares
/// storage for strings and collections.
pub fn from_utf8(input: &str) -> Result<Scalar> {
    from_utf8_with_limits(input, Limits::default())
}

/// Decode exactly one JSON value from borrowed UTF-8 text with explicit limits.
pub fn from_utf8_with_limits(input: &str, limits: Limits) -> Result<Scalar> {
    from_bytes_with_limits(input.as_bytes(), limits)
}

/// Decode exactly one JSON value from bytes.
pub fn from_bytes(input: &[u8]) -> Result<Scalar> {
    from_bytes_with_limits(input, Limits::default())
}

/// Decode exactly one JSON value from bytes with explicit limits.
pub fn from_bytes_with_limits(input: &[u8], limits: Limits) -> Result<Scalar> {
    check_input_size(input, limits, "json")?;
    if limits.max_documents() == 0 {
        return Err(codec_error(0, "document limit exceeded"));
    }
    let raw = parser::parse(input, limits)?;
    from_raw(raw, limits, "json")
}

/// Decode JSON and interpret the natural value under `field`.
pub fn from_utf8_with_field(input: &str, field: &Field) -> Result<Scalar> {
    from_utf8_with_field_and_limits(input, field, Limits::default())
}

/// Decode schema-directed JSON with explicit limits.
pub fn from_utf8_with_field_and_limits(
    input: &str,
    field: &Field,
    limits: Limits,
) -> Result<Scalar> {
    from_bytes_with_field_and_limits(input.as_bytes(), field, limits)
}

/// Decode JSON bytes and interpret the natural value under `field`.
pub fn from_bytes_with_field(input: &[u8], field: &Field) -> Result<Scalar> {
    from_bytes_with_field_and_limits(input, field, Limits::default())
}

/// Decode schema-directed JSON bytes with explicit limits.
pub fn from_bytes_with_field_and_limits(
    input: &[u8],
    field: &Field,
    limits: Limits,
) -> Result<Scalar> {
    field.from_natural_value(from_bytes_with_limits(input, limits)?)
}

/// Decode exactly one JSON value from a streaming reader.
pub fn from_reader<R: Read>(reader: R) -> Result<Scalar> {
    from_reader_with_limits(reader, Limits::default())
}

/// Decode exactly one JSON value from a streaming reader with explicit limits.
pub fn from_reader_with_limits<R: Read>(reader: R, limits: Limits) -> Result<Scalar> {
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
pub fn from_reader_with_field<R: Read>(reader: R, field: &Field) -> Result<Scalar> {
    from_reader_with_field_and_limits(reader, field, Limits::default())
}

/// Decode schema-directed JSON from a reader with explicit limits.
pub fn from_reader_with_field_and_limits<R: Read>(
    reader: R,
    field: &Field,
    limits: Limits,
) -> Result<Scalar> {
    field.from_natural_value(from_reader_with_limits(reader, limits)?)
}

/// Decode every whitespace-separated JSON value from bytes.
pub fn from_bytes_all(input: &[u8]) -> Result<Vec<Scalar>> {
    from_bytes_all_with_limits(input, Limits::default())
}

/// Decode every whitespace-separated JSON value from borrowed UTF-8 text.
pub fn from_utf8_all(input: &str) -> Result<Vec<Scalar>> {
    from_utf8_all_with_limits(input, Limits::default())
}

/// Decode every whitespace-separated JSON value from borrowed UTF-8 text with
/// explicit limits.
pub fn from_utf8_all_with_limits(input: &str, limits: Limits) -> Result<Vec<Scalar>> {
    from_bytes_all_with_limits(input.as_bytes(), limits)
}

/// Decode every whitespace-separated JSON value from bytes with explicit limits.
pub fn from_bytes_all_with_limits(input: &[u8], limits: Limits) -> Result<Vec<Scalar>> {
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
pub fn from_utf8_all_with_field(input: &str, field: &Field) -> Result<Vec<Scalar>> {
    from_utf8_all_with_field_and_limits(input, field, Limits::default())
}

/// Decode every schema-directed JSON value from UTF-8 with limits.
pub fn from_utf8_all_with_field_and_limits(
    input: &str,
    field: &Field,
    limits: Limits,
) -> Result<Vec<Scalar>> {
    from_bytes_all_with_field_and_limits(input.as_bytes(), field, limits)
}

/// Decode every JSON value from bytes under one field.
pub fn from_bytes_all_with_field(input: &[u8], field: &Field) -> Result<Vec<Scalar>> {
    from_bytes_all_with_field_and_limits(input, field, Limits::default())
}

/// Decode every schema-directed JSON value from bytes with limits.
pub fn from_bytes_all_with_field_and_limits(
    input: &[u8],
    field: &Field,
    limits: Limits,
) -> Result<Vec<Scalar>> {
    apply_field(from_bytes_all_with_limits(input, limits)?, field)
}

/// Decode every JSON value from a reader.
pub fn from_reader_all<R: Read>(reader: R) -> Result<Vec<Scalar>> {
    from_reader_all_with_limits(reader, Limits::default())
}

/// Decode every JSON value from a reader with explicit limits.
pub fn from_reader_all_with_limits<R: Read>(reader: R, limits: Limits) -> Result<Vec<Scalar>> {
    Reader::with_limits(reader, limits).collect()
}

/// Decode every JSON value from a reader under one field.
pub fn from_reader_all_with_field<R: Read>(reader: R, field: &Field) -> Result<Vec<Scalar>> {
    from_reader_all_with_field_and_limits(reader, field, Limits::default())
}

/// Decode every schema-directed JSON value from a reader with limits.
pub fn from_reader_all_with_field_and_limits<R: Read>(
    reader: R,
    field: &Field,
    limits: Limits,
) -> Result<Vec<Scalar>> {
    apply_field(from_reader_all_with_limits(reader, limits)?, field)
}

/// Lazily decode JSON values from a borrowed reader.
pub fn from_reader_iter<'a, R: Read + 'a>(reader: &'a mut R) -> ScalarIter<'a> {
    from_reader_iter_with_limits(reader, Limits::default())
}

/// Lazily decode JSON values from a borrowed reader with explicit limits.
pub fn from_reader_iter_with_limits<'a, R: Read + 'a>(
    reader: &'a mut R,
    limits: Limits,
) -> ScalarIter<'a> {
    ScalarIter::new(Reader::with_limits(reader, limits))
}

/// Lazily decode schema-directed JSON values from a reader.
pub fn from_reader_iter_with_field<'a, R: Read + 'a>(
    reader: &'a mut R,
    field: &'a Field,
) -> ScalarIter<'a> {
    from_reader_iter_with_field_and_limits(reader, field, Limits::default())
}

/// Lazily decode schema-directed JSON values with explicit limits.
pub fn from_reader_iter_with_field_and_limits<'a, R: Read + 'a>(
    reader: &'a mut R,
    field: &'a Field,
    limits: Limits,
) -> ScalarIter<'a> {
    ScalarIter::new(Reader::with_limits(reader, limits)).with_field(field)
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
    type Item = Result<Scalar>;

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
pub fn from_lines_bytes(input: &[u8]) -> Result<Vec<Scalar>> {
    from_lines_bytes_with_limits(input, Limits::default())
}

/// Decode strict newline-delimited JSON from borrowed UTF-8 text.
pub fn from_lines_utf8(input: &str) -> Result<Vec<Scalar>> {
    from_lines_utf8_with_limits(input, Limits::default())
}

/// Decode strict newline-delimited JSON from borrowed UTF-8 text with
/// explicit limits.
pub fn from_lines_utf8_with_limits(input: &str, limits: Limits) -> Result<Vec<Scalar>> {
    from_lines_bytes_with_limits(input.as_bytes(), limits)
}

/// Decode strict newline-delimited JSON bytes with explicit limits.
pub fn from_lines_bytes_with_limits(input: &[u8], limits: Limits) -> Result<Vec<Scalar>> {
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
pub fn from_lines_reader<R: Read>(reader: R) -> Result<Vec<Scalar>> {
    from_lines_reader_with_limits(reader, Limits::default())
}

/// Decode strict newline-delimited JSON from a reader with explicit limits.
pub fn from_lines_reader_with_limits<R: Read>(reader: R, limits: Limits) -> Result<Vec<Scalar>> {
    LinesReader::with_limits(reader, limits).collect()
}

/// Lazily decode strict newline-delimited JSON from a borrowed reader.
pub fn from_lines_reader_iter<'a, R: Read + 'a>(reader: &'a mut R) -> ScalarIter<'a> {
    from_lines_reader_iter_with_limits(reader, Limits::default())
}

/// Lazily decode strict newline-delimited JSON with explicit limits.
pub fn from_lines_reader_iter_with_limits<'a, R: Read + 'a>(
    reader: &'a mut R,
    limits: Limits,
) -> ScalarIter<'a> {
    ScalarIter::new(LinesReader::with_limits(reader, limits))
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
    type Item = Result<Scalar>;

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
pub fn into_bytes(value: &Scalar) -> Result<Vec<u8>> {
    into_bytes_with_formatting(value, Formatting::default())
}

/// Encode one value to JSON bytes laid out as `formatting` asks.
///
/// [`Indent::Default`](crate::text::Indent::Default) and [`Indent::None`](crate::text::Indent::None) are both today's compact output;
/// [`Indent::Spaces`](crate::text::Indent::Spaces) pretty-prints with that many spaces per nesting level,
/// exactly as another JSON formatter's `indent=n` option reads.
///
/// ```
/// use yggdryl::Scalar;
/// use yggdryl::text::Formatting;
///
/// # fn main() -> yggdryl::Result<()> {
/// let value = Scalar::from_struct([("id", Scalar::from(1))])?;
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
pub fn into_bytes_with_formatting(value: &Scalar, formatting: Formatting) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    into_writer_with_formatting(value, &mut output, formatting)?;
    Ok(output)
}

/// Encode one value as compact JSON UTF-8.
pub fn into_utf8(value: &Scalar) -> Result<String> {
    into_utf8_with_formatting(value, Formatting::default())
}

/// Encode one value as JSON UTF-8 with explicit formatting.
pub fn into_utf8_with_formatting(value: &Scalar, formatting: Formatting) -> Result<String> {
    String::from_utf8(into_bytes_with_formatting(value, formatting)?).map_err(|error| {
        Error::Codec {
            format: "json",
            position: error.utf8_error().valid_up_to(),
            reason: "encoded JSON is not valid UTF-8".into(),
        }
    })
}

/// Encode one value to a byte writer.
pub fn into_writer<W: Write>(value: &Scalar, writer: W) -> Result<()> {
    into_writer_with_formatting(value, writer, Formatting::default())
}

/// Encode one value to a byte writer, laid out as `formatting` asks.
///
/// # Errors
///
/// Returns the encoder's or the sink's failure.
pub fn into_writer_with_formatting<W: Write>(
    value: &Scalar,
    writer: W,
    formatting: Formatting,
) -> Result<()> {
    check_encode_depth(value, "json")?;
    write_one(writer, value, formatting)
}

/// Whether a datatype is a nested value text reaches through the JSON
/// document it spells: a struct, any serie layout, a map or a sorted map.
///
/// A union reads text through the member that takes it and a variant holds
/// text as the string it is, so neither is one; an encoding is not one
/// either, its values are, where every walk reaches them.
pub(crate) fn reads_json(dtype: &crate::DataType) -> bool {
    use crate::DataType as D;
    matches!(
        dtype,
        D::Struct(_)
            | D::Serie(_)
            | D::LargeSerie(_)
            | D::SerieView(_)
            | D::LargeSerieView(_)
            | D::FixedSizeSerie(..)
            | D::Map(_)
            | D::SortedMap(_)
    )
}

/// Whether a value is the JSON document `null` - text, or the bytes of it,
/// within JSON whitespace - entering a datatype that reads JSON: absence,
/// as an empty text cell is, so a scalar cast answers it exactly as a cast
/// of a column of them does.
pub(crate) fn is_null_document(target: &crate::DataType, value: &Scalar) -> bool {
    reads_json(crate::cast::text::encoded_value_of(target))
        && value
            .as_string()
            .map(|text| text.as_str().as_bytes())
            .or_else(|| value.as_binary().map(|bytes| bytes.as_bytes()))
            .is_some_and(|document| {
                let blank = |byte: &u8| matches!(byte, b' ' | b'\n' | b'\r' | b'\t');
                let start = document.iter().position(|byte| !blank(byte));
                let end = document.iter().rposition(|byte| !blank(byte));
                matches!((start, end), (Some(start), Some(end)) if &document[start..=end] == b"null")
            })
}

/// Read one JSON document as the natural value `dtype` then canonicalizes:
/// [`from_bytes_with_field`] for a scalar cast, which holds a datatype and
/// no field.
///
/// A document that is itself a string is refused rather than read again as
/// one: a nested value is spelled as an object or an array, never as text
/// holding one.
pub(crate) fn from_bytes_with_dtype(document: &[u8], dtype: &crate::DataType) -> Result<Scalar> {
    let value = from_bytes(document)?;
    if value.as_string().is_some() {
        return Err(Error::InvalidRecord {
            path: smol_str::SmolStr::new_static("$"),
            reason: smol_str::format_smolstr!(
                "expected a JSON object or array for {}, got a JSON string",
                dtype.name()
            ),
        });
    }
    crate::text::typed::prepare_dtype(value, dtype)
}

/// Append one canonical value of `dtype` to `output` as compact natural
/// JSON: the writing half of [`from_bytes_with_field`], which reads it back.
///
/// The value is written straight from its canonical shape - a struct row as
/// the object its field names key, in declaration order - so no natural
/// value is built per call. A datatype's nesting is bounded at its import
/// or its plan's compile far below the encoder's depth limit, and a variant
/// by its own decoder, so no depth walk runs per value either.
pub(crate) fn into_field_vec(
    value: &Scalar,
    dtype: &crate::DataType,
    output: &mut Vec<u8>,
) -> Result<()> {
    serde_json::to_writer(output, &wire::JsonField(value, dtype))
        .map_err(|error| codec_error(0, &error.to_string()))
}

/// Encode values as newline-delimited JSON bytes.
pub fn into_bytes_all(values: &[Scalar]) -> Result<Vec<u8>> {
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
pub fn into_bytes_all_with_formatting(
    values: &[Scalar],
    formatting: Formatting,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    into_writer_all_with_formatting(values, &mut output, formatting)?;
    Ok(output)
}

/// Encode values as newline-delimited JSON UTF-8.
pub fn into_utf8_all(values: &[Scalar]) -> Result<String> {
    into_utf8_all_with_formatting(values, Formatting::default())
}

/// Encode values as newline-delimited JSON UTF-8 with explicit formatting.
pub fn into_utf8_all_with_formatting(values: &[Scalar], formatting: Formatting) -> Result<String> {
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
    V: Borrow<Scalar>,
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
    V: Borrow<Scalar>,
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
fn write_one<W: Write>(writer: W, value: &Scalar, formatting: Formatting) -> Result<()> {
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

/// The cast engine's JSON kernels: a nested column written as the text or
/// bytes of the natural JSON each row spells, and text or bytes read back
/// as a nested column - the same two halves a CSV cell crosses.
pub(crate) mod casts {
    use std::sync::Arc;

    use arrow_array::builder::{BinaryViewBuilder, StringViewBuilder};
    use arrow_array::types::{GenericBinaryType, GenericStringType};
    use arrow_array::{
        Array, ArrayRef, BinaryArray, BinaryViewArray, FixedSizeBinaryArray, GenericByteArray,
        LargeBinaryArray, LargeStringArray, OffsetSizeTrait, StringArray, StringViewArray,
    };
    use arrow_buffer::{BooleanBuffer, BooleanBufferBuilder, Buffer, NullBuffer, OffsetBuffer};
    use arrow_schema::DataType as ArrowDataType;

    pub(crate) use super::field::FieldReader;
    use super::field::Pool;
    use crate::arrow::{Error, Result};
    use crate::budget::{MaterializationBudget, reserve_vec_bytes};
    use crate::cast::columns::is_exposed;
    use crate::cast::{downcast, internal_target_error, named_cell};
    use crate::serie::{Proof, Resolved, canonical_rows, land_planned_under};
    use crate::{Field, Scalar};

    /// Writes every exposed row of a nested column as its compact natural
    /// JSON, laid out as `layout` - one of the six variable text and byte
    /// layouts - with every other row null.
    ///
    /// The column lands once under `source`, resolved when the plan was
    /// compiled, under what the plan that produced `array` certified: a leaf
    /// it did not is proven at the landing, once and column by column. Its
    /// leaves are then narrowed once ([`JsonColumn`](super::column::JsonColumn))
    /// and each row written from them straight into one payload, with no
    /// value and no text built per row. A row the encoder refuses - a non-finite float, a map
    /// key JSON has no spelling for - is null under `safe` and an error
    /// naming the row otherwise.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_json_array(
        array: &ArrayRef,
        source: &Resolved,
        proof: &Proof,
        layout: &ArrowDataType,
        safe: bool,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        let rows = array.len();
        budget.add_array(field.dtype(), rows)?;
        reserve_vec_bytes::<usize>(budget, rows.saturating_add(1))?;
        // A row an ancestor hid is null to the landing, so it is neither
        // judged nor read.
        let hidden = exposure.map(|exposure| NullBuffer::new(exposure.clone()));
        let column = land_planned_under(source, Arc::clone(array), hidden.as_ref(), proof, budget)?;
        let spelled = super::column::JsonColumn::bind(&column)?;
        let mut payload = Vec::new();
        let mut ends = Vec::new();
        ends.try_reserve_exact(rows.saturating_add(1))
            .map_err(|error| allocation_failed(&error))?;
        ends.push(0);
        let mut validity = BooleanBufferBuilder::new(rows);
        for index in 0..rows {
            let start = payload.len();
            let present = is_exposed(exposure, index)
                && match column.is_null(index).and_then(|absent| {
                    if absent {
                        return Ok(false);
                    }
                    spelled.write(index, &mut payload).map(|()| true)
                }) {
                    Ok(present) => present,
                    // A refused row may have left half a document behind.
                    Err(_) if safe => {
                        payload.truncate(start);
                        false
                    }
                    Err(error) => return named_cell(field, index, Err(error)),
                };
            // Charged as it is written, so a column past the budget stops at
            // the row that crosses it rather than after the last.
            budget.add_bytes(payload.len() - start)?;
            validity.append(present);
            ends.push(payload.len());
        }
        let nulls =
            Some(NullBuffer::new(validity.finish())).filter(|nulls| nulls.null_count() != 0);
        Ok(match layout {
            ArrowDataType::Utf8 => Arc::new(StringArray::try_new(
                offsets(&ends, "utf8", field)?,
                Buffer::from_vec(payload),
                nulls,
            )?),
            ArrowDataType::LargeUtf8 => Arc::new(LargeStringArray::try_new(
                offsets(&ends, "large_utf8", field)?,
                Buffer::from_vec(payload),
                nulls,
            )?),
            ArrowDataType::Binary => Arc::new(BinaryArray::try_new(
                offsets(&ends, "binary", field)?,
                Buffer::from_vec(payload),
                nulls,
            )?),
            ArrowDataType::LargeBinary => Arc::new(LargeBinaryArray::try_new(
                offsets(&ends, "large_binary", field)?,
                Buffer::from_vec(payload),
                nulls,
            )?),
            // A view carries a prefix per cell rather than offsets, so it is
            // built cell by cell from the one payload.
            ArrowDataType::Utf8View => {
                let mut builder = StringViewBuilder::with_capacity(rows);
                for (index, cell) in ends.windows(2).enumerate() {
                    if nulls.as_ref().is_some_and(|nulls| nulls.is_null(index)) {
                        builder.append_null();
                    } else {
                        builder.append_value(
                            std::str::from_utf8(&payload[cell[0]..cell[1]])
                                .map_err(|_| internal_target_error("json"))?,
                        );
                    }
                }
                Arc::new(builder.finish())
            }
            ArrowDataType::BinaryView => {
                let mut builder = BinaryViewBuilder::with_capacity(rows);
                for (index, cell) in ends.windows(2).enumerate() {
                    if nulls.as_ref().is_some_and(|nulls| nulls.is_null(index)) {
                        builder.append_null();
                    } else {
                        builder.append_value(&payload[cell[0]..cell[1]]);
                    }
                }
                Arc::new(builder.finish())
            }
            _ => return Err(internal_target_error("json")),
        })
    }

    /// The payload ends as the offsets `O` states, refused by name when the
    /// payload is past what they reach.
    fn offsets<O: OffsetSizeTrait>(
        ends: &[usize],
        layout: &str,
        field: &Field,
    ) -> Result<OffsetBuffer<O>> {
        let offsets = ends
            .iter()
            .map(|&end| O::from_usize(end))
            .collect::<Option<Vec<O>>>()
            .ok_or_else(|| {
                Error::IncompatibleSchema(format!(
                    "field {:?}: the JSON its rows spell is {} bytes, past what a {layout} \
                     column's offsets reach",
                    field.name(),
                    ends.last().copied().unwrap_or_default(),
                ))
            })?;
        // The ends were pushed in order from zero, so they are the monotone
        // offsets the buffer asserts.
        Ok(OffsetBuffer::new(offsets.into()))
    }

    /// Reads every exposed cell of a text or byte column as one JSON
    /// document under `target` - the nullable field of the nested datatype
    /// the cast lands in - and lays the values out as its column.
    ///
    /// The reading half of [`render_json_array`]: each cell is read once,
    /// by `reader` - the target planned for documents, which reads one
    /// straight into its canonical row - where it answers, and otherwise by
    /// [`from_bytes_with_field`](super::from_bytes_with_field), the parse
    /// then the field's own value contract, which the plan answers exactly
    /// as. The canonical rows are laid out as they are, never checked again.
    /// A cell that is not JSON, or not a value of `target`, is null under
    /// `safe` and an error naming the row otherwise.
    pub(crate) fn ingest_json_array(
        array: &ArrayRef,
        target: &Field,
        reader: Option<&FieldReader>,
        safe: bool,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        let cells = Cells::of(array.as_ref())?;
        let rows = array.len();
        budget.add_array(field.dtype(), rows)?;
        reserve_vec_bytes::<Scalar>(budget, rows)?;
        if let Some(reader) = reader.filter(|reader| reader.reads_cells()) {
            return ingest_json_cells(&cells, rows, target, reader, safe, field, exposure, budget);
        }
        if let Some(reader) = reader.filter(|reader| reader.reads_items()) {
            return ingest_json_items(&cells, rows, target, reader, safe, field, exposure, budget);
        }
        let mut values = Vec::new();
        values
            .try_reserve_exact(rows)
            .map_err(|error| allocation_failed(&error))?;
        let mut pool = Pool::default();
        for index in 0..rows {
            let value = match is_exposed(exposure, index)
                .then(|| cells.get(index))
                .flatten()
            {
                // A cell with no byte holds no document: absence, as an
                // empty text cell is wherever text enters a column that is
                // not text, and the target's nullability says what follows.
                None | Some([]) => Scalar::Null,
                Some(document) => match reader
                    .and_then(|reader| reader.read(document, &mut pool))
                    .map_or_else(|| super::from_bytes_with_field(document, target), Ok)
                {
                    // The layout below charges a serie's or a map's offsets
                    // and not its items, so the rows are charged as read.
                    Ok(value) => {
                        budget.add_bytes(crate::arrow::scalar_memory_size(&value))?;
                        value
                    }
                    Err(_) if safe => Scalar::Null,
                    Err(error) => return Err(unread(field, index, document, error)),
                },
            };
            values.push(value);
        }
        let rows = values.iter().collect::<Vec<_>>();
        Ok(canonical_rows(target, &rows)?)
    }

    /// [`ingest_json_array`] for a struct target the reader plans: each
    /// document is read straight into one column per child - the rows a
    /// struct column is laid out from, never built as rows - and a document
    /// the reader leaves to the door is read there and its row's cells taken
    /// apart. Each row is charged what its row value would be.
    #[allow(clippy::too_many_arguments)]
    fn ingest_json_cells(
        cells: &Cells<'_>,
        rows: usize,
        target: &Field,
        reader: &FieldReader,
        safe: bool,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        let crate::DataType::Struct(fields) = target.dtype() else {
            return Err(internal_target_error("json"));
        };
        let mut columns = fields
            .iter()
            .map(|_| {
                let mut column = Vec::new();
                column
                    .try_reserve_exact(rows)
                    .map_err(|error| allocation_failed(&error))?;
                Ok(column)
            })
            .collect::<Result<Vec<Vec<Scalar>>>>()?;
        let mut present = Vec::with_capacity(rows);
        let mut pool = Pool::default();
        // A row the column holds as absent: a null cell in every child.
        let absent = |columns: &mut Vec<Vec<Scalar>>, present: &mut Vec<bool>| {
            columns
                .iter_mut()
                .for_each(|column| column.push(Scalar::Null));
            present.push(false);
        };
        for index in 0..rows {
            let Some(document) = is_exposed(exposure, index)
                .then(|| cells.get(index))
                .flatten()
                .filter(|document| !document.is_empty())
            else {
                absent(&mut columns, &mut present);
                continue;
            };
            match reader.read_cells(document, &mut pool, &mut columns) {
                Some(true) => {
                    budget.add_bytes(crate::arrow::size::run_memory_size(
                        columns.iter().filter_map(|column| column.last()),
                    ))?;
                    present.push(true);
                }
                Some(false) => {
                    budget.add_bytes(crate::arrow::scalar_memory_size(&Scalar::Null))?;
                    absent(&mut columns, &mut present);
                }
                None => match super::from_bytes_with_field(document, target) {
                    Ok(value) => {
                        budget.add_bytes(crate::arrow::scalar_memory_size(&value))?;
                        match value.sequence_rows() {
                            Some(row) if row.len() == columns.len() => {
                                for (column, cell) in columns.iter_mut().zip(row.iter()) {
                                    column.push(cell.clone());
                                }
                                present.push(true);
                            }
                            Some(_) => return Err(internal_target_error("json")),
                            None => absent(&mut columns, &mut present),
                        }
                    }
                    Err(_) if safe => absent(&mut columns, &mut present),
                    Err(error) => return Err(unread(field, index, document, error)),
                },
            }
        }
        crate::serie::value::struct_array_of_cells(target, &columns, &present)
    }

    /// [`ingest_json_array`] for a serie target the reader plans: each
    /// document's items are read straight onto one run - the items a serie
    /// column is laid out from, never built as rows - and a document the
    /// reader leaves to the door is read there and its row's items taken
    /// apart. Each row is charged what its row value would be.
    #[allow(clippy::too_many_arguments)]
    fn ingest_json_items(
        cells: &Cells<'_>,
        rows: usize,
        target: &Field,
        reader: &FieldReader,
        safe: bool,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        let mut items = Vec::new();
        let mut lengths = Vec::new();
        lengths
            .try_reserve_exact(rows)
            .map_err(|error| allocation_failed(&error))?;
        let mut pool = Pool::default();
        for index in 0..rows {
            let Some(document) = is_exposed(exposure, index)
                .then(|| cells.get(index))
                .flatten()
                .filter(|document| !document.is_empty())
            else {
                lengths.push(None);
                continue;
            };
            let held = items.len();
            match reader.read_items(document, &mut pool, &mut items) {
                Some(Some(count)) => {
                    budget.add_bytes(crate::arrow::size::run_memory_size(&items[held..]))?;
                    lengths.push(Some(count));
                }
                Some(None) => {
                    budget.add_bytes(crate::arrow::scalar_memory_size(&Scalar::Null))?;
                    lengths.push(None);
                }
                None => match super::from_bytes_with_field(document, target) {
                    Ok(value) => {
                        budget.add_bytes(crate::arrow::scalar_memory_size(&value))?;
                        match value.sequence_rows() {
                            Some(row) => {
                                items.extend(row.iter().cloned());
                                lengths.push(Some(row.len()));
                            }
                            None => lengths.push(None),
                        }
                    }
                    Err(_) if safe => lengths.push(None),
                    Err(error) => return Err(unread(field, index, document, error)),
                },
            }
        }
        crate::serie::value::serie_array_of_items(target, &items, &lengths)
    }

    /// A document the target does not read, named by its row and its text.
    fn unread(field: &Field, index: usize, document: &[u8], error: crate::Error) -> Error {
        let reason = match error {
            crate::Error::InvalidRecord { reason, .. } => reason.to_string(),
            other => other.to_string(),
        };
        Error::IncompatibleSchema(format!(
            "field {:?} row {index}: {:?} does not read as {}: {reason}",
            field.name(),
            crate::text::elide_to(
                &String::from_utf8_lossy(document),
                crate::text::ERROR_TEXT_LIMIT
            ),
            field.dtype(),
        ))
    }

    fn allocation_failed(error: &std::collections::TryReserveError) -> Error {
        Error::IncompatibleSchema(format!("JSON cast allocation failed: {error}"))
    }

    /// One cell's bytes, under whichever text or byte layout holds them.
    enum Cells<'a> {
        Utf8(&'a StringArray),
        LargeUtf8(&'a LargeStringArray),
        Utf8View(&'a StringViewArray),
        Binary(&'a BinaryArray),
        LargeBinary(&'a LargeBinaryArray),
        BinaryView(&'a BinaryViewArray),
        Fixed(&'a FixedSizeBinaryArray),
    }

    impl<'a> Cells<'a> {
        fn of(array: &'a dyn Array) -> Result<Self> {
            Ok(match array.data_type() {
                ArrowDataType::Utf8 => Self::Utf8(downcast(array)?),
                ArrowDataType::LargeUtf8 => Self::LargeUtf8(downcast(array)?),
                ArrowDataType::Utf8View => Self::Utf8View(downcast(array)?),
                ArrowDataType::Binary => Self::Binary(downcast(array)?),
                ArrowDataType::LargeBinary => Self::LargeBinary(downcast(array)?),
                ArrowDataType::BinaryView => Self::BinaryView(downcast(array)?),
                ArrowDataType::FixedSizeBinary(_) => Self::Fixed(downcast(array)?),
                other => {
                    return Err(Error::IncompatibleSchema(format!(
                        "expected a text or byte column of JSON documents, got {other:?}"
                    )));
                }
            })
        }

        /// The cell's bytes, `None` where it is null.
        fn get(&self, index: usize) -> Option<&'a [u8]> {
            fn bytes<T: arrow_array::types::ByteArrayType>(
                cells: &GenericByteArray<T>,
                index: usize,
            ) -> Option<&[u8]> {
                cells.is_valid(index).then(|| cells.value(index).as_ref())
            }
            match self {
                Self::Utf8(cells) => bytes::<GenericStringType<i32>>(cells, index),
                Self::LargeUtf8(cells) => bytes::<GenericStringType<i64>>(cells, index),
                Self::Binary(cells) => bytes::<GenericBinaryType<i32>>(cells, index),
                Self::LargeBinary(cells) => bytes::<GenericBinaryType<i64>>(cells, index),
                Self::Utf8View(cells) => {
                    cells.is_valid(index).then(|| cells.value(index).as_bytes())
                }
                Self::BinaryView(cells) => cells.is_valid(index).then(|| cells.value(index)),
                // A slot pads a shorter document with NUL, which no JSON
                // document ends with.
                Self::Fixed(cells) => cells
                    .is_valid(index)
                    .then(|| crate::trim_padding(cells.value(index))),
            }
        }
    }
}

fn parse_raw_document(input: &[u8], limits: Limits, base: usize) -> Result<Scalar> {
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
