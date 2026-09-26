//! An XMLA rowset document as record media over any byte handle.
//!
//! A `.xmla` handle holds one document: a SOAP message whose body is a
//! `DiscoverResponse` or `ExecuteResponse`, or the bare rowset `root` - what
//! a client saved off the wire, or what this crate wrote. Its rows are the
//! rowset's, its schema the rowset's `xsd:schema`, and a declared field types
//! a document written without one. Reading holds the parsed document, as
//! every structured text document is held: XML has no frame to read a prefix
//! of. Writing streams each batch into the document as it arrives and hands
//! the handle the whole once.

use std::sync::{Arc, OnceLock};

use arrow_array::RecordBatchIterator;

use crate::arrow::{BatchReader, arrow_schema_from_field, field_from_arrow_schema};
use crate::media::{IORecordOptions, RecordOptions};
use crate::soap::ENVELOPE_NAMESPACE;
use crate::xml::Element;
use crate::{ArrowCastOptions, Charset, Field, IOBase, IOMedia, Result, Serie, SerieReader};

use super::options::XmlaOptions;
use super::response::Response;
use super::rowset::Rowset;
use super::{ROWSET_NAMESPACE, invalid};

/// The rowset a handle's document holds: its columns and its rows.
fn read_document<H: IOBase + ?Sized>(
    handle: &H,
    field: Option<&Field>,
) -> Result<Option<(Rowset, Serie)>> {
    let encoded = handle.read_all_bytes()?;
    if encoded.is_empty() {
        return Ok(None);
    }
    let bytes = handle.codec().load(&encoded)?;
    let charset = Charset::from_media_type(handle.media_type());
    let text = charset.decode(&bytes)?;
    let document = crate::xml::from_utf8(&text)?;
    let root = Element::root(&document)?;
    if root.is(Some(ENVELOPE_NAMESPACE), "Envelope") {
        let response = Response::from_envelope(
            &crate::soap::Envelope::from_natural(&document)?,
            field,
        )?;
        return match response.answer() {
            super::response::Answer::Rowset { rowset, rows } => {
                Ok(Some((rowset.clone(), rows.clone())))
            }
            super::response::Answer::Empty => Ok(None),
            super::response::Answer::Dataset(_) => Err(invalid(
                "the document holds a multidimensional dataset, which has no rows to read",
            )),
        };
    }
    if root.local_name() == "root"
        && matches!(root.namespace(), Some(ROWSET_NAMESPACE) | None)
    {
        return Rowset::read_root(&root, field).map(Some);
    }
    Err(invalid(smol_str::format_smolstr!(
        "expected a SOAP envelope or a rowset `root`, got `{}`",
        root.name()
    )))
}

/// Read the schema of the document `handle` holds.
///
/// # Errors
///
/// Returns a read, decoding, or schema failure.
pub fn read_field<H: IOBase + ?Sized>(handle: &H, options: &XmlaOptions) -> Result<Field> {
    if let Some(field) = options.field() {
        return Ok(field.clone());
    }
    match read_document(handle, None)? {
        Some((rowset, _)) => Ok(rowset.field().clone().with_name(options.name())),
        None => Err(invalid("an empty document declares no schema")),
    }
}

/// Count the rows of the document `handle` holds.
///
/// # Errors
///
/// Returns a read, decoding, or schema failure.
pub(crate) fn row_size<H: IOBase + ?Sized>(handle: &H, options: &XmlaOptions) -> Result<u64> {
    let field = options.field();
    Ok(read_document(handle, field.as_ref())?
        .map_or(0, |(_, rows)| rows.len() as u64))
}

/// Read the rows of the document `handle` holds as one batch.
///
/// `field` types a document written without its schema; a document carrying
/// one states its own columns, and the declared field is what the caller casts
/// the batch onto.
///
/// # Errors
///
/// Returns a read, decoding, schema, or row failure.
pub fn read_batch_reader<H: IOBase + ?Sized>(
    handle: &H,
    field: Option<&Field>,
    options: &XmlaOptions,
) -> crate::arrow::Result<BatchReader> {
    match read_document(handle, field)? {
        Some((_, rows)) => {
            let batch = rows.into_arrow_batch()?;
            let schema = batch.schema();
            Ok(Box::new(RecordBatchIterator::new(
                std::iter::once(Ok(batch)),
                schema,
            )))
        }
        None => {
            // Per the laziness contract, a missing document holds no rows.
            let schema = match field.cloned().or_else(|| options.field()) {
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
/// The document is the response of the options' method when they say
/// `envelope`, else the bare `root`; it carries the schema and the rows the
/// options' content asks for. Each batch is written as it is pulled; the
/// handle receives the whole once, through its content coding.
///
/// # Errors
///
/// Returns a schema, value, encoding, compression, or write failure.
pub fn overwrite_arrow_reader<H: IOBase + ?Sized>(
    handle: &mut H,
    batches: BatchReader,
    options: &XmlaOptions,
) -> Result<()> {
    let charset = Charset::from_media_type(handle.media_type());
    if !charset.is_utf8() {
        return Err(invalid(smol_str::format_smolstr!(
            "an XMLA document is written in UTF-8; the handle declares {charset}"
        )));
    }
    let root = field_from_arrow_schema(options.name(), batches.schema().as_ref())?;
    let rowset = Rowset::new(root.clone())?;
    let rows = SerieReader::from_arrow_reader(Some(&root), batches, ArrowCastOptions::default())?;
    let mut document = Vec::new();
    if options.envelope {
        super::response::write_rowset(
            &mut document,
            &[],
            options.method,
            &rowset,
            rows,
            options.content,
        )?;
    } else {
        rowset.write_root(
            &mut document,
            rows,
            options.content.has_schema(),
            options.content.has_data(),
        )?;
    }
    let encoded = handle.codec().dump_with_level(&document, options.level())?;
    handle.write_all_bytes(&encoded)
}

/// A byte handle retained with one XMLA configuration.
///
/// Rows flow through the ordinary [`IOMedia`] methods, and the wrapper
/// retains the [`XmlaOptions`] that [`IOMedia::record_options`] answers with.
/// [`IOBase::open`] caches the document's schema until [`IOBase::close`].
#[derive(Debug)]
pub struct Xmla<H: IOBase> {
    handle: H,
    options: XmlaOptions,
    opened: bool,
    cached_schema: OnceLock<Field>,
}

impl<H: IOBase> Xmla<H> {
    /// Wrap a handle with default options.
    #[must_use]
    pub fn new(handle: H) -> Self {
        Self {
            handle,
            options: XmlaOptions::new(),
            opened: false,
            cached_schema: OnceLock::new(),
        }
    }

    /// Return this media with a complete configuration.
    #[must_use]
    pub fn with_options(mut self, options: XmlaOptions) -> Self {
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
    pub const fn options(&self) -> &XmlaOptions {
        &self.options
    }

    /// Borrow the retained options mutably.
    pub fn options_mut(&mut self) -> &mut XmlaOptions {
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

    fn require_options<'a>(&self, options: &'a RecordOptions) -> Result<&'a XmlaOptions> {
        match options {
            RecordOptions::Xmla(options) => Ok(options),
            _ => Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$.encoding"),
                reason: crate::text::expected_got("XMLA record options", options.mime_type()),
            }),
        }
    }

    fn invalidate(&mut self) {
        self.cached_schema = OnceLock::new();
    }
}

impl<H: IOBase> IOMedia for Xmla<H> {
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
        // One read answers both an empty document (no columns) and a held
        // one, so no size probe precedes it.
        Ok(read_document(&self.handle, None)?.map_or(0, |(rowset, _)| rowset.field().field_len()))
    }

    fn record_options(&self) -> Result<RecordOptions> {
        Ok(RecordOptions::Xmla(self.options.clone()))
    }

    fn read_arrow_field(&self, options: &RecordOptions) -> Result<Field> {
        let options = self.require_options(options)?;
        if let Some(field) = options.field() {
            return Ok(field.clone());
        }
        if self.opened {
            if let Some(cached) = self.cached_schema.get() {
                return Ok(cached.clone().with_name(options.name()));
            }
        }
        let field = read_field(&self.handle, options)?;
        if self.opened {
            let _ = self.cached_schema.set(field.clone());
        }
        Ok(field)
    }

    fn overwrite_arrow_reader(&mut self, batches: BatchReader, options: &RecordOptions) -> Result<()> {
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

impl<H: IOBase> IOBase for Xmla<H> {
    crate::delegate_iobase!(handle: pread, read_all_bytes, read_range_bytes, pstream_bytes,
        size, capacity, reserve, uri, url, bound_location, mtime, media_type, flush, parent,
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

    /// A rowset document is a record encoding, whatever media type the bytes
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
