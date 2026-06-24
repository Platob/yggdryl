//! # yggdryl-io
//!
//! The byte-IO foundation for the **yggdryl** project: one set of methods to
//! read, write, seek and stat bytes, **wherever they live** — in memory, on a
//! local path, or (via downstream crates) in cloud object storage. It is the
//! base buffer layer that columnar formats such as Arrow / Parquet sit on,
//! mixing *random* access (read a footer, a column chunk) with *streamed*
//! access (scan record batches) over the same handle.
//!
//! ## What this crate is for (read this first if you are extending it)
//!
//! The goal is a **single abstraction** — [`Io`] — that hides *where* bytes come
//! from. Code that reads Parquet should not care whether the source is a
//! `Vec<u8>`, a memory-mapped file, or an S3 object: it asks the same handle for
//! [`pread`](Io::pread), [`seek`](Seek::seek), [`stats`](Io::stats), its
//! [`url`](Io::url) and a streamed [`read_bytes`](ReadBytes::read_bytes). New
//! backends implement [`Io`] (and [`Path`] if they name a resource); everything
//! else composes on top.
//!
//! ## Layers
//!
//! - **Byte primitives** — [`ReadBytes`] (source) and [`WriteBytes`] (sink),
//!   with `&[u8]` and `Vec<u8>` as the trivial in-memory ends.
//! - **Cursor** — [`Seek`] adds `seek` / `stream_position` / `stream_len`, so a
//!   handle supports both streamed and positioned access.
//! - **The handle** — [`Io`]`: ReadBytes + Seek` is the base buffer: every handle
//!   has a [`url`](Io::url) (in-memory ones use `mem://<address>`); it reads and
//!   writes at a position via [`pread`](Io::pread) / [`pwrite`](Io::pwrite) — a
//!   `whence` chooses positional (cursor untouched, the default) versus
//!   cursor-relative; it exposes its bytes for **zero-copy** transfer via
//!   [`as_slice`](Io::as_slice), reports [`stats`](Io::stats), and
//!   [`copy_to`](Io::copy_to) another sink with a memory fast path. [`copy`] is
//!   the free-function form.
//! - **Metadata** — [`IoStats`] holds `size` / `mtime` / `content_type` / `etag`
//!   eagerly; expensive fields like `media_type` are discovered lazily (only
//!   when asked, then cached) — see [`Io::media_type`] under the `media` feature.
//! - **Named resources** — [`Path`]`: Io` is a local, hierarchical location
//!   (its writes auto-create missing parent dirs *lazily*, on failure, never by
//!   probing first); [`LocalPath`] is the filesystem backend, memory-mapping the
//!   file for direct zero-copy access when the `mmap` feature is on.
//!   [`RemotePath`]`: Io` is the URL-addressed cloud sibling (flat keys, no dir
//!   creation); concrete S3 / Azure backends are downstream crates that implement
//!   it — no change to this crate is needed.
//! - **Typed codecs** — [`Codec<T>`] reads/writes/streams values of `T` over any
//!   byte handle (e.g. a `Codec<RecordBatch>`); [`Frames`] is the reference
//!   length-delimited implementation.
//!
//! ## Optional features (off by default; the base build depends only on
//! `yggdryl-url`, for the universal [`Io::url`])
//!
//! - `log` — structured `log` events on the hot paths.
//! - `mmap` — [`LocalPath`] memory-maps files (zero-copy) instead of reading them.
//! - `media` — lazy [`media_type`](Io::media_type) discovery via `yggdryl-media`.
//!
//! ```
//! use yggdryl_io::{BytesIO, Io, Whence};
//!
//! let mut io = BytesIO::from_bytes(b"hello world".to_vec());
//! // Positional read at an offset, leaving the cursor untouched.
//! let mut footer = [0u8; 5];
//! io.pread(&mut footer, 6, Whence::Start).unwrap();
//! assert_eq!(&footer, b"world");
//! // Streamed access from the cursor, plus the handle's URL and size.
//! assert_eq!(io.read(Some(5)), b"hello");
//! assert_eq!(io.url().scheme(), "mem");
//! assert_eq!(io.stats().unwrap().size(), 11);
//! ```

use std::fmt;
use std::fs;
use std::marker::PhantomData;
use std::time::SystemTime;

use yggdryl_url::Url;

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise (so the crate pulls no `log` dependency by default and pays no
/// runtime cost).
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        log::$level!($($arg)+);
    }};
}

/// Error returned by every [`ReadBytes`], [`WriteBytes`], [`Seek`], [`Io`] and
/// [`Codec`] operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IoError {
    /// The source ended in the middle of a value (a read needed more bytes than
    /// were left).
    UnexpectedEof,
    /// The sink accepted no bytes and could make no progress (it is full or
    /// closed).
    WriteZero,
    /// The bytes were structurally malformed for the value being read or written.
    Invalid(String),
    /// The named resource does not exist or is unreachable.
    NotFound(String),
    /// The operation is not supported by this backend (e.g. writing a read-only
    /// mapping, or seeking a non-seekable stream).
    Unsupported(String),
    /// An underlying OS or backend error, carrying its message.
    Io(String),
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IoError::UnexpectedEof => write!(f, "unexpected end of input"),
            IoError::WriteZero => write!(f, "sink accepted no bytes"),
            IoError::Invalid(what) => write!(f, "malformed bytes: {what}"),
            IoError::NotFound(what) => write!(f, "resource not found: {what}"),
            IoError::Unsupported(what) => write!(f, "unsupported operation: {what}"),
            IoError::Io(what) => write!(f, "io error: {what}"),
        }
    }
}

impl std::error::Error for IoError {}

impl From<std::io::Error> for IoError {
    fn from(err: std::io::Error) -> IoError {
        match err.kind() {
            std::io::ErrorKind::NotFound => IoError::NotFound(err.to_string()),
            std::io::ErrorKind::UnexpectedEof => IoError::UnexpectedEof,
            _ => IoError::Io(err.to_string()),
        }
    }
}

/// Where a [`Seek::seek`] offset is measured from, mirroring the `whence` values
/// of Python's `io` module (`SEEK_SET` / `SEEK_CUR` / `SEEK_END`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Whence {
    /// From the start of the buffer (`0`).
    #[default]
    Start,
    /// From the current cursor position (`1`).
    Current,
    /// From the end of the buffer (`2`).
    End,
}

/// A byte **source**: pull raw bytes out of something.
///
/// Implementors only provide [`read_bytes`](ReadBytes::read_bytes), which fills
/// as much of `buf` as it can and returns the count; a count of `0` means the
/// source is drained (clean end of input). The provided
/// [`read_exact`](ReadBytes::read_exact) and [`read_to_end`](ReadBytes::read_to_end)
/// build on it. `&[u8]` is the built-in in-memory source.
pub trait ReadBytes {
    /// Reads into `buf`, returning how many bytes were written to its front.
    /// Returns `Ok(0)` only when the source is drained.
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, IoError>;

    /// Fills `buf` completely, or fails with [`IoError::UnexpectedEof`] if the
    /// source drains first.
    fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<(), IoError> {
        while !buf.is_empty() {
            let count = self.read_bytes(buf)?;
            if count == 0 {
                return Err(IoError::UnexpectedEof);
            }
            buf = &mut buf[count..];
        }
        Ok(())
    }

    /// Drains the source, appending every remaining byte to `out` and returning
    /// how many were read.
    fn read_to_end(&mut self, out: &mut Vec<u8>) -> Result<usize, IoError> {
        let mut chunk = [0u8; 4096];
        let mut total = 0;
        loop {
            let count = self.read_bytes(&mut chunk)?;
            if count == 0 {
                return Ok(total);
            }
            out.extend_from_slice(&chunk[..count]);
            total += count;
        }
    }
}

/// A byte **sink**: push raw bytes into something.
///
/// Implementors only provide [`write_bytes`](WriteBytes::write_bytes), which
/// accepts as much of `bytes` as it can and returns the count; the provided
/// [`write_all`](WriteBytes::write_all) loops until everything lands. `Vec<u8>`
/// is the built-in in-memory sink.
pub trait WriteBytes {
    /// Writes the front of `bytes`, returning how many were accepted. Returns
    /// `Ok(0)` only when the sink can make no progress.
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<usize, IoError>;

    /// Writes every byte of `bytes`, or fails with [`IoError::WriteZero`] if the
    /// sink stalls before they all land.
    fn write_all(&mut self, mut bytes: &[u8]) -> Result<(), IoError> {
        while !bytes.is_empty() {
            let count = self.write_bytes(bytes)?;
            if count == 0 {
                return Err(IoError::WriteZero);
            }
            bytes = &bytes[count..];
        }
        Ok(())
    }

    /// Flushes any buffered bytes to their destination. The default is a no-op,
    /// which suits unbuffered sinks like [`Vec<u8>`].
    fn flush(&mut self) -> Result<(), IoError> {
        Ok(())
    }
}

/// A movable read/write **cursor**, the seek half of an [`Io`] handle.
///
/// Positions are absolute byte offsets from the start. [`seek`](Seek::seek)
/// mirrors Python's `io.seek` (and is the basis for positioned cloud range
/// reads); [`stream_len`](Seek::stream_len) reports the total size when it is
/// known without I/O.
pub trait Seek {
    /// Moves the cursor to `offset` relative to `whence`, returning the new
    /// absolute position. Seeking before the start fails with
    /// [`IoError::Invalid`]; seeking past the end is allowed.
    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError>;

    /// The current absolute cursor position.
    fn stream_position(&self) -> u64;

    /// The total length in bytes when known cheaply (without I/O), else `None`.
    fn stream_len(&self) -> Option<u64> {
        None
    }
}

/// In-memory source: reading advances the slice past the bytes consumed, so a
/// `&[u8]` can be read to exhaustion.
impl ReadBytes for &[u8] {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        let count = buf.len().min(self.len());
        let (head, tail) = self.split_at(count);
        buf[..count].copy_from_slice(head);
        *self = tail;
        Ok(count)
    }
}

/// In-memory sink: writing appends to the vector, which never stalls.
impl WriteBytes for Vec<u8> {
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<usize, IoError> {
        self.extend_from_slice(bytes);
        Ok(bytes.len())
    }
}

/// Lazily-discovered metadata for an [`Io`] handle: cheap fields (`size`,
/// `mtime`, `content_type`, `etag`) are filled eagerly by [`Io::stats`], while
/// anything expensive (`media_type`, under the `media` feature) is discovered
/// only on demand — see [`Io::media_type`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IoStats {
    size: u64,
    mtime: Option<SystemTime>,
    content_type: Option<String>,
    etag: Option<String>,
    #[cfg(feature = "media")]
    media_type: Option<yggdryl_media::MediaType>,
}

impl IoStats {
    /// Creates stats for a resource of `size` bytes, with all other fields unset.
    pub fn new(size: u64) -> IoStats {
        IoStats {
            size,
            ..IoStats::default()
        }
    }

    /// The size in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The last-modified time, if the backend reports one.
    pub fn mtime(&self) -> Option<SystemTime> {
        self.mtime
    }

    /// The transport content type (e.g. a cloud `Content-Type`), if any.
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    /// The backend entity tag (e.g. an object-store `ETag`), if any.
    pub fn etag(&self) -> Option<&str> {
        self.etag.as_deref()
    }

    /// The discovered media type, if it has been filled in (see
    /// [`Io::media_type`]). Only present under the `media` feature.
    #[cfg(feature = "media")]
    pub fn media_type(&self) -> Option<&yggdryl_media::MediaType> {
        self.media_type.as_ref()
    }

    /// Returns a copy with `mtime` set.
    pub fn with_mtime(mut self, mtime: SystemTime) -> IoStats {
        self.mtime = Some(mtime);
        self
    }

    /// Returns a copy with `content_type` set.
    pub fn with_content_type(mut self, content_type: impl Into<String>) -> IoStats {
        self.content_type = Some(content_type.into());
        self
    }

    /// Returns a copy with `etag` set.
    pub fn with_etag(mut self, etag: impl Into<String>) -> IoStats {
        self.etag = Some(etag.into());
        self
    }

    /// Returns a copy with the discovered `media_type` set.
    #[cfg(feature = "media")]
    pub fn with_media_type(mut self, media_type: yggdryl_media::MediaType) -> IoStats {
        self.media_type = Some(media_type);
        self
    }
}

/// Resolves an `(offset, whence)` pair against the current `position` and an
/// optional total `len` into an absolute byte position — the one place the
/// cursor / start / end arithmetic lives, shared by [`Seek`] impls and
/// [`Io::pread`] / [`Io::pwrite`].
fn resolve(position: u64, len: Option<u64>, offset: i64, whence: Whence) -> Result<u64, IoError> {
    let base: i64 = match whence {
        Whence::Start => 0,
        Whence::Current => position as i64,
        Whence::End => len.ok_or_else(|| {
            IoError::Unsupported("offset from end without a known length".to_string())
        })? as i64,
    };
    let target = base
        .checked_add(offset)
        .ok_or_else(|| IoError::Invalid(format!("offset {offset} overflows")))?;
    if target < 0 {
        return Err(IoError::Invalid(format!(
            "position {target} is before the start"
        )));
    }
    Ok(target as u64)
}

/// The base **byte-IO handle**: a [`ReadBytes`] + [`Seek`] source that also knows
/// its [`url`](Io::url) and [`stats`](Io::stats), reads and writes at a position
/// via [`pread`](Io::pread) / [`pwrite`](Io::pwrite), exposes its bytes for
/// zero-copy transfer, and copies itself into a sink.
///
/// This is the abstraction Arrow/Parquet-style readers target: implement it once
/// per backend (memory, file, cloud) and the same reader works everywhere.
/// Implementors must provide [`url`](Io::url) and [`stats`](Io::stats); the rest
/// have defaults, but a memory-resident backend should override
/// [`as_slice`](Io::as_slice) to unlock the zero-copy fast paths, and a writable
/// backend should override [`pwrite`](Io::pwrite).
pub trait Io: ReadBytes + Seek {
    /// The address of this resource as a [`Url`]. **Every IO has one**: file
    /// backends use `file`, remote ones their store URL, and an in-memory handle
    /// the `mem` scheme with its buffer address (e.g. `mem://7f3c…`).
    fn url(&self) -> Url;

    /// Discovers metadata. Cheap fields are eager; see [`IoStats`].
    fn stats(&self) -> Result<IoStats, IoError>;

    /// Borrows the whole backing buffer when this handle is memory-resident,
    /// enabling zero-copy reads and transfers. Streamed backends return `None`
    /// (the default).
    fn as_slice(&self) -> Option<&[u8]> {
        None
    }

    /// Positional read into `buf`, starting at `offset` relative to `whence`, and
    /// returning the count read (short at end of input).
    ///
    /// `whence` selects whether the streaming cursor is used: with
    /// [`Whence::Start`] (the usual default) or [`Whence::End`] the read is purely
    /// positional and **leaves the cursor untouched** (footers, column chunks);
    /// with [`Whence::Current`] the cursor is the base and is advanced by the
    /// bytes read (a streamed read). The default serves it from
    /// [`as_slice`](Io::as_slice) when available, else seeks and restores.
    fn pread(&mut self, buf: &mut [u8], offset: i64, whence: Whence) -> Result<usize, IoError> {
        let start = resolve(self.stream_position(), self.stream_len(), offset, whence)?;
        let count = if let Some(all) = self.as_slice() {
            let begin = start.min(all.len() as u64) as usize;
            let count = buf.len().min(all.len() - begin);
            buf[..count].copy_from_slice(&all[begin..begin + count]);
            count
        } else {
            let saved = self.stream_position();
            self.seek(start as i64, Whence::Start)?;
            let mut filled = 0;
            let mut outcome = Ok(());
            while filled < buf.len() {
                match self.read_bytes(&mut buf[filled..]) {
                    Ok(0) => break,
                    Ok(count) => filled += count,
                    Err(error) => {
                        outcome = Err(error);
                        break;
                    }
                }
            }
            // Restore first, then apply the cursor policy below.
            self.seek(saved as i64, Whence::Start)?;
            outcome?;
            filled
        };
        // Only a cursor-relative read moves the cursor; positional reads do not.
        if matches!(whence, Whence::Current) {
            self.seek((start + count as u64) as i64, Whence::Start)?;
        }
        Ok(count)
    }

    /// Positional write of `bytes`, starting at `offset` relative to `whence`,
    /// returning the count written — the mirror of [`pread`](Io::pread).
    ///
    /// As with `pread`, [`Whence::Current`] uses and advances the cursor while
    /// [`Whence::Start`] / [`Whence::End`] leave it untouched. The default is
    /// [`IoError::Unsupported`]; a writable backend overrides it.
    fn pwrite(&mut self, bytes: &[u8], offset: i64, whence: Whence) -> Result<usize, IoError> {
        let _ = (bytes, offset, whence);
        Err(IoError::Unsupported("pwrite".to_string()))
    }

    /// Copies every byte from the cursor to the end into `dst`, returning the
    /// count. A memory-resident source writes its tail in a single
    /// [`write_all`](WriteBytes::write_all) (zero intermediate copies); otherwise
    /// it streams in 64 KiB chunks. See also the free [`copy`] function.
    fn copy_to(&mut self, dst: &mut dyn WriteBytes) -> Result<u64, IoError> {
        if self.as_slice().is_some() {
            // Fast path: hand the remaining slice straight to the sink, then
            // advance our cursor to the end. The inner block scopes the borrow of
            // `self` so the trailing `seek` can take `&mut self`.
            let copied = {
                let all = self.as_slice().unwrap();
                let start = self.stream_position().min(all.len() as u64) as usize;
                let tail = &all[start..];
                dst.write_all(tail)?;
                tail.len() as u64
            };
            log_event!(trace, "Io::copy_to fast path, {copied} bytes");
            self.seek(0, Whence::End)?;
            return Ok(copied);
        }
        let mut chunk = [0u8; 64 * 1024];
        let mut copied = 0u64;
        loop {
            let count = self.read_bytes(&mut chunk)?;
            if count == 0 {
                break;
            }
            dst.write_all(&chunk[..count])?;
            copied += count as u64;
        }
        log_event!(trace, "Io::copy_to streamed, {copied} bytes");
        Ok(copied)
    }

    /// Lazily discovers the media type of this handle, caching nothing by default
    /// — the cheap path infers from magic bytes via [`as_slice`](Io::as_slice).
    /// Path-backed handles override this to use the file name (and cache it).
    /// Only present under the `media` feature.
    #[cfg(feature = "media")]
    fn media_type(&self) -> Option<yggdryl_media::MediaType> {
        let head = self.as_slice()?;
        yggdryl_media::MimeType::from_magic(head)
            .map(|mime| yggdryl_media::MediaType::new(vec![mime]))
    }
}

/// Copies every byte from `src`'s cursor to the end into `dst`, with `src`'s
/// memory fast path — the free-function form of [`Io::copy_to`], for transferring
/// between two IO implementations (e.g. a [`LocalPath`] into a [`BytesIO`]).
pub fn copy<S: Io + ?Sized>(src: &mut S, dst: &mut dyn WriteBytes) -> Result<u64, IoError> {
    src.copy_to(dst)
}

/// A simple in-memory byte buffer with a cursor, modelled on Python's
/// `io.BytesIO`: it is both a [`ReadBytes`] source and a [`WriteBytes`] sink and
/// a full [`Io`] handle, so it plugs straight into any [`Codec`] and exposes its
/// bytes for zero-copy [`copy`].
///
/// A `BytesIO` owns a [`Vec<u8>`] and a `position` cursor; [`seek`](BytesIO::seek)
/// / [`tell`](BytesIO::tell) move and read that cursor, [`getvalue`](BytesIO::getvalue)
/// borrows the whole buffer, and writes past the end zero-fill the gap (as in
/// Python).
///
/// The [`stream`](BytesIO::stream) flag governs the **Python-style** helpers
/// [`read`](BytesIO::read) / [`read_line`](BytesIO::read_line) /
/// [`write`](BytesIO::write): when `true` (the default) they advance the cursor,
/// replicating Python's stateful streaming; when `false` the cursor stays put
/// for random access. The lower-level [`ReadBytes`] / [`WriteBytes`] / [`Seek`]
/// primitives always advance, so codecs work whatever the flag.
///
/// ```
/// use yggdryl_io::{BytesIO, Whence};
///
/// let mut io = BytesIO::from_bytes(b"hello world".to_vec());
/// assert_eq!(io.read(Some(5)), b"hello");
/// assert_eq!(io.tell(), 5);
/// io.seek(6, Whence::Start).unwrap();
/// assert_eq!(io.read(None), b"world");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BytesIO {
    buffer: Vec<u8>,
    position: usize,
    stream: bool,
}

impl Default for BytesIO {
    fn default() -> BytesIO {
        BytesIO::new()
    }
}

impl BytesIO {
    /// Creates an empty buffer with the cursor at `0` and streaming on.
    pub fn new() -> BytesIO {
        BytesIO::from_bytes(Vec::new())
    }

    /// Wraps existing `bytes`, with the cursor at the start and streaming on.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> BytesIO {
        BytesIO {
            buffer: bytes.into(),
            position: 0,
            stream: true,
        }
    }

    /// Whether the Python-style [`read`](BytesIO::read) / [`read_line`](BytesIO::read_line)
    /// / [`write`](BytesIO::write) helpers advance the cursor.
    pub fn stream(&self) -> bool {
        self.stream
    }

    /// Sets the [`stream`](BytesIO::stream) flag.
    pub fn set_stream(&mut self, stream: bool) {
        log_event!(debug, "BytesIO::set_stream {stream}");
        self.stream = stream;
    }

    /// The current cursor position.
    pub fn tell(&self) -> usize {
        self.position
    }

    /// The total number of bytes held, regardless of the cursor.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Whether the buffer holds no bytes.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// The number of bytes between the cursor and the end of the buffer.
    pub fn remaining(&self) -> usize {
        self.buffer.len().saturating_sub(self.position)
    }

    /// Borrows the whole buffer, ignoring the cursor (the inverse of
    /// [`from_bytes`](BytesIO::from_bytes)).
    pub fn getvalue(&self) -> &[u8] {
        &self.buffer
    }

    /// Reads up to `size` bytes from the cursor, or all remaining bytes when
    /// `size` is `None`. Advances the cursor when [`stream`](BytesIO::stream).
    pub fn read(&mut self, size: Option<usize>) -> Vec<u8> {
        log_event!(trace, "BytesIO::read {size:?} at {}", self.position);
        let end = match size {
            Some(n) => self.position.saturating_add(n),
            None => self.buffer.len(),
        };
        self.take(end, self.stream)
    }

    /// Reads from the cursor through the next `\n` (inclusive), or to the end of
    /// the buffer. Advances the cursor when [`stream`](BytesIO::stream).
    pub fn read_line(&mut self) -> Vec<u8> {
        let start = self.position.min(self.buffer.len());
        let end = self.buffer[start..]
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(self.buffer.len(), |offset| start + offset + 1);
        self.take(end, self.stream)
    }

    /// Writes `bytes` at the cursor, overwriting any overlap and extending (zero-
    /// filling any gap) as needed. Returns the count written and advances the
    /// cursor when [`stream`](BytesIO::stream).
    pub fn write(&mut self, bytes: &[u8]) -> usize {
        log_event!(
            trace,
            "BytesIO::write {} bytes at {}",
            bytes.len(),
            self.position
        );
        self.put(bytes, self.stream)
    }

    /// Moves the cursor to `offset` relative to `whence`, returning the new
    /// position. Seeking past the end is allowed (a later write zero-fills the
    /// gap); seeking before the start fails with [`IoError::Invalid`].
    pub fn seek(&mut self, offset: i64, whence: Whence) -> Result<usize, IoError> {
        log_event!(trace, "BytesIO::seek {offset} from {whence:?}");
        let base = match whence {
            Whence::Start => 0,
            Whence::Current => self.position as i64,
            Whence::End => self.buffer.len() as i64,
        };
        let target = base
            .checked_add(offset)
            .ok_or_else(|| IoError::Invalid(format!("seek offset {offset} overflows")))?;
        if target < 0 {
            return Err(IoError::Invalid(format!(
                "seek to {target} is before the start"
            )));
        }
        self.position = target as usize;
        Ok(self.position)
    }

    /// Truncates the buffer to `size` bytes (the current cursor when `None`),
    /// returning the new length. The cursor is left where it is, as in Python.
    pub fn truncate(&mut self, size: Option<usize>) -> usize {
        let size = size.unwrap_or(self.position);
        log_event!(debug, "BytesIO::truncate to {size}");
        self.buffer.truncate(size);
        self.buffer.len()
    }

    /// Empties the buffer and resets the cursor to `0`.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.position = 0;
    }

    /// No-op flush, present for parity with Python's `io` API.
    pub fn flush(&mut self) {}

    /// Reads `[cursor..end]` (clamped to the buffer) as an owned vector, advancing
    /// the cursor by the count actually read when `advance` (so a cursor seeked
    /// past the end stays put, as in Python). Shared by the read helpers.
    fn take(&mut self, end: usize, advance: bool) -> Vec<u8> {
        let start = self.position.min(self.buffer.len());
        let end = end.clamp(start, self.buffer.len());
        if advance {
            self.position += end - start;
        }
        self.buffer[start..end].to_vec()
    }

    /// Writes `bytes` at the cursor, zero-filling any gap and extending as needed,
    /// moving the cursor past them when `advance`. Shared by [`write`](BytesIO::write)
    /// and the [`WriteBytes`] primitive.
    fn put(&mut self, bytes: &[u8], advance: bool) -> usize {
        let start = self.position;
        let end = start + bytes.len();
        if self.buffer.len() < end {
            self.buffer.resize(end, 0);
        }
        self.buffer[start..end].copy_from_slice(bytes);
        if advance {
            self.position = end;
        }
        bytes.len()
    }
}

/// In-memory source: reads from the cursor and advances it, so a `BytesIO` drains
/// like any other [`ReadBytes`] when driving a [`Codec`].
impl ReadBytes for BytesIO {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        let start = self.position.min(self.buffer.len());
        let count = buf.len().min(self.buffer.len() - start);
        buf[..count].copy_from_slice(&self.buffer[start..start + count]);
        self.position += count;
        Ok(count)
    }
}

/// In-memory sink: writes at the cursor and advances it, never stalling.
impl WriteBytes for BytesIO {
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<usize, IoError> {
        Ok(self.put(bytes, true))
    }
}

impl Seek for BytesIO {
    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        BytesIO::seek(self, offset, whence).map(|position| position as u64)
    }

    fn stream_position(&self) -> u64 {
        self.position as u64
    }

    fn stream_len(&self) -> Option<u64> {
        Some(self.buffer.len() as u64)
    }
}

impl Io for BytesIO {
    /// `mem://<buffer-address>` — the `mem` scheme with the hex address of the
    /// backing bytes, so every in-memory handle still has a stable-shape URL.
    fn url(&self) -> Url {
        Url::new("mem", format!("{:x}", self.buffer.as_ptr() as usize))
    }

    fn stats(&self) -> Result<IoStats, IoError> {
        Ok(IoStats::new(self.buffer.len() as u64))
    }

    fn as_slice(&self) -> Option<&[u8]> {
        Some(&self.buffer)
    }

    /// Positional write into the buffer, overwriting and zero-filling as needed.
    /// [`Whence::Current`] advances the cursor; otherwise it is left put.
    fn pwrite(&mut self, bytes: &[u8], offset: i64, whence: Whence) -> Result<usize, IoError> {
        let start = resolve(
            self.position as u64,
            Some(self.buffer.len() as u64),
            offset,
            whence,
        )? as usize;
        let saved = self.position;
        self.position = start;
        let count = self.put(bytes, true);
        if !matches!(whence, Whence::Current) {
            self.position = saved;
        }
        Ok(count)
    }
}

/// A **local, hierarchical** named byte resource — a *location* in a directory
/// tree that is itself an [`Io`] handle once constructed. [`LocalPath`] is the
/// filesystem implementation. For remote object stores see [`RemotePath`].
///
/// ## Directory contract (auto-create, lazily)
///
/// A `Path` writes into a directory hierarchy, so a write target's parent may not
/// exist yet. Implementations **auto-create missing parent directories**, but do
/// so the cheap way: they do *not* check whether the directory exists before
/// every write. They attempt the write, and only when it fails because a parent
/// is missing do they create the tree once and retry. The common case (the
/// directory already exists) therefore pays no `exists` probe — see
/// [`LocalPath::write`].
pub trait Path: Io {
    /// The resource location (a filesystem path).
    fn location(&self) -> &str;

    /// Whether the resource currently exists / is reachable.
    fn exists(&self) -> bool;
}

/// A **remote, URL-addressed** named byte resource — the cloud sibling of
/// [`Path`].
///
/// Object stores (S3, Azure, GCS) and HTTP endpoints are addressed by URL (the
/// universal [`Io::url`]) and have **no directory hierarchy**: keys are flat, so
/// — unlike a [`Path`] — a write never creates parent directories; the object is
/// created directly. Reads are range-based through [`Io::pread`]. Network SDKs
/// are heavy, so concrete remote paths live in downstream crates that implement
/// this trait; nothing in `yggdryl-io` pulls them in.
pub trait RemotePath: Io {
    /// Whether the object currently exists (a metadata / `HEAD` probe).
    fn exists(&self) -> bool;
}

/// How a [`LocalPath`] holds its bytes: a memory map (zero-copy, `mmap` feature)
/// or an eagerly-read buffer.
#[derive(Debug)]
enum Backing {
    Buffered(Vec<u8>),
    #[cfg(feature = "mmap")]
    Mapped(memmap2::Mmap),
}

impl Backing {
    /// The backing bytes, however they are held.
    fn bytes(&self) -> &[u8] {
        match self {
            Backing::Buffered(buffer) => buffer,
            #[cfg(feature = "mmap")]
            Backing::Mapped(map) => map,
        }
    }
}

/// A local filesystem [`Path`]: an [`Io`] handle over a file, **memory-mapped**
/// for zero-copy direct access when the `mmap` feature is on (otherwise the file
/// is read into a buffer on [`open`](LocalPath::open)). Reads and [`pread`](Io::pread)
/// then never touch the disk again, and [`copy`] hands the mapping straight to a
/// sink.
///
/// The mapped path is read-only; use the [`write`](LocalPath::write) associated
/// function for the write side.
#[derive(Debug)]
pub struct LocalPath {
    location: String,
    backing: Backing,
    position: usize,
    size: u64,
    mtime: Option<SystemTime>,
    #[cfg(feature = "media")]
    media: std::sync::OnceLock<Option<yggdryl_media::MediaType>>,
}

impl LocalPath {
    /// Opens `location` for reading, memory-mapping it under the `mmap` feature
    /// (an empty file is held as an empty buffer, since zero-length maps are
    /// invalid on some platforms). Fails with [`IoError::NotFound`] if missing.
    pub fn open(location: impl Into<String>) -> Result<LocalPath, IoError> {
        let location = location.into();
        log_event!(debug, "LocalPath::open {location:?}");
        let file = fs::File::open(&location)?;
        let meta = file.metadata()?;
        let size = meta.len();
        let mtime = meta.modified().ok();

        #[cfg(feature = "mmap")]
        let backing = if size == 0 {
            Backing::Buffered(Vec::new())
        } else {
            // SAFETY: we map a file we just opened for reading. The standard mmap
            // caveat applies — external truncation while mapped is undefined — and
            // is the caller's responsibility for the paths they hand us.
            let map = unsafe { memmap2::Mmap::map(&file)? };
            Backing::Mapped(map)
        };
        #[cfg(not(feature = "mmap"))]
        let backing = {
            use std::io::Read;
            let mut buffer = Vec::with_capacity(size as usize);
            let mut file = file;
            file.read_to_end(&mut buffer)?;
            Backing::Buffered(buffer)
        };

        Ok(LocalPath {
            location,
            backing,
            position: 0,
            size,
            mtime,
            #[cfg(feature = "media")]
            media: std::sync::OnceLock::new(),
        })
    }

    /// Writes `bytes` to `location` on disk, creating or truncating the file and
    /// **auto-creating missing parent directories**.
    ///
    /// This follows the [`Path`] contract: it does *not* stat the directory first.
    /// It writes straight away, and only when the write fails because a parent is
    /// missing does it create the directory tree once and retry — so the common
    /// case (the directory already exists) pays nothing.
    pub fn write(location: &str, bytes: &[u8]) -> Result<(), IoError> {
        log_event!(
            info,
            "LocalPath::write {} bytes -> {location:?}",
            bytes.len()
        );
        match fs::write(location, bytes) {
            Ok(()) => Ok(()),
            // The directory was missing: create it once, then retry the write.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let parent = std::path::Path::new(location).parent();
                if let Some(parent) = parent.filter(|p| !p.as_os_str().is_empty()) {
                    log_event!(debug, "LocalPath::write creating parent dir {parent:?}");
                    fs::create_dir_all(parent)?;
                }
                fs::write(location, bytes)?;
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }
}

impl ReadBytes for LocalPath {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        let data = self.backing.bytes();
        let start = self.position.min(data.len());
        let count = buf.len().min(data.len() - start);
        buf[..count].copy_from_slice(&data[start..start + count]);
        self.position += count;
        Ok(count)
    }
}

impl Seek for LocalPath {
    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        let len = self.backing.bytes().len() as i64;
        let base = match whence {
            Whence::Start => 0,
            Whence::Current => self.position as i64,
            Whence::End => len,
        };
        let target = base
            .checked_add(offset)
            .ok_or_else(|| IoError::Invalid(format!("seek offset {offset} overflows")))?;
        if target < 0 {
            return Err(IoError::Invalid(format!(
                "seek to {target} is before the start"
            )));
        }
        self.position = target as usize;
        Ok(self.position as u64)
    }

    fn stream_position(&self) -> u64 {
        self.position as u64
    }

    fn stream_len(&self) -> Option<u64> {
        Some(self.size)
    }
}

impl Io for LocalPath {
    /// `file://<path>` — the `file` scheme over the resource location.
    fn url(&self) -> Url {
        Url::new("file", "").with_path(self.location.clone())
    }

    fn stats(&self) -> Result<IoStats, IoError> {
        let mut stats = IoStats::new(self.size);
        if let Some(mtime) = self.mtime {
            stats = stats.with_mtime(mtime);
        }
        #[cfg(feature = "media")]
        if let Some(media_type) = self.media_type() {
            stats = stats.with_media_type(media_type);
        }
        Ok(stats)
    }

    fn as_slice(&self) -> Option<&[u8]> {
        Some(self.backing.bytes())
    }

    /// Infers the media type from the file name (cheap, by extension) and falls
    /// back to sniffing the mapped magic bytes; the result is cached so repeated
    /// calls are free.
    #[cfg(feature = "media")]
    fn media_type(&self) -> Option<yggdryl_media::MediaType> {
        self.media
            .get_or_init(|| {
                let by_name = yggdryl_media::MediaType::from_path(&self.location);
                if !by_name.is_empty() {
                    Some(by_name)
                } else {
                    yggdryl_media::MimeType::from_magic(self.backing.bytes())
                        .map(|mime| yggdryl_media::MediaType::new(vec![mime]))
                }
            })
            .clone()
    }
}

impl Path for LocalPath {
    fn location(&self) -> &str {
        &self.location
    }

    fn exists(&self) -> bool {
        std::path::Path::new(&self.location).exists()
    }
}

/// The abstract **typed codec**: read and write values of `T` across the byte
/// primitives, in one of three shapes.
///
/// An implementor provides exactly two methods — [`read_opt`](Codec::read_opt),
/// which decodes one `T` (or `None` at a clean end of input), and
/// [`write`](Codec::write), which encodes one `T`. The rest is derived:
///
/// - single value — [`read`](Codec::read), which turns a clean end of input into
///   an [`IoError::UnexpectedEof`];
/// - many values — [`stream`](Codec::stream), an iterator that reads until the
///   source drains.
///
/// A codec composes with any [`Io`] handle: a `Codec<RecordBatch>` reads batches
/// straight out of a [`BytesIO`], a [`LocalPath`], or a cloud path alike. In-memory
/// round-trips need no extra methods — `&[u8]` is a [`ReadBytes`] and `Vec<u8>` a
/// [`WriteBytes`].
///
/// ```
/// use yggdryl_io::{Codec, Frames};
///
/// let mut bytes: Vec<u8> = Vec::new();
/// Frames.write(&mut bytes, &b"payload".to_vec()).unwrap();
/// assert_eq!(Frames.read(&mut &bytes[..]).unwrap(), b"payload".to_vec());
/// ```
pub trait Codec<T> {
    /// Reads the next value, or `Ok(None)` when the source is cleanly drained at
    /// a value boundary. This is the one read primitive an implementor defines.
    fn read_opt(&self, reader: &mut impl ReadBytes) -> Result<Option<T>, IoError>;

    /// Writes one value to the sink.
    fn write(&self, writer: &mut impl WriteBytes, value: &T) -> Result<(), IoError>;

    /// Reads exactly one value, treating a clean end of input as an error.
    fn read(&self, reader: &mut impl ReadBytes) -> Result<T, IoError> {
        self.read_opt(reader)?.ok_or(IoError::UnexpectedEof)
    }

    /// Returns an iterator that reads values from `reader` until it drains,
    /// yielding `Result<T, IoError>` for each.
    fn stream<R: ReadBytes>(&self, reader: R) -> Stream<'_, Self, R, T>
    where
        Self: Sized,
    {
        Stream {
            codec: self,
            reader,
            _marker: PhantomData,
        }
    }
}

/// Iterator returned by [`Codec::stream`]: pulls one value per step from a
/// borrowed codec and an owned byte source, ending when the source is cleanly
/// drained.
pub struct Stream<'codec, C, R, T> {
    codec: &'codec C,
    reader: R,
    _marker: PhantomData<fn() -> T>,
}

impl<C, R, T> Iterator for Stream<'_, C, R, T>
where
    C: Codec<T>,
    R: ReadBytes,
{
    type Item = Result<T, IoError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.codec.read_opt(&mut self.reader) {
            Ok(Some(value)) => Some(Ok(value)),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        }
    }
}

/// The reference [`Codec`] implementation: **length-delimited byte frames**.
///
/// Each value is written as a big-endian `u32` byte length followed by that many
/// payload bytes, so frames pack back to back and a [`stream`](Codec::stream)
/// reads them out one at a time until the source drains.
///
/// ```
/// use yggdryl_io::{Codec, Frames};
///
/// let mut sink: Vec<u8> = Vec::new();
/// Frames.write(&mut sink, &b"hi".to_vec()).unwrap();
/// assert_eq!(sink, vec![0, 0, 0, 2, b'h', b'i']);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Frames;

impl Codec<Vec<u8>> for Frames {
    fn read_opt(&self, reader: &mut impl ReadBytes) -> Result<Option<Vec<u8>>, IoError> {
        log_event!(trace, "Frames::read_opt");
        // Read the 4-byte length prefix. Zero bytes at the very start is a clean
        // end of the stream; a partial prefix is a truncated frame.
        let mut prefix = [0u8; 4];
        let mut filled = 0;
        while filled < prefix.len() {
            let count = reader.read_bytes(&mut prefix[filled..])?;
            if count == 0 {
                if filled == 0 {
                    log_event!(debug, "Frames::read_opt reached end of stream");
                    return Ok(None);
                }
                return Err(IoError::UnexpectedEof);
            }
            filled += count;
        }
        // Read the payload directly into the output, growing in bounded steps:
        // an honest frame takes a single allocation and a single copy, while a
        // malformed prefix (e.g. claiming 4 GiB with no body) fails fast having
        // reserved at most one step instead of gigabytes up front.
        const GROWTH_STEP: usize = 1 << 20;
        let len = u32::from_be_bytes(prefix) as usize;
        let mut payload = Vec::new();
        let mut filled = 0;
        while filled < len {
            let target = len.min(filled + GROWTH_STEP);
            payload.resize(target, 0);
            while filled < target {
                let count = reader.read_bytes(&mut payload[filled..target])?;
                if count == 0 {
                    return Err(IoError::UnexpectedEof);
                }
                filled += count;
            }
        }
        Ok(Some(payload))
    }

    fn write(&self, writer: &mut impl WriteBytes, value: &Vec<u8>) -> Result<(), IoError> {
        log_event!(trace, "Frames::write {} bytes", value.len());
        let len = u32::try_from(value.len())
            .map_err(|_| IoError::Invalid(format!("frame of {} bytes exceeds u32", value.len())))?;
        writer.write_all(&len.to_be_bytes())?;
        writer.write_all(value)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytesio_reads_and_advances_the_cursor() {
        let mut io = BytesIO::from_bytes(b"hello world".to_vec());
        assert_eq!(io.read(Some(5)), b"hello");
        assert_eq!(io.tell(), 5);
        assert_eq!(io.remaining(), 6);
        assert_eq!(io.read(Some(1)), b" ");
        assert_eq!(io.read(None), b"world");
        // Reading at the end yields nothing and the cursor stays put.
        assert_eq!(io.read(None), b"");
        assert_eq!(io.tell(), 11);
    }

    #[test]
    fn bytesio_without_stream_keeps_the_cursor_fixed() {
        let mut io = BytesIO::from_bytes(b"abcdef".to_vec());
        io.set_stream(false);
        assert_eq!(io.read(Some(3)), b"abc");
        assert_eq!(io.read(Some(3)), b"abc");
        assert_eq!(io.tell(), 0);
        io.seek(3, Whence::Start).unwrap();
        assert_eq!(io.read(Some(3)), b"def");
    }

    #[test]
    fn bytesio_seek_whences_and_errors() {
        let mut io = BytesIO::from_bytes(b"0123456789".to_vec());
        assert_eq!(io.seek(4, Whence::Start).unwrap(), 4);
        assert_eq!(io.seek(2, Whence::Current).unwrap(), 6);
        assert_eq!(io.seek(-1, Whence::End).unwrap(), 9);
        assert_eq!(io.read(None), b"9");
        assert!(matches!(
            io.seek(-1, Whence::Start),
            Err(IoError::Invalid(_))
        ));
        assert_eq!(io.seek(3, Whence::End).unwrap(), 13);
        assert_eq!(io.read(None), b"");
        assert_eq!(io.tell(), 13);
    }

    #[test]
    fn bytesio_write_overwrites_and_zero_fills() {
        let mut io = BytesIO::from_bytes(b"abc".to_vec());
        io.seek(1, Whence::Start).unwrap();
        assert_eq!(io.write(b"XY"), 2);
        assert_eq!(io.getvalue(), b"aXY");
        io.seek(5, Whence::Start).unwrap();
        io.write(b"Z");
        assert_eq!(io.getvalue(), b"aXY\0\0Z");
    }

    #[test]
    fn bytesio_read_line_walks_lines() {
        let mut io = BytesIO::from_bytes(b"one\ntwo\nthree".to_vec());
        assert_eq!(io.read_line(), b"one\n");
        assert_eq!(io.read_line(), b"two\n");
        assert_eq!(io.read_line(), b"three");
        assert_eq!(io.read_line(), b"");
    }

    #[test]
    fn bytesio_truncate_and_clear() {
        let mut io = BytesIO::from_bytes(b"abcdef".to_vec());
        io.seek(3, Whence::Start).unwrap();
        assert_eq!(io.truncate(None), 3);
        assert_eq!(io.getvalue(), b"abc");
        io.clear();
        assert!(io.is_empty());
        assert_eq!(io.tell(), 0);
    }

    #[test]
    fn bytesio_drives_a_frames_codec() {
        let mut io = BytesIO::new();
        Frames.write(&mut io, &b"one".to_vec()).unwrap();
        Frames.write(&mut io, &b"two".to_vec()).unwrap();
        io.seek(0, Whence::Start).unwrap();
        let items: Vec<Vec<u8>> = Frames.stream(io).collect::<Result<_, _>>().unwrap();
        assert_eq!(items, vec![b"one".to_vec(), b"two".to_vec()]);
    }

    #[test]
    fn io_pread_positional_vs_cursor_relative() {
        let mut io = BytesIO::from_bytes(b"0123456789".to_vec());
        io.seek(2, Whence::Start).unwrap();
        let mut buf = [0u8; 4];
        // Positional (Start): reads at offset 6, cursor stays at 2.
        assert_eq!(io.pread(&mut buf, 6, Whence::Start).unwrap(), 4);
        assert_eq!(&buf, b"6789");
        assert_eq!(Seek::stream_position(&io), 2);
        // Cursor-relative (Current): reads from the cursor and advances it.
        let mut at = [0u8; 3];
        assert_eq!(io.pread(&mut at, 0, Whence::Current).unwrap(), 3);
        assert_eq!(&at, b"234");
        assert_eq!(Seek::stream_position(&io), 5);
        // A positional read past the end is short and clamps the fill count.
        let mut tail = [0u8; 4];
        assert_eq!(io.pread(&mut tail, 8, Whence::Start).unwrap(), 2);
        assert_eq!(&tail[..2], b"89");
    }

    #[test]
    fn io_pwrite_positional_vs_cursor_relative() {
        let mut io = BytesIO::from_bytes(b"0123456789".to_vec());
        io.seek(4, Whence::Start).unwrap();
        // Positional write leaves the cursor put.
        assert_eq!(io.pwrite(b"AB", 0, Whence::Start).unwrap(), 2);
        assert_eq!(&io.getvalue()[..2], b"AB");
        assert_eq!(Seek::stream_position(&io), 4);
        // Cursor-relative write advances the cursor.
        assert_eq!(io.pwrite(b"XY", 0, Whence::Current).unwrap(), 2);
        assert_eq!(io.tell(), 6);
        assert_eq!(io.getvalue(), b"AB23XY6789");
        // A read-only handle reports pwrite as unsupported.
        let mut ro = Drip(BytesIO::new());
        assert!(matches!(
            ro.pwrite(b"x", 0, Whence::Start),
            Err(IoError::Unsupported(_))
        ));
    }

    #[test]
    fn io_stats_reports_size() {
        let io = BytesIO::from_bytes(b"abcdef".to_vec());
        assert_eq!(io.stats().unwrap().size(), 6);
        assert_eq!(io.stats().unwrap().mtime(), None);
    }

    #[test]
    fn copy_uses_the_memory_fast_path() {
        let mut src = BytesIO::from_bytes(b"hello world".to_vec());
        src.seek(6, Whence::Start).unwrap();
        let mut dst: Vec<u8> = Vec::new();
        // Copies the tail from the cursor, then leaves the cursor at the end.
        assert_eq!(copy(&mut src, &mut dst).unwrap(), 5);
        assert_eq!(dst, b"world");
        assert_eq!(Seek::stream_position(&src), 11);
    }

    /// A read-only [`Io`] with no `as_slice`, to exercise the streamed (non
    /// zero-copy) fallbacks in `pread` / `copy_to`.
    struct Drip(BytesIO);

    impl ReadBytes for Drip {
        fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
            // Hand out at most one byte at a time, to stress the loops.
            let one = buf.len().min(1);
            self.0.read_bytes(&mut buf[..one])
        }
    }
    impl Seek for Drip {
        fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
            Seek::seek(&mut self.0, offset, whence)
        }
        fn stream_position(&self) -> u64 {
            Seek::stream_position(&self.0)
        }
    }
    impl Io for Drip {
        fn url(&self) -> Url {
            self.0.url()
        }
        fn stats(&self) -> Result<IoStats, IoError> {
            self.0.stats()
        }
        // No `as_slice` override: forces the streamed paths.
    }

    #[test]
    fn copy_and_pread_streamed_fallback() {
        let mut src = Drip(BytesIO::from_bytes(b"streamed bytes".to_vec()));
        // pread via seek/restore on a one-byte-at-a-time reader.
        let mut buf = [0u8; 8];
        assert_eq!(src.pread(&mut buf, 0, Whence::Start).unwrap(), 8);
        assert_eq!(&buf, b"streamed");
        assert_eq!(Seek::stream_position(&src), 0);
        // copy_to via the chunked loop.
        let mut dst: Vec<u8> = Vec::new();
        assert_eq!(copy(&mut src, &mut dst).unwrap(), 14);
        assert_eq!(dst, b"streamed bytes");
    }

    /// A read-only [`Io`] whose reads always error, to check that `pread`
    /// restores the cursor even when a positioned read fails.
    struct Boom {
        position: u64,
    }
    impl ReadBytes for Boom {
        fn read_bytes(&mut self, _buf: &mut [u8]) -> Result<usize, IoError> {
            Err(IoError::Io("boom".to_string()))
        }
    }
    impl Seek for Boom {
        fn seek(&mut self, offset: i64, _whence: Whence) -> Result<u64, IoError> {
            self.position = offset as u64;
            Ok(self.position)
        }
        fn stream_position(&self) -> u64 {
            self.position
        }
    }
    impl Io for Boom {
        fn url(&self) -> Url {
            Url::new("mem", "boom")
        }
        fn stats(&self) -> Result<IoStats, IoError> {
            Ok(IoStats::new(0))
        }
    }

    #[test]
    fn pread_restores_cursor_even_on_error() {
        let mut io = Boom { position: 5 };
        let mut buf = [0u8; 4];
        assert!(io.pread(&mut buf, 0, Whence::Start).is_err());
        // The streaming cursor is back where it started despite the failed read.
        assert_eq!(Seek::stream_position(&io), 5);
    }

    #[test]
    fn bytesio_seek_overflow_is_reported_not_panicked() {
        let mut io = BytesIO::from_bytes(b"abc".to_vec());
        io.seek(2, Whence::Start).unwrap();
        assert!(matches!(
            io.seek(i64::MAX, Whence::Current),
            Err(IoError::Invalid(_))
        ));
    }

    #[test]
    fn bytesio_url_is_mem_scheme_with_address() {
        let io = BytesIO::from_bytes(b"abc".to_vec());
        let url = io.url();
        assert_eq!(url.scheme(), "mem");
        // The host is the buffer's hex address — non-empty.
        assert!(!url.host().is_empty());
    }

    #[test]
    fn frames_lying_length_fails_fast() {
        // A prefix claiming ~4 GiB with no body errors immediately, without
        // reserving gigabytes.
        let bytes = [0xFFu8, 0xFF, 0xFF, 0xFF];
        assert_eq!(Frames.read(&mut &bytes[..]), Err(IoError::UnexpectedEof));
    }

    #[test]
    fn iostats_builders() {
        let when = SystemTime::UNIX_EPOCH;
        let stats = IoStats::new(42)
            .with_mtime(when)
            .with_content_type("text/csv")
            .with_etag("abc123");
        assert_eq!(stats.size(), 42);
        assert_eq!(stats.mtime(), Some(when));
        assert_eq!(stats.content_type(), Some("text/csv"));
        assert_eq!(stats.etag(), Some("abc123"));
    }

    #[test]
    fn io_error_displays() {
        assert_eq!(
            IoError::UnexpectedEof.to_string(),
            "unexpected end of input"
        );
        assert_eq!(IoError::WriteZero.to_string(), "sink accepted no bytes");
        assert_eq!(
            IoError::Invalid("too big".to_string()).to_string(),
            "malformed bytes: too big"
        );
        assert_eq!(
            IoError::NotFound("/x".to_string()).to_string(),
            "resource not found: /x"
        );
    }

    #[test]
    fn read_exact_and_to_end_drain_a_slice() {
        let data = [1u8, 2, 3, 4, 5];
        let mut reader: &[u8] = &data;
        let mut head = [0u8; 2];
        reader.read_exact(&mut head).unwrap();
        assert_eq!(head, [1, 2]);
        let mut rest = Vec::new();
        assert_eq!(reader.read_to_end(&mut rest).unwrap(), 3);
        assert_eq!(rest, vec![3, 4, 5]);
        assert_eq!(reader.read_bytes(&mut head).unwrap(), 0);
        assert_eq!(reader.read_exact(&mut head), Err(IoError::UnexpectedEof));
    }

    #[test]
    fn write_all_appends_to_a_vec() {
        let mut sink: Vec<u8> = Vec::new();
        sink.write_all(b"ab").unwrap();
        sink.write_all(b"cd").unwrap();
        sink.flush().unwrap();
        assert_eq!(sink, b"abcd");
    }

    #[test]
    fn frames_round_trip_one_value() {
        let value = b"payload".to_vec();
        let mut bytes: Vec<u8> = Vec::new();
        Frames.write(&mut bytes, &value).unwrap();
        assert_eq!(bytes, [&[0, 0, 0, 7][..], b"payload"].concat());
        assert_eq!(Frames.read(&mut &bytes[..]).unwrap(), value);
    }

    #[test]
    fn stream_yields_every_frame_then_ends() {
        let mut sink: Vec<u8> = Vec::new();
        for value in [&b"one"[..], b"", b"three"] {
            Frames.write(&mut sink, &value.to_vec()).unwrap();
        }
        let items: Vec<Vec<u8>> = Frames.stream(&sink[..]).collect::<Result<_, _>>().unwrap();
        assert_eq!(items, vec![b"one".to_vec(), Vec::new(), b"three".to_vec()]);
    }

    #[test]
    fn truncated_frame_is_unexpected_eof() {
        let bytes = [0u8, 0, 0, 5, b'h', b'i'];
        assert_eq!(Frames.read(&mut &bytes[..]), Err(IoError::UnexpectedEof));
        assert_eq!(Frames.read(&mut &[0u8, 0][..]), Err(IoError::UnexpectedEof));
    }

    /// A unique scratch path under the system temp dir for the file-backed tests.
    fn temp_file(name: &str) -> String {
        std::env::temp_dir()
            .join(format!("yggdryl_io_{}_{name}", std::process::id()))
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn local_path_reads_seeks_and_stats() {
        let path = temp_file("read");
        LocalPath::write(&path, b"hello world").unwrap();

        let mut io = LocalPath::open(&path).unwrap();
        assert_eq!(io.location(), path);
        assert!(io.exists());
        assert_eq!(io.url().scheme(), "file");
        assert_eq!(io.url().path(), path);
        assert_eq!(io.stats().unwrap().size(), 11);
        assert!(io.stats().unwrap().mtime().is_some());

        // Streamed read advances the cursor; positional pread does not.
        let mut head = [0u8; 5];
        io.read_exact(&mut head).unwrap();
        assert_eq!(&head, b"hello");
        let mut tail = [0u8; 5];
        assert_eq!(io.pread(&mut tail, 6, Whence::Start).unwrap(), 5);
        assert_eq!(&tail, b"world");
        assert_eq!(Seek::stream_position(&io), 5);

        // Zero-copy transfer of the whole mapping into memory.
        io.seek(0, Whence::Start).unwrap();
        let mut dst: Vec<u8> = Vec::new();
        assert_eq!(copy(&mut io, &mut dst).unwrap(), 11);
        assert_eq!(dst, b"hello world");
        assert_eq!(io.as_slice(), Some(&b"hello world"[..]));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn local_path_missing_is_not_found() {
        let err = LocalPath::open("/no/such/yggdryl/path").unwrap_err();
        assert!(matches!(err, IoError::NotFound(_)));
    }

    #[test]
    fn local_path_empty_file() {
        let path = temp_file("empty");
        LocalPath::write(&path, b"").unwrap();
        let mut io = LocalPath::open(&path).unwrap();
        assert_eq!(io.stats().unwrap().size(), 0);
        assert_eq!(io.as_slice(), Some(&[][..]));
        let mut buf = [0u8; 4];
        assert_eq!(io.read_bytes(&mut buf).unwrap(), 0);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn local_path_write_auto_creates_missing_parent_dirs() {
        let base = temp_file("autodir");
        let nested = format!("{base}/a/b/c.bin");
        // The parent directories do not exist yet; the write creates them.
        LocalPath::write(&nested, b"deep").unwrap();
        let mut io = LocalPath::open(&nested).unwrap();
        let mut buf = [0u8; 4];
        assert_eq!(io.pread(&mut buf, 0, Whence::Start).unwrap(), 4);
        assert_eq!(&buf, b"deep");
        // A second write into the now-existing tree still succeeds.
        LocalPath::write(&nested, b"again").unwrap();
        std::fs::remove_dir_all(&base).ok();
    }

    /// A mock [`RemotePath`] over a memory buffer, to check the trait composes as
    /// an [`Io`] handle and carries its remote URL through [`Io::url`].
    struct FakeRemote {
        inner: BytesIO,
    }
    impl ReadBytes for FakeRemote {
        fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
            self.inner.read_bytes(buf)
        }
    }
    impl Seek for FakeRemote {
        fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
            Seek::seek(&mut self.inner, offset, whence)
        }
        fn stream_position(&self) -> u64 {
            Seek::stream_position(&self.inner)
        }
        fn stream_len(&self) -> Option<u64> {
            Seek::stream_len(&self.inner)
        }
    }
    impl Io for FakeRemote {
        fn url(&self) -> Url {
            Url::new("s3", "bucket").with_path("/key")
        }
        fn stats(&self) -> Result<IoStats, IoError> {
            self.inner.stats()
        }
        fn as_slice(&self) -> Option<&[u8]> {
            self.inner.as_slice()
        }
    }
    impl RemotePath for FakeRemote {
        fn exists(&self) -> bool {
            true
        }
    }

    #[test]
    fn remote_path_carries_its_url_and_composes_as_io() {
        let mut remote = FakeRemote {
            inner: BytesIO::from_bytes(b"object".to_vec()),
        };
        assert_eq!(remote.url().scheme(), "s3");
        assert_eq!(remote.url().host(), "bucket");
        assert!(remote.exists());
        // It is a full Io handle: stats and positional read.
        assert_eq!(remote.stats().unwrap().size(), 6);
        let mut buf = [0u8; 6];
        assert_eq!(remote.pread(&mut buf, 0, Whence::Start).unwrap(), 6);
        assert_eq!(&buf, b"object");
    }

    #[cfg(feature = "media")]
    #[test]
    fn local_path_infers_media_type_from_name() {
        let path = format!("{}.csv", temp_file("media"));
        LocalPath::write(&path, b"a,b,c\n1,2,3\n").unwrap();
        let io = LocalPath::open(&path).unwrap();
        let media = io.media_type().expect("csv inferred from extension");
        assert_eq!(media.first().map(|m| m.subtype()), Some("csv"));
        // It is surfaced through stats() too.
        assert!(io.stats().unwrap().media_type().is_some());
        std::fs::remove_file(&path).ok();
    }

    #[cfg(feature = "media")]
    #[test]
    fn bytesio_sniffs_media_type_from_magic() {
        let io = BytesIO::from_bytes(b"\x1f\x8b\x08\x00rest".to_vec());
        let media = io.media_type().expect("gzip magic");
        assert_eq!(media.first().map(|m| m.subtype()), Some("gzip"));
    }
}
