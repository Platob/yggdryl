//! Shared natural structured-text values and format dispatch.

use std::borrow::Borrow;
use std::io::{Read, Write};

mod codec;
mod display;
mod format;
mod formatting;
mod io;
mod limits;
pub mod line;
mod loading;
mod placeholder;
pub(crate) mod position;
pub(crate) mod typed;
pub(crate) mod wire;

pub use crate::generic::TypedScalar;
pub use crate::generic::scalar::{Children, Float16, Float32, Float64, Scalar};
pub use codec::{Json, Jsonl, Limited, TextCodec, Toml, Yaml};
pub(crate) use display::ERROR_TEXT_LIMIT;
pub(crate) use display::{
    elide_display, elide_to, expected_got, stable_hash_display, stable_hash_of,
};
pub use display::{stable_hash_bytes, stable_hash_chunks};
pub use format::Format;
pub use formatting::{Formatting, Indent};
pub use io::{
    Plan, from_io, from_io_all, from_io_all_with_limits, from_io_with, from_io_with_field,
    from_io_with_field_and_limits, from_io_with_limits, into_io, into_io_all,
    into_io_all_with_formatting, into_io_with_formatting, into_io_with_level,
};
pub use limits::Limits;
pub use line::{LineSep, Text, TextOptions};
pub use loading::Loading;
pub use placeholder::Placeholders;

use crate::{Error, Field, Result, json, toml, yaml};

/// A lazy iterator over decoded values.
pub struct ScalarIter<'a> {
    inner: Box<dyn Iterator<Item = Result<Scalar>> + 'a>,
    field: Option<&'a Field>,
}

impl<'a> ScalarIter<'a> {
    pub(crate) fn new(iterator: impl Iterator<Item = Result<Scalar>> + 'a) -> Self {
        Self {
            inner: Box::new(iterator),
            field: None,
        }
    }

    /// Interpret every yielded natural value under one field without
    /// materializing the iterator.
    pub(crate) fn with_field(mut self, field: &'a Field) -> Self {
        self.field = Some(field);
        self
    }
}

impl Iterator for ScalarIter<'_> {
    type Item = Result<Scalar>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|value| match self.field {
            Some(field) => value.and_then(|value| field.from_natural_value(value)),
            None => value,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

fn filled(value: Scalar, input: &[u8], format: Format, loading: &Loading) -> Result<Scalar> {
    let value = if let Some(placeholders) = loading.placeholders() {
        if matches!(format, Format::Json | Format::JsonLines) {
            return Err(Error::InvalidRecord {
                path: "$.placeholders".into(),
                reason: expected_got(
                    "a format with placeholder support (yaml, toml)",
                    format.as_str(),
                ),
            });
        }
        if placeholder::present(input) {
            placeholder::substitute(value, placeholders)?
        } else {
            value
        }
    } else {
        value
    };
    match loading.field() {
        Some(field) => field.from_natural_value(value),
        None => Ok(value),
    }
}

/// Decode one value from UTF-8 using `format`.
pub fn from_utf8(input: &str, format: Format) -> Result<Scalar> {
    from_utf8_with_limits(input, format, Limits::default())
}

/// Decode one value from UTF-8 with explicit limits.
pub fn from_utf8_with_limits(input: &str, format: Format, limits: Limits) -> Result<Scalar> {
    match format {
        Format::Json => json::from_utf8_with_limits(input, limits),
        Format::JsonLines => {
            json::from_lines_utf8_with_limits(input, limits).map(Scalar::from_sequence)
        }
        Format::Yaml => yaml::from_utf8_with_limits(input, limits),
        Format::Toml => toml::from_utf8_with_limits(input, limits),
    }
}

/// Decode UTF-8 and interpret the natural value under `field`.
pub fn from_utf8_with_field(input: &str, format: Format, field: &Field) -> Result<Scalar> {
    from_utf8_with_field_and_limits(input, format, field, Limits::default())
}

/// Decode schema-directed UTF-8 with explicit limits.
pub fn from_utf8_with_field_and_limits(
    input: &str,
    format: Format,
    field: &Field,
    limits: Limits,
) -> Result<Scalar> {
    field.from_natural_value(from_utf8_with_limits(input, format, limits)?)
}

/// Decode one value from UTF-8 under `loading`.
pub fn from_utf8_with(input: &str, format: Format, loading: &Loading) -> Result<Scalar> {
    let value = from_utf8_with_limits(input, format, loading.limits())?;
    filled(value, input.as_bytes(), format, loading)
}

/// Decode one value from bytes using `format`.
pub fn from_bytes(input: &[u8], format: Format) -> Result<Scalar> {
    from_bytes_with_limits(input, format, Limits::default())
}

/// Decode one value from bytes with explicit limits.
pub fn from_bytes_with_limits(input: &[u8], format: Format, limits: Limits) -> Result<Scalar> {
    match format {
        Format::Json => json::from_bytes_with_limits(input, limits),
        Format::JsonLines => {
            json::from_lines_bytes_with_limits(input, limits).map(Scalar::from_sequence)
        }
        Format::Yaml => yaml::from_bytes_with_limits(input, limits),
        Format::Toml => toml::from_bytes_with_limits(input, limits),
    }
}

/// Decode bytes and interpret the natural value under `field`.
pub fn from_bytes_with_field(input: &[u8], format: Format, field: &Field) -> Result<Scalar> {
    from_bytes_with_field_and_limits(input, format, field, Limits::default())
}

/// Decode schema-directed bytes with explicit limits.
pub fn from_bytes_with_field_and_limits(
    input: &[u8],
    format: Format,
    field: &Field,
    limits: Limits,
) -> Result<Scalar> {
    field.from_natural_value(from_bytes_with_limits(input, format, limits)?)
}

/// Decode one value from bytes under `loading`.
pub fn from_bytes_with(input: &[u8], format: Format, loading: &Loading) -> Result<Scalar> {
    let value = from_bytes_with_limits(input, format, loading.limits())?;
    filled(value, input, format, loading)
}

/// Decode one value from a standard byte reader.
pub fn from_reader<R: Read>(reader: R, format: Format) -> Result<Scalar> {
    from_reader_with_limits(reader, format, Limits::default())
}

/// Decode one value from a reader with explicit limits.
pub fn from_reader_with_limits<R: Read>(
    reader: R,
    format: Format,
    limits: Limits,
) -> Result<Scalar> {
    match format {
        Format::Json => json::from_reader_with_limits(reader, limits),
        Format::JsonLines => {
            json::from_lines_reader_with_limits(reader, limits).map(Scalar::from_sequence)
        }
        Format::Yaml => yaml::from_reader_with_limits(reader, limits),
        Format::Toml => toml::from_reader_with_limits(reader, limits),
    }
}

/// Decode a reader and interpret the natural value under `field`.
pub fn from_reader_with_field<R: Read>(reader: R, format: Format, field: &Field) -> Result<Scalar> {
    from_reader_with_field_and_limits(reader, format, field, Limits::default())
}

/// Decode a schema-directed reader with explicit limits.
pub fn from_reader_with_field_and_limits<R: Read>(
    reader: R,
    format: Format,
    field: &Field,
    limits: Limits,
) -> Result<Scalar> {
    field.from_natural_value(from_reader_with_limits(reader, format, limits)?)
}

/// Decode one value from a reader under `loading`.
pub fn from_reader_with<R: Read>(reader: R, format: Format, loading: &Loading) -> Result<Scalar> {
    if loading.placeholders().is_none() {
        let value = from_reader_with_limits(reader, format, loading.limits())?;
        return match loading.field() {
            Some(field) => field.from_natural_value(value),
            None => Ok(value),
        };
    }
    let mut bytes = Vec::new();
    reader
        .take(loading.limits().max_input_bytes().saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    from_bytes_with(&bytes, format, loading)
}

/// Decode all values from bytes.
pub fn from_bytes_all(input: &[u8], format: Format) -> Result<Vec<Scalar>> {
    from_bytes_all_with_limits(input, format, Limits::default())
}

/// Decode all values from bytes with explicit limits.
pub fn from_bytes_all_with_limits(
    input: &[u8],
    format: Format,
    limits: Limits,
) -> Result<Vec<Scalar>> {
    match format {
        Format::Json => json::from_bytes_all_with_limits(input, limits),
        Format::JsonLines => json::from_lines_bytes_with_limits(input, limits),
        Format::Yaml => yaml::from_bytes_all_with_limits(input, limits),
        Format::Toml => toml::from_bytes_all_with_limits(input, limits),
    }
}

/// Decode all byte documents under `field`.
pub fn from_bytes_all_with_field(
    input: &[u8],
    format: Format,
    field: &Field,
) -> Result<Vec<Scalar>> {
    from_bytes_all_with_field_and_limits(input, format, field, Limits::default())
}

/// Decode all schema-directed byte documents with explicit limits.
pub fn from_bytes_all_with_field_and_limits(
    input: &[u8],
    format: Format,
    field: &Field,
    limits: Limits,
) -> Result<Vec<Scalar>> {
    apply_field(from_bytes_all_with_limits(input, format, limits)?, field)
}

/// Decode all values from UTF-8.
pub fn from_utf8_all(input: &str, format: Format) -> Result<Vec<Scalar>> {
    from_utf8_all_with_limits(input, format, Limits::default())
}

/// Decode all values from UTF-8 with explicit limits.
pub fn from_utf8_all_with_limits(
    input: &str,
    format: Format,
    limits: Limits,
) -> Result<Vec<Scalar>> {
    match format {
        Format::Json => json::from_utf8_all_with_limits(input, limits),
        Format::JsonLines => json::from_lines_utf8_with_limits(input, limits),
        Format::Yaml => yaml::from_utf8_all_with_limits(input, limits),
        Format::Toml => toml::from_utf8_all_with_limits(input, limits),
    }
}

/// Decode all UTF-8 documents under `field`.
pub fn from_utf8_all_with_field(input: &str, format: Format, field: &Field) -> Result<Vec<Scalar>> {
    from_utf8_all_with_field_and_limits(input, format, field, Limits::default())
}

/// Decode all schema-directed UTF-8 documents with explicit limits.
pub fn from_utf8_all_with_field_and_limits(
    input: &str,
    format: Format,
    field: &Field,
    limits: Limits,
) -> Result<Vec<Scalar>> {
    apply_field(from_utf8_all_with_limits(input, format, limits)?, field)
}

/// Decode all values from a reader.
pub fn from_reader_all<R: Read>(reader: R, format: Format) -> Result<Vec<Scalar>> {
    from_reader_all_with_limits(reader, format, Limits::default())
}

/// Decode all reader documents with explicit limits.
pub fn from_reader_all_with_limits<R: Read>(
    reader: R,
    format: Format,
    limits: Limits,
) -> Result<Vec<Scalar>> {
    match format {
        Format::Json => json::from_reader_all_with_limits(reader, limits),
        Format::JsonLines => json::from_lines_reader_with_limits(reader, limits),
        Format::Yaml => yaml::from_reader_all_with_limits(reader, limits),
        Format::Toml => toml::from_reader_all_with_limits(reader, limits),
    }
}

/// Decode all reader documents under `field`.
pub fn from_reader_all_with_field<R: Read>(
    reader: R,
    format: Format,
    field: &Field,
) -> Result<Vec<Scalar>> {
    from_reader_all_with_field_and_limits(reader, format, field, Limits::default())
}

/// Decode all schema-directed reader documents with explicit limits.
pub fn from_reader_all_with_field_and_limits<R: Read>(
    reader: R,
    format: Format,
    field: &Field,
    limits: Limits,
) -> Result<Vec<Scalar>> {
    apply_field(from_reader_all_with_limits(reader, format, limits)?, field)
}

/// Lazily decode values from a borrowed reader.
pub fn from_reader_iter<'a, R: Read + 'a>(reader: &'a mut R, format: Format) -> ScalarIter<'a> {
    from_reader_iter_with_limits(reader, format, Limits::default())
}

/// Lazily decode values with explicit limits.
pub fn from_reader_iter_with_limits<'a, R: Read + 'a>(
    reader: &'a mut R,
    format: Format,
    limits: Limits,
) -> ScalarIter<'a> {
    match format {
        Format::Json => json::from_reader_iter_with_limits(reader, limits),
        Format::JsonLines => json::from_lines_reader_iter_with_limits(reader, limits),
        Format::Yaml => yaml::from_reader_iter_with_limits(reader, limits),
        Format::Toml => toml::from_reader_iter_with_limits(reader, limits),
    }
}

/// Lazily decode values under `field`.
pub fn from_reader_iter_with_field<'a, R: Read + 'a>(
    reader: &'a mut R,
    format: Format,
    field: &'a Field,
) -> ScalarIter<'a> {
    from_reader_iter_with_field_and_limits(reader, format, field, Limits::default())
}

/// Lazily decode schema-directed values with explicit limits.
pub fn from_reader_iter_with_field_and_limits<'a, R: Read + 'a>(
    reader: &'a mut R,
    format: Format,
    field: &'a Field,
    limits: Limits,
) -> ScalarIter<'a> {
    from_reader_iter_with_limits(reader, format, limits).with_field(field)
}

/// Encode one value to bytes.
pub fn into_bytes(value: &Scalar, format: Format) -> Result<Vec<u8>> {
    into_bytes_with_formatting(value, format, Formatting::default())
}

/// Encode one value to bytes with explicit formatting.
pub fn into_bytes_with_formatting(
    value: &Scalar,
    format: Format,
    formatting: Formatting,
) -> Result<Vec<u8>> {
    match format {
        Format::Json => json::into_bytes_with_formatting(value, formatting),
        Format::JsonLines => match value {
            Scalar::Sequence(values) => json::into_bytes_all_with_formatting(values, formatting),
            value => json::into_bytes_all_with_formatting(std::slice::from_ref(value), formatting),
        },
        Format::Yaml => yaml::into_bytes_with_formatting(value, formatting),
        Format::Toml => toml::into_bytes_with_formatting(value, formatting),
    }
}

/// Encode one value to UTF-8.
pub fn into_utf8(value: &Scalar, format: Format) -> Result<String> {
    into_utf8_with_formatting(value, format, Formatting::default())
}

/// Encode one value to UTF-8 with explicit formatting.
pub fn into_utf8_with_formatting(
    value: &Scalar,
    format: Format,
    formatting: Formatting,
) -> Result<String> {
    match format {
        Format::Json => json::into_utf8_with_formatting(value, formatting),
        Format::JsonLines => match value {
            Scalar::Sequence(values) => json::into_utf8_all_with_formatting(values, formatting),
            value => json::into_utf8_all_with_formatting(std::slice::from_ref(value), formatting),
        },
        Format::Yaml => yaml::into_utf8_with_formatting(value, formatting),
        Format::Toml => toml::into_utf8_with_formatting(value, formatting),
    }
}

/// Encode one value to a standard byte writer.
pub fn into_writer<W: Write>(value: &Scalar, writer: W, format: Format) -> Result<()> {
    into_writer_with_formatting(value, writer, format, Formatting::default())
}

/// Encode one value to a writer with explicit formatting.
pub fn into_writer_with_formatting<W: Write>(
    value: &Scalar,
    writer: W,
    format: Format,
    formatting: Formatting,
) -> Result<()> {
    match format {
        Format::Json => json::into_writer_with_formatting(value, writer, formatting),
        Format::JsonLines => match value {
            Scalar::Sequence(values) => {
                json::into_writer_all_with_formatting(values.iter(), writer, formatting)
            }
            value => json::into_writer_all_with_formatting(
                std::slice::from_ref(value),
                writer,
                formatting,
            ),
        },
        Format::Yaml => yaml::into_writer_with_formatting(value, writer, formatting),
        Format::Toml => toml::into_writer_with_formatting(value, writer, formatting),
    }
}

/// Encode values to bytes.
pub fn into_bytes_all(values: &[Scalar], format: Format) -> Result<Vec<u8>> {
    into_bytes_all_with_formatting(values, format, Formatting::default())
}

/// Encode values to bytes with explicit formatting.
pub fn into_bytes_all_with_formatting(
    values: &[Scalar],
    format: Format,
    formatting: Formatting,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    into_writer_all_with_formatting(values, &mut output, format, formatting)?;
    Ok(output)
}

/// Encode values to UTF-8.
pub fn into_utf8_all(values: &[Scalar], format: Format) -> Result<String> {
    into_utf8_all_with_formatting(values, format, Formatting::default())
}

/// Encode values to UTF-8 with explicit formatting.
pub fn into_utf8_all_with_formatting(
    values: &[Scalar],
    format: Format,
    formatting: Formatting,
) -> Result<String> {
    String::from_utf8(into_bytes_all_with_formatting(values, format, formatting)?).map_err(
        |error| Error::Codec {
            format: format.as_str(),
            position: error.utf8_error().valid_up_to(),
            reason: "encoded output is not valid UTF-8".into(),
        },
    )
}

/// Encode values to a standard byte writer.
pub fn into_writer_all<W, I, V>(values: I, writer: W, format: Format) -> Result<()>
where
    W: Write,
    I: IntoIterator<Item = V>,
    V: Borrow<Scalar>,
{
    into_writer_all_with_formatting(values, writer, format, Formatting::default())
}

/// Encode values to a writer with explicit formatting.
pub fn into_writer_all_with_formatting<W, I, V>(
    values: I,
    writer: W,
    format: Format,
    formatting: Formatting,
) -> Result<()>
where
    W: Write,
    I: IntoIterator<Item = V>,
    V: Borrow<Scalar>,
{
    match format {
        Format::Json | Format::JsonLines => {
            json::into_writer_all_with_formatting(values, writer, formatting)
        }
        Format::Yaml => yaml::into_writer_all_with_formatting(values, writer, formatting),
        Format::Toml => toml::into_writer_all_with_formatting(values, writer, formatting),
    }
}

/// Infer JSON, TOML, or YAML from document content.
pub fn infer_format(input: &[u8]) -> Result<Format> {
    let limits = Limits::default();
    check_input_size(input, limits, "format")?;
    let input = std::str::from_utf8(input).map_err(|error| Error::Codec {
        format: "format",
        position: error.valid_up_to(),
        reason: "structured-text content is not valid UTF-8".into(),
    })?;
    Ok(match infer_utf8_decision(input, limits) {
        Inferred::Decoded(format, _) => format,
        Inferred::Yaml => Format::Yaml,
    })
}

/// Infer and decode one UTF-8 document without parsing it twice.
pub fn from_utf8_inferred(input: &str) -> Result<(Format, Scalar)> {
    from_utf8_inferred_with_limits(input, Limits::default())
}

/// Infer and decode one UTF-8 document with explicit limits.
pub fn from_utf8_inferred_with_limits(input: &str, limits: Limits) -> Result<(Format, Scalar)> {
    check_input_size(input.as_bytes(), limits, "format")?;
    infer_utf8_impl(input, limits)
}

/// Infer and decode UTF-8 under `field` without parsing twice.
pub fn from_utf8_inferred_with_field(input: &str, field: &Field) -> Result<(Format, Scalar)> {
    let (format, value) = from_utf8_inferred(input)?;
    Ok((format, field.from_natural_value(value)?))
}

/// Infer and decode one byte document without parsing it twice.
pub fn from_bytes_inferred(input: &[u8]) -> Result<(Format, Scalar)> {
    from_bytes_inferred_with_limits(input, Limits::default())
}

/// Infer and decode one byte document with explicit limits.
pub fn from_bytes_inferred_with_limits(input: &[u8], limits: Limits) -> Result<(Format, Scalar)> {
    check_input_size(input, limits, "format")?;
    let input = std::str::from_utf8(input).map_err(|error| Error::Codec {
        format: "format",
        position: error.valid_up_to(),
        reason: "structured-text content is not valid UTF-8".into(),
    })?;
    infer_utf8_impl(input, limits)
}

/// Infer and decode bytes under `field` without parsing twice.
pub fn from_bytes_inferred_with_field(input: &[u8], field: &Field) -> Result<(Format, Scalar)> {
    let (format, value) = from_bytes_inferred(input)?;
    Ok((format, field.from_natural_value(value)?))
}

fn infer_utf8_impl(input: &str, limits: Limits) -> Result<(Format, Scalar)> {
    match infer_utf8_decision(input, limits) {
        Inferred::Decoded(format, value) => Ok((format, value)),
        Inferred::Yaml => {
            yaml::from_utf8_with_limits(input, limits).map(|value| (Format::Yaml, value))
        }
    }
}

enum Inferred {
    Decoded(Format, Scalar),
    Yaml,
}

fn infer_utf8_decision(input: &str, limits: Limits) -> Inferred {
    if let Ok(value) = json::from_utf8_with_limits(input, limits) {
        return Inferred::Decoded(Format::Json, value);
    }
    if is_empty_or_comment_only(input.as_bytes()) {
        return Inferred::Yaml;
    }
    if let Ok(value) = toml::from_utf8_with_limits(input, limits) {
        return Inferred::Decoded(Format::Toml, value);
    }
    Inferred::Yaml
}

fn is_empty_or_comment_only(input: &[u8]) -> bool {
    input.split(|byte| *byte == b'\n').all(|line| {
        let first = line
            .iter()
            .copied()
            .find(|byte| !matches!(byte, b' ' | b'\t' | b'\r'));
        first.is_none() || first == Some(b'#')
    })
}

pub(crate) fn apply_field(values: Vec<Scalar>, field: &Field) -> Result<Vec<Scalar>> {
    values
        .into_iter()
        .map(|value| field.from_natural_value(value))
        .collect()
}

pub(crate) fn input_too_large(format: &'static str, position: usize) -> Error {
    Error::Codec {
        format,
        position,
        reason: "input byte limit exceeded".into(),
    }
}

pub(crate) fn check_input_size(input: &[u8], limits: Limits, format: &'static str) -> Result<()> {
    if input.len() > limits.max_input_bytes() {
        Err(input_too_large(format, limits.max_input_bytes()))
    } else {
        Ok(())
    }
}

pub(crate) fn check_encode_depth(value: &Scalar, format: &'static str) -> Result<()> {
    fn visit(value: &Scalar, depth: usize, maximum: usize, format: &'static str) -> Result<()> {
        if depth > maximum {
            return Err(Error::Codec {
                format,
                position: 0,
                reason: "nesting depth limit exceeded while encoding".into(),
            });
        }
        let child_depth = depth.saturating_add(1);
        match value {
            Scalar::Sequence(values) => {
                for value in values.iter() {
                    visit(value, child_depth, maximum, format)?;
                }
            }
            Scalar::Mapping(entries) => {
                for (key, value) in entries.iter() {
                    visit(key, child_depth, maximum, format)?;
                    visit(value, child_depth, maximum, format)?;
                }
            }
            Scalar::Record(entries) => {
                for value in entries.values() {
                    visit(value, child_depth, maximum, format)?;
                }
            }
            Scalar::Null
            | Scalar::Bool(_)
            | Scalar::I8(_)
            | Scalar::I16(_)
            | Scalar::I32(_)
            | Scalar::I64(_)
            | Scalar::U8(_)
            | Scalar::U16(_)
            | Scalar::U32(_)
            | Scalar::U64(_)
            | Scalar::I128(_)
            | Scalar::U128(_)
            | Scalar::F16(_)
            | Scalar::F32(_)
            | Scalar::F64(_)
            | Scalar::D128(..)
            | Scalar::D256(..)
            | Scalar::String(_)
            | Scalar::Enum(_)
            | Scalar::Bytes(_)
            | Scalar::Geospatial(_)
            | Scalar::Date32(..)
            | Scalar::Date64(..)
            | Scalar::Time32(..)
            | Scalar::Time64(..)
            | Scalar::DateTime64(..)
            | Scalar::Duration32(..)
            | Scalar::Duration64(..) => {}
        }
        Ok(())
    }

    visit(value, 0, Limits::default().max_depth(), format)
}
