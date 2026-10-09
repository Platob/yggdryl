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

use std::sync::Arc;

use arrow_array::RecordBatchIterator;

use crate::arrow::{BatchReader, arrow_schema_from_field, field_from_arrow_schema};
use crate::holder::Holder;
use crate::media::{IORecordOptions, Media, MediaCodec, MediaWrapper, RecordOptions};
use crate::soap::ENVELOPE_NAMESPACE;
use crate::xml::Element;
use crate::{
    ArrowCastOptions, Charset, Field, IOBase, IOMedia, MimeType, Result, Serie, StreamChunkedSerie,
};

use super::options::XmlaOptions;
use super::response::Response;
use super::rowset::Rowset;
use super::{ROWSET_NAMESPACE, invalid};

/// The rowset a handle's document holds: its columns and its rows.
fn read_document<H: IOBase + ?Sized>(
    handle: &H,
    field: Option<&Field>,
    cast: ArrowCastOptions,
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
        let response = Response::from_envelope_with(
            &crate::soap::Envelope::from_natural(&document)?,
            field,
            cast,
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
    if root.local_name() == "root" && matches!(root.namespace(), Some(ROWSET_NAMESPACE) | None) {
        return Rowset::read_root_with(&root, field, cast).map(Some);
    }
    Err(invalid(smol_str::format_smolstr!(
        "expected a SOAP envelope or a rowset `root`, got `{}`",
        root.name()
    )))
}

/// The field the document `handle` holds states for itself, `None` where it
/// states none: an empty handle, a rowset written with a `Content` of `Data`
/// or `None`, a fault. This is what a write asks before it completes its
/// rows onto what is stored, so a document that carries no schema is one
/// that has no shape yet, never a refusal.
///
/// # Errors
///
/// Returns a read, decoding, or schema failure.
pub(crate) fn stated_field<H: IOBase + ?Sized>(handle: &H) -> Result<Option<Field>> {
    let encoded = handle.read_all_bytes()?;
    if encoded.is_empty() {
        return Ok(None);
    }
    let bytes = handle.codec().load(&encoded)?;
    let charset = Charset::from_media_type(handle.media_type());
    let text = charset.decode(&bytes)?;
    let document = crate::xml::from_utf8(&text)?;
    let root = Element::root(&document)?;
    let envelope;
    let root = if root.is(Some(ENVELOPE_NAMESPACE), "Envelope") {
        envelope = crate::soap::Envelope::from_natural(&document)?;
        let Some(payload) = envelope.payload() else {
            return Ok(None);
        };
        let element = payload.element();
        let Ok(returned) = element.one_child_in(super::NAMESPACE, "return") else {
            return Ok(None);
        };
        returned
            .children()
            .find(|child| child.local_name() == "root")
            .map(|root| Rowset::stated_field(&root))
            .transpose()?
            .flatten()
    } else {
        Rowset::stated_field(&root)?
    };
    Ok(root)
}

/// The cast a read under `options` runs: strict unless the options say `safe`.
fn cast_of(options: &XmlaOptions) -> ArrowCastOptions {
    ArrowCastOptions::default().with_safe(options.safe())
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
    match read_document(handle, None, cast_of(options))? {
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
    Ok(read_document(handle, field.as_ref(), cast_of(options))?
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
    match read_document(handle, field, cast_of(options))? {
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
    let rows =
        StreamChunkedSerie::from_arrow_reader(Some(&root), batches, ArrowCastOptions::default())?;
    let mut document = Vec::new();
    if options.envelope {
        super::response::write_rowset(
            &mut document,
            &[],
            options.method,
            &rowset,
            rows.into_chunks(),
            options.content,
        )?;
    } else {
        rowset.write_root(
            &mut document,
            rows.into_chunks(),
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
/// What the document states - the rowset's field, its row count - is held
/// in a [`MediaCache`](crate::media::MediaCache) from [`IOBase::open`] until
/// [`IOBase::close`], or for the options' `cache_ttl` on a closed handle.
#[derive(Debug)]
pub struct Xmla<H: IOBase> {
    handle: H,
    options: XmlaOptions,
    /// What the document states, read in one parse of it; never held for a
    /// container.
    cache: crate::media::MediaCache,
}

impl<H: IOBase> Xmla<H> {
    /// Wrap a handle with default options.
    #[must_use]
    pub fn new(handle: H) -> Self {
        Self {
            handle,
            options: XmlaOptions::new(),
            cache: crate::media::MediaCache::new(),
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

    /// Borrow the underlying byte handle mutably, dropping what the cache
    /// holds before any byte mutation can occur.
    pub fn handle_mut(&mut self) -> &mut H {
        self.cache.invalidate();
        &mut self.handle
    }

    /// Consume this media and return its byte handle.
    pub fn into_handle(self) -> H {
        self.handle
    }

    fn require_options<'a>(&self, options: &'a RecordOptions) -> Result<&'a XmlaOptions> {
        options
            .settings::<XmlaOptions>()
            .ok_or_else(|| crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$.encoding"),
                reason: crate::text::expected_got("XMLA record options", options.mime_type()),
            })
    }

    /// One parse of the document as a cache entry: the field its rowset
    /// states and its row count, or the empty entry for an empty handle. A
    /// fault, or a rowset stating no schema, is refused here as the read
    /// refuses it.
    fn read_entry(&self) -> Result<crate::media::Entry> {
        Ok(
            match read_document(&self.handle, None, cast_of(&self.options))? {
                Some((rowset, rows)) => {
                    let origin = rowset.field().clone();
                    crate::media::Entry {
                        columns: Some(origin.field_len()),
                        origin: Some(origin),
                        rows: Some(rows.len() as u64),
                        state: None,
                    }
                }
                None => crate::media::Entry {
                    origin: None,
                    rows: Some(0),
                    columns: Some(0),
                    state: None,
                },
            },
        )
    }

    /// The leaf's entry under `ttl`, `served` where the cache had one: the
    /// cache's, read and kept where it holds none.
    fn entry(
        &self,
        ttl: crate::media::CacheTtl,
        served: Option<crate::media::Entry>,
    ) -> Result<crate::media::Entry> {
        match served {
            Some(entry) => Ok(entry),
            None => self
                .cache
                .get_or_fill(ttl, crate::media::cache::now(), || self.read_entry()),
        }
    }
}

impl<H: IOBase> IOMedia for Xmla<H> {
    fn as_io_base(&self) -> &dyn IOBase {
        self
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self
    }

    /// The rows of the document - typed by the declared field where one is
    /// declared, which a document stating no schema needs, and then read
    /// each time - or, over a container, of every rowset beneath it.
    fn row_size(&self) -> Result<u64> {
        let ttl = self.options.cache_ttl;
        let served = self.cache.entry(ttl, crate::media::cache::now());
        let declared = self.options.field.is_some();
        if !declared && let Some(rows) = served.as_ref().and_then(|entry| entry.rows) {
            return Ok(rows);
        }
        // Past the cache, which only a leaf ever fills.
        if served.is_none() && self.handle.is_container() {
            return crate::iomedia::container_row_size(
                &self.handle,
                &crate::iomedia::dimension_options(self)?,
            );
        }
        if declared || !self.cache.keeps(ttl) {
            return row_size(&self.handle, &self.options);
        }
        Ok(self.entry(ttl, served)?.rows.unwrap_or_default())
    }

    fn column_size(&self) -> Result<usize> {
        if let Some(field) = self.options.field() {
            return Ok(field.field_len());
        }
        let ttl = self.options.cache_ttl;
        let served = self.cache.entry(ttl, crate::media::cache::now());
        if let Some(columns) = served.as_ref().and_then(|entry| entry.columns) {
            return Ok(columns);
        }
        // Past the cache, which only a leaf ever fills.
        if served.is_none() && self.handle.is_container() {
            return Ok(crate::iomedia::container_origin(
                &self.handle,
                crate::iomedia::dimension_options(self)?,
            )?
            .map_or(0, |field| field.field_len()));
        }
        if self.cache.keeps(ttl) {
            return Ok(self.entry(ttl, served)?.columns.unwrap_or_default());
        }
        // One read answers both an empty document (no columns) and a held
        // one, so no size probe precedes it.
        Ok(read_document(&self.handle, None, cast_of(&self.options))?
            .map_or(0, |(rowset, _)| rowset.field().field_len()))
    }

    fn record_options(&self) -> Result<RecordOptions> {
        Ok(self.options.clone().into())
    }

    /// The one schema answer, the declared root else the rowset's own field,
    /// as the plan's sections leave it - kept here because options of
    /// another encoding are refused by name, and because a document holding
    /// no rowset, or a fault, is refused as its read refuses it rather than
    /// answered as a resource stating no shape.
    fn read_arrow_field(&self, options: &RecordOptions) -> Result<Field> {
        let xmla = self.require_options(options)?;
        let root = match xmla.field() {
            Some(field) => field,
            None => {
                let ttl = xmla.cache_ttl;
                let served = self.cache.entry(ttl, crate::media::cache::now());
                // Past the cache, which only a leaf ever fills.
                if served.is_none() && self.handle.is_container() {
                    return crate::iomedia::container_field(&self.handle, options);
                }
                if served.is_none() && !self.cache.keeps(ttl) {
                    read_field(&self.handle, xmla)?
                } else {
                    self.entry(ttl, served)?
                        .origin
                        .ok_or_else(|| invalid("an empty document declares no schema"))?
                        .with_name(xmla.name())
                }
            }
        };
        crate::iomedia::field_under(options, &root)
    }

    /// The field the document's rowset states, named as the options name
    /// it; `None` for an empty document, a rowset stating no schema, or a
    /// fault, as a write onto it reads it.
    fn read_origin_field(&self) -> Result<Option<Field>> {
        let ttl = self.options.cache_ttl;
        let served = self.cache.entry(ttl, crate::media::cache::now());
        if served.is_none() && self.handle.is_container() {
            return crate::iomedia::container_origin(
                &self.handle,
                crate::iomedia::dimension_options(self)?,
            );
        }
        let origin = match served {
            Some(entry) => entry.origin,
            None if self.cache.keeps(ttl) => match self.entry(ttl, None) {
                Ok(entry) => entry.origin,
                // A document the rowset read refuses - a fault, no schema -
                // states no shape.
                Err(_) => stated_field(&self.handle)?,
            },
            None => stated_field(&self.handle)?,
        };
        Ok(origin.map(|origin| origin.with_name(self.options.name())))
    }

    fn read_serie(&self, options: Option<&RecordOptions>) -> Result<crate::Serie> {
        let options = crate::iomedia::own_options(self, options)?;
        self.require_options(&options)?;
        crate::iomedia::read_record_serie(self, Some(&options))
    }

    fn overwrite_serie(
        &mut self,
        value: crate::Serie,
        options: Option<&RecordOptions>,
    ) -> Result<crate::IOResult> {
        let options = crate::iomedia::own_options(self, options)?;
        let options = options.as_ref();
        let batches = crate::StreamChunkedSerie::from_serie(value)?.into_arrow_reader();
        self.require_options(options)?;
        let result =
            crate::iobase::overwrite_arrow_reader_default_with_field(self, batches, options)
                .map(|(_, result)| result);
        // What a read states is the rowset the document carries - its schema
        // only where the options' content wrote one - so the next ask reads
        // the document afresh.
        self.cache.invalidate();
        result
    }

    fn overwrite_prepared_serie(
        &mut self,
        value: crate::StreamChunkedSerie,
        options: &RecordOptions,
    ) -> Result<()> {
        let batches = value.into_arrow_reader();
        self.require_options(options)?;
        let result = crate::iobase::leaf_writer(self, batches, options);
        self.cache.invalidate();
        result
    }

    fn append_serie(
        &mut self,
        value: crate::Serie,
        options: Option<&RecordOptions>,
    ) -> Result<crate::IOResult> {
        let options = crate::iomedia::own_options(self, options)?;
        let options = options.as_ref();
        let batches = crate::StreamChunkedSerie::from_serie(value)?.into_arrow_reader();
        self.require_options(options)?;
        let result = crate::iobase::append_arrow_reader_default(self, batches, options);
        self.cache.invalidate();
        result
    }

    fn merge_serie(
        &mut self,
        value: crate::Serie,
        options: Option<&RecordOptions>,
    ) -> Result<crate::IOResult> {
        let options = crate::iomedia::own_options(self, options)?;
        let options = options.as_ref();
        let batches = crate::StreamChunkedSerie::from_serie(value)?.into_arrow_reader();
        self.require_options(options)?;
        let result = crate::iobase::merge_arrow_reader_default(self, batches, options);
        self.cache.invalidate();
        result
    }
}

impl<H: IOBase> IOBase for Xmla<H> {
    crate::delegate_iobase!(handle: pread, read_all_bytes, read_range_bytes, read_tail_bytes,
        pstream_bytes,
        size, capacity, reserve, uri, url, bound_location, mtime, media_type, applied_codec, flush, parent,
        child_by_path, ls, kind, is_container);

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.cache.invalidate();
        self.handle.pwrite(offset, bytes)
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        self.cache.invalidate();
        self.handle.truncate(size)
    }

    fn create_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.cache.invalidate();
        self.handle.create_bytes(bytes)
    }

    fn set_media_type(&mut self, media_type: crate::MediaType) {
        self.cache.invalidate();
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

    /// Materialize the handle and hold what the document states, as it is
    /// first asked, until [`close`](IOBase::close).
    fn open(&mut self) -> Result<()> {
        if self.cache.is_open() {
            return Ok(());
        }
        self.handle.open()?;
        self.cache.invalidate();
        self.cache.open();
        Ok(())
    }

    fn opened(&self) -> bool {
        self.cache.is_open()
    }

    fn close(&mut self) -> Result<()> {
        self.cache.close();
        self.handle.close()
    }

    /// Empty the document; the cache then holds what an empty document
    /// states - no rowset, no row, no column - where it keeps.
    fn clear(&mut self) -> Result<()> {
        self.cache.invalidate();
        self.handle.clear()?;
        // A container caches nothing: its leaves answer for it on every ask.
        if self.cache.keeps(self.options.cache_ttl) && !self.handle.is_container() {
            self.cache.update(crate::media::cache::now(), |entry| {
                entry.origin = None;
                entry.rows = Some(0);
                entry.columns = Some(0);
                entry.state = None;
            });
        }
        Ok(())
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.cache.close();
        self.handle.remove(recursive)
    }
}

/// The MIME type an XMLA rowset document answers.
static XMLA_TYPES: [MimeType; 1] = [MimeType::XMLA];

/// An XMLA rowset document as a record medium: [`read_batch_reader`],
/// [`read_field`] and [`overwrite_arrow_reader`] behind the one contract
/// every medium answers.
#[derive(Debug)]
pub struct XmlaCodec;

/// The XMLA rowset medium, claimed by the core under its MIME type.
pub static XMLA_CODEC: XmlaCodec = XmlaCodec;

impl MediaCodec for XmlaCodec {
    fn name(&self) -> &'static str {
        "xmla"
    }

    fn title(&self) -> &'static str {
        "XMLA"
    }

    fn rank(&self) -> u8 {
        4
    }

    fn mime_types(&self) -> &'static [MimeType] {
        &XMLA_TYPES
    }

    fn default_options(&self, _base: &MimeType) -> RecordOptions {
        RecordOptions::registered(XmlaOptions::new())
    }

    fn read_batch_reader(
        &self,
        handle: &dyn IOBase,
        declared: Option<&Field>,
        options: &RecordOptions,
    ) -> Result<BatchReader> {
        let xmla = options.require_settings::<XmlaOptions>()?;
        Ok(read_batch_reader(handle, declared, xmla)?)
    }

    fn row_size(&self, handle: &dyn IOBase, options: &RecordOptions) -> Result<u64> {
        row_size(handle, options.require_settings::<XmlaOptions>()?)
    }

    fn read_field(&self, handle: &dyn IOBase, options: &RecordOptions) -> Result<Field> {
        read_field(handle, options.require_settings::<XmlaOptions>()?)
    }

    /// A rowset document may state no schema at all (a `Content` of
    /// `Data`), which is a resource with no shape yet rather than one that
    /// cannot be read.
    fn stated_field(&self, handle: &dyn IOBase, _options: &RecordOptions) -> Result<Option<Field>> {
        stated_field(handle)
    }

    fn overwrite_arrow_reader(
        &self,
        handle: &mut dyn IOBase,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        let xmla = options.require_settings::<XmlaOptions>()?;
        overwrite_arrow_reader(handle, batches, xmla)
    }

    fn open(&self, handle: Holder) -> Media {
        Media::Registered(Box::new(Xmla::new(handle)))
    }
}

impl MediaWrapper for Xmla<Holder> {
    fn medium(&self) -> &'static dyn MediaCodec {
        &XMLA_CODEC
    }

    fn handle(&self) -> &Holder {
        &self.handle
    }

    fn into_handle(self: Box<Self>) -> Holder {
        self.handle
    }

    fn with_field(self: Box<Self>, field: Field) -> Box<dyn MediaWrapper> {
        Box::new(Xmla::with_field(*self, field))
    }
}
