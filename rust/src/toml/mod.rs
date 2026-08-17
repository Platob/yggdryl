//! TOML 1.1 value encoding, typed envelopes, and bounded single-document I/O.

use std::borrow::Borrow;
use std::io::{Read, Write};

mod parser;
mod wire;

use crate::text::{Limits, Value, ValueIter, check_input_size};
use crate::{Error, Result};

/// Maximum structural nesting accepted by TOML parsing and conversion.
///
/// The pinned parser has its own recursion guard at 80. Yggdryl's lower public
/// ceiling leaves headroom for parser bookkeeping and keeps conversion stack
/// usage conservative even when caller limits are larger.
pub const MAX_PARSER_DEPTH: usize = 64;

/// Decode one TOML document from borrowed UTF-8 text.
///
/// TOML's root is always a table. Empty and comment-only documents therefore
/// decode as an empty [`Value::Mapping`].
pub fn from_str(input: &str) -> Result<Value> {
    from_str_with_limits(input, Limits::default())
}

/// Decode one TOML document from borrowed text with explicit limits.
pub fn from_str_with_limits(input: &str, limits: Limits) -> Result<Value> {
    check_input_size(input.as_bytes(), limits, "toml")?;
    parser::parse(input, limits)
}

/// Decode one TOML document from UTF-8 bytes.
pub fn from_slice(input: &[u8]) -> Result<Value> {
    from_slice_with_limits(input, Limits::default())
}

/// Decode one TOML document from bytes with explicit limits.
pub fn from_slice_with_limits(input: &[u8], limits: Limits) -> Result<Value> {
    check_input_size(input, limits, "toml")?;
    let input = std::str::from_utf8(input).map_err(|error| Error::Codec {
        format: "toml",
        position: error.valid_up_to(),
        reason: "input is not valid UTF-8".into(),
    })?;
    parser::parse(input, limits)
}

/// Decode one TOML document from a byte reader.
///
/// The maintained TOML parser accepts contiguous UTF-8 input, so this adapter
/// reads into one explicitly byte-bounded buffer before parsing.
pub fn from_reader<R: Read>(reader: R) -> Result<Value> {
    from_reader_with_limits(reader, Limits::default())
}

/// Decode one TOML document from a reader with explicit limits.
pub fn from_reader_with_limits<R: Read>(reader: R, limits: Limits) -> Result<Value> {
    let mut reader = Reader::with_limits(reader, limits);
    reader.next().unwrap_or_else(|| {
        Err(Error::Codec {
            format: "toml",
            position: 0,
            reason: "TOML reader did not yield its single document".into(),
        })
    })
}

/// Decode the single TOML document as a one-element collection.
pub fn from_str_all(input: &str) -> Result<Vec<Value>> {
    from_str_all_with_limits(input, Limits::default())
}

/// Decode the single TOML document as a one-element collection with limits.
pub fn from_str_all_with_limits(input: &str, limits: Limits) -> Result<Vec<Value>> {
    from_str_with_limits(input, limits).map(|value| vec![value])
}

/// Decode the single TOML document as a one-element collection.
pub fn from_slice_all(input: &[u8]) -> Result<Vec<Value>> {
    from_slice_all_with_limits(input, Limits::default())
}

/// Decode the single TOML document as a one-element collection with limits.
pub fn from_slice_all_with_limits(input: &[u8], limits: Limits) -> Result<Vec<Value>> {
    from_slice_with_limits(input, limits).map(|value| vec![value])
}

/// Decode the single reader document as a one-element collection.
pub fn from_reader_all<R: Read>(reader: R) -> Result<Vec<Value>> {
    from_reader_all_with_limits(reader, Limits::default())
}

/// Decode the single reader document as a one-element collection with limits.
pub fn from_reader_all_with_limits<R: Read>(reader: R, limits: Limits) -> Result<Vec<Value>> {
    from_reader_with_limits(reader, limits).map(|value| vec![value])
}

/// Lazily decode the one TOML document from a borrowed byte reader.
pub fn from_reader_iter<'a, R: Read + 'a>(reader: &'a mut R) -> ValueIter<'a> {
    from_reader_iter_with_limits(reader, Limits::default())
}

/// Lazily decode the one TOML document with explicit limits.
pub fn from_reader_iter_with_limits<'a, R: Read + 'a>(
    reader: &'a mut R,
    limits: Limits,
) -> ValueIter<'a> {
    ValueIter::new(Reader::with_limits(reader, limits))
}

/// An owning lazy iterator that yields exactly one TOML document.
pub struct Reader<R: Read> {
    reader: Option<R>,
    limits: Limits,
    byte_offset: usize,
}

impl<R: Read> Reader<R> {
    /// Construct a single-document reader with default limits.
    pub fn new(reader: R) -> Self {
        Self::with_limits(reader, Limits::default())
    }

    /// Construct a single-document reader with explicit limits.
    pub const fn with_limits(reader: R, limits: Limits) -> Self {
        Self {
            reader: Some(reader),
            limits,
            byte_offset: 0,
        }
    }

    /// Return the number of bytes pulled from the source.
    pub const fn byte_offset(&self) -> usize {
        self.byte_offset
    }
}

impl<R: Read> Iterator for Reader<R> {
    type Item = Result<Value>;

    fn next(&mut self) -> Option<Self::Item> {
        let reader = self.reader.take()?;
        if self.limits.max_documents() == 0 {
            return Some(Err(Error::Codec {
                format: "toml",
                position: 0,
                reason: "document limit exceeded".into(),
            }));
        }
        let maximum = self.limits.max_input_bytes();
        let read_limit = maximum.saturating_add(1);
        let mut reader = reader.take(u64::try_from(read_limit).unwrap_or(u64::MAX));
        let mut input = Vec::with_capacity(maximum.min(8 * 1024));
        let result = reader.read_to_end(&mut input).map_err(Error::from);
        self.byte_offset = input.len();
        Some(result.and_then(|_| {
            if input.len() > maximum {
                return Err(Error::Codec {
                    format: "toml",
                    position: maximum,
                    reason: "input byte limit exceeded".into(),
                });
            }
            from_slice_with_limits(&input, self.limits)
        }))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = usize::from(self.reader.is_some());
        (remaining, Some(remaining))
    }
}

impl<R: Read> ExactSizeIterator for Reader<R> {}

/// Encode one value as a TOML document in a new byte vector.
pub fn to_vec(value: &Value) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    to_writer(&mut output, value)?;
    Ok(output)
}

/// Consume and encode one value as a TOML document.
pub fn into_vec(value: Value) -> Result<Vec<u8>> {
    to_vec(&value)
}

/// Validate the exact TOML wire projection before opening a destination.
///
/// Typed envelopes can add containers beyond the source [`Value`] depth.
/// Redirected bindings use this preflight before creating or truncating a
/// destination, while [`to_writer`] repeats it to keep the direct API safe.
pub fn validate_for_write(value: &Value) -> Result<()> {
    validate_for_write_with_limits(value, Limits::default())
}

/// Validate the exact TOML wire projection against explicit limits.
///
/// The effective maximum is the smaller of `limits.max_depth()` and
/// [`MAX_PARSER_DEPTH`]. This is useful when a runtime exposes a lower nesting
/// ceiling than the Rust core default.
pub fn validate_for_write_with_limits(value: &Value, limits: Limits) -> Result<()> {
    wire::check_depth(value, limits.max_depth())
}

/// Encode one value as a TOML document to a byte writer.
pub fn to_writer<W: Write>(mut writer: W, value: &Value) -> Result<()> {
    validate_for_write(value)?;
    wire::write_document(&mut writer, value)
}

/// Encode exactly one value as a TOML document in a new byte vector.
pub fn to_vec_all(values: &[Value]) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    to_writer_all(&mut output, values)?;
    Ok(output)
}

/// Encode exactly one value to a TOML writer.
///
/// TOML has no multi-document stream syntax. Zero values and a second value
/// are rejected before any bytes are written.
pub fn to_writer_all<W, I, V>(writer: W, values: I) -> Result<()>
where
    W: Write,
    I: IntoIterator<Item = V>,
    V: Borrow<Value>,
{
    let mut values = values.into_iter();
    let value = values.next().ok_or_else(|| Error::Codec {
        format: "toml",
        position: 0,
        reason: "expected exactly one value for a TOML document".into(),
    })?;
    if values.next().is_some() {
        return Err(Error::Codec {
            format: "toml",
            position: 0,
            reason: "TOML does not support multiple documents".into(),
        });
    }
    to_writer(writer, value.borrow())
}
