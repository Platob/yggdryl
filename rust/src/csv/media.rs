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
    BoundReader, NonemptyDecodedReader, NonemptySendDecodedReader, encoded_terminator, ends_with,
    fetched, owned_handle, update_suffix,
};
use crate::{Charset, Codec, Cursor, Error, Field, IOBase, IOMedia, Result};

use super::options::CsvOptions;
use super::reader;
use super::writer;

fn invalid(reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$.csv"),
        reason: reason.into(),
    }
}

/// The decoded transport over `handle`, owned, for a reader that outlives
/// the borrow: one stream, the codings peeled, the declared charset laid
/// over it where it is not UTF-8 or US-ASCII.
fn owned_transport(handle: &(impl IOBase + ?Sized)) -> Result<Box<dyn Read + Send + 'static>> {
    let owned = owned_handle(handle)?;
    // One ask of the handle answers both: the codings the transport peels
    // and the charset it decodes under.
    let media_type = owned.media_type();
    let codings = media_type.encodings().to_vec();
    let charset = Charset::from_media_type(media_type);
    Ok(match owned.bound_location().cloned() {
        Some(bound) => Box::new(BoundReader::new(bound, codings, charset)),
        None => Box::new(NonemptySendDecodedReader::new(
            Box::new(Cursor::new(owned)),
            codings,
            charset,
        )),
    })
}

/// The same transport borrowed, for a read that ends inside the call: a
/// count, or the header and the sample a schema is read off.
fn borrowed_transport(handle: &(impl IOBase + ?Sized)) -> Result<NonemptyDecodedReader<'_>> {
    let media_type = handle.media_type();
    let codings = media_type.encodings().to_vec();
    let charset = Charset::from_media_type(media_type);
    let raw: Box<dyn Read + '_> =
        Box::new(handle.pstream_bytes(0, crate::DEFAULT_FETCH_BYTE_SIZE)?);
    Ok(NonemptyDecodedReader::new(raw, codings, charset))
}

/// The field the document `handle` holds states for itself - its header
/// and the datatypes its sample infers - or `None` where it holds no record
/// at all.
///
/// This is what a write asks before it completes its rows onto what is
/// stored, and what the row count and the column count are answered from
/// without decoding a row past the sample.
///
/// # Errors
///
/// Returns a read or decoding failure, or a header naming a column twice.
pub(crate) fn stated_field<H: IOBase + ?Sized>(
    handle: &H,
    options: &CsvOptions,
) -> Result<Option<Field>> {
    let url = handle.url().cloned();
    let bytes = borrowed_transport(handle)?;
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

/// Count the records of the document `handle` holds, the header left out,
/// reading no cell.
///
/// # Errors
///
/// Returns a read or decoding failure.
pub(crate) fn row_size<H: IOBase + ?Sized>(handle: &H, options: &CsvOptions) -> Result<u64> {
    reader::count(borrowed_transport(handle)?, options)
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
    let bytes = owned_transport(handle)?;
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
/// [`IOBase::open`] caches the document's schema until [`IOBase::close`].
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
    cached_schema: OnceLock<Field>,
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
}

impl<H: IOBase> IOMedia for Csv<H> {
    fn as_io_base(&self) -> &dyn IOBase {
        self
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self
    }

    fn row_size(&self) -> Result<u64> {
        row_size(&self.handle, &self.options)
    }

    fn column_size(&self) -> Result<usize> {
        if let Some(field) = self.options.field() {
            return Ok(field.field_len());
        }
        if self.opened {
            if let Some(cached) = self.cached_schema.get() {
                return Ok(cached.field_len());
            }
        }
        // One read of the header and the sample answers both an empty
        // document (no columns) and a held one; while open, what it read is
        // what the field and the width are answered from until close.
        match stated_field(&self.handle, &self.options)? {
            Some(field) => {
                if self.opened {
                    let _ = self.cached_schema.set(field.clone());
                }
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
            return Ok(field.clone());
        }
        let stored = match self.cached_schema.get().filter(|_| self.opened) {
            Some(cached) => cached.clone().with_name(options.name()),
            None => {
                let field = read_field(&self.handle, options)?;
                if self.opened {
                    let _ = self.cached_schema.set(field.clone());
                }
                field
            }
        };
        // The shape the rows come back in, so the schema a caller reads and
        // the batches a caller gets never disagree: the `select` clause
        // bound over the stored columns, as the record surface binds it.
        if options.select().is_all() {
            return Ok(stored);
        }
        let empty = crate::arrow::batch_reader(arrow_schema_from_field(&stored)?, []);
        let published = options.apply_arrow_expressions(empty)?.schema();
        Ok(crate::arrow::field_from_arrow_schema(
            options.name(),
            published.as_ref(),
        )?)
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
        child_by_path, ls, kind);

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
