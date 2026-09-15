//! XML documents as a record encoding over any byte handle.
//!
//! The encoding lives in free functions - [`read_field`], [`read_batch_reader`],
//! [`overwrite_arrow_reader`] - that take any [`IOBase`] handle and one
//! [`XmlOptions`]. That is what [`crate::IOMedia::read_arrow_reader`] and its
//! two siblings call, so reading a document needs nothing but a handle whose
//! media type says XML.
//!
//! # What a read costs
//!
//! An XML document states no schema and carries no index: there is no header
//! to read a field out of and no footer to count rows in. So unlike Avro or
//! Parquet, [`read_field`] and [`row_size`] read the document - they decode no
//! more than the shape needs, but they do read it. That is a property of the
//! encoding rather than a shortcut taken here, and it is stated wherever the
//! cost of a record surface is stated.
//!
//! # Structure, never type
//!
//! Every leaf on the wire is text. A read given no field infers the shape and
//! types every leaf `utf8`; a read given one crosses each leaf through that
//! field's own value contract, which is what turns `9.50` into a decimal at
//! the scale the column declares. There is no second inference here.
//!
//! XML does not compress internally, so - like IPC and plain text, and unlike
//! Avro and Parquet - a handle declaring an outer content coding is read and
//! written through that coding rather than refused.

use smol_str::SmolStr;

use crate::arrow::{ArrowScalar, BatchReader, Result};
use crate::media::IORecordOptions;
use crate::{Field, IOBase, Scalar};

use super::document::{DEFAULT_ROW_NAME, read_rows};
use super::options::XmlOptions;
use super::writer::write_document;

/// Read the canonical Struct field the document's rows prove.
///
/// # Errors
///
/// Returns a read, decompression, parse, or inference failure.
pub fn read_field<H: IOBase + ?Sized>(handle: &H, options: &XmlOptions) -> Result<Field> {
    if let Some(field) = options.declared() {
        return Ok(field.clone());
    }
    let (name, rows) = rows_of(handle, options)?;
    Ok(field_of(&name, rows, options)?)
}

/// Count the rows a document holds.
///
/// # Errors
///
/// Returns a read, decompression, or parse failure.
pub(crate) fn row_size<H: IOBase + ?Sized>(handle: &H, options: &XmlOptions) -> crate::Result<u64> {
    let (_, rows) = rows_of(handle, options)?;
    Ok(rows.len() as u64)
}

/// Read a document's rows as Arrow batches.
///
/// # Errors
///
/// Returns a read, decompression, parse, inference, or cast failure.
pub fn read_batch_reader<H: IOBase + ?Sized>(
    handle: &H,
    field: Option<&Field>,
    options: &XmlOptions,
) -> Result<BatchReader> {
    let (name, rows) = rows_of(handle, options)?;
    let root = match field {
        Some(field) => field.clone(),
        None => field_of(&name, rows.clone(), options)?,
    };
    // Every row crosses the field's own value contract, which is what types
    // an all-text encoding's leaves. The batch build canonicalizes again on
    // the way in; that second pass is the price of the first being the only
    // thing that reads a document's spellings.
    let canonical = rows
        .into_iter()
        .map(|row| root.from_natural_value(row))
        .collect::<crate::Result<Vec<_>>>()?;
    let value = ArrowScalar::from_rows(&root, &Scalar::from_sequence(canonical))?;
    value.into_reader()
}

/// Replace a handle's contents with one stream of rows, as one document.
///
/// # Errors
///
/// Returns a schema, value, encoding, compression, or write failure.
pub fn overwrite_arrow_reader<H>(
    handle: &mut H,
    batches: BatchReader,
    options: &XmlOptions,
) -> Result<()>
where
    H: IOBase + ?Sized,
{
    let value = ArrowScalar::from_reader(batches)?;
    let root = value.root()?;
    // A write reaches an encoding with the declared field already applied and
    // popped, so the incoming stream's root carries no name a caller chose.
    // The options' own name does - it is set from the declared field and
    // survives that pop - which is what a row element is called.
    let row_name = options
        .row_element
        .clone()
        .unwrap_or_else(|| SmolStr::new(options.name()));
    let rows = value.into_scalar()?;
    // The names go back on the way out, so a row renders as the element a
    // reader is expected to find rather than as a positional list.
    let rendered = rows
        .as_sequence()
        .unwrap_or_default()
        .iter()
        .map(|row| root.into_natural_value(row.clone()))
        .collect::<crate::Result<Vec<_>>>()?;
    let mut encoded = Vec::new();
    write_document(
        &mut encoded,
        &options.document,
        &row_name,
        &rendered,
        options.formatting(),
    )?;
    handle.write_all_bytes(&encoded)?;
    Ok(())
}

/// Read one handle's whole document as rows.
fn rows_of<H: IOBase + ?Sized>(
    handle: &H,
    options: &XmlOptions,
) -> crate::Result<(SmolStr, Vec<Scalar>)> {
    let bytes = handle.read_all_bytes()?;
    if bytes.is_empty() {
        return Ok((SmolStr::new_static(DEFAULT_ROW_NAME), Vec::new()));
    }
    read_rows(&bytes, options.limits, options.row_element.as_deref())
}

/// Infer the Struct field a document's rows prove.
fn field_of(name: &str, rows: Vec<Scalar>, options: &XmlOptions) -> crate::Result<Field> {
    let name = if name.is_empty() {
        options.name.as_str()
    } else {
        name
    };
    if rows.is_empty() {
        return Ok(Field::new(name, crate::DataType::from_fields([])?, false));
    }
    let rows = Scalar::from_sequence(rows);
    let field = rows.inferred_struct_field()?;
    Ok(field.with_name(name))
}
