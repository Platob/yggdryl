//! A CSV or TSV document as record media over any byte handle.
//!
//! The document streams: records are cut from the decoded transport one at
//! a time, so a read holds one batch, a count holds one record, and the
//! schema is read off the header and a bounded sample. A declared field is
//! the contract every cell is read under; without one the header names the
//! columns and the sample types them. Writing renders each batch as it
//! arrives and hands the handle the whole once, through its content coding
//! and in the charset it declares.

use std::io::{Read, Write};
use std::sync::{Arc, OnceLock};

use arrow_array::RecordBatchIterator;
use smol_str::SmolStr;

use crate::arrow::{BatchReader, arrow_schema_from_field};
use crate::media::{IORecordOptions, RecordOptions};
use crate::text::transport::{
    borrowed_decoded, encoded_terminator, ends_with, fetched, owned_decoded, update_suffix,
};
use crate::{Charset, Codec, Error, Field, IOBase, IOMedia, Result};

use super::options::CsvOptions;
use super::reader;
use super::writer;

fn invalid(reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$.csv"),
        reason: reason.into(),
    }
}

/// The field the document `handle` holds states for itself - its header
/// and the datatypes its sample infers - or `None` where it holds no record
/// at all.
///
/// This is a reading, and what the column count and a folder's schema are
/// answered from without decoding a row past the sample; a write completes
/// its rows onto [`write_target`] instead, since a CSV stores text and a
/// sample's datatypes are no contract the rows written may be cast onto.
///
/// # Errors
///
/// Returns a read or decoding failure, or a header naming a column twice.
pub(crate) fn stated_field<H: IOBase + ?Sized>(
    handle: &H,
    options: &CsvOptions,
) -> Result<Option<Field>> {
    let url = handle.url().cloned();
    let bytes = borrowed_decoded(handle)?;
    Ok(reader::open(bytes, options, None, url)?.map(|opened| opened.field))
}

/// Read the schema of the document `handle` holds.
///
/// The declared field where the options declare one; else the header names
/// the columns and the first `infer_row_size` records type them, every
/// inferred column nullable.
///
/// # Errors
///
/// Returns a read or decoding failure, a header naming a column twice, or
/// a refusal for an empty document, which declares no schema.
pub fn read_field<H: IOBase + ?Sized>(handle: &H, options: &CsvOptions) -> Result<Field> {
    if let Some(field) = options.field() {
        return Ok(field.clone());
    }
    stated_field(handle, options)?.ok_or_else(|| invalid("an empty document declares no schema"))
}

/// The field a write completes its rows `incoming` onto the document
/// `handle` holds, or `None` where it holds no record and the rows are the
/// shape they arrive in.
///
/// A CSV stores text, so the datatypes its sample infers are a reading and
/// never a contract to cast the rows written onto: the stored shape is the
/// header. The target is the header's names in the header's order, each
/// typed as the incoming column of that name - so the rows render exactly as
/// an overwrite renders them, and a merge reads the stored cells under the
/// same columns - and a header column the rows do not carry is nullable text,
/// written empty. Without a header the document names nothing and its
/// columns are positions, so the rows are the target as they arrive.
///
/// # Errors
///
/// Returns a read or decoding failure, a header naming a column twice, an
/// incoming column the header does not name - naming it and the header's
/// columns - and, without a header, a stored record of another width than
/// the columns written, naming both counts.
pub(crate) fn write_target<H: IOBase + ?Sized>(
    handle: &H,
    options: &CsvOptions,
    incoming: &Field,
) -> Result<Option<Field>> {
    // Per the laziness contract, a resource that holds nothing is not read.
    if handle.is_empty() {
        return Ok(None);
    }
    let url = handle.url().cloned();
    let Some(names) = reader::stated_columns(borrowed_decoded(handle)?, options, url.as_ref())?
    else {
        return Ok(None);
    };
    let location = reader::location_of(url.as_ref());
    if !options.header() {
        if names.len() != incoming.field_len() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: crate::text::expected_got(
                    format_args!(
                        "records of {} {}, one per column written",
                        incoming.field_len(),
                        if incoming.field_len() == 1 {
                            "cell"
                        } else {
                            "cells"
                        }
                    ),
                    format_args!("{} cells in the first record of {location}", names.len()),
                ),
            });
        }
        return Ok(Some(incoming.clone()));
    }
    if let Some(unnamed) = incoming
        .fields()
        .iter()
        .find(|child| !names.iter().any(|name| name == child.name()))
    {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$.header"),
            reason: crate::text::expected_got(
                format_args!("a column the header of {location} names, one of {names:?}"),
                format_args!("{:?}", unnamed.name()),
            ),
        });
    }
    let columns = names.iter().map(|name| {
        incoming
            .fields()
            .iter()
            .find(|child| child.name() == name.as_str())
            .cloned()
            .unwrap_or_else(|| crate::DataType::utf8().nullable_field(name.clone()))
    });
    Ok(Some(
        crate::DataType::from(crate::StructType::from_fields(columns)?)
            .required_field(incoming.name()),
    ))
}

/// Count the records of the document `handle` holds, the header left out,
/// reading no cell.
///
/// # Errors
///
/// Returns a read or decoding failure, and a stream ending inside a quoted
/// cell.
pub(crate) fn row_size<H: IOBase + ?Sized>(handle: &H, options: &CsvOptions) -> Result<u64> {
    // The location is asked for only to name a refusal, so a count that
    // succeeds asks the handle nothing but its bytes.
    reader::count(borrowed_decoded(handle)?, options)
        .map_err(|error| reader::located(error, &reader::location_of(handle.url())))
}

/// Read the records of the document `handle` holds as streamed batches.
///
/// `field` is the contract each cell is read under, else the options'
/// declared field, else the header and the sample state the columns. Per
/// the laziness contract an empty or absent handle is an empty reader under
/// the declared schema, or the empty schema where none is declared.
///
/// # Errors
///
/// Returns a read or decoding failure before the first batch, and a cell
/// its column refuses, a ragged record or a transport failure as the batch
/// it lands in.
pub fn read_batch_reader<H: IOBase + ?Sized>(
    handle: &H,
    field: Option<&Field>,
    options: &CsvOptions,
) -> crate::arrow::Result<BatchReader> {
    let declared = field.cloned().or_else(|| options.field());
    let url = handle.url().cloned();
    let bytes = owned_decoded(crate::iobase::owned_handle(handle)?);
    match reader::open(bytes, options, declared.as_ref(), url)? {
        Some(opened) => {
            // The record surface applies a total row limit after projection
            // and filtering; pulled one record at a time under one, so
            // satisfying it never cuts a later record.
            let batch_row_size =
                if options.max_row_size().is_some() && options.max_byte_size().is_none() {
                    Some(1)
                } else {
                    options.batch_row_size()
                };
            crate::arrow::rows::result_reader(
                &opened.field,
                opened.rows,
                batch_row_size,
                options.batch_byte_size(),
                None,
                None,
            )
        }
        None => {
            let schema = match declared {
                Some(field) => arrow_schema_from_field(&field)?,
                None => Arc::new(arrow_schema::Schema::empty()),
            };
            Ok(Box::new(RecordBatchIterator::new(
                std::iter::empty(),
                schema,
            )))
        }
    }
}

/// Replace the document `handle` holds with one carrying `batches`.
///
/// The header where the options say so, then one record per row, each
/// batch rendered as it is pulled; the handle receives the whole once,
/// through its content coding and in the charset it declares.
///
/// # Errors
///
/// Returns a schema, value, encoding, compression or write failure, and a
/// cell that needs quoting under options that quote nothing.
pub fn overwrite_arrow_reader<H: IOBase + ?Sized>(
    handle: &mut H,
    batches: BatchReader,
    options: &CsvOptions,
) -> Result<()> {
    let charset = Charset::from_media_type(handle.media_type());
    let encoded = writer::encoded(batches, options, charset, handle.codec(), options.header())?;
    handle.write_all_bytes(&encoded)
}

/// Add `batches` as records after the ones `handle` holds.
///
/// An uncoded handle takes the rendered records after its current tail -
/// the header only when it holds nothing - so nothing already there is
/// read. A coded handle is decoded, re-encoded and rewritten with the new
/// records after its last, the header written once.
///
/// # Errors
///
/// Returns the failures [`overwrite_arrow_reader`] does.
pub(crate) fn append_arrow_reader<H: IOBase + ?Sized>(
    handle: &mut H,
    batches: BatchReader,
    options: &CsvOptions,
) -> Result<()> {
    let charset = Charset::from_media_type(handle.media_type());
    let terminator = encoded_terminator(charset, options.linesep().as_bytes())?;
    let codec = handle.codec();
    let empty = handle.is_empty();
    let header = options.header() && empty;
    if codec == Codec::Identity {
        let rendered = writer::encoded(batches, options, charset, codec, header)?;
        let mut offset = handle.size();
        if offset > 0 && !ends_with(handle, &terminator)? {
            handle.pwrite_all(offset, &terminator)?;
            offset += terminator.len() as u64;
        }
        handle.pwrite_all(offset, &rendered)?;
        return handle.flush();
    }

    let mut encoded = Vec::new();
    {
        let mut encoder = codec.writer_with_level(&mut encoded, options.level());
        let mut suffix = Vec::new();
        if !empty {
            let source = handle.pstream_bytes(0, crate::DEFAULT_FETCH_BYTE_SIZE)?;
            let mut decoder = codec.reader(fetched(source));
            let mut chunk = vec![0; crate::DEFAULT_STREAM_BATCH_SIZE];
            loop {
                let read = decoder.read(&mut chunk)?;
                if read == 0 {
                    break;
                }
                update_suffix(&mut suffix, &chunk[..read], terminator.len());
                encoder.write_all(&chunk[..read])?;
            }
        }
        if !suffix.is_empty() && suffix.as_slice() != terminator.as_ref() {
            encoder.write_all(&terminator)?;
        }
        writer::render_declared(batches, options, charset, header, &mut encoder)?;
        encoder.finish()?;
    }
    handle.write_all_bytes(&encoded)
}

/// A byte handle retained with one CSV configuration.
///
/// Rows flow through the ordinary [`IOMedia`] methods, and the wrapper
/// retains the [`CsvOptions`] that [`IOMedia::record_options`] answers with.
/// [`IOBase::open`] caches the document's schema until [`IOBase::close`],
/// answered only for options that read the document as the ones it was read
/// under did: another separator or sample reads it afresh.
///
/// ```
/// use yggdryl::csv::Csv;
/// use yggdryl::holder::Buffer;
/// use yggdryl::{DataType, IOMedia, MediaType, Scalar, StructType};
///
/// # fn main() -> yggdryl::Result<()> {
/// let field = DataType::from(StructType::from_fields([
///     DataType::utf8().required_field("symbol"),
///     DataType::Int64.required_field("size"),
/// ])?)
/// .required_field("trade");
/// let mut media = Csv::new(Buffer::new().with_media_type(MediaType::from_file_name("trades.csv")))
///     .with_field(field);
/// let options = media.record_options()?;
/// media.overwrite_records(
///     [
///         Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100_i64)]),
///         Scalar::from_sequence([Scalar::from("MSFT"), Scalar::from(250_i64)]),
///     ],
///     &options,
/// )?;
/// assert_eq!(media.handle().as_slice(), b"symbol,size\nAAPL,100\nMSFT,250\n");
/// assert_eq!(media.row_size()?, 2);
/// assert_eq!(media.column_size()?, 2);
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct Csv<H: IOBase> {
    handle: H,
    options: CsvOptions,
    opened: bool,
    /// The schema read while open, beside the options it was read under;
    /// boxed, since it is filled once per open and the options are large.
    cached_schema: OnceLock<Box<(CsvOptions, Field)>>,
}

impl<H: IOBase> Csv<H> {
    /// Wrap a handle with the default options its name states: a tab
    /// separator under `text/tab-separated-values`, a comma otherwise.
    #[must_use]
    pub fn new(handle: H) -> Self {
        let options = if handle.media_type().base() == &crate::MimeType::TSV {
            CsvOptions::tsv()
        } else {
            CsvOptions::new()
        };
        Self {
            handle,
            options,
            opened: false,
            cached_schema: OnceLock::new(),
        }
    }

    /// Return this media with a complete configuration.
    #[must_use]
    pub fn with_options(mut self, options: CsvOptions) -> Self {
        self.options = options;
        self
    }

    /// Return this media with a declared canonical row field.
    #[must_use]
    pub fn with_field(mut self, field: Field) -> Self {
        self.options.set_field(field);
        self
    }

    /// Borrow the retained options.
    pub const fn options(&self) -> &CsvOptions {
        &self.options
    }

    /// Borrow the retained options mutably.
    pub fn options_mut(&mut self) -> &mut CsvOptions {
        &mut self.options
    }

    /// Borrow the underlying byte handle.
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// Borrow the underlying byte handle mutably.
    pub fn handle_mut(&mut self) -> &mut H {
        &mut self.handle
    }

    /// Consume this media and return its byte handle.
    pub fn into_handle(self) -> H {
        self.handle
    }

    fn require_options<'a>(&self, options: &'a RecordOptions) -> Result<&'a CsvOptions> {
        match options {
            RecordOptions::Csv(options) => Ok(options),
            _ => Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.encoding"),
                reason: crate::text::expected_got("CSV record options", options.mime_type()),
            }),
        }
    }

    fn invalidate(&mut self) {
        self.cached_schema = OnceLock::new();
    }

    /// The schema cached while open, where it was read under options that
    /// read the document as `options` do.
    fn cached(&self, options: &CsvOptions) -> Option<&Field> {
        if !self.opened {
            return None;
        }
        let (read, field) = self.cached_schema.get()?.as_ref();
        read.reads_as(options).then_some(field)
    }

    /// Keep `field`, read under `options`, while open.
    fn cache(&self, options: &CsvOptions, field: &Field) {
        if self.opened {
            let _ = self
                .cached_schema
                .set(Box::new((options.clone(), field.clone())));
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

    /// Count the records of the one document this wraps, or - over a
    /// container - of every delimited leaf beneath it, each counted as the
    /// document it is, header and all, as the record read reads them.
    fn row_size(&self) -> Result<u64> {
        if self.handle.is_container() {
            return crate::iomedia::container_row_size(
                &self.handle,
                &crate::iomedia::dimension_options(self)?,
            );
        }
        row_size(&self.handle, &self.options)
    }

    fn column_size(&self) -> Result<usize> {
        if let Some(field) = self.options.field() {
            return Ok(field.field_len());
        }
        if let Some(cached) = self.cached(&self.options) {
            return Ok(cached.field_len());
        }
        // Past the session's cache, which only a leaf ever fills.
        if self.handle.is_container() {
            return Ok(crate::iomedia::container_field(
                &self.handle,
                &crate::iomedia::dimension_options(self)?,
            )?
            .field_len());
        }

        // One read of the header and the sample answers both an empty
        // document (no columns) and a held one; while open, what it read is
        // what the field and the width are answered from until close.
        match stated_field(&self.handle, &self.options)? {
            Some(field) => {
                self.cache(&self.options, &field);
                Ok(field.field_len())
            }
            None => Ok(0),
        }
    }

    fn record_options(&self) -> Result<RecordOptions> {
        Ok(RecordOptions::Csv(self.options.clone()))
    }

    fn read_arrow_field(&self, options: &RecordOptions) -> Result<Field> {
        let options = self.require_options(options)?;
        if let Some(field) = options.field() {
            return options.result_field(field);
        }
        let source = match self.cached(options) {
            Some(cached) => cached.clone().with_name(options.name()),
            None => {
                // Past the session's cache, which only a leaf ever fills.
                if self.handle.is_container() {
                    return crate::iomedia::container_field(&self.handle, &options.clone().into());
                }
                let field = read_field(&self.handle, options)?;
                self.cache(options, &field);
                field
            }
        };
        options.result_field(source)
    }

    fn overwrite_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        self.require_options(options)?;
        self.invalidate();
        crate::iobase::overwrite_arrow_reader_default(self, batches, options)
    }

    fn overwrite_prepared_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        self.require_options(options)?;
        self.invalidate();
        crate::iobase::leaf_writer(self, batches, options)
    }

    fn append_arrow_reader(&mut self, batches: BatchReader, options: &RecordOptions) -> Result<()> {
        self.require_options(options)?;
        self.invalidate();
        crate::iobase::append_arrow_reader_default(self, batches, options)
    }

    fn merge_arrow_reader(&mut self, batches: BatchReader, options: &RecordOptions) -> Result<()> {
        self.require_options(options)?;
        self.invalidate();
        crate::iobase::merge_arrow_reader_default(self, batches, options)
    }
}

impl<H: IOBase> IOBase for Csv<H> {
    crate::delegate_iobase!(handle: pread, read_all_bytes, read_range_bytes, pstream_bytes,
        size, capacity, reserve, uri, url, bound_location, mtime, media_type, applied_codec, flush, parent,
        child_by_path, ls, kind, is_container);

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.invalidate();
        self.handle.pwrite(offset, bytes)
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        self.invalidate();
        self.handle.truncate(size)
    }

    fn set_media_type(&mut self, media_type: crate::MediaType) {
        self.invalidate();
        self.handle.set_media_type(media_type);
    }

    /// A CSV document is a record encoding, whatever media type the bytes
    /// underneath carry.
    fn is_tabular(&self) -> bool {
        true
    }

    /// A record encoding is never read as one whole byte value.
    fn is_atomic(&self) -> bool {
        false
    }

    fn open(&mut self) -> Result<()> {
        if self.opened {
            return Ok(());
        }
        self.handle.open()?;
        self.invalidate();
        self.opened = true;
        Ok(())
    }

    fn opened(&self) -> bool {
        self.opened
    }

    fn close(&mut self) -> Result<()> {
        self.invalidate();
        self.opened = false;
        self.handle.close()
    }

    fn clear(&mut self) -> Result<()> {
        self.invalidate();
        self.handle.clear()
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.invalidate();
        self.opened = false;
        self.handle.remove(recursive)
    }
}
