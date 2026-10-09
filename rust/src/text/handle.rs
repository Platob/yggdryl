//! Stateful plain-text record media over one byte handle.

use smol_str::SmolStr;

use crate::arrow::BatchReader;
use crate::holder::Holder;
use crate::media::{Entry, IORecordOptions as _, Media, MediaCache, MediaCodec, RecordOptions};
use crate::{Field, MimeType, Result, StreamSerie};
use crate::{IOBase, IOMedia};

use super::TextOptions;

/// The MIME type plain-text rows answer.
static TEXT_TYPES: [MimeType; 1] = [MimeType::PLAIN_TEXT];

/// Plain-text lines as a record medium: the line decode and the body
/// render behind the one contract every medium answers.
///
/// Named apart from [`TextCodec`](super::TextCodec), the structured
/// document codec contract this module already names.
#[derive(Debug)]
pub struct PlainTextCodec;

/// The plain-text medium, claimed by the core under `text/plain`.
pub static TEXT_CODEC: PlainTextCodec = PlainTextCodec;

impl MediaCodec for PlainTextCodec {
    fn name(&self) -> &'static str {
        "text"
    }

    fn title(&self) -> &'static str {
        "text"
    }

    fn rank(&self) -> u8 {
        3
    }

    fn mime_types(&self) -> &'static [MimeType] {
        &TEXT_TYPES
    }

    fn default_options(&self, _base: &MimeType) -> RecordOptions {
        RecordOptions::Text(Box::default())
    }

    /// A line has no identity a merge could match a stored one on.
    fn has_row_identity(&self) -> bool {
        false
    }

    fn read_batch_reader(
        &self,
        handle: &dyn IOBase,
        _declared: Option<&Field>,
        options: &RecordOptions,
    ) -> Result<BatchReader> {
        super::arrow::read_arrow_reader(handle, options.require_settings::<TextOptions>()?)
    }

    fn row_size(&self, handle: &dyn IOBase, options: &RecordOptions) -> Result<u64> {
        super::arrow::row_size(handle, options.require_settings::<TextOptions>()?)
    }

    fn read_field(&self, _handle: &dyn IOBase, options: &RecordOptions) -> Result<Field> {
        options.require_settings::<TextOptions>()?.source_field()
    }

    /// Text lines store no record shape of their own: any row shape writes,
    /// rendered line by line, so there is nothing to complete a cast onto.
    fn stated_field(
        &self,
        _handle: &dyn IOBase,
        _options: &RecordOptions,
    ) -> Result<Option<Field>> {
        Ok(None)
    }

    fn read_stream(
        &self,
        handle: &dyn IOBase,
        _declared: Option<&Field>,
        options: &RecordOptions,
    ) -> Result<Option<StreamSerie>> {
        let text = options.require_settings::<TextOptions>()?;
        Ok(Some(super::arrow::read_leaf_stream(handle, text)?))
    }

    fn overwrite_arrow_reader(
        &self,
        handle: &mut dyn IOBase,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        let text = options.require_settings::<TextOptions>()?;
        super::arrow::write_arrow_reader(handle, batches, text)
    }

    fn open(&self, handle: Holder) -> Media {
        Media::Text(Text::new(handle))
    }
}

/// A byte handle retained with one flat plain-text record configuration.
///
/// Rows flow through the ordinary [`IOMedia`] methods, and the wrapper retains
/// the `TextOptions` that [`IOMedia::record_options`] answers with. The one
/// method beside them is [`read_text_lines`](Self::read_text_lines), the
/// decode those methods already route through, reached under the retained
/// configuration rather than a second one.
///
/// The record shape is the options' line projection, answered with no byte
/// read - a line states no shape of its own, so the origin is `None` - and
/// the line count, which streams the whole object, is held in a
/// [`MediaCache`] from [`IOBase::open`] until [`IOBase::close`], or for the
/// options' `cache_ttl` on a closed handle.
#[derive(Debug)]
pub struct Text<H: IOBase> {
    handle: H,
    options: TextOptions,
    /// The line count, never held for a container.
    cache: MediaCache,
}

impl<H: IOBase> Text<H> {
    /// Wrap a handle with default plain-text record options.
    #[must_use]
    pub fn new(handle: H) -> Self {
        Self {
            handle,
            options: TextOptions::new(),
            cache: MediaCache::new(),
        }
    }

    /// Return this media with a complete flat text configuration, dropping
    /// the line count the previous one cut.
    #[must_use]
    pub fn with_options(mut self, options: TextOptions) -> Self {
        self.options = options;
        self.cache.invalidate();
        self
    }

    /// Return this media with a declared canonical row field.
    #[must_use]
    pub fn with_field(mut self, field: Field) -> Self {
        self.options.set_field(field);
        self
    }

    /// Borrow the retained text options.
    pub const fn options(&self) -> &TextOptions {
        &self.options
    }

    /// Borrow the retained text options mutably, dropping the held line
    /// count: the separator, the header and the framing decide what a line
    /// is.
    pub fn options_mut(&mut self) -> &mut TextOptions {
        self.cache.invalidate();
        &mut self.options
    }

    /// Borrow the underlying byte handle.
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// Borrow the underlying byte handle mutably, dropping the held line
    /// count before any byte mutation can occur.
    pub fn handle_mut(&mut self) -> &mut H {
        self.cache.invalidate();
        &mut self.handle
    }

    /// Consume this media and return its byte handle.
    pub fn into_handle(self) -> H {
        self.handle
    }

    /// Return this text media unchanged.
    ///
    /// This inherent method makes ordinary `handle.into_text().into_text()`
    /// idempotent because inherent methods win over [`IOBase::into_text`].
    #[must_use]
    pub const fn into_text(self) -> Self {
        self
    }

    /// Replace the retained configuration without nesting another wrapper.
    #[must_use]
    pub fn into_text_with(self, options: TextOptions) -> Self {
        self.with_options(options)
    }

    /// Decode this handle into typed lines under its retained configuration.
    ///
    /// The one decode entry point, reached with the options this wrapper
    /// already holds. Every record method routes through the same iterator, so
    /// a caller reading lines and a caller reading batches read one decode.
    ///
    /// One decode, two answers: the record methods keep the rows the retained
    /// `where` names and publish the columns its `select` names, and this
    /// yields every line under it, exactly as
    /// [`read_text_lines`](super::read_text_lines) states. A caller wanting the
    /// clauses answered reads rows.
    ///
    /// # Errors
    ///
    /// Returns the configuration's refusals - a framing mode with no header
    /// pattern, a rename naming no column, a lifted path with no name - before
    /// a byte is read.
    pub fn read_text_lines(&self) -> Result<super::TextLines> {
        super::read_text_lines(&self.handle, &self.options)
    }

    fn require_text_options<'a>(&self, options: &'a RecordOptions) -> Result<&'a TextOptions> {
        match options {
            RecordOptions::Text(options) => Ok(options),
            _ => Err(crate::Error::InvalidRecord {
                path: SmolStr::new_static("$.encoding"),
                reason: crate::text::expected_got("plain-text record options", options.mime_type()),
            }),
        }
    }
}

/// The cache entry of an object of `rows` lines: no shape, since a line
/// states no record shape of its own - the projection is the options' - and
/// no width the store states.
fn lines(rows: u64) -> Entry {
    Entry {
        origin: None,
        rows: Some(rows),
        columns: None,
        state: None,
    }
}

impl<H: IOBase> IOMedia for Text<H> {
    fn as_io_base(&self) -> &dyn IOBase {
        self
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self
    }

    /// Count the lines of the one object this wraps, or - over a container -
    /// of every text leaf beneath it, each counted as the object it is, as
    /// the record read reads them.
    fn row_size(&self) -> Result<u64> {
        let ttl = self.options.cache_ttl;
        let now = crate::media::cache::now();
        let served = self.cache.entry(ttl, now);
        if let Some(rows) = served.as_ref().and_then(|entry| entry.rows) {
            return Ok(rows);
        }
        // Past the cache, which only a leaf ever fills.
        if served.is_none() && self.handle.is_container() {
            return crate::iomedia::container_row_size(&self.handle, &self.options.clone().into());
        }
        if !self.cache.keeps(ttl) {
            return super::arrow::row_size(&self.handle, &self.options);
        }
        let entry = self.cache.get_or_fill(ttl, now, || {
            Ok(lines(super::arrow::row_size(&self.handle, &self.options)?))
        })?;
        Ok(entry.rows.unwrap_or_default())
    }

    fn column_size(&self) -> Result<usize> {
        if let Some(field) = self.options.field() {
            return Ok(field.field_len());
        }
        // A container's rows carry the partition columns its layout spells
        // beside the line's, as the record read of it lays them out.
        if self.handle.is_container() {
            return Ok(crate::iomedia::container_field(
                &self.handle,
                &crate::iomedia::dimension_options(self)?,
            )?
            .field_len());
        }
        Ok(self.options.source_field()?.field_len())
    }

    fn record_options(&self) -> Result<RecordOptions> {
        Ok(self.options.clone().into())
    }

    /// `None` over one object, asking it nothing: a line states no record
    /// shape of its own - the projection is the options', which
    /// [`read_arrow_field`](IOMedia::read_arrow_field) answers. Over a
    /// container, the root its leaves state, never cached.
    fn read_origin_field(&self) -> Result<Option<Field>> {
        let served = self
            .cache
            .entry(self.options.cache_ttl, crate::media::cache::now());
        if served.is_none() && self.handle.is_container() {
            return crate::iomedia::container_origin(
                &self.handle,
                crate::iomedia::dimension_options(self)?,
            );
        }
        Ok(None)
    }

    /// The one schema answer, the declared root else the line projection
    /// `options` state, as the `where` and `select` leave it - kept here
    /// because a line states no record shape the default could read, and
    /// because options of another encoding are refused by name.
    fn read_arrow_field(&self, options: &RecordOptions) -> Result<Field> {
        let text = self.require_text_options(options)?;
        let root = match text.field() {
            Some(field) => field,
            None if self.handle.is_container() => {
                return crate::iomedia::container_field(&self.handle, options);
            }
            None => text.source_field()?,
        };
        crate::iomedia::field_under(options, &root)
    }

    fn read_serie(&self, options: Option<&RecordOptions>) -> Result<crate::Serie> {
        let options = crate::iomedia::own_options(self, options)?;
        let options = options.as_ref();
        self.require_text_options(options)?;
        IOMedia::read_serie(&self.handle, Some(options))
    }

    fn overwrite_serie(
        &mut self,
        value: crate::Serie,
        options: Option<&RecordOptions>,
    ) -> Result<crate::IOResult> {
        let options = crate::iomedia::own_options(self, options)?;
        let options = options.as_ref();
        self.require_text_options(options)?;
        let result = IOMedia::overwrite_serie(&mut self.handle, value, Some(options));
        // A row written is not a line read: a body may hold a line break,
        // and the header and the framing cut lines afresh.
        self.cache.invalidate();
        result
    }

    fn overwrite_prepared_serie(
        &mut self,
        value: crate::StreamChunkedSerie,
        options: &RecordOptions,
    ) -> Result<()> {
        self.require_text_options(options)?;
        let result = IOMedia::overwrite_prepared_serie(&mut self.handle, value, options);
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
        self.require_text_options(options)?;
        let result = IOMedia::append_serie(&mut self.handle, value, Some(options));
        // A row written is not a line read: a body may hold a line break,
        // and the header and the framing cut lines afresh.
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
        self.require_text_options(options)?;
        let result = IOMedia::merge_serie(&mut self.handle, value, Some(options));
        // A row written is not a line read: a body may hold a line break,
        // and the header and the framing cut lines afresh.
        self.cache.invalidate();
        result
    }
}

impl<H: IOBase> IOBase for Text<H> {
    crate::delegate_iobase!(handle: pread, read_all_bytes, read_range_bytes, read_tail_bytes,
        pstream_bytes,
        read_digest, read_range_digest, size, capacity, reserve,
        uri, url, bound_location, mtime, media_type, applied_codec, flush, parent,
        child_by_path, ls, kind, is_container, is_atomic, is_io);

    fn is_tabular(&self) -> bool {
        true
    }

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

    fn write_all_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.cache.invalidate();
        self.handle.write_all_bytes(bytes)
    }

    fn append_bytes(&mut self, bytes: &[u8]) -> Result<u64> {
        self.cache.invalidate();
        self.handle.append_bytes(bytes)
    }

    fn set_media_type(&mut self, media_type: crate::MediaType) {
        self.cache.invalidate();
        self.handle.set_media_type(media_type);
    }

    /// Materialize the handle and hold the line count, as it is first asked
    /// for, until [`close`](IOBase::close).
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

    /// Empty the object; the cache then holds what an empty object states -
    /// no line - where it keeps.
    fn clear(&mut self) -> Result<()> {
        self.cache.invalidate();
        self.handle.clear()?;
        // A container caches nothing: its leaves answer for it on every ask.
        if self.cache.keeps(self.options.cache_ttl) && !self.handle.is_container() {
            self.cache.fill(crate::media::cache::now(), lines(0));
        }
        Ok(())
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.cache.close();
        self.handle.remove(recursive)
    }
}

crate::media_serie::media_serie!(
    TextSerie,
    Text,
    as_text,
    get_text_mut,
    accepts = Some(&TEXT_TYPES)
);
