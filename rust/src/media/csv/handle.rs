//! Stateful CSV record media over one byte handle.
//!
//! `Csv<H>` is the ordinary record surface plus the one thing a delimited text
//! resource can offer that a container cannot: positional access. A row or a
//! cell is addressable, readable as raw bytes, as text, or as a value, and
//! writable in place - which is what the byte handle underneath was already
//! shaped for.

#[cfg(feature = "arrow")]
use std::ops::Range;
#[cfg(feature = "arrow")]
use std::sync::OnceLock;

#[cfg(feature = "arrow")]
use smol_str::{SmolStr, format_smolstr};

#[cfg(feature = "arrow")]
use crate::media::{IORecordOptions as _, RecordOptions};
#[cfg(feature = "arrow")]
use crate::{Error, Field, Result, Scalar};
use crate::{IOBase, IOMedia};

use super::options::CsvOptions;

#[cfg(feature = "arrow")]
use super::arrow::{Layout, cast_row, read_layout, render_value, stored_terminator, write_row};
#[cfg(feature = "arrow")]
use super::index::RowIndex;
#[cfg(feature = "arrow")]
use super::random::{OwnedRecord, read_record, splice};
#[cfg(feature = "arrow")]
use super::scan::cell_bytes;

/// A byte handle retained with one flat CSV configuration.
#[derive(Debug)]
pub struct Csv<H: IOBase> {
    handle: H,
    options: CsvOptions,
    /// Explicit lifecycle state: an opened empty resource caches an answer
    /// that is indistinguishable from no cache at all.
    opened: bool,
    #[cfg(feature = "arrow")]
    cached_layout: OnceLock<Layout>,
    #[cfg(feature = "arrow")]
    cached_index: OnceLock<RowIndex>,
}

impl<H: IOBase> Csv<H> {
    /// Wrap a handle with comma-separated defaults and a header row.
    #[must_use]
    pub fn new(handle: H) -> Self {
        Self {
            handle,
            options: CsvOptions::new(),
            opened: false,
            #[cfg(feature = "arrow")]
            cached_layout: OnceLock::new(),
            #[cfg(feature = "arrow")]
            cached_index: OnceLock::new(),
        }
    }

    /// Return this media with a complete CSV configuration.
    #[must_use]
    pub fn with_options(mut self, options: CsvOptions) -> Self {
        self.options = options;
        self.invalidate();
        self
    }

    /// Return this media with a declared canonical row field.
    #[must_use]
    #[cfg(feature = "arrow")]
    pub fn with_field(mut self, field: Field) -> Self {
        self.options.set_field(field);
        self.invalidate();
        self
    }

    /// Borrow the retained CSV options.
    pub const fn options(&self) -> &CsvOptions {
        &self.options
    }

    /// Borrow the retained CSV options mutably.
    pub fn options_mut(&mut self) -> &mut CsvOptions {
        self.invalidate();
        &mut self.options
    }

    /// Borrow the underlying byte handle.
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// Borrow the underlying byte handle mutably.
    pub fn handle_mut(&mut self) -> &mut H {
        self.invalidate();
        &mut self.handle
    }

    /// Consume this media and return its byte handle.
    pub fn into_handle(self) -> H {
        self.handle
    }

    /// Return this CSV media unchanged.
    ///
    /// This inherent method makes ordinary `handle.into_csv().into_csv()`
    /// idempotent because inherent methods win over [`IOBase::into_csv`].
    #[must_use]
    pub fn into_csv(self) -> Self {
        self
    }

    /// Replace the retained configuration without nesting another wrapper.
    #[must_use]
    pub fn into_csv_with(self, options: CsvOptions) -> Self {
        self.with_options(options)
    }

    /// Drop what an opened session cached after the bytes moved under it.
    fn invalidate(&mut self) {
        #[cfg(feature = "arrow")]
        {
            self.cached_layout.take();
            self.cached_index.take();
        }
    }
}

#[cfg(feature = "arrow")]
impl<H: IOBase> Csv<H> {
    /// Return how many data records the resource holds.
    ///
    /// # Errors
    ///
    /// Returns a read or decoding failure.
    pub fn row_size(&self) -> Result<u64> {
        // One counter, whatever this handle has cached: wrapping a resource
        // must not change what counting its rows answers.
        super::arrow::row_size(&self.handle, &self.options)
    }

    /// Read one row as its ordered column values, or `None` past the end.
    ///
    /// # Errors
    ///
    /// Returns a read, decoding, ragged-record, or cell-conversion failure.
    pub fn read_row_scalar(&self, row: u64) -> Result<Option<Scalar>> {
        let layout = self.layout()?;
        let Some(record) = self.record_at(row)? else {
            return Ok(None);
        };
        let dtypes = layout
            .read
            .fields()
            .iter()
            .map(|column| column.dtype().clone())
            .collect::<Vec<_>>();
        let borrowed = super::reader::Record {
            bytes: &record.bytes,
            spans: &record.spans,
            offset: record.offset,
            stride: record.stride,
        };
        let decoded = super::arrow::record_scalar(
            &borrowed,
            &self.options.dialect(),
            &dtypes,
            self.options.null(),
            self.options.timezone(),
            row,
            self.handle.url(),
        )?;
        if layout.is_exact() {
            return Ok(Some(decoded));
        }
        // A declared column the cells cannot spell was read as text; the same
        // cast the batch path applies makes it the column that was declared.
        cast_row(&layout.read, &layout.field, &decoded, self.options.safe).map(Some)
    }

    /// Read one cell's exact bytes, unescaped, or `None` past the end.
    ///
    /// # Errors
    ///
    /// Returns a read, decoding, or missing-column failure.
    pub fn read_cell_bytes(&self, row: u64, column: usize) -> Result<Option<Vec<u8>>> {
        let Some(record) = self.record_at(row)? else {
            return Ok(None);
        };
        let span = self.require_column(&record, column, row)?;
        Ok(Some(
            cell_bytes(&record.bytes, span, &self.options.dialect()).into_owned(),
        ))
    }

    /// Read one cell as text, or `None` past the end.
    ///
    /// # Errors
    ///
    /// Returns the same failures as [`Self::read_cell_bytes`], plus a refusal
    /// for a cell that is not UTF-8.
    pub fn read_cell_text(&self, row: u64, column: usize) -> Result<Option<String>> {
        let Some(bytes) = self.read_cell_bytes(row, column)? else {
            return Ok(None);
        };
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|error| Error::InvalidRecord {
                path: format_smolstr!("$[{row}][{column}]"),
                reason: format_smolstr!(
                    "expected a UTF-8 cell, got an invalid byte at {}",
                    error.utf8_error().valid_up_to()
                ),
            })
    }

    /// Read one cell as the value its column declares, or `None` past the end.
    ///
    /// # Errors
    ///
    /// Returns the same failures as [`Self::read_cell_bytes`], plus a cell
    /// conversion failure.
    pub fn read_cell_scalar(&self, row: u64, column: usize) -> Result<Option<Scalar>> {
        let Some(values) = self.read_row_scalar(row)? else {
            return Ok(None);
        };
        values
            .as_sequence()
            .and_then(|values| values.get(column).cloned())
            .map(Some)
            .ok_or_else(|| self.missing_column(column, row))
    }

    /// Return where one row's bytes sit in the resource, terminator included.
    ///
    /// # Errors
    ///
    /// Returns a read or decoding failure.
    pub fn read_row_byte_range(&self, row: u64) -> Result<Option<Range<u64>>> {
        Ok(self
            .record_at(row)?
            .map(|record| record.offset..record.offset + record.stride))
    }

    /// Replace one cell with exact bytes, quoting them if the dialect needs it.
    ///
    /// # Errors
    ///
    /// Returns a coded-resource refusal, a missing row or column, or a read or
    /// write failure.
    pub fn write_cell_bytes(&mut self, row: u64, column: usize, value: &[u8]) -> Result<()> {
        let record = self.require_row(row)?;
        let (start, end) = record
            .cell_range(column)
            .ok_or_else(|| self.missing_column(column, row))?;
        let dialect = self.options.dialect();
        let mut rendered = Vec::with_capacity(value.len() + 2);
        // A cell that would read back as absence is quoted, so writing the
        // empty string does not delete the value.
        super::scan::render_cell(
            value,
            &dialect,
            value == self.options.null().as_bytes(),
            &mut rendered,
        )?;
        self.invalidate();
        splice(&mut self.handle, start, end, &rendered)
    }

    /// Replace one cell with text.
    ///
    /// # Errors
    ///
    /// Returns the same failures as [`Self::write_cell_bytes`].
    pub fn write_cell_text(&mut self, row: u64, column: usize, value: &str) -> Result<()> {
        self.write_cell_bytes(row, column, value.as_bytes())
    }

    /// Replace one cell with a value, spelled as its column spells it.
    ///
    /// # Errors
    ///
    /// Returns the same failures as [`Self::write_cell_bytes`], plus a value
    /// the column refuses.
    pub fn write_cell_scalar(&mut self, row: u64, column: usize, value: &Scalar) -> Result<()> {
        let layout = self.layout()?;
        let field = layout
            .field
            .fields()
            .get(column)
            .cloned()
            .ok_or_else(|| self.missing_column(column, row))?;
        let (start, end) = {
            let record = self.require_row(row)?;
            record
                .cell_range(column)
                .ok_or_else(|| self.missing_column(column, row))?
        };
        let dialect = self.options.dialect();
        let mut rendered = Vec::new();
        render_value(&field, value, &self.options, &dialect, &mut rendered)?;
        self.invalidate();
        splice(&mut self.handle, start, end, &rendered)
    }

    /// Replace one whole row with ordered column values.
    ///
    /// # Errors
    ///
    /// Returns a coded-resource refusal, a missing row, a value the columns
    /// refuse, or a read or write failure.
    pub fn write_row_scalar(&mut self, row: u64, value: &Scalar) -> Result<()> {
        let layout = self.layout()?;
        let record = self.require_row(row)?;
        let mut line = Vec::new();
        write_row(&layout.field, value, &self.options, &[], &mut line)?;
        let start = record.offset;
        let end = record.offset + record.bytes.len() as u64;
        self.invalidate();
        splice(&mut self.handle, start, end, &line)
    }

    /// Add one row after the resource's current final record.
    ///
    /// # Errors
    ///
    /// Returns a coded-resource refusal, a value the columns refuse, or a read
    /// or write failure.
    pub fn append_row_scalar(&mut self, value: &Scalar) -> Result<()> {
        if self.options.header() && self.handle.size() == 0 {
            // There is no header to add a row under, and a row written where
            // one belongs is read back as one.
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: SmolStr::new_static(
                    "expected a resource with a header to add a row to, got an empty one - \
                     write the rows so the header is written with them",
                ),
            });
        }
        let layout = self.layout()?;
        let terminator = stored_terminator(&self.handle, &self.options)?;
        let mut line = Vec::new();
        write_row(&layout.field, value, &self.options, &terminator, &mut line)?;
        let size = self.handle.size();
        if size > 0 && !crate::media::stream::ends_with(&self.handle, &terminator)? {
            // The stored final record never got its terminator, so the added
            // row would otherwise continue it rather than follow it.
            let mut opened = terminator.clone();
            opened.append(&mut line);
            line = opened;
        }
        self.invalidate();
        // An append is a splice of nothing, so one implementation answers for
        // every positional write, coded refusal included.
        splice(&mut self.handle, size, size, &line)
    }

    /// Remove one row, closing the gap it leaves.
    ///
    /// # Errors
    ///
    /// Returns a coded-resource refusal, a missing row, or a read or write
    /// failure.
    pub fn remove_row(&mut self, row: u64) -> Result<()> {
        let record = self.require_row(row)?;
        let start = record.offset;
        let end = record.offset + record.stride;
        self.invalidate();
        splice(&mut self.handle, start, end, &[])
    }

    /// Read the record `row` names, through the sparse index when there is one.
    ///
    /// An opened session has the index and pays one positional read plus a
    /// bounded scan. A closed one scans from the first data record: building an
    /// index for a single row, then dropping it, would cost strictly more.
    fn record_at(&self, row: u64) -> Result<Option<OwnedRecord>> {
        let (offset, skip) = match self.opened {
            true => match self.index()?.anchor(row) {
                Some(anchor) => anchor,
                None => return Ok(None),
            },
            false => (
                super::arrow::read_data_start(&self.handle, &self.options)?,
                row,
            ),
        };
        read_record(&self.handle, &self.options.dialect(), offset, skip)
    }

    /// Read the record `row` names, refusing a row the resource does not hold.
    fn require_row(&self, row: u64) -> Result<OwnedRecord> {
        self.record_at(row)?.ok_or_else(|| Error::InvalidRecord {
            path: format_smolstr!("$[{row}]"),
            reason: format_smolstr!("expected a stored row, got row {row} past the final one"),
        })
    }

    /// Return one cell's span, refusing a column the record does not hold.
    fn require_column(
        &self,
        record: &OwnedRecord,
        column: usize,
        row: u64,
    ) -> Result<super::scan::Span> {
        record
            .spans
            .get(column)
            .copied()
            .ok_or_else(|| self.missing_column(column, row))
    }

    /// Name a column a row does not hold.
    fn missing_column(&self, column: usize, row: u64) -> Error {
        Error::InvalidRecord {
            path: format_smolstr!("$[{row}][{column}]"),
            reason: format_smolstr!(
                "expected a stored cell, got column {column} past the final one"
            ),
        }
    }

    /// Return the resolved columns, cached for an opened session.
    fn layout(&self) -> Result<Layout> {
        if !self.opened {
            return read_layout(&self.handle, &self.options);
        }
        if let Some(cached) = self.cached_layout.get() {
            return Ok(cached.clone());
        }
        let resolved = read_layout(&self.handle, &self.options)?;
        // Concurrent immutable asks may race to fill an invalidated cache;
        // whichever answer wins defines this opened session consistently.
        let _ = self.cached_layout.set(resolved.clone());
        Ok(self.cached_layout.get().cloned().unwrap_or(resolved))
    }

    /// Return the row index, cached for an opened session.
    ///
    /// The index needs where the rows begin, not what they hold, so it does not
    /// resolve the columns: reaching a row must not depend on an inference a
    /// ragged resource can refuse.
    fn index(&self) -> Result<RowIndex> {
        let build = || {
            let data_start = super::arrow::read_data_start(&self.handle, &self.options)?;
            RowIndex::build(&self.handle, &self.options.dialect(), data_start)
        };
        if !self.opened {
            return build();
        }
        if let Some(cached) = self.cached_index.get() {
            return Ok(cached.clone());
        }
        let built = build()?;
        let _ = self.cached_index.set(built.clone());
        Ok(self.cached_index.get().cloned().unwrap_or(built))
    }

    /// Refuse options for a different encoding before a write pulls a batch.
    fn require_csv_options<'a>(&self, options: &'a RecordOptions) -> Result<&'a CsvOptions> {
        match options {
            RecordOptions::Csv(options) => Ok(options),
            _ => Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.encoding"),
                reason: crate::text::expected_got("CSV record options", options.mime_type()),
            }),
        }
    }
}

impl<H: IOBase> IOMedia for Csv<H> {
    fn as_io_base(&self) -> &dyn IOBase {
        self
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self
    }

    #[cfg(feature = "arrow")]
    fn row_size(&self) -> Result<u64> {
        Self::row_size(self)
    }

    #[cfg(feature = "arrow")]
    fn column_size(&self) -> Result<usize> {
        Ok(self.layout()?.field.field_len())
    }

    #[cfg(feature = "arrow")]
    fn record_options(&self) -> Result<RecordOptions> {
        Ok(self.options.clone().into())
    }

    #[cfg(feature = "arrow")]
    fn read_arrow_field(&self, options: &RecordOptions) -> Result<Field> {
        let csv = self.require_csv_options(options)?;
        super::arrow::read_field(&self.handle, csv)
    }

    #[cfg(feature = "arrow")]
    fn read_arrow_reader(&self, options: &RecordOptions) -> Result<crate::arrow::BatchReader> {
        self.require_csv_options(options)?;
        IOMedia::read_arrow_reader(&self.handle, options)
    }

    #[cfg(feature = "arrow")]
    fn overwrite_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        self.require_csv_options(options)?;
        self.invalidate();
        IOMedia::overwrite_arrow_reader(&mut self.handle, batches, options)
    }

    #[cfg(feature = "arrow")]
    fn overwrite_prepared_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        self.require_csv_options(options)?;
        self.invalidate();
        IOMedia::overwrite_prepared_arrow_reader(&mut self.handle, batches, options)
    }

    #[cfg(feature = "arrow")]
    fn append_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        self.require_csv_options(options)?;
        self.invalidate();
        IOMedia::append_arrow_reader(&mut self.handle, batches, options)
    }

    #[cfg(feature = "arrow")]
    fn merge_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        self.require_csv_options(options)?;
        self.invalidate();
        IOMedia::merge_arrow_reader(&mut self.handle, batches, options)
    }
}

impl<H: IOBase> IOBase for Csv<H> {
    crate::delegate_iobase!(handle: pread, read_all_bytes, read_range_bytes, pstream_bytes,
        size, capacity, reserve, url, bound_location, media_type, set_media_type, flush,
        parent, child_by_path, ls, kind, is_io);

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
        self.invalidate();
        self.handle.pwrite(offset, bytes)
    }

    fn truncate(&mut self, size: u64) -> crate::Result<()> {
        self.invalidate();
        self.handle.truncate(size)
    }

    /// Delimited text stores rows and columns, whatever the bytes underneath
    /// would otherwise be taken for.
    fn is_tabular(&self) -> bool {
        true
    }

    /// A record encoding is never read as one whole byte value.
    fn is_atomic(&self) -> bool {
        false
    }

    /// Materialize the handle and cache the columns this session reads with.
    fn open(&mut self) -> crate::Result<()> {
        if self.opened {
            return Ok(());
        }
        self.handle.open()?;
        self.invalidate();
        self.opened = true;
        Ok(())
    }

    /// Return explicit lifecycle state, including for an empty resource.
    fn opened(&self) -> bool {
        self.opened
    }

    /// Publish the handle and drop what this session cached.
    fn close(&mut self) -> crate::Result<()> {
        self.opened = false;
        self.invalidate();
        self.handle.close()
    }

    /// Empty the resource and drop the cached columns with it.
    fn clear(&mut self) -> crate::Result<()> {
        self.invalidate();
        self.handle.clear()
    }

    /// Delete the resource and drop the cached columns with it.
    fn remove(&mut self, recursive: bool) -> crate::Result<()> {
        self.invalidate();
        self.handle.remove(recursive)
    }
}
