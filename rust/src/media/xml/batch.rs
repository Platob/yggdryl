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

use super::document::{DEFAULT_ROW_NAME, Spelling, read_rows};
use super::options::XmlOptions;
use super::writer::write_rows;

/// Read the canonical Struct field the document's rows prove.
///
/// # Errors
///
/// Returns a read, decompression, parse, or inference failure.
pub fn read_field<H: IOBase + ?Sized>(handle: &H, options: &XmlOptions) -> Result<Field> {
    if let Some(field) = options.declared() {
        return Ok(field.clone());
    }
    let (name, rows, spelling) = rows_of(handle, options)?;
    Ok(spelling.apply(field_of(&name, &rows, options)?)?)
}

/// Count the rows a document holds.
///
/// # Errors
///
/// Returns a read, decompression, or parse failure.
pub(crate) fn row_size<H: IOBase + ?Sized>(handle: &H, options: &XmlOptions) -> crate::Result<u64> {
    let (_, rows, _) = rows_of(handle, options)?;
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
    let (name, rows, spelling) = rows_of(handle, options)?;
    let root = match field {
        Some(field) => field.clone(),
        None => spelling.apply(field_of(&name, &rows, options)?)?,
    };
    // Every row crosses the field's own value contract, which is what types
    // an all-text encoding's leaves. The batch build canonicalizes again on
    // the way in; that second pass is the price of the first being the only
    // thing that reads a document's spellings.
    let canonical = rows
        .into_iter()
        .map(|row| root.from_natural_value(fit(row, &root)))
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
    // The declared field carries what each column was spelled as, so a write
    // under the field a read produced puts the document back the way it was.
    let spelled = root.clone().with_name(row_name.as_str());
    let mut encoded = Vec::new();
    write_rows(
        &mut encoded,
        &options.document,
        &spelled,
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
) -> crate::Result<(SmolStr, Vec<Scalar>, Spelling)> {
    let bytes = handle.read_all_bytes()?;
    if bytes.is_empty() {
        return Ok((
            SmolStr::new_static(DEFAULT_ROW_NAME),
            Vec::new(),
            Spelling::default(),
        ));
    }
    // The byte budget bounds what a record read decodes exactly as it bounds
    // what a value read does; a document reached through a handle is not a
    // document the limits stop applying to.
    crate::text::check_input_size(&bytes, options.limits, "xml")?;
    read_rows(&bytes, options.limits, options.row_element.as_deref())
}

/// Infer the Struct field a document's rows prove, and how it spelled them.
///
/// The rows are borrowed: inference reads their shape and a copy of every
/// decoded row would be the largest allocation a schemaless read makes.
fn field_of(name: &str, rows: &[Scalar], options: &XmlOptions) -> crate::Result<Field> {
    let name = if name.is_empty() {
        options.name.as_str()
    } else {
        name
    };
    if rows.is_empty() {
        return Ok(Field::new(name, crate::DataType::from_fields([])?, false));
    }
    let rows = Scalar::from_sequence(rows.iter().cloned());
    let field = rows.inferred_struct_field()?;
    Ok(field.with_name(name))
}

/// Restate one decoded row in the shape the declared field asks for.
///
/// XML spells a repeated child by repeating it, so a column that a schema
/// calls a list arrives as one value when the row happened to carry one, and
/// as a sequence when it carried more. Only the declaration knows which it is,
/// and one reading of a single occurrence under a list column is unambiguous -
/// it is a list of one - so this is best effort rather than a refusal.
///
/// Nothing else is coerced: a value the field cannot take still meets the
/// value contract and is still refused there.
fn fit(value: Scalar, field: &Field) -> Scalar {
    use crate::DataType;

    if value.is_null() {
        return value;
    }
    match field.dtype() {
        DataType::Struct(children) => {
            let Some(record) = value.as_record() else {
                return value;
            };
            let fitted = children.iter().filter_map(|child| {
                record
                    .get(child.name())
                    .map(|held| (SmolStr::new(child.name()), fit(held.clone(), child)))
            });
            Scalar::from_record(fitted).unwrap_or(value)
        }
        DataType::List(child)
        | DataType::LargeList(child)
        | DataType::ListView(child)
        | DataType::LargeListView(child)
        | DataType::FixedSizeList(child, _) => match value.as_sequence() {
            Some(values) => {
                Scalar::from_sequence(values.iter().map(|item| fit(item.clone(), child)))
            }
            None => Scalar::from_sequence([fit(value, child)]),
        },
        _ => value,
    }
}
