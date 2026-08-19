//! Shared byte-first structured-text values and format dispatch.

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
pub(crate) mod wire;

pub use crate::generic::TypedValue;
pub use crate::generic::value::{Children, Float, Float32, Value};
pub use codec::{Json, Jsonl, Limited, TextCodec, Toml, Yaml};
#[cfg(feature = "arrow")]
pub(crate) use display::ERROR_TEXT_LIMIT;
pub(crate) use display::{elide_display, elide_to, expected_got, stable_hash_display};
pub use display::{stable_hash_bytes, stable_hash_chunks};
pub use format::Format;
pub use formatting::{Formatting, Indent};
pub use io::{
    Plan, dump, dump_all, dump_all_with, dump_with, dump_with_level, load, load_all,
    load_all_with_limits, load_with, load_with_limits,
};
pub use limits::Limits;
#[cfg(feature = "arrow")]
pub use line::TextOptions;
pub use line::{
    LineSep, Opening, Strip, Text, TextLine, TextLineBuf, TextLineOptions, TextLines,
    schema_from_pattern,
};
pub use loading::Loading;
pub use placeholder::Placeholders;

use crate::{Error, Result, json, toml, yaml};

/// A lazy iterator over values decoded from a borrowed byte reader.
///
/// Each item is parsed only when [`Iterator::next`] is called. The iterator
/// keeps the reader borrowed for its lifetime and never materializes all
/// documents as a prerequisite to iteration.
pub struct ValueIter<'a> {
    inner: Box<dyn Iterator<Item = Result<Value>> + 'a>,
}

impl<'a> ValueIter<'a> {
    pub(crate) fn new(iterator: impl Iterator<Item = Result<Value>> + 'a) -> Self {
        Self {
            inner: Box::new(iterator),
        }
    }
}

impl Iterator for ValueIter<'_> {
    type Item = Result<Value>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

/// Apply `loading`'s placeholders to a freshly parsed value, if any apply.
///
/// The cheap guard lives here: substitution is off unless a caller turned it
/// on, and even then a document whose bytes contain no `{{` is returned
/// untouched - no walk, no allocation, no per-scalar inspection. The
/// overwhelming majority of documents have no placeholders and must not pay for
/// the feature.
///
/// JSON is refused, not skipped: JSON is a data interchange format, and a
/// JSON document that wants configuration templating is better written as
/// YAML or TOML. Refusing loudly here keeps a misconfigured caller from
/// silently reading `{{ NAME }}` as literal text.
fn filled(value: Value, input: &[u8], format: Format, loading: &Loading) -> Result<Value> {
    let Some(placeholders) = loading.placeholders() else {
        return Ok(value);
    };
    if matches!(format, Format::Json | Format::JsonLines) {
        return Err(Error::InvalidRecord {
            path: smol_str::SmolStr::new_static("$.placeholders"),
            reason: expected_got(
                "a format with placeholder support (yaml, toml)",
                format.as_str(),
            ),
        });
    }
    if !placeholder::present(input) {
        return Ok(value);
    }
    placeholder::substitute(value, placeholders)
}

/// Decode one value from borrowed UTF-8 text using the selected format.
///
/// This delegates through the string's borrowed bytes without an intermediate
/// UTF-8/byte input buffer. The returned owned value still allocates or shares
/// storage for strings and collections.
pub fn from_str(input: &str, format: Format) -> Result<Value> {
    from_str_with_limits(input, format, Limits::default())
}

/// Decode one value from borrowed UTF-8 text with explicit resource limits.
pub fn from_str_with_limits(input: &str, format: Format, limits: Limits) -> Result<Value> {
    match format {
        Format::Json => json::from_str_with_limits(input, limits),
        Format::JsonLines => {
            json::from_lines_str_with_limits(input, limits).map(Value::from_sequence)
        }
        Format::Yaml => yaml::from_str_with_limits(input, limits),
        Format::Toml => toml::from_str_with_limits(input, limits),
    }
}

/// Decode one value from borrowed UTF-8 text under [`Loading`].
///
/// The parse itself is unchanged - `loading`'s limits are the same limits - so
/// a malformed document still fails exactly where it is malformed, with exact
/// byte positions. `{{ }}` placeholders, when [`Loading::with_placeholders`]
/// turned them on, are resolved *after* that, by walking the parsed value.
///
/// # Errors
///
/// Returns the codec's parse failure, or the substitution's refusal - an
/// unresolved variable, a malformed placeholder - naming where it sits.
pub fn from_str_with(input: &str, format: Format, loading: &Loading) -> Result<Value> {
    let value = from_str_with_limits(input, format, loading.limits())?;
    filled(value, input.as_bytes(), format, loading)
}

/// Decode one value using the selected format.
pub fn from_slice(input: &[u8], format: Format) -> Result<Value> {
    from_slice_with_limits(input, format, Limits::default())
}

/// Decode one value using explicit resource limits.
pub fn from_slice_with_limits(input: &[u8], format: Format, limits: Limits) -> Result<Value> {
    match format {
        Format::Json => json::from_slice_with_limits(input, limits),
        Format::JsonLines => {
            json::from_lines_slice_with_limits(input, limits).map(Value::from_sequence)
        }
        Format::Yaml => yaml::from_slice_with_limits(input, limits),
        Format::Toml => toml::from_slice_with_limits(input, limits),
    }
}

/// Decode one value from bytes under [`Loading`], as [`from_str_with`] does.
///
/// # Errors
///
/// Returns the codec's parse failure, or the substitution's refusal.
pub fn from_slice_with(input: &[u8], format: Format, loading: &Loading) -> Result<Value> {
    let value = from_slice_with_limits(input, format, loading.limits())?;
    filled(value, input, format, loading)
}

/// Decode one value from a byte reader using the selected format.
pub fn from_reader<R: Read>(reader: R, format: Format) -> Result<Value> {
    from_reader_with_limits(reader, format, Limits::default())
}

/// Decode one value from a byte reader with explicit resource limits.
pub fn from_reader_with_limits<R: Read>(
    reader: R,
    format: Format,
    limits: Limits,
) -> Result<Value> {
    match format {
        Format::Json => json::from_reader_with_limits(reader, limits),
        Format::JsonLines => {
            json::from_lines_reader_with_limits(reader, limits).map(Value::from_sequence)
        }
        Format::Yaml => yaml::from_reader_with_limits(reader, limits),
        Format::Toml => toml::from_reader_with_limits(reader, limits),
    }
}

/// Decode one value from a byte reader under [`Loading`].
///
/// With placeholders off this is [`from_reader_with_limits`] exactly, reader
/// and all. With them on the reader is drained into memory first - bounded by
/// [`Limits::max_input_bytes`] - because the cheap `{{` guard needs the bytes,
/// and a document small enough to want substitution is small enough to hold.
///
/// # Errors
///
/// Returns the read failure, the codec's parse failure, or the substitution's
/// refusal.
pub fn from_reader_with<R: Read>(reader: R, format: Format, loading: &Loading) -> Result<Value> {
    let Some(_) = loading.placeholders() else {
        return from_reader_with_limits(reader, format, loading.limits());
    };
    let mut bytes = Vec::new();
    let limit = loading.limits().max_input_bytes();
    // One past the limit, so exceeding it is the codec's own refusal rather
    // than a silent truncation here.
    reader
        .take(limit.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    from_slice_with(&bytes, format, loading)
}

/// Decode all values or documents in an in-memory byte stream.
pub fn from_slice_all(input: &[u8], format: Format) -> Result<Vec<Value>> {
    from_slice_all_with_limits(input, format, Limits::default())
}

/// Decode all values or documents in an in-memory byte stream with explicit
/// resource limits.
pub fn from_slice_all_with_limits(
    input: &[u8],
    format: Format,
    limits: Limits,
) -> Result<Vec<Value>> {
    match format {
        Format::Json => json::from_slice_all_with_limits(input, limits),
        Format::JsonLines => json::from_lines_slice_with_limits(input, limits),
        Format::Yaml => yaml::from_slice_all_with_limits(input, limits),
        Format::Toml => toml::from_slice_all_with_limits(input, limits),
    }
}

/// Decode all values or documents from borrowed UTF-8 text.
pub fn from_str_all(input: &str, format: Format) -> Result<Vec<Value>> {
    from_str_all_with_limits(input, format, Limits::default())
}

/// Decode all values or documents from borrowed UTF-8 text with explicit
/// resource limits.
pub fn from_str_all_with_limits(input: &str, format: Format, limits: Limits) -> Result<Vec<Value>> {
    match format {
        Format::Json => json::from_str_all_with_limits(input, limits),
        Format::JsonLines => json::from_lines_str_with_limits(input, limits),
        Format::Yaml => yaml::from_str_all_with_limits(input, limits),
        Format::Toml => toml::from_str_all_with_limits(input, limits),
    }
}

/// Decode all values or documents from a byte reader.
pub fn from_reader_all<R: Read>(reader: R, format: Format) -> Result<Vec<Value>> {
    from_reader_all_with_limits(reader, format, Limits::default())
}

/// Decode all values or documents from a byte reader with explicit resource
/// limits.
pub fn from_reader_all_with_limits<R: Read>(
    reader: R,
    format: Format,
    limits: Limits,
) -> Result<Vec<Value>> {
    match format {
        Format::Json => json::from_reader_all_with_limits(reader, limits),
        Format::JsonLines => json::from_lines_reader_with_limits(reader, limits),
        Format::Yaml => yaml::from_reader_all_with_limits(reader, limits),
        Format::Toml => toml::from_reader_all_with_limits(reader, limits),
    }
}

/// Lazily decode values or documents from a borrowed byte reader.
pub fn from_reader_iter<'a, R: Read + 'a>(reader: &'a mut R, format: Format) -> ValueIter<'a> {
    from_reader_iter_with_limits(reader, format, Limits::default())
}

/// Lazily decode with explicit resource limits.
pub fn from_reader_iter_with_limits<'a, R: Read + 'a>(
    reader: &'a mut R,
    format: Format,
    limits: Limits,
) -> ValueIter<'a> {
    match format {
        Format::Json => json::from_reader_iter_with_limits(reader, limits),
        Format::JsonLines => json::from_lines_reader_iter_with_limits(reader, limits),
        Format::Yaml => yaml::from_reader_iter_with_limits(reader, limits),
        Format::Toml => toml::from_reader_iter_with_limits(reader, limits),
    }
}

/// Encode one value to a new byte vector.
pub fn to_vec(value: &Value, format: Format) -> Result<Vec<u8>> {
    to_vec_with_formatting(value, format, Formatting::default())
}

/// Encode one value to a new byte vector, laid out as `formatting` asks.
///
/// Each format resolves the layout its own way - see
/// [`json::to_vec_with_formatting`], [`yaml::to_vec_with_formatting`], and
/// [`toml::to_vec_with_formatting`]. Formatting changes bytes, never meaning:
/// parsing any formatting of the same value yields an equal value.
///
/// # Errors
///
/// Returns the format's encoding failure.
pub fn to_vec_with_formatting(
    value: &Value,
    format: Format,
    formatting: Formatting,
) -> Result<Vec<u8>> {
    match format {
        Format::Json => json::to_vec_with_formatting(value, formatting),
        Format::JsonLines => {
            if let Value::Sequence(values) = value {
                json::to_vec_all_with_formatting(values, formatting)
            } else {
                json::to_vec_all_with_formatting(std::slice::from_ref(value), formatting)
            }
        }
        Format::Yaml => yaml::to_vec_with_formatting(value, formatting),
        Format::Toml => toml::to_vec_with_formatting(value, formatting),
    }
}

/// Consume and encode one value to a new byte vector.
///
/// Encoding necessarily creates a distinct output buffer; consuming avoids an
/// ownership-preserving clone but cannot reuse the value's typed storage.
pub fn into_vec(value: Value, format: Format) -> Result<Vec<u8>> {
    to_vec(&value, format)
}

/// Consume and encode one value, laid out as `formatting` asks.
///
/// # Errors
///
/// Returns the format's encoding failure.
pub fn into_vec_with_formatting(
    value: Value,
    format: Format,
    formatting: Formatting,
) -> Result<Vec<u8>> {
    to_vec_with_formatting(&value, format, formatting)
}

/// Encode one value to a byte writer.
pub fn to_writer<W: Write>(writer: W, value: &Value, format: Format) -> Result<()> {
    to_writer_with_formatting(writer, value, format, Formatting::default())
}

/// Encode one value to a byte writer, laid out as `formatting` asks.
///
/// # Errors
///
/// Returns the format's encoding failure or the sink's.
pub fn to_writer_with_formatting<W: Write>(
    writer: W,
    value: &Value,
    format: Format,
    formatting: Formatting,
) -> Result<()> {
    match format {
        Format::Json => json::to_writer_with_formatting(writer, value, formatting),
        Format::JsonLines => {
            if let Value::Sequence(values) = value {
                json::to_writer_all_with_formatting(writer, values.iter(), formatting)
            } else {
                json::to_writer_all_with_formatting(writer, std::slice::from_ref(value), formatting)
            }
        }
        Format::Yaml => yaml::to_writer_with_formatting(writer, value, formatting),
        Format::Toml => toml::to_writer_with_formatting(writer, value, formatting),
    }
}

/// Encode multiple values or documents to a byte writer.
pub fn to_writer_all<W, I, V>(writer: W, values: I, format: Format) -> Result<()>
where
    W: Write,
    I: IntoIterator<Item = V>,
    V: Borrow<Value>,
{
    to_writer_all_with_formatting(writer, values, format, Formatting::default())
}

/// Encode multiple values or documents, laid out as `formatting` asks.
///
/// # Errors
///
/// Returns the format's encoding failure or the sink's.
pub fn to_writer_all_with_formatting<W, I, V>(
    writer: W,
    values: I,
    format: Format,
    formatting: Formatting,
) -> Result<()>
where
    W: Write,
    I: IntoIterator<Item = V>,
    V: Borrow<Value>,
{
    match format {
        Format::Json | Format::JsonLines => {
            json::to_writer_all_with_formatting(writer, values, formatting)
        }
        Format::Yaml => yaml::to_writer_all_with_formatting(writer, values, formatting),
        Format::Toml => toml::to_writer_all_with_formatting(writer, values, formatting),
    }
}

/// Infer JSON, TOML, or YAML from document content without path information.
///
/// Valid JSON wins because many JSON values are also valid YAML. Empty and
/// YAML-comment-only inputs retain their historical YAML interpretation.
/// Otherwise TOML is selected only when the complete bounded TOML document is
/// valid; all remaining content is delegated to YAML. JSON Lines requires an
/// explicit format or a path suffix because arbitrary newlines are ambiguous.
pub fn infer_format(input: &[u8]) -> Result<Format> {
    let limits = Limits::default();
    check_input_size(input, limits, "format")?;
    let input = std::str::from_utf8(input).map_err(|error| Error::Codec {
        format: "format",
        position: error.valid_up_to(),
        reason: "structured-text content is not valid UTF-8".into(),
    })?;
    Ok(match infer_str_decision(input, limits) {
        Inferred::Decoded(format, _) => format,
        Inferred::Yaml => Format::Yaml,
    })
}

/// Infer and decode one borrowed UTF-8 document without parsing it twice.
pub fn from_str_inferred(input: &str) -> Result<(Format, Value)> {
    from_str_inferred_with_limits(input, Limits::default())
}

/// Infer and decode one borrowed document with explicit resource limits.
pub fn from_str_inferred_with_limits(input: &str, limits: Limits) -> Result<(Format, Value)> {
    check_input_size(input.as_bytes(), limits, "format")?;
    infer_str_impl(input, limits)
}

/// Infer and decode one byte document without parsing it twice.
pub fn from_slice_inferred(input: &[u8]) -> Result<(Format, Value)> {
    from_slice_inferred_with_limits(input, Limits::default())
}

/// Infer and decode one byte document with explicit resource limits.
pub fn from_slice_inferred_with_limits(input: &[u8], limits: Limits) -> Result<(Format, Value)> {
    check_input_size(input, limits, "format")?;
    let input = std::str::from_utf8(input).map_err(|error| Error::Codec {
        format: "format",
        position: error.valid_up_to(),
        reason: "structured-text content is not valid UTF-8".into(),
    })?;
    infer_str_impl(input, limits)
}

fn infer_str_impl(input: &str, limits: Limits) -> Result<(Format, Value)> {
    match infer_str_decision(input, limits) {
        Inferred::Decoded(format, value) => Ok((format, value)),
        Inferred::Yaml => {
            yaml::from_str_with_limits(input, limits).map(|value| (Format::Yaml, value))
        }
    }
}

enum Inferred {
    Decoded(Format, Value),
    Yaml,
}

fn infer_str_decision(input: &str, limits: Limits) -> Inferred {
    if let Ok(value) = json::from_str_with_limits(input, limits) {
        return Inferred::Decoded(Format::Json, value);
    }
    if is_empty_or_comment_only(input.as_bytes()) {
        return Inferred::Yaml;
    }
    if let Ok(value) = toml::from_str_with_limits(input, limits) {
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

pub(crate) fn check_encode_depth(value: &Value, format: &'static str) -> Result<()> {
    fn visit(value: &Value, depth: usize, maximum: usize, format: &'static str) -> Result<()> {
        if depth > maximum {
            return Err(Error::Codec {
                format,
                position: 0,
                reason: "nesting depth limit exceeded while encoding".into(),
            });
        }
        let child_depth = depth.saturating_add(1);
        match value {
            Value::Sequence(values) | Value::Record(_, values) => {
                for value in values.iter() {
                    visit(value, child_depth, maximum, format)?;
                }
            }
            Value::Mapping(entries) => {
                for (key, value) in entries.iter() {
                    visit(key, child_depth, maximum, format)?;
                    visit(value, child_depth, maximum, format)?;
                }
            }
            // Spelled out rather than left to a wildcard: a value that holds
            // other values and is not named here would escape the depth limit
            // silently, so a new variant has to be classified to compile.
            Value::Null
            | Value::Bool(_)
            | Value::I8(_)
            | Value::I16(_)
            | Value::I32(_)
            | Value::I64(_)
            | Value::U8(_)
            | Value::U16(_)
            | Value::U32(_)
            | Value::U64(_)
            | Value::I128(_)
            | Value::U128(_)
            | Value::F32(_)
            | Value::F64(_)
            | Value::Decimal(..)
            | Value::String(_)
            | Value::Bytes(_)
            | Value::Geospatial(_)
            | Value::Date(_)
            | Value::Time(..)
            | Value::Timestamp(..)
            | Value::DateTime(..)
            | Value::Duration(..) => {}
        }
        Ok(())
    }

    visit(value, 0, Limits::default().max_depth(), format)
}
