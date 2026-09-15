//! XML documents as rows.
//!
//! XML is implemented here as a first-class codec module - a sibling of
//! [`crate::text::json`] on the byte side and of [`crate::media::avro`] on the
//! record side - because it is a record encoding rather than a structured-text
//! one. JSON, YAML and TOML carry documents whose leaves state their own type,
//! so they are read as values and bridged to Arrow by the structured-text
//! bridge; XML carries text and shape only, so what it needs is a schema, a
//! projection and a batch, which is what a media is.
//!
//! That is why `MimeType::XML` deliberately answers no
//! [`crate::text::Format`]: making it one would route every XML read through
//! the document bridge and refuse every write but overwrite.
//!
//! # Structure, never type
//!
//! Every leaf on the wire is text. So a read that is given no schema infers
//! the *shape* a document proves - which columns, which are repeated, which
//! are nested - and types every leaf `utf8`. A caller who wants typed columns
//! declares a [`Field`](crate::Field), and each leaf crosses that field's own
//! value contract. There is no second type inference and no per-row guessing.

#[cfg(feature = "arrow")]
mod batch;
mod document;
mod handle;
mod options;
mod reader;
mod writer;
mod xsd;

use crate::text::{Formatting, Limits};
use crate::{Result, Scalar};
#[cfg(feature = "arrow")]
pub(crate) use batch::row_size;
#[cfg(feature = "arrow")]
pub use batch::{overwrite_arrow_reader, read_batch_reader, read_field};

pub use handle::Xml;
pub use options::{DEFAULT_DOCUMENT_NAME, XmlOptions};
pub use reader::MAX_PARSER_DEPTH;
pub(crate) use writer::check_element_name;
pub use xsd::{MAX_SCHEMA_DEPTH, field_from_xsd, field_into_xsd};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

/// Decode one XML document from bytes.
///
/// The value is the document element's own: its attributes and child elements
/// are one namespace of names, a leaf is its characters, and a repeated child
/// is a sequence. Every leaf is text, because that is all a document proves.
///
/// # Errors
///
/// Returns a parse, limit, or shape refusal naming its byte position.
pub fn from_bytes(input: &[u8]) -> Result<Scalar> {
    from_bytes_with_limits(input, Limits::default())
}

/// Decode one XML document from bytes with explicit limits.
///
/// # Errors
///
/// Returns a parse, limit, or shape refusal naming its byte position.
pub fn from_bytes_with_limits(input: &[u8], limits: Limits) -> Result<Scalar> {
    crate::text::check_input_size(input, limits, reader::FORMAT)?;
    document::read_document(input, limits)
}

/// Decode one XML document from UTF-8.
///
/// # Errors
///
/// Returns a parse, limit, or shape refusal naming its byte position.
pub fn from_utf8(input: &str) -> Result<Scalar> {
    from_bytes(input.as_bytes())
}

/// Decode one XML document from UTF-8 with explicit limits.
///
/// # Errors
///
/// Returns a parse, limit, or shape refusal naming its byte position.
pub fn from_utf8_with_limits(input: &str, limits: Limits) -> Result<Scalar> {
    from_bytes_with_limits(input.as_bytes(), limits)
}

/// Encode one value as a whole XML document rooted at `name`.
///
/// XML has no anonymous document: a value needs an element to be written as,
/// so the root's name is an argument rather than a default nobody chose.
///
/// # Errors
///
/// Returns a refusal naming a value or a name XML cannot spell.
pub fn into_bytes(name: &str, value: &Scalar) -> Result<Vec<u8>> {
    into_bytes_with_formatting(name, value, Formatting::new())
}

/// Encode one value as a whole XML document with explicit formatting.
///
/// # Errors
///
/// Returns a refusal naming a value or a name XML cannot spell.
pub fn into_bytes_with_formatting(
    name: &str,
    value: &Scalar,
    formatting: Formatting,
) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    writer::write_value(&mut out, name, value, formatting)?;
    Ok(out)
}

/// Encode one value as a whole XML document in UTF-8.
///
/// # Errors
///
/// Returns a refusal naming a value or a name XML cannot spell.
pub fn into_utf8(name: &str, value: &Scalar) -> Result<String> {
    let bytes = into_bytes(name, value)?;
    String::from_utf8(bytes).map_err(|error| crate::Error::Codec {
        format: reader::FORMAT,
        position: 0,
        reason: smol_str::format_smolstr!("{error}"),
    })
}
