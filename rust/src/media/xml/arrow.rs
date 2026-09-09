//! XML rows through the shared Scalar/Arrow record boundary.

use std::io::Read;

use smol_str::SmolStr;

use crate::arrow::BatchReader;
use crate::media::IORecordOptions;
use crate::media::source::{DecodedReader, SendDecodedReader, owned_leaf};
use crate::types::Nested;
use crate::{Cursor, DataType, Field, IOBase, Result, Scalar};

use super::index::{self, RowIndex};
use super::options::XmlOptions;
use super::reader::{FETCH_BYTE_SIZE, Rows};
use super::writer::{self, RowLayout};

/// Read the canonical Struct field this document's rows describe.
///
/// A declared field is the answer when there is one. Otherwise XML declares no
/// types, so the answer is what the document proves: every element is `utf8`,
/// an element holding elements is a struct, an element that repeats anywhere
/// is a list, and a column some rows leave out is nullable. The rows are
/// scanned to union those names, because a document with no header has nowhere
/// else to keep them.
///
/// # Errors
///
/// Returns a read or decoding failure, or a row whose shape cannot meet the
/// rows before it.
pub fn read_field<H: IOBase + ?Sized>(handle: &H, options: &XmlOptions) -> Result<Field> {
    match options.field() {
        Some(field) => Ok(field),
        None => read_stored_field(handle, options),
    }
}

/// Read the shape the document itself stores, ignoring any declared field.
fn read_stored_field<H: IOBase + ?Sized>(handle: &H, options: &XmlOptions) -> Result<Field> {
    let name = options.name().to_owned();
    if handle.is_empty() {
        return empty_field(&name);
    }
    let mut dtype: Option<DataType> = None;
    for row in Rows::new(decoded(handle)?, options) {
        let next = row?.dtype()?;
        dtype = Some(match dtype {
            Some(current) => merged(current, &next)?,
            None => next,
        });
    }
    match dtype {
        Some(DataType::Struct(fields)) => Ok(Field::new(name, DataType::Struct(fields), false)),
        Some(_) | None => empty_field(&name),
    }
}

/// Meet two row shapes the way the wire spells them.
///
/// This is [`DataType::merge_with`] with the one rule an element adds: a name
/// that repeats in any row is a list everywhere, because XML spells a list by
/// repeating the element and a row holding one item looks exactly like a row
/// holding a value. Everything else - a name only one row carries becoming
/// nullable, a null yielding to what is defined beside it - is the shared
/// merge, so the two cannot disagree.
fn merged(current: DataType, next: &DataType) -> Result<DataType> {
    if current == *next {
        return Ok(current);
    }
    if matches!(current, DataType::Null) {
        return Ok(next.clone());
    }
    if matches!(next, DataType::Null) {
        return Ok(current);
    }
    if let (DataType::Struct(left), DataType::Struct(right)) = (&current, next) {
        return DataType::from_fields(merged_fields(left, right)?);
    }
    match (list_item(&current), list_item(next)) {
        (Some(left), Some(right)) => {
            let item = merged(left.dtype().clone(), right.dtype())?;
            Ok(DataType::list(Field::new("item", item, true)))
        }
        (Some(left), None) => {
            let item = merged(left.dtype().clone(), next)?;
            Ok(DataType::list(Field::new("item", item, true)))
        }
        (None, Some(right)) => {
            let item = merged(current, right.dtype())?;
            Ok(DataType::list(Field::new("item", item, true)))
        }
        (None, None) => current.merge_with(next, true),
    }
}

/// Meet two struct field lists by name, in the order they were first seen.
fn merged_fields(left: &[Field], right: &[Field]) -> Result<Vec<Field>> {
    let mut fields: Vec<Field> = Vec::with_capacity(left.len().max(right.len()));
    for field in left {
        match right.iter().find(|other| other.name() == field.name()) {
            Some(other) => {
                let dtype = merged(field.dtype().clone(), other.dtype())?;
                fields.push(Field::new(
                    field.name(),
                    dtype,
                    field.is_nullable() || other.is_nullable(),
                ));
            }
            // A name the other side does not carry describes rows that leave
            // the element out, so it is absent there.
            None => fields.push(field.clone().with_nullable(true)),
        }
    }
    for field in right {
        if !left.iter().any(|other| other.name() == field.name()) {
            fields.push(field.clone().with_nullable(true));
        }
    }
    Ok(fields)
}

/// Borrow one list datatype's item field.
fn list_item(dtype: &DataType) -> Option<&Field> {
    match dtype {
        DataType::List(item)
        | DataType::ListView(item)
        | DataType::LargeList(item)
        | DataType::LargeListView(item)
        | DataType::FixedSizeList(item, _) => Some(item),
        _ => None,
    }
}

/// Restate a declared field as the shape the wire actually carries.
///
/// A declared field is a projection and a cast, not a description of what is
/// stored: an element carries text, so the rows are built as text under the
/// declared names and finished by the one cast every encoding's read ends
/// with. Everything a row can leave out is nullable here, because an element
/// a document omits is a column that is absent rather than a document that is
/// wrong; the declared nullability is what the cast then enforces.
fn textual(declared: &Field) -> Result<Field> {
    fn shape(field: &Field) -> Result<Field> {
        let dtype = match field.dtype() {
            DataType::Struct(fields) => {
                DataType::from_fields(fields.iter().map(shape).collect::<Result<Vec<_>>>()?)?
            }
            DataType::List(item)
            | DataType::ListView(item)
            | DataType::LargeList(item)
            | DataType::LargeListView(item)
            | DataType::FixedSizeList(item, _) => DataType::list(shape(item)?),
            _ => DataType::Utf8,
        };
        Ok(Field::new(field.name(), dtype, true))
    }

    let Some(fields) = declared.dtype().as_fields() else {
        return Ok(Field::new(declared.name(), DataType::Utf8, true));
    };
    Ok(Field::new(
        declared.name(),
        DataType::from_fields(fields.iter().map(shape).collect::<Result<Vec<_>>>()?)?,
        false,
    ))
}

/// Return the number of row elements this document holds.
///
/// The scan reads the document's markup and decodes none of its values, so
/// counting a large document never materializes a row.
///
/// # Errors
///
/// Returns a read or decoding failure.
pub fn row_size<H: IOBase + ?Sized>(handle: &H, options: &XmlOptions) -> Result<u64> {
    if handle.is_empty() {
        return Ok(0);
    }
    Ok(index::scan(decoded(handle)?, options)?.len())
}

/// Read this document's rows as one batch reader.
///
/// # Errors
///
/// Returns a read, decoding, schema, or cast failure.
pub fn read_batch_reader<H: IOBase + ?Sized>(
    handle: &H,
    declared: Option<&Field>,
    options: &XmlOptions,
) -> Result<BatchReader> {
    let field = match declared {
        Some(declared) => textual(declared)?,
        None => read_stored_field(handle, options)?,
    };
    let rows = Rows::new(owned_source(handle)?, options);
    read_rows(field, rows, options.batch_row_size())
}

/// Read a bounded run of rows straight out of their own byte spans.
///
/// This is the positional read: the rows before `offset` are never parsed,
/// because the index already says where the ones being asked for begin.
///
/// # Errors
///
/// Returns a read, decoding, schema, or cast failure.
pub(crate) fn read_range_batch_reader<H: IOBase + ?Sized>(
    handle: &H,
    index: &RowIndex,
    options: &XmlOptions,
    offset: u64,
    count: usize,
) -> Result<BatchReader> {
    let declared = options.field();
    let field = match declared.as_ref() {
        Some(declared) => textual(declared)?,
        None => read_stored_field(handle, options)?,
    };
    let rows = super::random::read_range_scalars(handle, index, offset, count)?;
    let reader = read_rows(field, rows.into_iter().map(Ok), options.batch_row_size())?;
    // The positional read answers the same rows the whole-document read does,
    // so it is finished by the same cast rather than by a second contract.
    match declared {
        Some(declared) => Ok(crate::arrow::cast_reader(
            reader,
            &declared,
            crate::ArrowCastOptions::new().with_safe(options.safe()),
        )?),
        None => Ok(reader),
    }
}

/// Shape and type one row stream against the field it is read under.
fn read_rows<I>(field: Field, rows: I, batch_row_size: Option<usize>) -> Result<BatchReader>
where
    I: Iterator<Item = Result<Scalar>> + Send + 'static,
{
    let typed = field.clone();
    let rows = rows.map(move |row| {
        let row = shaped(row?, &typed)?;
        typed.from_natural_value(row)
    });
    Ok(crate::arrow::rows::result_reader(
        &field,
        rows,
        batch_row_size,
        None,
        None,
    )?)
}

/// Replace this document with every row the reader yields.
///
/// A stored document keeps its own document and row element names, because
/// replacing a document's rows is not renaming the document; a declared
/// [`root`](XmlOptions::root) or [`row`](XmlOptions::row) is what changes them.
///
/// # Errors
///
/// Returns an encoding or write failure.
pub fn overwrite_arrow_reader<H: IOBase + ?Sized>(
    handle: &mut H,
    batches: BatchReader,
    options: &XmlOptions,
) -> Result<()> {
    let (root, row) = stored_names(handle, options)?;
    let mut encoded = Vec::new();
    {
        let mut encoder = handle
            .codec()
            .writer_with_level(&mut encoded, options.level());
        writer::write_document(
            &mut encoder,
            batches,
            &root,
            &row,
            RowLayout::from(options.formatting()),
        )?;
        encoder.finish()?;
    }
    handle.write_all_bytes(&encoded)
}

/// Add every row the reader yields after this document's current last one.
///
/// A stored document keeps its own document and row element names, and only
/// its end tag is rewritten, so an append costs the rows it adds. A content
/// coding has no addressable end tag, so a coded document is re-encoded whole.
///
/// # Errors
///
/// Returns a read, encoding, or write failure.
pub fn append_arrow_reader<H: IOBase + ?Sized>(
    handle: &mut H,
    batches: BatchReader,
    options: &XmlOptions,
) -> Result<()> {
    if handle.is_empty() {
        return overwrite_arrow_reader(handle, batches, options);
    }
    let layout = RowLayout::from(options.formatting());
    let stored = stored_names(handle, options)?;
    let root = stored.0;
    let row = stored.1;

    if handle.codec() == crate::Codec::Identity {
        if let Some(end) = document_end(handle, &root)? {
            let mut rendered = Vec::new();
            for batch in batches {
                let batch = batch.map_err(crate::arrow::from_reader_error)?;
                writer::write_batch(&mut rendered, &row, &batch, layout)?;
            }
            writer::write_document_end(&mut rendered, &root, layout)?;
            handle.truncate(end)?;
            handle.pwrite_all(end, &rendered)?;
            return handle.flush();
        }
    }

    // No addressable end tag: the document is decoded, its rows are carried
    // forward, and the whole value is written back once.
    let stored_rows = read_batch_reader(handle, None, options)?;
    let combined = crate::arrow::combined(stored_rows, batches)?;
    overwrite_arrow_reader(handle, combined, options)
}

/// Return where the document element's end tag begins.
///
/// The end tag is the last markup a document holds, so this reads the tail
/// rather than the document. `None` means the tail is not a plain end tag -
/// an empty document element, or trailing content - and the caller falls back
/// to reading the document.
fn document_end<H: IOBase + ?Sized>(handle: &H, root: &str) -> Result<Option<u64>> {
    let size = handle.size();
    let mut window = 64_u64;
    loop {
        let start = size.saturating_sub(window);
        let length = usize::try_from(size - start).unwrap_or(usize::MAX);
        let bytes = handle.read_range_bytes(start, length)?;
        if let Some(offset) = memchr::memrchr(b'<', &bytes) {
            let Ok(tail) = std::str::from_utf8(&bytes[offset..]) else {
                return Ok(None);
            };
            let Some(rest) = tail.strip_prefix("</") else {
                return Ok(None);
            };
            let Some(close) = rest.find('>') else {
                return Ok(None);
            };
            if rest[..close].trim() != root || !rest[close + 1..].trim().is_empty() {
                return Ok(None);
            }
            // The whitespace that only introduced the end tag goes with it, so
            // an append writes its own layout instead of stacking a blank line
            // on every one before it.
            let kept = bytes[..offset]
                .iter()
                .rposition(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
                .map_or(0, |index| index + 1);
            return Ok(Some(start + kept as u64));
        }
        if start == 0 || window >= u64::from(u32::MAX) {
            return Ok(None);
        }
        window = window.saturating_mul(8);
    }
}

/// Return the document and row element names a write should keep using.
///
/// A stored document decides both, because appending under a different name
/// would leave two shapes in one document; a document with nothing stored
/// takes the declared names.
pub(crate) fn stored_names<H: IOBase + ?Sized>(
    handle: &H,
    options: &XmlOptions,
) -> Result<(String, String)> {
    if handle.is_empty() {
        return Ok((
            options.write_root().to_owned(),
            options.write_row().to_owned(),
        ));
    }
    let (root, row) = super::reader::read_names(decoded(handle)?, options)?;
    Ok((
        root.map_or_else(|| options.write_root().to_owned(), Into::into),
        row.map_or_else(|| options.write_row().to_owned(), Into::into),
    ))
}

/// Restate one natural row so the field it is read under can accept it.
///
/// Two things an element says differ from what a record says, and both are
/// answered here rather than by widening what every format accepts: an element
/// a row leaves out is that column absent, and an element that occurs once
/// under a list column is a list of one, because the wire spells a list by
/// repeating the element.
fn shaped(value: Scalar, field: &Field) -> Result<Scalar> {
    match field.dtype() {
        DataType::Struct(fields) => {
            let Scalar::Nested(Nested::Record(_)) = &value else {
                return Ok(value);
            };
            let entries = fields
                .iter()
                .map(|child| {
                    let held = value
                        .get_key_str(child.name())
                        .cloned()
                        .unwrap_or(Scalar::Null);
                    Ok((SmolStr::new(child.name()), shaped(held, child)?))
                })
                .collect::<Result<Vec<_>>>()?;
            Scalar::from_record(entries)
        }
        DataType::List(item)
        | DataType::ListView(item)
        | DataType::LargeList(item)
        | DataType::LargeListView(item)
        | DataType::FixedSizeList(item, _) => {
            if value.is_null() {
                return Ok(value);
            }
            match value.as_sequence() {
                Some(values) => values
                    .iter()
                    .cloned()
                    .map(|value| shaped(value, item))
                    .collect::<Result<Vec<_>>>()
                    .map(Scalar::from_sequence),
                None => shaped(value, item).map(|value| Scalar::from_sequence([value])),
            }
        }
        _ => Ok(value),
    }
}

/// The canonical field of a document that holds no rows.
fn empty_field(name: &str) -> Result<Field> {
    Ok(Field::new(
        name,
        DataType::from_fields(Vec::<Field>::new())?,
        false,
    ))
}

/// Read a handle's bytes through whatever content coding it declares.
fn decoded<H: IOBase + ?Sized>(handle: &H) -> Result<Box<dyn Read + '_>> {
    let codings = handle.media_type().encodings().to_vec();
    Ok(Box::new(DecodedReader::new(
        Box::new(handle.pstream_bytes(0, FETCH_BYTE_SIZE)?),
        codings,
    )))
}

/// Return a decoded source that outlives the borrow it was built from.
fn owned_source<H: IOBase + ?Sized>(handle: &H) -> Result<Box<dyn Read + Send + 'static>> {
    let owned = owned_leaf(handle)?;
    let codings = owned.media_type().encodings().to_vec();
    Ok(Box::new(SendDecodedReader::new(
        Box::new(Cursor::new(owned)),
        codings,
    )))
}
