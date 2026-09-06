//! Stateful XML record media over one byte handle.

use std::sync::{Arc, OnceLock};

use smol_str::SmolStr;

use crate::arrow::BatchReader;
use crate::media::{IORecordOptions as _, RecordOptions};
use crate::{Field, IOBase, IOMedia, Result, Scalar};

use super::index::{self, RowIndex};
use super::options::XmlOptions;
use super::{arrow, random};

/// An XML document read and written as rows.
///
/// Every read and write goes through this type, so the handle, the options and
/// the row index live in one place instead of being repeated at each call.
/// [`IOBase::open`] materializes the handle and caches that index, so a run of
/// positional reads and writes shares one scan of the document;
/// [`IOBase::close`] publishes and drops it, and every mutation that could move
/// a row invalidates it as part of the call.
#[derive(Debug)]
pub struct Xml<H: IOBase> {
    handle: H,
    options: XmlOptions,
    /// Explicit lifecycle state: an opened empty document has no rows, so the
    /// presence of an index cannot answer whether the wrapper is open.
    opened: bool,
    cached: OnceLock<Arc<RowIndex>>,
}

impl<H: IOBase> Xml<H> {
    /// Bind an XML document to a handle.
    #[must_use]
    pub fn new(handle: H) -> Self {
        Self {
            handle,
            options: XmlOptions::new(),
            opened: false,
            cached: OnceLock::new(),
        }
    }

    /// Return this document with a complete configuration.
    #[must_use]
    pub fn with_options(mut self, options: XmlOptions) -> Self {
        self.options = options;
        self.invalidate_index();
        self
    }

    /// Return this document with a declared canonical row field.
    #[must_use]
    pub fn with_field(mut self, field: Field) -> Self {
        self.options.set_field(field);
        self.invalidate_index();
        self
    }

    /// Return this document with a different root Field name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<SmolStr>) -> Self {
        self.options.set_name(name.into());
        self.invalidate_index();
        self
    }

    /// Borrow the retained options.
    pub const fn options(&self) -> &XmlOptions {
        &self.options
    }

    /// Borrow the retained options mutably.
    pub fn options_mut(&mut self) -> &mut XmlOptions {
        self.invalidate_index();
        &mut self.options
    }

    /// Borrow the underlying byte handle.
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// Borrow the underlying byte handle mutably.
    pub fn handle_mut(&mut self) -> &mut H {
        self.invalidate_index();
        &mut self.handle
    }

    /// Consume this media and return its byte handle.
    pub fn into_handle(self) -> H {
        self.handle
    }

    /// Return this XML media unchanged.
    ///
    /// This inherent method makes `handle.into_xml().into_xml()` idempotent,
    /// because an inherent method wins over [`IOBase::into_xml`].
    #[must_use]
    pub fn into_xml(self) -> Self {
        self
    }

    /// Replace the retained configuration without nesting another wrapper.
    #[must_use]
    pub fn into_xml_with(self, options: XmlOptions) -> Self {
        self.with_options(options)
    }

    /// Read where every row of this document begins and ends.
    ///
    /// An opened document reads the index once and answers every later ask
    /// from it; a closed one answers a fresh scan each time.
    ///
    /// # Errors
    ///
    /// Returns an error when the handle declares a content coding, because a
    /// row's address is an offset into stored bytes, plus any read or parse
    /// failure.
    pub fn read_row_index(&self) -> Result<Arc<RowIndex>> {
        if !self.opened {
            return index::read_row_index(&self.handle, &self.options).map(Arc::new);
        }
        if let Some(cached) = self.cached.get() {
            return Ok(Arc::clone(cached));
        }
        let read = Arc::new(index::read_row_index(&self.handle, &self.options)?);
        // Concurrent immutable asks may race to refill an invalidated index;
        // whichever answer wins describes this opened session consistently.
        let _ = self.cached.set(Arc::clone(&read));
        Ok(read)
    }

    /// Read one row by position.
    ///
    /// Only that row's own bytes are read and parsed: the rows before it are
    /// never decoded.
    ///
    /// # Errors
    ///
    /// Returns an error for a row the document does not hold, and any read or
    /// parse failure.
    pub fn read_row_scalar(&self, row: u64) -> Result<Scalar> {
        let index = self.read_row_index()?;
        random::read_row_scalar(&self.handle, &index, row)
    }

    /// Read `count` rows starting at `offset`.
    ///
    /// # Errors
    ///
    /// Returns any read or parse failure.
    pub fn read_range_scalars(&self, offset: u64, count: usize) -> Result<Vec<Scalar>> {
        let index = self.read_row_index()?;
        random::read_range_scalars(&self.handle, &index, offset, count)
    }

    /// Read `count` rows starting at `offset` as record batches.
    ///
    /// # Errors
    ///
    /// Returns any read, parse, schema, or cast failure.
    pub fn read_range_arrow_reader(&self, offset: u64, count: usize) -> Result<BatchReader> {
        let index = self.read_row_index()?;
        arrow::read_range_batch_reader(&self.handle, &index, &self.options, offset, count)
    }

    /// Replace one row by position.
    ///
    /// A replacement of the same byte length is written exactly where the old
    /// row was; a different length moves only the bytes after it.
    ///
    /// # Errors
    ///
    /// Returns an error for a row the document does not hold, a value XML
    /// cannot spell, and any read or write failure.
    pub fn write_row_scalar(&mut self, row: u64, value: &Scalar) -> Result<()> {
        self.splice(row, 1, [value]).map(|_| ())
    }

    /// Add rows after the document's current last one.
    ///
    /// Only the document element's end tag is rewritten, so this costs the
    /// rows being added rather than the document already there.
    ///
    /// # Errors
    ///
    /// Returns a value XML cannot spell, and any read or write failure.
    pub fn append_row_scalars<I>(&mut self, values: I) -> Result<u64>
    where
        I: IntoIterator,
        I::Item: std::borrow::Borrow<Scalar>,
    {
        let end = self.read_row_index()?.len();
        self.splice(end, 0, values)
    }

    /// Remove one row by position, and the layout that introduced it.
    ///
    /// # Errors
    ///
    /// Returns an error for a row the document does not hold, and any read or
    /// write failure.
    pub fn remove_row(&mut self, row: u64) -> Result<()> {
        self.splice(row, 1, Vec::<Scalar>::new()).map(|_| ())
    }

    /// Replace `count` rows at `offset` with the rows `batches` yields.
    ///
    /// This is the positional write [`IOMode::Random`](crate::IOMode) names:
    /// the rows before `offset` are never read, and the ones after it move
    /// only by the difference the replacement makes.
    ///
    /// # Errors
    ///
    /// Returns a read, decoding, cast, or write failure.
    pub fn write_range_arrow_reader(
        &mut self,
        offset: u64,
        count: usize,
        batches: BatchReader,
    ) -> Result<u64> {
        let mut rows = Vec::new();
        for batch in batches {
            let batch = batch.map_err(crate::arrow::from_reader_error)?;
            let names =
                crate::arrow::field_from_arrow_schema(self.options.name(), &batch.schema())?;
            let values = crate::arrow::batch_to_value(&batch)?;
            let Some(values) = values.as_sequence() else {
                continue;
            };
            let children = names.fields();
            for row in values {
                let Some(row) = row.as_sequence() else {
                    continue;
                };
                rows.push(Scalar::from_record(
                    children
                        .iter()
                        .map(|field| SmolStr::new(field.name()))
                        .zip(row.iter().cloned()),
                )?);
            }
        }
        self.splice(offset, count, rows)
    }

    /// Apply one positional change and keep the index describing the result.
    fn splice<I>(&mut self, offset: u64, count: usize, values: I) -> Result<u64>
    where
        I: IntoIterator,
        I::Item: std::borrow::Borrow<Scalar>,
    {
        let mut index = self.take_index()?;
        let result = random::splice_rows(
            &mut self.handle,
            &mut index,
            &self.options,
            offset,
            count,
            values,
        );
        // The index describes what is stored either way: a failed splice may
        // have moved bytes before it failed, so a stale index is dropped
        // rather than kept.
        match result {
            Ok(added) => {
                self.restore_index(index);
                Ok(added)
            }
            Err(error) => Err(error),
        }
    }

    /// Take the row index out of the cache, or read one.
    fn take_index(&mut self) -> Result<RowIndex> {
        match self.cached.take() {
            Some(index) => Ok(Arc::try_unwrap(index).unwrap_or_else(|shared| (*shared).clone())),
            None => index::read_row_index(&self.handle, &self.options),
        }
    }

    /// Keep an index the current session may reuse.
    fn restore_index(&mut self, index: RowIndex) {
        if self.opened {
            let _ = self.cached.set(Arc::new(index));
        }
    }

    /// Drop the row index after a change that could move a row.
    fn invalidate_index(&mut self) {
        self.cached.take();
    }

    fn require_xml_options<'a>(&self, options: &'a RecordOptions) -> Result<&'a XmlOptions> {
        match options {
            RecordOptions::Xml(options) => Ok(options),
            _ => Err(crate::Error::InvalidRecord {
                path: SmolStr::new_static("$.encoding"),
                reason: crate::text::expected_got("XML record options", options.mime_type()),
            }),
        }
    }
}

impl<H: IOBase> IOMedia for Xml<H> {
    fn as_io_base(&self) -> &dyn IOBase {
        self
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self
    }

    fn row_size(&self) -> Result<u64> {
        if self.opened && self.handle.codec() == crate::Codec::Identity {
            return Ok(self.read_row_index()?.len());
        }
        arrow::row_size(&self.handle, &self.options)
    }

    fn column_size(&self) -> Result<usize> {
        if let Some(field) = self.options.field() {
            return Ok(field.fields().len());
        }
        Ok(arrow::read_field(&self.handle, &self.options)?
            .fields()
            .len())
    }

    fn record_options(&self) -> Result<RecordOptions> {
        Ok(self.options.clone().into())
    }

    fn read_arrow_field(&self, options: &RecordOptions) -> Result<Field> {
        let options = self.require_xml_options(options)?;
        arrow::read_field(&self.handle, options)
    }

    fn read_arrow_reader(&self, options: &RecordOptions) -> Result<BatchReader> {
        self.require_xml_options(options)?;
        IOMedia::read_arrow_reader(&self.handle, options)
    }

    fn overwrite_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        self.require_xml_options(options)?;
        self.invalidate_index();
        IOMedia::overwrite_arrow_reader(&mut self.handle, batches, options)
    }

    fn overwrite_prepared_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        self.require_xml_options(options)?;
        self.invalidate_index();
        IOMedia::overwrite_prepared_arrow_reader(&mut self.handle, batches, options)
    }

    fn append_arrow_reader(&mut self, batches: BatchReader, options: &RecordOptions) -> Result<()> {
        self.require_xml_options(options)?;
        self.invalidate_index();
        IOMedia::append_arrow_reader(&mut self.handle, batches, options)
    }

    fn merge_arrow_reader(&mut self, batches: BatchReader, options: &RecordOptions) -> Result<()> {
        self.require_xml_options(options)?;
        self.invalidate_index();
        IOMedia::merge_arrow_reader(&mut self.handle, batches, options)
    }
}

impl<H: IOBase> IOBase for Xml<H> {
    crate::delegate_iobase!(handle: pread, read_all_bytes, read_range_bytes, pstream_bytes,
        size, capacity, reserve, url, bound_location, media_type, flush, parent, child_by_path,
        ls, kind, is_io);

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.invalidate_index();
        self.handle.pwrite(offset, bytes)
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        self.invalidate_index();
        self.handle.truncate(size)
    }

    fn set_media_type(&mut self, media_type: crate::MediaType) {
        self.invalidate_index();
        self.handle.set_media_type(media_type);
    }

    /// An XML document read as rows holds rows, whatever the bytes underneath
    /// are named - no probe, no listing, no read.
    fn is_tabular(&self) -> bool {
        true
    }

    /// A record encoding is never read as one whole byte value.
    fn is_atomic(&self) -> bool {
        false
    }

    /// Materialize the handle and read the row index once.
    fn open(&mut self) -> Result<()> {
        if self.opened {
            return Ok(());
        }
        self.handle.open()?;
        self.invalidate_index();
        self.opened = true;
        Ok(())
    }

    /// Return whether a row index is currently retained.
    fn opened(&self) -> bool {
        self.opened
    }

    /// Flush the handle and drop the retained index.
    fn close(&mut self) -> Result<()> {
        self.invalidate_index();
        self.opened = false;
        self.handle.close()
    }

    /// Empty the document and drop the index describing it.
    fn clear(&mut self) -> Result<()> {
        self.invalidate_index();
        self.handle.clear()
    }

    /// Delete the document, and the index describing it with it.
    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.invalidate_index();
        self.opened = false;
        self.handle.remove(recursive)
    }
}
