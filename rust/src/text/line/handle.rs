//! [`Text<H>`], the wrapping handle the whole line surface hangs off.
//!
//! A wrapping handle in the same shape as [`Coded`](crate::io::Coded),
//! [`Gzip`](crate::gzip::Gzip), [`Ipc`](crate::ipc::Ipc), and
//! `Parquet`: it owns its inner handle, mirrors
//! every byte method through
//! [`delegate_iobase!`](crate::delegate_iobase), and exposes the **raw encoded
//! bytes unchanged** - so a `.log.gz` behind a `Text` can still be copied or
//! uploaded verbatim.
//!
//! Its lifecycle and byte-mutation overrides exist only to manage cached
//! derivations. `open` materializes the resolved field and peeled coding plan;
//! the first dimension request lazily fills its opened-session cell, and
//! `close` publishes and releases them. Nothing fills that cache as a side
//! effect of an ordinary read: a cache nobody asked for is how a handle serves
//! a stale answer after the resource changes underneath it.

use std::io::Read;
use std::sync::Arc;
#[cfg(feature = "arrow")]
use std::sync::OnceLock;

use crate::io::{Cursor, DEFAULT_STREAM_BATCH_SIZE, IOBase};
use crate::{Codec, Field, MimeType, Result};

use super::options::TextLineOptions;
use super::reader::Window;
use super::view::TextLine;

/// A handle reading and writing text records over any other handle.
///
/// Construction never touches the resource, exactly as
/// [`Coded::new`](crate::io::Coded::new) does not: absence reads as zero
/// records, and a write creates.
///
/// ```
/// use yggdryl::io::{Buffer, IOBase};
/// use yggdryl::text::LineSep;
///
/// # fn main() -> yggdryl::Result<()> {
/// // A constructor and one method call - no format strings, no mode flags.
/// let mut handle = Buffer::new().into_text();
/// handle.write_lines(["one", "two"])?;
/// assert_eq!(handle.read_all_bytes()?, b"one\ntwo\n");
///
/// // A pinned terminator is written verbatim and read back exactly.
/// let mut pinned = Buffer::new().into_text().with_linesep(LineSep::CRLF);
/// pinned.write_lines(["one", "two"])?;
/// assert_eq!(pinned.read_all_bytes()?, b"one\r\ntwo\r\n");
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct Text<H: IOBase> {
    handle: H,
    options: Arc<TextLineOptions>,
    /// A declared canonical field, cast onto the projected rows.
    ///
    /// What [`with_field`](Self::with_field) sets, exactly as on every other
    /// media handle; unset, the projection's own root is the field.
    declared: Option<Field>,
    /// The location the records report, when it is not the handle's own.
    ///
    /// A container's leaf is reopened through `parent`/`child_by_path`, and its rows
    /// still name the location the caller addressed rather than the reopened
    /// one.
    url: Option<Arc<str>>,
    /// What `open` materialized, released by `close`.
    cached: Option<Cached>,
}

/// What repeated calls would otherwise re-derive.
#[derive(Clone, Debug)]
struct Cached {
    /// The emitted root, resolved once.
    field: Field,
    /// The content codings to peel, outermost first, resolved once.
    codings: Vec<MimeType>,
    /// The logical row count, filled only when the opened caller asks for it.
    #[cfg(feature = "arrow")]
    row_size: OnceLock<u64>,
    /// The canonical field width, filled only when the opened caller asks.
    #[cfg(feature = "arrow")]
    column_size: OnceLock<usize>,
}

impl Cached {
    /// Start one opened-session cache without reading record bytes.
    fn new(field: Field, codings: Vec<MimeType>) -> Self {
        Self {
            field,
            codings,
            #[cfg(feature = "arrow")]
            row_size: OnceLock::new(),
            #[cfg(feature = "arrow")]
            column_size: OnceLock::new(),
        }
    }
}

impl<H: IOBase> Text<H> {
    /// Wrap a handle in the text-line surface without touching it.
    #[must_use]
    pub fn new(handle: H) -> Self {
        Self::with_options(handle, TextLineOptions::new())
    }

    /// Wrap a handle with an extractor already in hand.
    #[must_use]
    pub fn with_options(handle: H, options: TextLineOptions) -> Self {
        Self {
            handle,
            options: Arc::new(options),
            declared: None,
            url: None,
            cached: None,
        }
    }

    /// Declare the canonical field the projected rows are cast onto.
    ///
    /// The same setting every media handle carries: unset, the projection's
    /// own root is the field.
    pub fn set_field(&mut self, field: Field) {
        self.declared = Some(field);
        self.refresh_open_cache();
    }

    /// Return this handler with a declared canonical field.
    #[must_use]
    pub fn with_field(mut self, field: Field) -> Self {
        self.set_field(field);
        self
    }

    /// Report a different location on this handler's records.
    ///
    /// Used where a leaf is reopened through its parent: the rows still name
    /// the location the caller addressed.
    #[cfg(feature = "arrow")]
    pub(crate) fn set_url_override(&mut self, url: Arc<str>) {
        self.url = Some(url);
    }

    /// The location this handler's records report.
    fn url(&self) -> Arc<str> {
        self.url.clone().unwrap_or_else(|| url_text(&self.handle))
    }

    /// Return this handler unchanged.
    ///
    /// The inherent method that makes [`IOBase::into_text`] idempotent:
    /// inherent methods win method resolution over trait ones, so
    /// `file.into_text()` yields `Text<File>` while `text.into_text()` yields
    /// the same `Text<H>` - no `Text<Text<H>>`, no second layer of delegation,
    /// no re-derived media type or re-compiled options.
    ///
    /// One edge the resolution rule leaves open: an explicit
    /// `IOBase::into_text(text_handle)` still wraps, producing `Text<Text<H>>`.
    /// That composition behaves correctly through delegation - it is wasteful,
    /// not broken - and the fix is to call the method normally.
    ///
    /// ```
    /// use yggdryl::io::{Buffer, IOBase};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut once = Buffer::from_bytes(b"line\n".to_vec()).into_text();
    /// once.set_batch_size(7);
    ///
    /// // Idempotent: the same handle, with the options it already carried.
    /// let twice = once.into_text();
    /// assert_eq!(twice.options().batch_size(), Some(7));
    /// assert_eq!(twice.read_all_bytes()?, b"line\n");
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn into_text(self) -> Self {
        self
    }

    /// Borrow the wrapped handle.
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// Refuse options for a different encoding before a write can pull its
    /// first incoming batch.
    #[cfg(feature = "arrow")]
    fn require_record_options(&self, options: &crate::generic::RecordOptions) -> Result<()> {
        if matches!(options, crate::generic::RecordOptions::Text(_)) {
            return Ok(());
        }
        Err(crate::Error::InvalidRecord {
            path: smol_str::SmolStr::new_static("$.encoding"),
            reason: crate::text::expected_got("text record options", options.mime_type()),
        })
    }

    /// Give the wrapped handle back, publishing any pending write first.
    ///
    /// # Errors
    ///
    /// Returns the wrapped handle's flush failure.
    pub fn into_handle(mut self) -> Result<H> {
        self.handle.flush()?;
        Ok(self.handle)
    }

    /// Borrow the extractor this handler reads and writes under.
    pub fn options(&self) -> &TextLineOptions {
        &self.options
    }

    /// Replace the extractor.
    ///
    /// Any cached derivation is dropped: it described the old extractor.
    pub fn set_options(&mut self, options: TextLineOptions) {
        self.options = Arc::new(options);
        self.refresh_open_cache();
    }

    /// Return this handler with a different extractor.
    #[must_use]
    pub fn with_options_of(mut self, options: TextLineOptions) -> Self {
        self.set_options(options);
        self
    }

    /// Mutate the extractor in place, dropping any cached derivation.
    fn refine(&mut self, change: impl FnOnce(&mut TextLineOptions)) {
        let mut options = self.options.as_ref().clone();
        change(&mut options);
        self.set_options(options);
    }

    /// Read records as a log: they open where a timestamp opens.
    ///
    /// The zero-configuration log path, with no expression written anywhere.
    /// The schema gains the fixed `level`, `logger`, and `thread` columns.
    ///
    /// # Errors
    ///
    /// Returns an error when a capture name already in the options collides
    /// with one of the log columns.
    pub fn set_log_opening(&mut self) -> Result<()> {
        let mut options = self.options.as_ref().clone();
        options.set_opening(super::Opening::Timestamp)?;
        self.set_options(options);
        Ok(())
    }

    /// Return this handler reading records as a log.
    ///
    /// # Errors
    ///
    /// Returns an error when a capture name collides with a log column.
    pub fn try_with_log_opening(mut self) -> Result<Self> {
        self.set_log_opening()?;
        Ok(self)
    }

    /// Set the record-opening pattern.
    ///
    /// # Errors
    ///
    /// Returns an error when the pattern does not parse or a capture name
    /// collides.
    pub fn set_pattern(&mut self, pattern: &str) -> Result<()> {
        let mut options = self.options.as_ref().clone();
        options.set_pattern(Some(pattern))?;
        self.set_options(options);
        Ok(())
    }

    /// Return this handler with a record-opening pattern.
    ///
    /// # Errors
    ///
    /// Returns an error when the pattern does not parse or a capture name
    /// collides.
    pub fn try_with_pattern(mut self, pattern: &str) -> Result<Self> {
        self.set_pattern(pattern)?;
        Ok(self)
    }

    /// Pin the record terminator.
    pub fn set_linesep(&mut self, linesep: super::LineSep) {
        self.refine(|options| options.set_linesep(Some(linesep)));
    }

    /// Return this handler with a pinned record terminator.
    #[must_use]
    pub fn with_linesep(mut self, linesep: super::LineSep) -> Self {
        self.set_linesep(linesep);
        self
    }

    /// Set the decoded-input-bytes bound a batch closes at.
    pub fn set_byte_size(&mut self, byte_size: usize) {
        self.refine(|options| options.set_byte_size(Some(byte_size)));
    }

    /// Return this handler with a decoded-input-bytes batch bound.
    #[must_use]
    pub fn with_byte_size(mut self, byte_size: usize) -> Self {
        self.set_byte_size(byte_size);
        self
    }

    /// Set the row-per-batch bound.
    pub fn set_batch_size(&mut self, batch_size: usize) {
        self.refine(|options| options.set_batch_size(Some(batch_size)));
    }

    /// Return this handler with a row-per-batch bound.
    #[must_use]
    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.set_batch_size(batch_size);
        self
    }

    /// The root Struct Field this handler's records project onto.
    ///
    /// The declared field when one was set; otherwise, answered from the
    /// options alone when the handle is closed, and from the cache between
    /// `open` and `close`. No resource is read either way.
    #[must_use]
    pub fn field(&self) -> &Field {
        if let Some(declared) = &self.declared {
            return declared;
        }
        match &self.cached {
            Some(cached) => &cached.field,
            None => self.options.field(),
        }
    }

    /// The content codings to peel, outermost first.
    fn codings(&self) -> Vec<MimeType> {
        match &self.cached {
            Some(cached) => cached.codings.clone(),
            None => self.handle.media_type().encodings().to_vec(),
        }
    }

    /// Replace stale opened-session derivations without ending the session.
    ///
    /// Closed handles deliberately stay uncached. An opened handle keeps its
    /// lifecycle state, but every lazy dimension is empty again after a field,
    /// extractor, media-type, or byte mutation.
    fn refresh_open_cache(&mut self) {
        if self.cached.is_some() {
            self.cached = Some(Cached::new(
                self.options.field().clone(),
                self.handle.media_type().encodings().to_vec(),
            ));
        }
    }

    /// Read this resource's records, one in memory at a time.
    ///
    /// Bytes stream through a bounded window, any content codings the media
    /// type declares are peeled as streaming decoders - a `trades.log.gz` reads
    /// its records without ever holding the decompressed value - and each item
    /// is a [`TextLine`] borrowed from that window. A resource that does not
    /// exist yields no records, exactly as it reads zero bytes.
    ///
    /// # Errors
    ///
    /// Construction itself cannot fail; each yielded item carries the read or
    /// decode failure of its record.
    pub fn read_lines(&self) -> Result<TextLines<Box<dyn Read + '_>>> {
        let mut lines = borrowed_lines(&self.handle, Arc::clone(&self.options));
        lines.url = self.url();
        Ok(lines)
    }

    /// [`Self::read_lines`], consuming the handle so the records own it.
    ///
    /// The shape a caller needs when the iterator must outlive the scope that
    /// built the handle - handing records across an FFI boundary, or returning
    /// them from a function that constructed the handle itself.
    ///
    /// # Errors
    ///
    /// Construction itself cannot fail; each yielded item carries the read or
    /// decode failure of its record.
    pub fn into_read_lines(self) -> Result<TextLines<Box<dyn Read + Send + 'static>>>
    where
        H: 'static,
    {
        let codings = self.codings();
        let url = self.url();
        let options = Arc::clone(&self.options);
        let mut stream: Box<dyn Read + Send + 'static> = Box::new(Cursor::new(self.handle));
        for coding in codings.iter().rev() {
            stream = Codec::from_mime_type(coding).reader_send(stream);
        }
        Ok(TextLines::over(stream, options, url))
    }

    /// Replace this resource's records with `lines`, each terminated.
    ///
    /// Streaming: it takes an iterator, never a `Vec` or a slice, for the same
    /// reason the record surface refuses one - a shape requiring
    /// materialization cannot describe a resource larger than memory. Records
    /// accumulate in one reused buffer and flush in chunks, so a million-line
    /// write allocates a constant amount.
    ///
    /// Generic over what the caller already holds: `&str`, `String`, `&[u8]`,
    /// `Vec<u8>`, `Cow<'_, str>`, and `SmolStr` all pass with no conversion at
    /// the call site.
    ///
    /// # Errors
    ///
    /// Returns the resource's resize or write failure.
    pub fn write_lines<I, L>(&mut self, lines: I) -> Result<()>
    where
        I: IntoIterator<Item = L>,
        L: AsRef<[u8]>,
    {
        let result = (|| {
            self.handle.truncate(0)?;
            self.append_lines_inner(lines)
        })();
        self.refresh_open_cache();
        result
    }

    /// Append `lines` after this resource's current end, each terminated.
    ///
    /// Streams exactly as [`Self::write_lines`] does.
    ///
    /// The last chunk is followed by a flush, because appending records is a
    /// *complete* operation: a handle that over-allocates - the memory-mapped
    /// [`local::File`](crate::local::File) grows geometrically - would
    /// otherwise leave its slack on disk, and a second handle opened on the
    /// same location would read the padding as one more record.
    ///
    /// # Errors
    ///
    /// Returns the resource's write failure.
    pub fn append_lines<I, L>(&mut self, lines: I) -> Result<()>
    where
        I: IntoIterator<Item = L>,
        L: AsRef<[u8]>,
    {
        let result = self.append_lines_inner(lines);
        self.refresh_open_cache();
        result
    }

    /// The shared append implementation; callers own cache invalidation.
    fn append_lines_inner<I, L>(&mut self, lines: I) -> Result<()>
    where
        I: IntoIterator<Item = L>,
        L: AsRef<[u8]>,
    {
        let linesep = self.options.write_linesep().to_vec();
        let mut offset = self.handle.size();
        // One reused buffer, flushed in chunks: never `lines.join(linesep)`,
        // and never a `format!` per record.
        let mut pending: Vec<u8> = Vec::with_capacity(WRITE_CHUNK);
        for line in lines {
            pending.extend_from_slice(line.as_ref());
            pending.extend_from_slice(&linesep);
            if pending.len() >= WRITE_CHUNK {
                self.handle.pwrite_all(offset, &pending)?;
                offset += pending.len() as u64;
                pending.clear();
            }
        }
        if !pending.is_empty() {
            self.handle.pwrite_all(offset, &pending)?;
        }
        self.handle.flush()
    }
}

/// Bytes accumulated before a write is flushed to the resource.
const WRITE_CHUNK: usize = 64 * 1024;

/// Read one borrowed handle's records, peeling its codings as streams.
///
/// The one borrowed-read implementation: [`Text::read_lines`] and the
/// [`IOBase::read_lines`](crate::io::IOBase::read_lines) shim both come here,
/// so there is exactly one place that splits lines.
pub(crate) fn borrowed_lines<'handle, H: IOBase + ?Sized>(
    handle: &'handle H,
    options: Arc<TextLineOptions>,
) -> TextLines<Box<dyn Read + 'handle>> {
    let codings = handle.media_type().encodings().to_vec();
    // The shared default is non-zero, the only construction failure the public
    // byte stream permits. Reads and decoding failures stay lazy items.
    let mut stream: Box<dyn Read + 'handle> = Box::new(
        handle
            .pstream_bytes(0, DEFAULT_STREAM_BATCH_SIZE)
            .expect("the internal text byte-stream batch is non-zero"),
    );
    for coding in codings.iter().rev() {
        stream = Codec::from_mime_type(coding).reader(stream);
    }
    TextLines::over(stream, options, url_text(handle))
}

/// Count one handle's logical text records through the borrowed extractor.
///
/// Record boundaries are exactly those of [`borrowed_lines`], including
/// multiline pattern and log records. No Arrow array or record batch is built:
/// one borrowed record window is advanced at a time. Containers count their
/// leaves lazily, while an absent handle contributes zero rows.
#[cfg(any(feature = "arrow", test))]
pub(crate) fn row_size<H: IOBase + ?Sized>(handle: &H, options: &TextLineOptions) -> Result<u64> {
    match handle.kind() {
        crate::IOKind::Directory => {
            let mut total = 0_u64;
            for child in handle.children_where(&[], false)? {
                let child = child?;
                total = total
                    .checked_add(row_size(&child, options)?)
                    .ok_or_else(|| crate::Error::InvalidRecord {
                        path: smol_str::SmolStr::new_static("$"),
                        reason: smol_str::SmolStr::new_static("logical row count exceeds u64::MAX"),
                    })?;
            }
            Ok(total)
        }
        crate::IOKind::Unknown => Ok(0),
        _ => {
            let mut lines = borrowed_lines(handle, Arc::new(options.clone()));
            let mut total = 0_u64;
            while let Some(line) = lines.next() {
                line?;
                total = total
                    .checked_add(1)
                    .ok_or_else(|| crate::Error::InvalidRecord {
                        path: smol_str::SmolStr::new_static("$"),
                        reason: smol_str::SmolStr::new_static("logical row count exceeds u64::MAX"),
                    })?;
            }
            Ok(total)
        }
    }
}

/// The canonical `Url` display a resource's records carry, empty when unlocated.
pub(crate) fn url_text(handle: &(impl IOBase + ?Sized)) -> Arc<str> {
    handle
        .url()
        .map_or_else(|| Arc::from(""), |url| Arc::from(url.to_string().as_str()))
}

/// An iterator over one resource's text records.
///
/// Built by [`Text::read_lines`] and [`Text::into_read_lines`]. One record is in
/// memory at a time whether the resource is plain or compressed, and each item
/// borrows the reader's window rather than owning a copy.
pub struct TextLines<R> {
    window: Window<R>,
    options: Arc<TextLineOptions>,
    url: Arc<str>,
    rownum: i64,
    /// The capture buffers every record borrows instead of allocating.
    scratch: super::view::Scratch,
}

impl<R: Read> TextLines<R> {
    /// Wrap a decoded byte stream in record iteration.
    fn over(source: R, options: Arc<TextLineOptions>, url: Arc<str>) -> Self {
        let scratch = super::view::Scratch::for_options(&options);
        Self {
            window: Window::new(source),
            options,
            url,
            rownum: 0,
            scratch,
        }
    }

    /// The extractor these records are read under.
    pub fn options(&self) -> &TextLineOptions {
        &self.options
    }

    /// The next record, borrowed from the window.
    ///
    /// Not [`Iterator`]: an item borrows `self`, which the trait's signature
    /// cannot express. That is the whole point - the alternative is a `String`
    /// per record.
    ///
    /// # Errors
    ///
    /// Each item carries the read or decode failure of its record.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<Result<TextLine<'_>>> {
        // Split the borrow explicitly: the record borrows the window while the
        // extractor and the url borrow their own fields.
        let Self {
            window,
            options,
            url,
            rownum,
            scratch,
        } = self;
        let linesep = options.linesep();
        let mut opens = super::opener(options);
        let record = match window.next_record(linesep, &mut opens) {
            Some(Ok(record)) => record,
            Some(Err(error)) => return Some(Err(error)),
            None => return None,
        };
        *rownum += 1;
        Some(Ok(TextLine::new(
            record.bytes,
            options,
            url,
            *rownum,
            record.offset,
            record.lines,
            scratch,
        )))
    }
}

/// A [`Text`] handler mirrors its wrapped handle's bytes exactly.
///
/// The encoded value is exposed unchanged, so a `.log.gz` behind a `Text` is
/// still a `.log.gz` to anything that copies or uploads it. Its lifecycle and
/// mutation overrides only keep the opened-session derivation cache coherent.
impl<H: IOBase> crate::io::IOMedia for Text<H> {
    fn as_io_base(&self) -> &dyn IOBase {
        self
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self
    }

    /// Count logical text records without constructing Arrow batches.
    ///
    /// Closed handles stream a fresh count every time. Inside an explicit
    /// `open`/`close` session, the first request fills the cache even when the
    /// answer is zero; later requests reuse it until a relevant mutation.
    #[cfg(feature = "arrow")]
    fn row_size(&self) -> Result<u64> {
        if let Some(cached) = &self.cached {
            if let Some(size) = cached.row_size.get() {
                return Ok(*size);
            }
            let size = row_size(&self.handle, &self.options)?;
            return Ok(*cached.row_size.get_or_init(|| size));
        }
        row_size(&self.handle, &self.options)
    }

    /// Return the width of the canonical Struct field without reading bytes.
    #[cfg(feature = "arrow")]
    fn column_size(&self) -> Result<usize> {
        let size = self.field().fields().len();
        match &self.cached {
            Some(cached) => Ok(*cached.column_size.get_or_init(|| size)),
            None => Ok(size),
        }
    }

    /// The record options of a `Text` are its own extractor.
    ///
    /// This is what routes the three record methods through the line
    /// projection: a read parses records under these options, a write renders
    /// batches back as lines - the default path, with no options in sight.
    #[cfg(feature = "arrow")]
    fn record_options(&self) -> Result<crate::generic::RecordOptions> {
        let mut options = super::record::TextOptions::with_lines(self.options.as_ref().clone());
        options.field = self.declared.clone();
        Ok(crate::generic::RecordOptions::Text(Box::new(options)))
    }

    #[cfg(feature = "arrow")]
    fn overwrite_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &crate::generic::RecordOptions,
    ) -> Result<()> {
        self.require_record_options(options)?;
        crate::io::overwrite_arrow_reader_default(self, batches, options)
    }

    #[cfg(feature = "arrow")]
    fn append_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &crate::generic::RecordOptions,
    ) -> Result<()> {
        self.require_record_options(options)?;
        crate::io::append_arrow_reader_default(self, batches, options)
    }

    #[cfg(feature = "arrow")]
    fn merge_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &crate::generic::RecordOptions,
    ) -> Result<()> {
        self.require_record_options(options)?;
        crate::io::merge_arrow_reader_default(self, batches, options)
    }
}

impl<H: IOBase> IOBase for Text<H> {
    crate::delegate_iobase!(handle: pread, pstream_bytes, size, capacity, reserve, url, media_type, flush,
        parent, child_by_path, ls, kind);

    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        self.handle.read_all_bytes()
    }

    fn read_range(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        self.handle.read_range(offset, length)
    }

    /// Write bytes through and invalidate every opened-session dimension.
    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        let result = self.handle.pwrite(offset, bytes);
        self.refresh_open_cache();
        result
    }

    /// Resize through and invalidate every opened-session dimension.
    fn truncate(&mut self, size: u64) -> Result<()> {
        let result = self.handle.truncate(size);
        self.refresh_open_cache();
        result
    }

    /// Replace the declared representation and refresh its coding plan.
    fn set_media_type(&mut self, media_type: crate::MediaType) {
        self.handle.set_media_type(media_type);
        self.refresh_open_cache();
    }

    // The byte surface is the wrapped handle's; the record surface is this
    // handler's own, so `is_tabular` is answered here rather than delegated.
    fn is_atomic(&self) -> bool {
        self.handle.is_atomic()
    }

    /// A `Text` always presents its value as rows: the line projection needs
    /// no stored schema, so the record surface answers for any bytes.
    fn is_tabular(&self) -> bool {
        true
    }

    /// Preserve a wrapped decoding view instead of reopening its raw encoded
    /// location under this view's decoded media type.
    #[cfg(feature = "arrow")]
    fn read_arrow_lines(&self, options: &TextLineOptions) -> Result<crate::arrow::BatchReader> {
        self.handle.read_arrow_lines(options)
    }

    /// Materialize what repeated calls would re-derive.
    ///
    /// The resolved field and peeled coding plan are captured immediately;
    /// dimension cells remain empty until asked. In particular, opening text
    /// never scans its bytes merely to count rows.
    fn open(&mut self) -> Result<()> {
        self.handle.open()?;
        if self.cached.is_none() {
            self.cached = Some(Cached::new(
                self.options.field().clone(),
                self.handle.media_type().encodings().to_vec(),
            ));
        }
        Ok(())
    }

    /// Return whether the derivation cache is currently held.
    fn opened(&self) -> bool {
        self.cached.is_some()
    }

    /// Publish the wrapped handle and release the derivation cache.
    fn close(&mut self) -> Result<()> {
        self.cached = None;
        self.handle.close()
    }

    /// Empty the wrapped resource and invalidate opened-session dimensions.
    fn clear(&mut self) -> Result<()> {
        let result = self.handle.clear();
        self.refresh_open_cache();
        result
    }

    /// Delete the wrapped resource and release the opened-session cache.
    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.cached = None;
        self.handle.remove(recursive)
    }
}
