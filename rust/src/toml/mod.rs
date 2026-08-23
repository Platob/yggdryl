//! Natural TOML values with bounded single-document I/O.

use std::borrow::Borrow;
use std::io::{Read, Write};

mod parser;
mod wire;

use crate::text::{Formatting, Limits, Scalar, ScalarIter, check_input_size};
use crate::{Error, Field, Result};

/// Maximum structural nesting accepted by the TOML parser.
pub const MAX_PARSER_DEPTH: usize = 64;

/// Decode one TOML document from UTF-8.
pub fn from_utf8(input: &str) -> Result<Scalar> {
    from_utf8_with_limits(input, Limits::default())
}

/// Decode one TOML document from UTF-8 with explicit limits.
pub fn from_utf8_with_limits(input: &str, limits: Limits) -> Result<Scalar> {
    check_input_size(input.as_bytes(), limits, "toml")?;
    parser::parse(input, limits)
}

/// Decode one TOML document from bytes.
pub fn from_bytes(input: &[u8]) -> Result<Scalar> {
    from_bytes_with_limits(input, Limits::default())
}

/// Decode one TOML document from bytes with explicit limits.
pub fn from_bytes_with_limits(input: &[u8], limits: Limits) -> Result<Scalar> {
    check_input_size(input, limits, "toml")?;
    let input = std::str::from_utf8(input).map_err(|error| Error::Codec {
        format: "toml",
        position: error.valid_up_to(),
        reason: "input is not valid UTF-8".into(),
    })?;
    parser::parse(input, limits)
}

/// Decode TOML UTF-8 and interpret its natural value under `field`.
pub fn from_utf8_with_field(input: &str, field: &Field) -> Result<Scalar> {
    from_utf8_with_field_and_limits(input, field, Limits::default())
}

/// Decode schema-directed TOML UTF-8 with explicit limits.
pub fn from_utf8_with_field_and_limits(
    input: &str,
    field: &Field,
    limits: Limits,
) -> Result<Scalar> {
    from_bytes_with_field_and_limits(input.as_bytes(), field, limits)
}

/// Decode TOML bytes and interpret the natural value under `field`.
pub fn from_bytes_with_field(input: &[u8], field: &Field) -> Result<Scalar> {
    from_bytes_with_field_and_limits(input, field, Limits::default())
}

/// Decode schema-directed TOML bytes with explicit limits.
pub fn from_bytes_with_field_and_limits(
    input: &[u8],
    field: &Field,
    limits: Limits,
) -> Result<Scalar> {
    field.from_natural_value(from_bytes_with_limits(input, limits)?)
}

/// Decode one TOML document from a byte reader.
pub fn from_reader<R: Read>(reader: R) -> Result<Scalar> {
    from_reader_with_limits(reader, Limits::default())
}

/// Decode one TOML document from a reader with explicit limits.
pub fn from_reader_with_limits<R: Read>(reader: R, limits: Limits) -> Result<Scalar> {
    let mut reader = Reader::with_limits(reader, limits);
    reader.next().unwrap_or_else(|| {
        Err(Error::Codec {
            format: "toml",
            position: 0,
            reason: "TOML reader did not yield its document".into(),
        })
    })
}

/// Decode TOML from a reader under `field`.
pub fn from_reader_with_field<R: Read>(reader: R, field: &Field) -> Result<Scalar> {
    from_reader_with_field_and_limits(reader, field, Limits::default())
}

/// Decode schema-directed TOML from a reader with explicit limits.
pub fn from_reader_with_field_and_limits<R: Read>(
    reader: R,
    field: &Field,
    limits: Limits,
) -> Result<Scalar> {
    field.from_natural_value(from_reader_with_limits(reader, limits)?)
}

/// Decode the single TOML document as a one-element collection.
pub fn from_utf8_all(input: &str) -> Result<Vec<Scalar>> {
    from_utf8_all_with_limits(input, Limits::default())
}

/// Decode the single TOML UTF-8 document with explicit limits.
pub fn from_utf8_all_with_limits(input: &str, limits: Limits) -> Result<Vec<Scalar>> {
    from_utf8_with_limits(input, limits).map(|value| vec![value])
}

/// Decode the single TOML byte document as a one-element collection.
pub fn from_bytes_all(input: &[u8]) -> Result<Vec<Scalar>> {
    from_bytes_all_with_limits(input, Limits::default())
}

/// Decode the single TOML byte document with explicit limits.
pub fn from_bytes_all_with_limits(input: &[u8], limits: Limits) -> Result<Vec<Scalar>> {
    from_bytes_with_limits(input, limits).map(|value| vec![value])
}

/// Decode the TOML UTF-8 document under `field`.
pub fn from_utf8_all_with_field(input: &str, field: &Field) -> Result<Vec<Scalar>> {
    from_utf8_all_with_field_and_limits(input, field, Limits::default())
}

/// Decode schema-directed TOML UTF-8 with explicit limits.
pub fn from_utf8_all_with_field_and_limits(
    input: &str,
    field: &Field,
    limits: Limits,
) -> Result<Vec<Scalar>> {
    from_utf8_with_field_and_limits(input, field, limits).map(|value| vec![value])
}

/// Decode the TOML byte document under `field`.
pub fn from_bytes_all_with_field(input: &[u8], field: &Field) -> Result<Vec<Scalar>> {
    from_bytes_all_with_field_and_limits(input, field, Limits::default())
}

/// Decode schema-directed TOML bytes with explicit limits.
pub fn from_bytes_all_with_field_and_limits(
    input: &[u8],
    field: &Field,
    limits: Limits,
) -> Result<Vec<Scalar>> {
    from_bytes_with_field_and_limits(input, field, limits).map(|value| vec![value])
}

/// Decode the reader's TOML document as a one-element collection.
pub fn from_reader_all<R: Read>(reader: R) -> Result<Vec<Scalar>> {
    from_reader_all_with_limits(reader, Limits::default())
}

/// Decode the reader's TOML document with explicit limits.
pub fn from_reader_all_with_limits<R: Read>(reader: R, limits: Limits) -> Result<Vec<Scalar>> {
    from_reader_with_limits(reader, limits).map(|value| vec![value])
}

/// Decode the reader's TOML document under `field`.
pub fn from_reader_all_with_field<R: Read>(reader: R, field: &Field) -> Result<Vec<Scalar>> {
    from_reader_all_with_field_and_limits(reader, field, Limits::default())
}

/// Decode schema-directed TOML from a reader with explicit limits.
pub fn from_reader_all_with_field_and_limits<R: Read>(
    reader: R,
    field: &Field,
    limits: Limits,
) -> Result<Vec<Scalar>> {
    from_reader_with_field_and_limits(reader, field, limits).map(|value| vec![value])
}

/// Lazily decode the one TOML document from a borrowed reader.
pub fn from_reader_iter<'a, R: Read + 'a>(reader: &'a mut R) -> ScalarIter<'a> {
    from_reader_iter_with_limits(reader, Limits::default())
}

/// Lazily decode the one TOML document with explicit limits.
pub fn from_reader_iter_with_limits<'a, R: Read + 'a>(
    reader: &'a mut R,
    limits: Limits,
) -> ScalarIter<'a> {
    ScalarIter::new(Reader::with_limits(reader, limits))
}

/// Lazily decode the TOML document under `field`.
pub fn from_reader_iter_with_field<'a, R: Read + 'a>(
    reader: &'a mut R,
    field: &'a Field,
) -> ScalarIter<'a> {
    from_reader_iter_with_field_and_limits(reader, field, Limits::default())
}

/// Lazily decode schema-directed TOML with explicit limits.
pub fn from_reader_iter_with_field_and_limits<'a, R: Read + 'a>(
    reader: &'a mut R,
    field: &'a Field,
    limits: Limits,
) -> ScalarIter<'a> {
    ScalarIter::new(Reader::with_limits(reader, limits)).with_field(field)
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
    type Item = Result<Scalar>;

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
        let mut reader = reader.take(u64::try_from(maximum.saturating_add(1)).unwrap_or(u64::MAX));
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
            from_bytes_with_limits(&input, self.limits)
        }))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = usize::from(self.reader.is_some());
        (remaining, Some(remaining))
    }
}

impl<R: Read> ExactSizeIterator for Reader<R> {}

/// Encode one value as TOML bytes.
pub fn into_bytes(value: &Scalar) -> Result<Vec<u8>> {
    into_bytes_with_formatting(value, Formatting::default())
}

/// Encode one value as TOML bytes with explicit formatting.
pub fn into_bytes_with_formatting(value: &Scalar, formatting: Formatting) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    into_writer_with_formatting(value, &mut output, formatting)?;
    Ok(output)
}

/// Encode one value as TOML UTF-8.
pub fn into_utf8(value: &Scalar) -> Result<String> {
    into_utf8_with_formatting(value, Formatting::default())
}

/// Encode one value as TOML UTF-8 with explicit formatting.
pub fn into_utf8_with_formatting(value: &Scalar, formatting: Formatting) -> Result<String> {
    String::from_utf8(into_bytes_with_formatting(value, formatting)?).map_err(|error| {
        Error::Codec {
            format: "toml",
            position: error.utf8_error().valid_up_to(),
            reason: "encoded TOML is not valid UTF-8".into(),
        }
    })
}

/// Validate the natural TOML projection before opening a destination.
pub fn validate_for_write(value: &Scalar) -> Result<()> {
    validate_for_write_with_limits(value, Limits::default())
}

/// Validate the natural TOML projection against explicit limits.
pub fn validate_for_write_with_limits(value: &Scalar, limits: Limits) -> Result<()> {
    wire::check_depth(value, limits.max_depth())
}

/// Encode one value to a byte writer.
pub fn into_writer<W: Write>(value: &Scalar, writer: W) -> Result<()> {
    into_writer_with_formatting(value, writer, Formatting::default())
}

/// Encode one value to a byte writer with explicit formatting.
pub fn into_writer_with_formatting<W: Write>(
    value: &Scalar,
    mut writer: W,
    formatting: Formatting,
) -> Result<()> {
    validate_for_write(value)?;
    wire::write_document(&mut writer, value, formatting.into())
}

/// Encode exactly one value as TOML bytes.
pub fn into_bytes_all(values: &[Scalar]) -> Result<Vec<u8>> {
    into_bytes_all_with_formatting(values, Formatting::default())
}

/// Encode exactly one value as TOML bytes with explicit formatting.
pub fn into_bytes_all_with_formatting(
    values: &[Scalar],
    formatting: Formatting,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    into_writer_all_with_formatting(values, &mut output, formatting)?;
    Ok(output)
}

/// Encode exactly one value as TOML UTF-8.
pub fn into_utf8_all(values: &[Scalar]) -> Result<String> {
    into_utf8_all_with_formatting(values, Formatting::default())
}

/// Encode exactly one value as TOML UTF-8 with explicit formatting.
pub fn into_utf8_all_with_formatting(values: &[Scalar], formatting: Formatting) -> Result<String> {
    String::from_utf8(into_bytes_all_with_formatting(values, formatting)?).map_err(|error| {
        Error::Codec {
            format: "toml",
            position: error.utf8_error().valid_up_to(),
            reason: "encoded TOML is not valid UTF-8".into(),
        }
    })
}

/// Encode exactly one value to a TOML writer.
pub fn into_writer_all<W, I, V>(values: I, writer: W) -> Result<()>
where
    W: Write,
    I: IntoIterator<Item = V>,
    V: Borrow<Scalar>,
{
    into_writer_all_with_formatting(values, writer, Formatting::default())
}

/// Encode exactly one value to a TOML writer with explicit formatting.
pub fn into_writer_all_with_formatting<W, I, V>(
    values: I,
    writer: W,
    formatting: Formatting,
) -> Result<()>
where
    W: Write,
    I: IntoIterator<Item = V>,
    V: Borrow<Scalar>,
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
    into_writer_with_formatting(value.borrow(), writer, formatting)
}
