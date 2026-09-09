//! `text/csv` rows through the shared Scalar/Arrow record boundary.
//!
//! Reading is two bounded passes: one resolves the columns - the header names
//! them, a sample of cells types them - and one streams the rows under that
//! answer. Writing renders through Arrow's own array formatter, which is the
//! same formatter a partition directory name goes through, so a value spells
//! itself one way across the crate.

use std::io::{Read, Write};

use arrow_cast::display::{ArrayFormatter, FormatOptions};
use arrow_schema::Schema;
use smol_str::{SmolStr, format_smolstr};

use crate::arrow::BatchReader;
use crate::media::IORecordOptions;
use crate::{Codec, DataType, Error, Field, IOBase, Result, Scalar, Timezone, Url};

use super::infer::Inference;
use super::options::{CsvOptions, positional_name, root_field};
use super::reader::{Record, Records};
use super::scan::{Dialect, cell_bytes, render_cell};

/// The columns a resource holds and where its rows begin.
#[derive(Clone, Debug)]
pub(crate) struct Layout {
    /// The canonical non-null Struct root the records read as.
    pub(crate) field: Field,
    /// Decoded byte offset of the first data record.
    pub(crate) data_start: u64,
}

/// Read the root Field a resource's own bytes declare.
///
/// A declared datatype is the answer when there is one: it names the columns,
/// and the shared cast reconciles them with what the file holds.
///
/// # Errors
///
/// Returns a read, decoding, header, or ragged-record failure.
pub fn read_field(handle: &(impl IOBase + ?Sized), options: &CsvOptions) -> Result<Field> {
    if let Some(field) = options.field() {
        return Ok(field);
    }
    Ok(read_layout(handle, options)?.field)
}

/// Resolve the columns and the first data offset in one bounded pass.
pub(crate) fn read_layout(handle: &(impl IOBase + ?Sized), options: &CsvOptions) -> Result<Layout> {
    let dialect = options.dialect();
    let source = crate::media::stream::decoded_reader(handle)?;
    let mut records = Records::new(source, dialect.clone(), 0);
    layout_from(&mut records, &dialect, options, handle.url())
}

/// Resolve the columns from a record splitter positioned at the first record.
fn layout_from<R: Read>(
    records: &mut Records<R>,
    dialect: &Dialect,
    options: &CsvOptions,
    url: Option<&Url>,
) -> Result<Layout> {
    let mut names: Vec<SmolStr> = Vec::new();
    let mut data_start = 0;
    if options.header() {
        if let Some(record) = records.next_record() {
            let record = record?;
            names = header_names(&record, dialect, url)?;
            data_start = record.offset + record.stride;
        }
    }
    if let Some(declared) = options.field() {
        return Ok(Layout {
            field: declared_columns(&declared, &names, &options.name)?,
            data_start,
        });
    }
    // Without a header the first record's width names the columns, so one
    // record is read even when nothing is being typed.
    let sample = if options.autotype() {
        options.infer_row_size()
    } else {
        Some(1)
    };
    let mut inferences: Vec<Inference> = names.iter().map(|_| Inference::new()).collect();
    let mut seen = 0_usize;
    while sample.is_none_or(|limit| seen < limit) {
        let Some(record) = records.next_record() else {
            break;
        };
        let record = record?;
        if names.is_empty() {
            names = (0..record.spans.len()).map(positional_name).collect();
            inferences = names.iter().map(|_| Inference::new()).collect();
        }
        require_width(&record, names.len(), seen as u64, url)?;
        if options.autotype() {
            for (span, inference) in record.spans.iter().zip(&mut inferences) {
                let cell = cell_bytes(record.bytes, *span, dialect);
                inference.observe(&cell, span.is_quoted(), options.null(), options.timezone());
            }
        }
        seen += 1;
    }
    let fields = names
        .into_iter()
        .zip(inferences)
        .map(|(name, inference)| {
            if options.autotype() {
                inference.into_field(name)
            } else {
                DataType::Utf8.nullable_field(name)
            }
        })
        .collect();
    Ok(Layout {
        field: root_field(&options.name, fields)?,
        data_start,
    })
}

/// Emit the columns a declaration names, keyed by the header where there is one.
///
/// The header is what says which cell is which, so a declaration whose columns
/// are in another order still reads the right cells: the emitted column keeps
/// the header's name and takes the declared datatype by that name, and the
/// shared cast reorders, renames, and completes from there. A declared column
/// the cells cannot spell is read as text and converted by that same cast, so
/// a decimal or an unsigned width needs no second text-to-value path here.
/// Every emitted column is nullable - an empty cell is absence - and the
/// declared nullability is enforced by the cast.
fn declared_columns(declared: &Field, header: &[SmolStr], name: &SmolStr) -> Result<Field> {
    let named = |column: &Field| {
        if super::infer::is_read_dtype(column.dtype()) {
            column.dtype().clone()
        } else {
            DataType::Utf8
        }
    };
    if header.is_empty() {
        // No header: the declaration's own order is the only thing that can
        // say which cell is which.
        let columns = declared
            .fields()
            .iter()
            .map(|column| named(column).nullable_field(column.name()))
            .collect();
        return root_field(name, columns);
    }
    let columns = header
        .iter()
        .map(|column| {
            let dtype = declared
                .fields()
                .iter()
                .find(|declared| declared.name() == column.as_str())
                .map_or(DataType::Utf8, named);
            dtype.nullable_field(column.clone())
        })
        .collect();
    root_field(name, columns)
}

/// Read the column names one header record spells.
fn header_names(record: &Record<'_>, dialect: &Dialect, url: Option<&Url>) -> Result<Vec<SmolStr>> {
    record
        .spans
        .iter()
        .enumerate()
        .map(|(index, span)| {
            let cell = cell_bytes(record.bytes, *span, dialect);
            let name = std::str::from_utf8(&cell).map_err(|error| Error::InvalidRecord {
                path: format_smolstr!("$[0][{index}]"),
                reason: format_smolstr!(
                    "expected a UTF-8 column name in the header of {}, got an invalid byte at {}",
                    named(url),
                    error.valid_up_to()
                ),
            })?;
            let name = name.trim();
            Ok(if name.is_empty() {
                positional_name(index)
            } else {
                SmolStr::new(name)
            })
        })
        .collect()
}

/// Refuse a record whose width is not the width the columns declare.
fn require_width(record: &Record<'_>, columns: usize, index: u64, url: Option<&Url>) -> Result<()> {
    if record.spans.len() == columns {
        return Ok(());
    }
    Err(Error::InvalidRecord {
        path: format_smolstr!("$[{index}]"),
        reason: format_smolstr!(
            "expected {columns} cells in row {index} of {}, got {}",
            named(url),
            record.spans.len()
        ),
    })
}

/// Name the resource a failure happened in.
fn named(url: Option<&Url>) -> SmolStr {
    url.map_or_else(
        || SmolStr::new_static("the resource"),
        |url| format_smolstr!("{url}"),
    )
}

/// Decode one borrowed leaf into ordinary record batches.
///
/// # Errors
///
/// Returns a read, decoding, header, ragged-record, or cell-conversion failure.
pub fn read_arrow_reader(
    handle: &(impl IOBase + ?Sized),
    options: &CsvOptions,
) -> Result<BatchReader> {
    read_owned_arrow_reader(crate::media::stream::owned_handle(handle)?, options)
}

/// Decode an owned leaf without retaining decoded pages in its caller.
///
/// # Errors
///
/// Returns the same failures as [`read_arrow_reader`].
pub fn read_owned_arrow_reader<H: IOBase + 'static>(
    handle: H,
    options: &CsvOptions,
) -> Result<BatchReader> {
    let url = handle.url().cloned();
    let layout = read_layout(&handle, options)?;
    let dialect = options.dialect();
    let dtypes = layout
        .field
        .fields()
        .iter()
        .map(|column| column.dtype().clone())
        .collect::<Vec<_>>();
    let source = crate::media::stream::owned_decoded_reader(handle);
    let mut records = Records::new(source, dialect.clone(), 0);
    if layout.data_start > 0 {
        // The header is one record; consuming it puts the splitter on the
        // first data row without a second positional read.
        let _ = records.next_record().transpose()?;
    }
    let rows = Rows {
        records,
        dialect,
        dtypes,
        null: SmolStr::new(options.null()),
        timezone: options.timezone().copied(),
        url,
        index: 0,
        done: false,
    };
    Ok(crate::arrow::rows::result_reader(
        &layout.field,
        rows,
        options.batch_row_size(),
        None,
        None,
    )?)
}

/// Count data records without materializing rows or Arrow arrays.
///
/// # Errors
///
/// Returns a read, decoding, or row-count overflow failure.
pub fn row_size(handle: &(impl IOBase + ?Sized), options: &CsvOptions) -> Result<u64> {
    let dialect = options.dialect();
    let source = crate::media::stream::decoded_reader(handle)?;
    let mut records = Records::new(source, dialect, 0);
    let mut rows = 0_u64;
    while let Some(record) = records.next_record() {
        record?;
        rows = rows.checked_add(1).ok_or_else(|| Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("record count exceeds u64::MAX"),
        })?;
    }
    // The header is a record of the resource and not a row of the table.
    Ok(if options.header() {
        rows.saturating_sub(1)
    } else {
        rows
    })
}

/// Records converted under the columns resolved before the first read.
struct Rows<R> {
    records: Records<R>,
    dialect: Dialect,
    dtypes: Vec<DataType>,
    null: SmolStr,
    timezone: Option<Timezone>,
    url: Option<Url>,
    index: u64,
    done: bool,
}

impl<R: Read> Iterator for Rows<R> {
    type Item = Result<Scalar>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        // Destructured so the record's borrow of the splitter and the reads of
        // the conversion settings are the disjoint field borrows they are.
        let Self {
            records,
            dialect,
            dtypes,
            null,
            timezone,
            url,
            index,
            done,
        } = self;
        let record = match records.next_record()? {
            Ok(record) => record,
            Err(error) => {
                *done = true;
                return Some(Err(error));
            }
        };
        let row = record_scalar(
            &record,
            dialect,
            dtypes,
            null,
            timezone.as_ref(),
            *index,
            url.as_ref(),
        );
        if row.is_err() {
            *done = true;
        }
        *index += 1;
        Some(row)
    }
}

/// Convert one record into the ordered column values of one row.
pub(crate) fn record_scalar(
    record: &Record<'_>,
    dialect: &Dialect,
    dtypes: &[DataType],
    null: &str,
    timezone: Option<&Timezone>,
    index: u64,
    url: Option<&Url>,
) -> Result<Scalar> {
    require_width(record, dtypes.len(), index, url)?;
    let mut values = Vec::with_capacity(dtypes.len());
    for (column, (span, dtype)) in record.spans.iter().zip(dtypes).enumerate() {
        let cell = cell_bytes(record.bytes, *span, dialect);
        let value =
            cell_scalar(&cell, span.is_quoted(), dtype, null, timezone).map_err(|reason| {
                Error::InvalidRecord {
                    path: format_smolstr!("$[{index}][{column}]"),
                    reason: format_smolstr!("{reason} in row {index} of {}", named(url)),
                }
            })?;
        values.push(value);
    }
    Ok(Scalar::from_sequence(values))
}

/// Read one cell as the value its column declares.
///
/// Absence is the `null` spelling on an *unquoted* cell. A quoted cell is
/// content whatever it spells, which is what lets a resource hold both an
/// absent value and an empty string under the default spelling.
fn cell_scalar(
    cell: &[u8],
    quoted: bool,
    dtype: &DataType,
    null: &str,
    timezone: Option<&Timezone>,
) -> std::result::Result<Scalar, SmolStr> {
    if !quoted && cell == null.as_bytes() {
        return Ok(Scalar::Null);
    }
    if cell.is_empty() {
        // An empty cell is the empty string only where the column can hold
        // one; nothing else this vocabulary spells has an empty reading.
        return Ok(match dtype {
            DataType::Utf8 => Scalar::from(""),
            _ => Scalar::Null,
        });
    }
    let text = std::str::from_utf8(cell).map_err(|error| {
        format_smolstr!(
            "expected a UTF-8 cell, got an invalid byte at {}",
            error.valid_up_to()
        )
    })?;
    crate::media::text::arrow::parse_capture(text, dtype, timezone)
}

/// Replace a leaf with the rows an incoming reader carries.
///
/// # Errors
///
/// Returns a rendering, encoding, or write failure.
pub fn overwrite_arrow_reader(
    handle: &mut (impl IOBase + ?Sized),
    batches: BatchReader,
    options: &CsvOptions,
) -> Result<()> {
    // The rendered value is staged whole and published once, as every encoding
    // in this crate does, so a batch that fails to render never leaves a
    // half-written resource behind.
    let mut encoded = Vec::new();
    {
        let mut encoder = handle
            .codec()
            .writer_with_level(&mut encoded, options.level());
        render_rows(batches, options, options.header(), None, &mut encoder)?;
        encoder.finish()?;
    }
    handle.write_all_bytes(&encoded)
}

/// Add an incoming reader's rows after the leaf's current final record.
///
/// # Errors
///
/// Returns a header-agreement, rendering, encoding, or write failure.
pub fn append_arrow_reader(
    handle: &mut (impl IOBase + ?Sized),
    batches: BatchReader,
    options: &CsvOptions,
) -> Result<()> {
    // A resource holding no records has no header either, whatever its byte
    // size says, so it is filled rather than appended to.
    let stored = stored_header(handle, options)?;
    if handle.is_empty() || (options.header() && stored.is_none()) {
        return overwrite_arrow_reader(handle, batches, options);
    }
    // The stored header decides the cell order, so an incoming batch naming the
    // same columns in another order still lands in the columns it named.
    let order = stored
        .map(|stored| column_order(&stored, batches.schema().as_ref()))
        .transpose()?;
    let terminator = options.output_linesep();
    let codec = handle.codec();
    if codec == Codec::Identity {
        // Only the added rows are rendered: what is already stored stays where
        // it is, which is what makes an append cheap on a delimited resource.
        let mut rendered = Vec::new();
        render_rows(batches, options, false, order.as_deref(), &mut rendered)?;
        let mut offset = handle.size();
        if offset > 0 && !ends_with(handle, terminator)? {
            handle.pwrite_all(offset, terminator)?;
            offset += terminator.len() as u64;
        }
        handle.pwrite_all(offset, &rendered)?;
        return handle.flush();
    }
    let mut encoded = Vec::new();
    {
        let mut encoder = codec.writer_with_level(&mut encoded, options.level());
        let mut suffix = Vec::new();
        let source = crate::media::stream::decoded_reader(handle)?;
        let mut decoder = crate::media::stream::fetched(source);
        let mut chunk = vec![0; crate::DEFAULT_STREAM_BATCH_SIZE];
        loop {
            let read = decoder.read(&mut chunk).map_err(Error::Io)?;
            if read == 0 {
                break;
            }
            update_suffix(&mut suffix, &chunk[..read], terminator.len());
            encoder.write_all(&chunk[..read])?;
        }
        if !suffix.is_empty() && suffix.as_slice() != terminator {
            encoder.write_all(terminator)?;
        }
        render_rows(batches, options, false, order.as_deref(), &mut encoder)?;
        encoder.finish()?;
    }
    handle.write_all_bytes(&encoded)
}

/// Read the column names the resource already stores, if it stores any.
fn stored_header(
    handle: &(impl IOBase + ?Sized),
    options: &CsvOptions,
) -> Result<Option<Vec<SmolStr>>> {
    if !options.header() {
        return Ok(None);
    }
    let dialect = options.dialect();
    let source = crate::media::stream::decoded_reader(handle)?;
    let mut records = Records::new(source, dialect.clone(), 0);
    let Some(record) = records.next_record() else {
        return Ok(None);
    };
    header_names(&record?, &dialect, handle.url()).map(Some)
}

/// Map the stored header's columns onto an incoming batch's columns.
fn column_order(stored: &[SmolStr], incoming: &Schema) -> Result<Vec<usize>> {
    if stored.len() != incoming.fields().len() {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: format_smolstr!(
                "expected an appended row to carry the {} stored columns [{}], got {}",
                stored.len(),
                stored.join(", "),
                incoming.fields().len()
            ),
        });
    }
    stored
        .iter()
        .map(|name| {
            incoming
                .fields()
                .iter()
                .position(|field| field.name() == name.as_str())
                .ok_or_else(|| Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!(
                        "expected an appended row to carry the stored column {name:?}, got [{}]",
                        incoming
                            .fields()
                            .iter()
                            .map(|field| field.name().as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                })
        })
        .collect()
}

/// Render an incoming reader's header and rows into one byte sink.
fn render_rows(
    batches: BatchReader,
    options: &CsvOptions,
    header: bool,
    order: Option<&[usize]>,
    target: &mut impl Write,
) -> Result<()> {
    let dialect = options.dialect();
    let terminator = options.output_linesep();
    let schema = batches.schema();
    let mut line = Vec::with_capacity(crate::DEFAULT_STREAM_BATCH_SIZE);
    if header {
        let names = schema.fields().iter().map(|field| field.name().as_bytes());
        write_record(names, &dialect, terminator, &mut line);
        target.write_all(&line).map_err(Error::Io)?;
    }
    // Arrow's own formatter renders every column, so a value spells itself the
    // same way here as in a partition directory name.
    let format = FormatOptions::new().with_null(options.null());
    let mut cell = String::new();
    for batch in batches {
        let batch = batch.map_err(crate::arrow::from_reader_error)?;
        let columns: Vec<usize> = match order {
            Some(order) => order.to_vec(),
            None => (0..batch.num_columns()).collect(),
        };
        let arrays = columns
            .iter()
            .map(|index| batch.column(*index))
            .collect::<Vec<_>>();
        let formatters = arrays
            .iter()
            .map(|array| ArrayFormatter::try_new(array.as_ref(), &format).map_err(Error::Arrow))
            .collect::<Result<Vec<_>>>()?;
        line.clear();
        for row in 0..batch.num_rows() {
            for (column, formatter) in formatters.iter().enumerate() {
                if column > 0 {
                    line.push(dialect.separator);
                }
                if arrays[column].is_null(row) {
                    line.extend_from_slice(options.null().as_bytes());
                    continue;
                }
                cell.clear();
                formatter
                    .value(row)
                    .write(&mut cell)
                    .map_err(Error::Arrow)?;
                // A present value that happens to spell absence - the empty
                // string under the default spelling - is quoted, so reading it
                // back answers the value rather than a null.
                render_cell(cell.as_bytes(), &dialect, cell == options.null(), &mut line);
            }
            line.extend_from_slice(terminator);
            if line.len() >= crate::DEFAULT_STREAM_BATCH_SIZE {
                target.write_all(&line).map_err(Error::Io)?;
                line.clear();
            }
        }
        target.write_all(&line).map_err(Error::Io)?;
        line.clear();
    }
    Ok(())
}

/// Render one value as the cell text its column spells.
///
/// The one-element array is what Arrow's formatter reads, so a value written
/// through the positional surface spells itself exactly as the same value
/// written through a batch.
fn cell_text(field: &Field, value: &Scalar, null: &str) -> Result<String> {
    let array = crate::arrow::scalar_array(field, value)?;
    let format = FormatOptions::new().with_null(null);
    let formatter = ArrayFormatter::try_new(array.as_ref(), &format).map_err(Error::Arrow)?;
    let mut text = String::new();
    formatter.value(0).write(&mut text).map_err(Error::Arrow)?;
    Ok(text)
}

/// Append one value as the cell its column spells, quoted when it must be.
pub(crate) fn render_value(
    field: &Field,
    value: &Scalar,
    options: &CsvOptions,
    dialect: &Dialect,
    line: &mut Vec<u8>,
) -> Result<()> {
    if value.is_null() {
        line.extend_from_slice(options.null().as_bytes());
        return Ok(());
    }
    let text = cell_text(field, value, options.null())?;
    render_cell(text.as_bytes(), dialect, text == options.null(), line);
    Ok(())
}

/// Render one row's ordered column values as one complete record.
///
/// `terminator` is empty when the record replaces one already stored: the
/// terminator that ended it stays where it is, so a resource of mixed
/// terminators keeps each of them.
pub(crate) fn write_row(
    root: &Field,
    value: &Scalar,
    options: &CsvOptions,
    terminator: &[u8],
    line: &mut Vec<u8>,
) -> Result<()> {
    let row = root.canonicalize_value(value.clone())?;
    let values = row.as_sequence().ok_or_else(|| Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: crate::text::expected_got("an ordered sequence of column values", row.kind()),
    })?;
    if values.len() != root.field_len() {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: format_smolstr!(
                "expected {} column values, got {}",
                root.field_len(),
                values.len()
            ),
        });
    }
    let dialect = options.dialect();
    line.clear();
    for (column, (field, value)) in root.fields().iter().zip(values).enumerate() {
        if column > 0 {
            line.push(dialect.separator);
        }
        render_value(field, value, options, &dialect, line)?;
    }
    line.extend_from_slice(terminator);
    Ok(())
}

/// Render one complete record from already-rendered cells.
fn write_record<'cell>(
    cells: impl IntoIterator<Item = &'cell [u8]>,
    dialect: &Dialect,
    terminator: &[u8],
    line: &mut Vec<u8>,
) {
    line.clear();
    for (column, value) in cells.into_iter().enumerate() {
        if column > 0 {
            line.push(dialect.separator);
        }
        render_cell(value, dialect, false, line);
    }
    line.extend_from_slice(terminator);
}

/// Return whether the stored value already ends with `suffix`.
pub(crate) fn ends_with(handle: &(impl IOBase + ?Sized), suffix: &[u8]) -> Result<bool> {
    let size = handle.size();
    if size < suffix.len() as u64 {
        return Ok(false);
    }
    Ok(handle.read_range_bytes(size - suffix.len() as u64, suffix.len())? == suffix)
}

/// Retain only the last `width` bytes seen, across chunk boundaries.
fn update_suffix(suffix: &mut Vec<u8>, bytes: &[u8], width: usize) {
    if width == 0 {
        return;
    }
    if bytes.len() >= width {
        suffix.clear();
        suffix.extend_from_slice(&bytes[bytes.len() - width..]);
        return;
    }
    suffix.extend_from_slice(bytes);
    if suffix.len() > width {
        suffix.drain(..suffix.len() - width);
    }
}
