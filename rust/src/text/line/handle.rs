//! [`Text<H>`], the wrapping handle the whole line surface hangs off.
//!
//! A wrapping handle in the same shape as [`Coded`](crate::io::Coded),
//! [`Gzip`](crate::gzip::Gzip), [`Ipc`](crate::ipc::Ipc), and
//! [`Parquet`](crate::parquet::Parquet): it owns its inner handle, mirrors
//! every byte method through
//! [`delegate_iobase!`](crate::delegate_iobase), and exposes the **raw encoded
//! bytes unchanged** - so a `.log.gz` behind a `Text` can still be copied or
//! uploaded verbatim.
//!
//! It overrides only [`open`](crate::io::IOBase::open) and
//! [`close`](crate::io::IOBase::close). `open` materializes what repeated calls
//! would re-derive - the resolved schema and the peeled coding plan - and
//! `close` publishes and releases it. Nothing fills that cache as a side effect
//! of an ordinary read: a cache nobody asked for is how a handle serves a stale
//! answer after the resource changes underneath it.

use std::io::Read;
use std::sync::Arc;

use crate::io::{Cursor, IOBase};
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
    /// The location the records report, when it is not the handle's own.
    ///
    /// A container's leaf is reopened through `parent`/`child_by`, and its rows
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
    schema: Field,
    /// The content codings to peel, outermost first, resolved once.
    codings: Vec<MimeType>,
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
            url: None,
            cached: None,
        }
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
        self.cached = None;
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
    /// Answered from the options alone when the handle is closed, and from the
    /// cache between `open` and `close`. No resource is read either way.
    #[must_use]
    pub fn schema(&self) -> &Field {
        match &self.cached {
            Some(cached) => &cached.schema,
            None => self.options.schema(),
        }
    }

    /// The content codings to peel, outermost first.
    fn codings(&self) -> Vec<MimeType> {
        match &self.cached {
            Some(cached) => cached.codings.clone(),
            None => self.handle.media_type().encodings().to_vec(),
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
        self.handle.truncate(0)?;
        self.append_lines(lines)
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
    let mut stream: Box<dyn Read + 'handle> = Box::new(BorrowedReader {
        source: handle,
        offset: 0,
    });
    for coding in codings.iter().rev() {
        stream = Codec::from_mime_type(coding).reader(stream);
    }
    TextLines::over(stream, options, url_text(handle))
}

/// Read a *coding view's* records off the encoded handle beneath it.
///
/// A coding handle presents decoded bytes while its wrapped handle holds the
/// encoded form, so its records stream through one streaming decoder rather
/// than through the materialized value: a compressed resource pays one window
/// instead of its decompressed size. A pending write already has the decoded
/// value in memory, so it reads through the view itself.
///
/// This is the only place that difference lives; the splitting is the same one
/// implementation either way.
pub(crate) fn coded_lines<'handle, V, H>(
    view: &'handle V,
    encoded: &'handle H,
    codec: crate::Codec,
    dirty: bool,
) -> Result<TextLines<Box<dyn Read + 'handle>>>
where
    V: IOBase,
    H: IOBase,
{
    let options = Arc::new(TextLineOptions::new());
    if dirty {
        return Ok(borrowed_lines(view, options));
    }
    let mut stream: Box<dyn Read + 'handle> = codec.reader(Box::new(BorrowedReader {
        source: encoded,
        offset: 0,
    }));
    for coding in view.media_type().encodings().iter().rev() {
        stream = crate::Codec::from_mime_type(coding).reader(stream);
    }
    Ok(TextLines::over(stream, options, url_text(view)))
}

/// A streaming reader over a borrowed handle, at its own offset.
///
/// [`IOBase::reader_at`](crate::io::IOBase::reader_at) needs `Self: Sized` and
/// ties its lifetime to the receiver, which a `?Sized` borrow cannot satisfy.
/// This is the same three lines without either constraint.
struct BorrowedReader<'handle, H: IOBase + ?Sized> {
    source: &'handle H,
    offset: u64,
}

impl<H: IOBase + ?Sized> Read for BorrowedReader<'_, H> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self
            .source
            .pread(self.offset, buffer)
            .map_err(std::io::Error::other)?;
        self.offset += read as u64;
        Ok(read)
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
/// still a `.log.gz` to anything that copies or uploads it. Only `open` and
/// `close` are overridden, and they only manage the derivation cache.
impl<H: IOBase> IOBase for Text<H> {
    crate::delegate_iobase!(handle, except_lifecycle);

    // A `Text` reads its rows through the line options a caller supplies, not
    // through a record encoding of its own, so both surface questions are the
    // wrapped handle's answers.
    crate::delegate_iobase!(handle: is_atomic, is_tabular);

    /// Materialize what repeated calls would re-derive.
    ///
    /// The resolved schema and the peeled coding plan, and nothing else - no
    /// bytes are read, because the emitted shape follows from the options
    /// alone.
    fn open(&mut self) -> Result<()> {
        self.handle.open()?;
        if self.cached.is_none() {
            self.cached = Some(Cached {
                schema: self.options.schema().clone(),
                codings: self.handle.media_type().encodings().to_vec(),
            });
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

    /// Empty the wrapped resource, dropping the derivation cache with it.
    fn clear(&mut self) -> Result<()> {
        self.cached = None;
        self.handle.clear()
    }

    /// Delete the wrapped resource, and the derivation cache with it.
    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.cached = None;
        self.handle.remove(recursive)
    }
}
