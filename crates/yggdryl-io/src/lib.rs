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
//! from. Code that reads Parquet should not care whether the source is an
//! in-memory buffer, a memory-mapped file, or an S3 object: it asks the same
//! handle for [`read`](Io::read), [`pread`](Io::pread), [`seek`](Io::seek),
//! [`stats`](Io::stats) and its [`url`](Io::url). New backends implement [`Io`]
//! (and [`Path`] if they name a resource); everything else composes on top.
//!
//! ## Layers
//!
//! - **The handle** — [`Io`] is the one byte-IO trait: every handle has a
//!   [`url`](Io::url) (in-memory ones use `mem://<address>`); it carries a cursor
//!   moved with [`seek`](Io::seek) / [`stream_position`](Io::stream_position), does
//!   the **streamed** read/write with [`read`](Io::read) / [`write`](Io::write)
//!   (advancing the cursor), and **random** access with [`pread`](Io::pread) /
//!   [`pwrite`](Io::pwrite) — a `whence` chooses positional (cursor untouched) versus
//!   cursor-relative ([`Whence::Current`], the streamed case). [`BytesIO`] is the
//!   trivial in-memory backend. It manages storage with [`capacity`](Io::capacity) /
//!   [`reserve_capacity`](Io::reserve_capacity) / [`truncate`](Io::truncate)
//!   (defaulting to `Unsupported` on read-only backends); it carries an access
//!   [`mode`](Io::mode) ([`Mode`]) and an optional [`parent`](Io::parent), and
//!   [`open`](Io::open)s derived handles; it exposes its bytes for **zero-copy**
//!   transfer via [`as_slice`](Io::as_slice), reports [`stats`](Io::stats), and
//!   [`copy_to`](Io::copy_to) another sink with a memory fast path. [`copy`] is
//!   the free-function form.
//! - **Metadata** — [`IoStats`] holds the [`kind`](IoStats::kind)
//!   ([`Missing`](Kind::Missing) / [`File`](Kind::File) /
//!   [`Directory`](Kind::Directory)), `size`, `mtime`, `content_type` and `etag`
//!   eagerly; expensive fields like `media_type` are discovered lazily (only
//!   when asked, then cached) — see [`Io::media_type`] under the `media` feature.
//!   A [`LocalPath`] holds its `url` / `stats` from construction (statted, not
//!   mapped, up front).
//! - **Named resources** — [`Path`]`: Io` is a local, hierarchical location
//!   (its writes auto-create missing parent dirs *lazily*, on failure, never by
//!   probing first); [`LocalPath`] is the filesystem backend, memory-mapping the
//!   file for direct zero-copy access when the `mmap` feature is on.
//!   [`RemotePath`]`: Io` is the URL-addressed cloud sibling (flat keys, no dir
//!   creation); concrete S3 / Azure backends are downstream crates that implement
//!   it — no change to this crate is needed.
//! - **Typed codecs** — [`Codec<T>`] reads/writes/streams values of `T` over any
//!   byte handle (e.g. a `Codec<RecordBatch>`); [`Frames`] is the reference
//!   length-delimited implementation. Byte-stream **compression** (gzip / Zstd /
//!   Snappy) lives in the sibling `yggdryl-compression` crate, which wraps any
//!   handle from here.
//!
//! ## Optional features (off by default; the base build depends only on
//! `yggdryl-url`, for the universal [`Io::url`])
//!
//! - `log` — structured `log` events on the hot paths.
//! - `mmap` — [`LocalPath`] memory-maps files (zero-copy) instead of reading them.
//! - `media` — lazy [`media_type`](Io::media_type) discovery via `yggdryl-media`.
//! - `json` — [`Io::json`] parses a handle's bytes (zero-copy off [`as_slice`](Io::as_slice)).
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

pub use yggdryl_url::Url;

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise (so the crate pulls no `log` dependency by default and pays no
/// runtime cost).
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        log::$level!($($arg)+);
    }};
}

/// Error returned by every [`Io`] and
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

/// Where a [`Io::seek`] offset is measured from, mirroring the `whence` values
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

/// The access mode of an [`Io`] handle.
///
/// [`from_str`](Mode::from_str) parses the named forms (`read` / `write` /
/// `append` / `read_write`) and the Python letters (`r`, `w`, `a`, `x`, with an
/// optional `+` for read-write and ignored `b` / `t` modifiers): e.g. `rb` →
/// [`Read`](Mode::Read), `r+` → [`ReadWrite`](Mode::ReadWrite), `ab` →
/// [`Append`](Mode::Append).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    /// Read only (`r`, `rb`, `read`).
    #[default]
    Read,
    /// Write, truncating any existing content (`w`, `wb`, `x`, `write`).
    Write,
    /// Write, positioned at the end (`a`, `ab`, `append`).
    Append,
    /// Read and write (`r+`, `w+`, `a+`, `rw`, `read_write`).
    ReadWrite,
}

impl Mode {
    /// Parses a mode string, returning [`IoError::Invalid`] on an unknown one.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Mode, IoError> {
        match value.trim() {
            "read" => return Ok(Mode::Read),
            "write" => return Ok(Mode::Write),
            "append" => return Ok(Mode::Append),
            "read_write" | "readwrite" | "rw" => return Ok(Mode::ReadWrite),
            _ => {}
        }
        // Python letters: a single base letter, an optional `+` for read-write,
        // and ignored binary / text (`b` / `t`) modifiers.
        let plus = value.contains('+');
        let base: Vec<char> = value
            .trim()
            .chars()
            .filter(|c| !matches!(c, 'b' | 't' | '+'))
            .collect();
        let mode = match base.as_slice() {
            [b] if plus && matches!(b, 'r' | 'w' | 'a' | 'x') => Mode::ReadWrite,
            ['r'] => Mode::Read,
            ['w'] | ['x'] => Mode::Write,
            ['a'] => Mode::Append,
            _ => return Err(IoError::Invalid(format!("unknown mode {value:?}"))),
        };
        Ok(mode)
    }

    /// The canonical short string (`"r"` / `"w"` / `"a"` / `"r+"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Read => "r",
            Mode::Write => "w",
            Mode::Append => "a",
            Mode::ReadWrite => "r+",
        }
    }

    /// Whether reads are allowed.
    pub fn readable(&self) -> bool {
        matches!(self, Mode::Read | Mode::ReadWrite)
    }

    /// Whether writes are allowed.
    pub fn writable(&self) -> bool {
        !matches!(self, Mode::Read)
    }

    /// Whether writes are positioned at the end (append mode).
    pub fn appends(&self) -> bool {
        matches!(self, Mode::Append)
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a resource is, as reported by [`IoStats::kind`]: absent, a regular file,
/// a directory, or some other filesystem entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Kind {
    /// The resource does not exist (or could not be reached).
    Missing,
    /// A regular file — or an in-memory byte blob such as a [`BytesIO`].
    #[default]
    File,
    /// A directory.
    Directory,
    /// Some other entry (a symlink target, socket, device, …).
    Other,
}

impl Kind {
    /// The lowercase name (`"missing"` / `"file"` / `"directory"` / `"other"`),
    /// used by the bindings and [`Display`](fmt::Display).
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Missing => "missing",
            Kind::File => "file",
            Kind::Directory => "directory",
            Kind::Other => "other",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Lazily-discovered metadata for an [`Io`] handle: cheap fields (`kind`, `size`,
/// `mtime`, `content_type`, `etag`) are filled eagerly by [`Io::stats`], while
/// anything expensive (`media_type`, under the `media` feature) is discovered
/// only on demand — see [`Io::media_type`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IoStats {
    kind: Kind,
    size: u64,
    mtime: Option<SystemTime>,
    content_type: Option<String>,
    etag: Option<String>,
    #[cfg(feature = "media")]
    media_type: Option<yggdryl_media::MediaType>,
}

impl IoStats {
    /// Creates stats for a [`Kind::File`] of `size` bytes, with all other fields
    /// unset.
    pub fn new(size: u64) -> IoStats {
        IoStats {
            size,
            ..IoStats::default()
        }
    }

    /// What the resource is: missing, a file, a directory, or other.
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Whether the resource exists (its [`kind`](IoStats::kind) is not
    /// [`Kind::Missing`]).
    pub fn exists(&self) -> bool {
        self.kind != Kind::Missing
    }

    /// Whether the resource is a regular file (or in-memory blob).
    pub fn is_file(&self) -> bool {
        self.kind == Kind::File
    }

    /// Whether the resource is a directory.
    pub fn is_dir(&self) -> bool {
        self.kind == Kind::Directory
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

    /// Returns a copy with `kind` set.
    pub fn with_kind(mut self, kind: Kind) -> IoStats {
        self.kind = kind;
        self
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
/// cursor / start / end arithmetic lives, shared by [`Io::seek`] and
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

/// The base **byte-IO handle**: the single trait for reading, writing and seeking
/// bytes wherever they live (memory, a local file, a cloud object).
///
/// A handle knows its [`url`](Io::url) and [`stats`](Io::stats), carries a cursor
/// moved with [`seek`](Io::seek), and does **both random and streaming access**
/// through [`pread`](Io::pread) / [`pwrite`](Io::pwrite) — the [`Whence`] picks the
/// mode: [`Whence::Current`] uses and advances the cursor (a *streamed* read or
/// write), while [`Whence::Start`] / [`Whence::End`] are purely positional and
/// leave the cursor untouched (a footer, a column chunk). The convenience
/// [`read`](Io::read) / [`write`](Io::write) are exactly the cursor-relative case,
/// and [`read_to_end`](Io::read_to_end) / [`copy_to`](Io::copy_to) drain from it.
///
/// This is the abstraction Arrow/Parquet-style readers target: implement it once
/// per backend and the same reader works everywhere. Implementors must provide
/// [`url`](Io::url), [`stats`](Io::stats), [`seek`](Io::seek) and
/// [`stream_position`](Io::stream_position). A memory-resident backend then
/// overrides [`as_slice`](Io::as_slice) to unlock the zero-copy [`pread`](Io::pread)
/// / [`copy_to`](Io::copy_to) / [`read_to_end`](Io::read_to_end) fast paths; a
/// streamed backend overrides [`pread`](Io::pread) (and a writable one
/// [`pwrite`](Io::pwrite)).
pub trait Io: fmt::Debug + Send + Sync {
    /// The address of this resource as a [`Url`]. **Every IO has one**: file
    /// backends use `file`, remote ones their store URL, and an in-memory handle
    /// the `mem` scheme with its buffer address (e.g. `mem://7f3c…`).
    fn url(&self) -> Url;

    /// Discovers metadata. Cheap fields are eager; see [`IoStats`].
    fn stats(&self) -> Result<IoStats, IoError>;

    /// Moves the cursor to `offset` relative to `whence`, returning the new
    /// absolute position. Seeking before the start fails with [`IoError::Invalid`];
    /// seeking past the end is allowed (a later write zero-fills the gap).
    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError>;

    /// The current absolute cursor position.
    fn stream_position(&self) -> u64;

    /// The total length in bytes when known cheaply (without I/O), else `None`.
    fn stream_len(&self) -> Option<u64> {
        None
    }

    /// The access mode of this handle. Defaults to [`Mode::Read`]; a handle
    /// produced by [`open`](Io::open) carries the mode it was opened with.
    fn mode(&self) -> Mode {
        Mode::Read
    }

    /// Whether the cursor advances on the Python-style read helpers. Defaults to
    /// `true` (streaming); backends with a togglable cursor override it.
    fn stream(&self) -> bool {
        true
    }

    /// Sets the [`stream`](Io::stream) flag. The default is a no-op (a backend
    /// without a togglable cursor stays streaming).
    fn set_stream(&mut self, stream: bool) {
        let _ = stream;
    }

    /// The handle this one was [`open`](Io::open)ed from, if any — its provenance.
    /// Defaults to `None` (a root handle).
    fn parent(&self) -> Option<&dyn Io> {
        None
    }

    /// Opens a **new** handle from this one, recording `self` as its
    /// [`parent`](Io::parent) and applying `mode` and `stream`. The default is
    /// [`IoError::Unsupported`]; backends that support derived handles (e.g.
    /// [`BytesIO`]) override it.
    fn open(self: Box<Self>, mode: Mode, stream: bool) -> Result<Box<dyn Io>, IoError> {
        let _ = (mode, stream);
        Err(IoError::Unsupported("open".to_string()))
    }

    /// Releases any resources held by this handle (flushing buffers, closing OS
    /// handles, finishing a cloud upload, …). The default is a no-op returning
    /// `Ok(())` — in-memory and memory-mapped backends free their storage on
    /// drop (RAII). It is **idempotent**: calling it more than once is harmless.
    fn close(&mut self) -> Result<(), IoError> {
        Ok(())
    }

    /// Borrows the whole backing buffer when this handle is memory-resident,
    /// enabling zero-copy reads and transfers. Streamed backends return `None`
    /// (the default).
    fn as_slice(&self) -> Option<&[u8]> {
        None
    }

    /// Reads into `buf` from the cursor, advancing it, and returns the count
    /// (short at end of input; `Ok(0)` only once drained). This is the **streamed
    /// read primitive**: a memory-resident handle serves it zero-copy from
    /// [`as_slice`](Io::as_slice), so only a streamed backend (an HTTP body, a
    /// decoder) must override it.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        let count = match self.as_slice() {
            Some(all) => {
                let start = (self.stream_position() as usize).min(all.len());
                let count = buf.len().min(all.len() - start);
                buf[..count].copy_from_slice(&all[start..start + count]);
                count
            }
            None => {
                return Err(IoError::Unsupported(
                    "read on a streamed handle (override read)".to_string(),
                ))
            }
        };
        if count > 0 {
            self.seek(count as i64, Whence::Current)?;
        }
        Ok(count)
    }

    /// Writes `bytes` at the cursor, advancing it, and returns the count — the
    /// **streamed write primitive**. The default is [`IoError::Unsupported`]; a
    /// writable backend overrides it.
    fn write(&mut self, bytes: &[u8]) -> Result<usize, IoError> {
        let _ = bytes;
        Err(IoError::Unsupported(
            "write on a read-only handle (open it for writing)".to_string(),
        ))
    }

    /// Positional read into `buf`, starting at `offset` relative to `whence`, and
    /// returning the count read (short at end of input).
    ///
    /// `whence` selects whether the streaming cursor is used: with
    /// [`Whence::Start`] or [`Whence::End`] the read is purely positional and
    /// **leaves the cursor untouched** (footers, column chunks); with
    /// [`Whence::Current`] the cursor is the base and is advanced by the bytes read
    /// (the same as [`read`](Io::read)). The default serves it zero-copy from
    /// [`as_slice`](Io::as_slice) when memory-resident, else seeks, reads via
    /// [`read`](Io::read), and restores the cursor — so it works over any seekable
    /// streamed backend, and only a non-seekable one (a live HTTP body) overrides it.
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
                match self.read(&mut buf[filled..]) {
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

    /// Fills `buf` completely from the cursor, or fails with
    /// [`IoError::UnexpectedEof`] if the handle drains first.
    fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<(), IoError> {
        while !buf.is_empty() {
            let count = self.read(buf)?;
            if count == 0 {
                return Err(IoError::UnexpectedEof);
            }
            buf = &mut buf[count..];
        }
        Ok(())
    }

    /// Drains from the cursor to the end, appending every byte to `out` and
    /// returning how many were read. A memory-resident handle hands over its tail
    /// in one copy; otherwise it streams in 64 KiB chunks.
    fn read_to_end(&mut self, out: &mut Vec<u8>) -> Result<usize, IoError> {
        if self.as_slice().is_some() {
            let copied = {
                let all = self.as_slice().unwrap();
                let start = (self.stream_position() as usize).min(all.len());
                out.extend_from_slice(&all[start..]);
                all.len() - start
            };
            self.seek(0, Whence::End)?;
            return Ok(copied);
        }
        let mut chunk = [0u8; 64 * 1024];
        let mut total = 0;
        loop {
            let count = self.read(&mut chunk)?;
            if count == 0 {
                return Ok(total);
            }
            out.extend_from_slice(&chunk[..count]);
            total += count;
        }
    }

    /// Writes every byte of `bytes` at the cursor, or fails with
    /// [`IoError::WriteZero`] if the sink stalls before they all land.
    fn write_all(&mut self, mut bytes: &[u8]) -> Result<(), IoError> {
        while !bytes.is_empty() {
            let count = self.write(bytes)?;
            if count == 0 {
                return Err(IoError::WriteZero);
            }
            bytes = &bytes[count..];
        }
        Ok(())
    }

    /// Flushes any buffered bytes to their destination. The default is a no-op,
    /// which suits unbuffered backends like [`BytesIO`].
    fn flush(&mut self) -> Result<(), IoError> {
        Ok(())
    }

    /// The number of bytes this handle can hold before it must reallocate. The
    /// default reports the current length (no spare); a growable backend reports
    /// its real reserved capacity.
    fn capacity(&self) -> usize {
        self.stream_len().unwrap_or(0) as usize
    }

    /// Reserves room for at least `additional` more bytes beyond the current
    /// length, so a run of writes need not reallocate repeatedly. The default is
    /// [`IoError::Unsupported`]; a growable backend overrides it.
    fn reserve_capacity(&mut self, additional: usize) -> Result<(), IoError> {
        let _ = additional;
        Err(IoError::Unsupported("reserve_capacity".to_string()))
    }

    /// Resizes the resource to exactly `size` bytes — dropping the tail when
    /// shrinking, zero-filling when growing — and leaves the cursor where it is.
    /// The default is [`IoError::Unsupported`]; a writable backend overrides it.
    fn truncate(&mut self, size: u64) -> Result<(), IoError> {
        let _ = size;
        Err(IoError::Unsupported("truncate".to_string()))
    }

    /// Copies every byte from the cursor to the end into `dst`, returning the
    /// count. A memory-resident source writes its tail in a single
    /// [`write_all`](Io::write_all) (zero intermediate copies); otherwise it streams
    /// in 64 KiB chunks. See also the free [`copy`] function.
    fn copy_to(&mut self, dst: &mut dyn Io) -> Result<u64, IoError> {
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
            let count = self.read(&mut chunk)?;
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

    /// Parses the handle's full contents as JSON. A memory-resident backend is
    /// parsed **zero-copy** straight off its [`as_slice`](Io::as_slice); any other
    /// backend (e.g. an HTTP stream) is drained once and parsed. Only present
    /// under the `json` feature.
    #[cfg(feature = "json")]
    fn json(&mut self) -> Result<serde_json::Value, IoError> {
        if let Some(all) = self.as_slice() {
            return serde_json::from_slice(all)
                .map_err(|err| IoError::Invalid(format!("json: {err}")));
        }
        let mut buf = Vec::new();
        self.read_to_end(&mut buf)?;
        serde_json::from_slice(&buf).map_err(|err| IoError::Invalid(format!("json: {err}")))
    }
}

/// Copies every byte from `src`'s cursor to the end into `dst`, with `src`'s
/// memory fast path — the free-function form of [`Io::copy_to`], for transferring
/// between two IO implementations (e.g. a [`LocalPath`] into a [`BytesIO`]).
pub fn copy<S: Io + ?Sized>(src: &mut S, dst: &mut dyn Io) -> Result<u64, IoError> {
    src.copy_to(dst)
}

/// A `&mut` to any handle is itself a handle, so a borrowed [`Io`] can be handed
/// to an adapter that takes one by value (e.g. a streaming decoder) without giving
/// up ownership. Every method forwards to the borrowed handle.
impl<T: Io + ?Sized> Io for &mut T {
    fn url(&self) -> Url {
        (**self).url()
    }
    fn stats(&self) -> Result<IoStats, IoError> {
        (**self).stats()
    }
    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        (**self).seek(offset, whence)
    }
    fn stream_position(&self) -> u64 {
        (**self).stream_position()
    }
    fn stream_len(&self) -> Option<u64> {
        (**self).stream_len()
    }
    fn mode(&self) -> Mode {
        (**self).mode()
    }
    fn stream(&self) -> bool {
        (**self).stream()
    }
    fn set_stream(&mut self, stream: bool) {
        (**self).set_stream(stream)
    }
    fn parent(&self) -> Option<&dyn Io> {
        (**self).parent()
    }
    fn close(&mut self) -> Result<(), IoError> {
        (**self).close()
    }
    fn as_slice(&self) -> Option<&[u8]> {
        (**self).as_slice()
    }
    fn pread(&mut self, buf: &mut [u8], offset: i64, whence: Whence) -> Result<usize, IoError> {
        (**self).pread(buf, offset, whence)
    }
    fn pwrite(&mut self, bytes: &[u8], offset: i64, whence: Whence) -> Result<usize, IoError> {
        (**self).pwrite(bytes, offset, whence)
    }
    fn capacity(&self) -> usize {
        (**self).capacity()
    }
    fn reserve_capacity(&mut self, additional: usize) -> Result<(), IoError> {
        (**self).reserve_capacity(additional)
    }
    fn truncate(&mut self, size: u64) -> Result<(), IoError> {
        (**self).truncate(size)
    }
    fn copy_to(&mut self, dst: &mut dyn Io) -> Result<u64, IoError> {
        (**self).copy_to(dst)
    }
    #[cfg(feature = "media")]
    fn media_type(&self) -> Option<yggdryl_media::MediaType> {
        (**self).media_type()
    }
    #[cfg(feature = "json")]
    fn json(&mut self) -> Result<serde_json::Value, IoError> {
        (**self).json()
    }
}

/// A boxed handle is itself a handle, so a `Box<dyn Io>` (an HTTP response body, a
/// decoder) composes anywhere an `Io` is expected. Every method forwards to the
/// boxed handle (except [`open`](Io::open), which needs `Self` by value).
impl<T: Io + ?Sized> Io for Box<T> {
    fn url(&self) -> Url {
        (**self).url()
    }
    fn stats(&self) -> Result<IoStats, IoError> {
        (**self).stats()
    }
    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        (**self).seek(offset, whence)
    }
    fn stream_position(&self) -> u64 {
        (**self).stream_position()
    }
    fn stream_len(&self) -> Option<u64> {
        (**self).stream_len()
    }
    fn mode(&self) -> Mode {
        (**self).mode()
    }
    fn stream(&self) -> bool {
        (**self).stream()
    }
    fn set_stream(&mut self, stream: bool) {
        (**self).set_stream(stream)
    }
    fn parent(&self) -> Option<&dyn Io> {
        (**self).parent()
    }
    fn close(&mut self) -> Result<(), IoError> {
        (**self).close()
    }
    fn as_slice(&self) -> Option<&[u8]> {
        (**self).as_slice()
    }
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        (**self).read(buf)
    }
    fn write(&mut self, bytes: &[u8]) -> Result<usize, IoError> {
        (**self).write(bytes)
    }
    fn pread(&mut self, buf: &mut [u8], offset: i64, whence: Whence) -> Result<usize, IoError> {
        (**self).pread(buf, offset, whence)
    }
    fn pwrite(&mut self, bytes: &[u8], offset: i64, whence: Whence) -> Result<usize, IoError> {
        (**self).pwrite(bytes, offset, whence)
    }
    fn read_to_end(&mut self, out: &mut Vec<u8>) -> Result<usize, IoError> {
        (**self).read_to_end(out)
    }
    fn capacity(&self) -> usize {
        (**self).capacity()
    }
    fn reserve_capacity(&mut self, additional: usize) -> Result<(), IoError> {
        (**self).reserve_capacity(additional)
    }
    fn truncate(&mut self, size: u64) -> Result<(), IoError> {
        (**self).truncate(size)
    }
    fn copy_to(&mut self, dst: &mut dyn Io) -> Result<u64, IoError> {
        (**self).copy_to(dst)
    }
    #[cfg(feature = "media")]
    fn media_type(&self) -> Option<yggdryl_media::MediaType> {
        (**self).media_type()
    }
    #[cfg(feature = "json")]
    fn json(&mut self) -> Result<serde_json::Value, IoError> {
        (**self).json()
    }
}

/// Reads `data[cursor..end]` (clamped) as an owned vector, advancing `cursor` by
/// the count read when `advance` (so a cursor past the end stays put). Shared by
/// the Python-style `read` helpers of [`BytesIO`] and [`LocalPath`], so both
/// behave identically with streaming on or off.
fn read_cursor(data: &[u8], cursor: &mut usize, end: usize, advance: bool) -> Vec<u8> {
    let start = (*cursor).min(data.len());
    let end = end.clamp(start, data.len());
    if advance {
        *cursor += end - start;
    }
    data[start..end].to_vec()
}

/// Reads from `cursor` through the next `\n` (inclusive) or to the end of `data`,
/// advancing `cursor` when `advance`. Shared `read_line` for [`BytesIO`] and
/// [`LocalPath`].
fn read_line_cursor(data: &[u8], cursor: &mut usize, advance: bool) -> Vec<u8> {
    let start = (*cursor).min(data.len());
    let end = data[start..]
        .iter()
        .position(|&byte| byte == b'\n')
        .map_or(data.len(), |offset| start + offset + 1);
    read_cursor(data, cursor, end, advance)
}

/// A simple in-memory byte buffer with a cursor, modelled on Python's
/// `io.BytesIO`: a read/write [`Io`] handle, so it plugs straight into any
/// [`Codec`] and exposes its bytes for zero-copy [`copy`].
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
/// for random access. The lower-level [`Io::read`] / [`Io::write`] primitives
/// always advance, so codecs work whatever the flag.
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
#[derive(Debug)]
pub struct BytesIO {
    buffer: Vec<u8>,
    position: usize,
    stream: bool,
    mode: Mode,
    parent: Option<Box<dyn Io>>,
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

    /// Creates an empty buffer that can hold `capacity` bytes before
    /// reallocating — the preallocating constructor for write-heavy use.
    pub fn with_capacity(capacity: usize) -> BytesIO {
        BytesIO::from_bytes(Vec::with_capacity(capacity))
    }

    /// Wraps existing `bytes`, with the cursor at the start, streaming on, mode
    /// [`Read`](Mode::Read) and no parent.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> BytesIO {
        BytesIO {
            buffer: bytes.into(),
            position: 0,
            stream: true,
            mode: Mode::Read,
            parent: None,
        }
    }

    /// Opens a new in-memory handle derived from this one, recording `self` as
    /// the child's [`parent`](Io::parent) and applying `mode` and `stream`.
    ///
    /// The child's initial state follows `mode`, as in Python `open`:
    /// [`Write`](Mode::Write) starts empty (truncated), [`Append`](Mode::Append)
    /// copies the bytes with the cursor at the end, and [`Read`](Mode::Read) /
    /// [`ReadWrite`](Mode::ReadWrite) copy the bytes with the cursor at the start.
    pub fn open(self, mode: Mode, stream: bool) -> BytesIO {
        log_event!(debug, "BytesIO::open mode={mode} stream={stream}");
        let bytes = self.buffer.clone();
        BytesIO::derived(bytes, mode, stream, Box::new(self))
    }

    /// Builds a derived in-memory handle over `bytes`, applying `mode`
    /// ([`Write`](Mode::Write) truncates, [`Append`](Mode::Append) seeks to the
    /// end, otherwise the cursor starts at `0`) and `stream`, with `parent`
    /// recorded. Shared by [`open`](BytesIO::open) and [`LocalPath`]'s
    /// [`Io::open`].
    fn derived(bytes: Vec<u8>, mode: Mode, stream: bool, parent: Box<dyn Io>) -> BytesIO {
        let buffer = if mode == Mode::Write {
            Vec::new()
        } else {
            bytes
        };
        let position = if mode == Mode::Append {
            buffer.len()
        } else {
            0
        };
        BytesIO {
            buffer,
            position,
            stream,
            mode,
            parent: Some(parent),
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
        read_cursor(&self.buffer, &mut self.position, end, self.stream)
    }

    /// Reads from the cursor through the next `\n` (inclusive), or to the end of
    /// the buffer. Advances the cursor when [`stream`](BytesIO::stream).
    pub fn read_line(&mut self) -> Vec<u8> {
        read_line_cursor(&self.buffer, &mut self.position, self.stream)
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

    /// Resizes the buffer to `size` bytes (the current cursor when `None`),
    /// returning the new length. Shrinks (drops the tail) or grows (zero-fills),
    /// leaving the cursor where it is, as in Python.
    pub fn truncate(&mut self, size: Option<usize>) -> usize {
        let size = size.unwrap_or(self.position);
        log_event!(debug, "BytesIO::truncate to {size}");
        self.resize(size);
        self.buffer.len()
    }

    /// Resizes the backing buffer to exactly `size`, growing (zero-fill) or
    /// shrinking. Shared by [`truncate`](BytesIO::truncate) and [`Io::truncate`].
    fn resize(&mut self, size: usize) {
        if size > self.buffer.len() {
            self.buffer.resize(size, 0);
        } else {
            self.buffer.truncate(size);
        }
    }

    /// Empties the buffer and resets the cursor to `0`.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.position = 0;
    }

    /// No-op flush, present for parity with Python's `io` API.
    pub fn flush(&mut self) {}

    /// Writes `bytes` at the cursor, zero-filling any gap and extending as needed,
    /// moving the cursor past them when `advance`. Shared by [`write`](BytesIO::write)
    /// and [`Io::write`].
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

impl Io for BytesIO {
    /// `mem://<buffer-address>` — the `mem` scheme with the hex address of the
    /// backing bytes, so every in-memory handle still has a stable-shape URL.
    fn url(&self) -> Url {
        Url::new("mem", format!("{:x}", self.buffer.as_ptr() as usize))
    }

    fn stats(&self) -> Result<IoStats, IoError> {
        Ok(IoStats::new(self.buffer.len() as u64))
    }

    /// Delegates to the Python-style [`seek`](BytesIO::seek), widening to `u64`.
    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        BytesIO::seek(self, offset, whence).map(|position| position as u64)
    }

    fn stream_position(&self) -> u64 {
        self.position as u64
    }

    fn stream_len(&self) -> Option<u64> {
        Some(self.buffer.len() as u64)
    }

    fn mode(&self) -> Mode {
        self.mode
    }

    fn stream(&self) -> bool {
        self.stream
    }

    fn set_stream(&mut self, stream: bool) {
        self.stream = stream;
    }

    fn parent(&self) -> Option<&dyn Io> {
        self.parent.as_deref()
    }

    /// Opens a derived in-memory handle (see [`BytesIO::open`]).
    fn open(self: Box<Self>, mode: Mode, stream: bool) -> Result<Box<dyn Io>, IoError> {
        Ok(Box::new((*self).open(mode, stream)))
    }

    fn as_slice(&self) -> Option<&[u8]> {
        Some(&self.buffer)
    }

    /// Writes `bytes` at the cursor, advancing it — the in-memory streamed write.
    fn write(&mut self, bytes: &[u8]) -> Result<usize, IoError> {
        Ok(self.put(bytes, true))
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

    /// The reserved capacity of the backing [`Vec<u8>`].
    fn capacity(&self) -> usize {
        self.buffer.capacity()
    }

    /// Reserves room for `additional` more bytes in the backing buffer.
    fn reserve_capacity(&mut self, additional: usize) -> Result<(), IoError> {
        self.buffer.reserve(additional);
        Ok(())
    }

    /// Resizes the buffer to `size` bytes (grow zero-fills, shrink drops the
    /// tail); the cursor is left where it is.
    fn truncate(&mut self, size: u64) -> Result<(), IoError> {
        self.resize(size as usize);
        Ok(())
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

/// Stats `location` into [`IoStats`] (kind / size / mtime), reporting
/// [`Kind::Missing`] when it is absent or unreachable. Never opens the file.
fn stat_path(location: &str) -> IoStats {
    match fs::metadata(location) {
        Err(_) => IoStats::new(0).with_kind(Kind::Missing),
        Ok(meta) => {
            let kind = if meta.is_dir() {
                Kind::Directory
            } else if meta.is_file() {
                Kind::File
            } else {
                Kind::Other
            };
            let mut stats = IoStats::new(meta.len()).with_kind(kind);
            if let Ok(mtime) = meta.modified() {
                stats = stats.with_mtime(mtime);
            }
            stats
        }
    }
}

/// Loads the bytes of `location` for reading — memory-mapped under the `mmap`
/// feature, otherwise buffered. Non-files (and any failure) yield empty bytes.
fn load_backing(location: &str, stats: &IoStats) -> Backing {
    if !stats.is_file() {
        return Backing::Buffered(Vec::new());
    }
    #[cfg(feature = "mmap")]
    {
        if stats.size() == 0 {
            return Backing::Buffered(Vec::new());
        }
        // SAFETY: we map a file we open read-only here. The standard mmap caveat
        // applies — external truncation while mapped is undefined — and is the
        // caller's responsibility for the paths they hand us.
        match fs::File::open(location).and_then(|file| unsafe { memmap2::Mmap::map(&file) }) {
            Ok(map) => Backing::Mapped(map),
            Err(_) => Backing::Buffered(Vec::new()),
        }
    }
    #[cfg(not(feature = "mmap"))]
    {
        fs::read(location)
            .map(Backing::Buffered)
            .unwrap_or_else(|_| Backing::Buffered(Vec::new()))
    }
}

/// A local filesystem [`Path`] **instance**.
///
/// [`open`](LocalPath::open) stats `location` up front — so [`url`](Io::url),
/// [`stats`](Io::stats), the [`kind`](IoStats::kind) and [`location`](Path::location)
/// are held and ready immediately — and never fails: a missing path yields a
/// handle whose stats report [`Kind::Missing`]. The file's bytes are
/// memory-mapped **lazily**, on the first read (zero-copy under the `mmap`
/// feature, otherwise a buffered read), so a pure stat costs no map.
///
/// The mapped view is read-only; use the instance [`write`](LocalPath::write) to
/// create or overwrite the file.
#[derive(Debug)]
pub struct LocalPath {
    location: String,
    url: Url,
    stats: IoStats,
    backing: std::sync::OnceLock<Backing>,
    position: usize,
    stream: bool,
    #[cfg(feature = "media")]
    media: std::sync::OnceLock<Option<yggdryl_media::MediaType>>,
}

impl LocalPath {
    /// Creates a handle for `location`, statting it up front (so `url` / `stats`
    /// are held) and deferring the memory-map to the first read. Infallible — a
    /// missing path is reported through [`stats`](Io::stats) as [`Kind::Missing`].
    pub fn open(location: impl Into<String>) -> LocalPath {
        let location = location.into();
        log_event!(debug, "LocalPath::open {location:?}");
        let url = Url::new("file", "").with_path(location.clone());
        let stats = stat_path(&location);
        LocalPath {
            location,
            url,
            stats,
            backing: std::sync::OnceLock::new(),
            position: 0,
            stream: true,
            #[cfg(feature = "media")]
            media: std::sync::OnceLock::new(),
        }
    }

    /// The lazily memory-mapped (or buffered) bytes, loaded on first access.
    fn bytes(&self) -> &[u8] {
        self.backing
            .get_or_init(|| load_backing(&self.location, &self.stats))
            .bytes()
    }

    /// Whether the Python-style [`read`](LocalPath::read) / [`read_line`](LocalPath::read_line)
    /// helpers advance the cursor (the same flag as [`BytesIO::stream`]).
    pub fn stream(&self) -> bool {
        self.stream
    }

    /// Sets the [`stream`](LocalPath::stream) flag.
    pub fn set_stream(&mut self, stream: bool) {
        self.stream = stream;
    }

    /// The current cursor position.
    pub fn tell(&self) -> usize {
        self.position
    }

    /// The total number of bytes, regardless of the cursor.
    pub fn len(&self) -> usize {
        self.stats.size() as usize
    }

    /// Whether the file holds no bytes.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The number of bytes between the cursor and the end.
    pub fn remaining(&self) -> usize {
        self.len().saturating_sub(self.position)
    }

    /// Borrows the whole (lazily-mapped) contents, ignoring the cursor.
    pub fn getvalue(&self) -> &[u8] {
        self.bytes()
    }

    /// Reads up to `size` bytes from the cursor, or all remaining bytes when
    /// `size` is `None`. Advances the cursor when [`stream`](LocalPath::stream) —
    /// matching [`BytesIO::read`].
    pub fn read(&mut self, size: Option<usize>) -> Vec<u8> {
        let data = self
            .backing
            .get_or_init(|| load_backing(&self.location, &self.stats));
        let bytes = data.bytes();
        let end = match size {
            Some(n) => self.position.saturating_add(n),
            None => bytes.len(),
        };
        read_cursor(bytes, &mut self.position, end, self.stream)
    }

    /// Reads from the cursor through the next `\n` (inclusive), or to the end.
    /// Advances the cursor when [`stream`](LocalPath::stream).
    pub fn read_line(&mut self) -> Vec<u8> {
        let bytes = self
            .backing
            .get_or_init(|| load_backing(&self.location, &self.stats))
            .bytes();
        read_line_cursor(bytes, &mut self.position, self.stream)
    }

    /// Moves the cursor to `offset` relative to `whence`, returning the new
    /// position — matching [`BytesIO::seek`].
    pub fn seek(&mut self, offset: i64, whence: Whence) -> Result<usize, IoError> {
        Io::seek(self, offset, whence).map(|position| position as usize)
    }

    /// Writes `bytes` to this path, creating or truncating the file and
    /// **auto-creating missing parent directories**.
    ///
    /// This follows the [`Path`] contract: it does *not* stat the directory first.
    /// It writes straight away, and only when the write fails because a parent is
    /// missing does it create the tree once and retry. The held
    /// [`stats`](Io::stats) keep their open-time values; re-[`open`](LocalPath::open)
    /// to observe the new content.
    pub fn write(&self, bytes: &[u8]) -> Result<(), IoError> {
        log_event!(
            info,
            "LocalPath::write {} bytes -> {:?}",
            bytes.len(),
            self.location
        );
        match fs::write(&self.location, bytes) {
            Ok(()) => Ok(()),
            // The directory was missing: create it once, then retry the write.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let parent = std::path::Path::new(&self.location).parent();
                if let Some(parent) = parent.filter(|p| !p.as_os_str().is_empty()) {
                    log_event!(debug, "LocalPath::write creating parent dir {parent:?}");
                    fs::create_dir_all(parent)?;
                }
                fs::write(&self.location, bytes)?;
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }
}

impl Io for LocalPath {
    /// `file://<path>` — held since construction.
    fn url(&self) -> Url {
        self.url.clone()
    }

    /// The metadata held since [`open`](LocalPath::open) (plus a lazily-discovered
    /// `media_type` under the `media` feature).
    fn stats(&self) -> Result<IoStats, IoError> {
        let stats = self.stats.clone();
        #[cfg(feature = "media")]
        if let Some(media_type) = self.media_type() {
            return Ok(stats.with_media_type(media_type));
        }
        Ok(stats)
    }

    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        let len = self.stats.size() as i64;
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
        Some(self.stats.size())
    }

    fn stream(&self) -> bool {
        self.stream
    }

    fn set_stream(&mut self, stream: bool) {
        self.stream = stream;
    }

    /// Opens an in-memory [`BytesIO`] handle over this file's bytes, recording the
    /// path as its [`parent`](Io::parent) and applying `mode` / `stream` — so a
    /// `LocalPath` and a `BytesIO` `open` the same way.
    fn open(self: Box<Self>, mode: Mode, stream: bool) -> Result<Box<dyn Io>, IoError> {
        let bytes = self.as_slice().unwrap_or_default().to_vec();
        Ok(Box::new(BytesIO::derived(bytes, mode, stream, self)))
    }

    fn as_slice(&self) -> Option<&[u8]> {
        Some(
            self.backing
                .get_or_init(|| load_backing(&self.location, &self.stats))
                .bytes(),
        )
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
                    let bytes = self
                        .backing
                        .get_or_init(|| load_backing(&self.location, &self.stats))
                        .bytes();
                    yggdryl_media::MimeType::from_magic(bytes)
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
/// straight out of a [`BytesIO`], a [`LocalPath`], or a cloud path alike — they are
/// all [`Io`], the one byte handle.
///
/// ```
/// use yggdryl_io::{BytesIO, Codec, Frames, Io, Whence};
///
/// let mut io = BytesIO::new();
/// Frames.write(&mut io, &b"payload".to_vec()).unwrap();
/// io.seek(0, Whence::Start).unwrap();
/// assert_eq!(Frames.read(&mut io).unwrap(), b"payload".to_vec());
/// ```
pub trait Codec<T> {
    /// Reads the next value, or `Ok(None)` when the source is cleanly drained at
    /// a value boundary. This is the one read primitive an implementor defines.
    fn read_opt(&self, reader: &mut dyn Io) -> Result<Option<T>, IoError>;

    /// Writes one value to the sink.
    fn write(&self, writer: &mut dyn Io, value: &T) -> Result<(), IoError>;

    /// Reads exactly one value, treating a clean end of input as an error.
    fn read(&self, reader: &mut dyn Io) -> Result<T, IoError> {
        self.read_opt(reader)?.ok_or(IoError::UnexpectedEof)
    }

    /// Returns an iterator that reads values from `reader` until it drains,
    /// yielding `Result<T, IoError>` for each.
    fn stream<R: Io>(&self, reader: R) -> Stream<'_, Self, R, T>
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
    R: Io,
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
/// use yggdryl_io::{BytesIO, Codec, Frames};
///
/// let mut sink = BytesIO::new();
/// Frames.write(&mut sink, &b"hi".to_vec()).unwrap();
/// assert_eq!(sink.getvalue(), &[0, 0, 0, 2, b'h', b'i']);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Frames;

impl Codec<Vec<u8>> for Frames {
    fn read_opt(&self, reader: &mut dyn Io) -> Result<Option<Vec<u8>>, IoError> {
        log_event!(trace, "Frames::read_opt");
        // Read the 4-byte length prefix. Zero bytes at the very start is a clean
        // end of the stream; a partial prefix is a truncated frame.
        let mut prefix = [0u8; 4];
        let mut filled = 0;
        while filled < prefix.len() {
            let count = reader.read(&mut prefix[filled..])?;
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
                let count = reader.read(&mut payload[filled..target])?;
                if count == 0 {
                    return Err(IoError::UnexpectedEof);
                }
                filled += count;
            }
        }
        Ok(Some(payload))
    }

    fn write(&self, writer: &mut dyn Io, value: &Vec<u8>) -> Result<(), IoError> {
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

    #[cfg(feature = "json")]
    #[test]
    fn bytesio_json_parses_zero_copy() {
        let mut io = BytesIO::from_bytes(br#"{"n":42,"xs":[1,2]}"#.to_vec());
        let value = io.json().unwrap();
        assert_eq!(value["n"].as_u64(), Some(42));
        assert_eq!(value["xs"][1].as_u64(), Some(2));
        // Malformed JSON is an error, not a panic.
        let mut bad = BytesIO::from_bytes(b"{not json".to_vec());
        assert!(matches!(bad.json(), Err(IoError::Invalid(_))));
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
        assert_eq!(Io::stream_position(&io), 2);
        // Cursor-relative (Current): reads from the cursor and advances it.
        let mut at = [0u8; 3];
        assert_eq!(io.pread(&mut at, 0, Whence::Current).unwrap(), 3);
        assert_eq!(&at, b"234");
        assert_eq!(Io::stream_position(&io), 5);
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
        assert_eq!(Io::stream_position(&io), 4);
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
    fn io_capacity_reserve_and_truncate() {
        // with_capacity preallocates; capacity tracks the Vec.
        let mut io = BytesIO::with_capacity(64);
        assert!(io.capacity() >= 64);
        io.reserve_capacity(128).unwrap();
        assert!(io.capacity() >= 128);

        // truncate grows (zero-fills) and shrinks via the Io trait.
        io.write(b"abc");
        Io::truncate(&mut io, 5).unwrap();
        assert_eq!(io.getvalue(), b"abc\0\0");
        Io::truncate(&mut io, 2).unwrap();
        assert_eq!(io.getvalue(), b"ab");
        // The inherent (Python-facing) truncate also grows now.
        assert_eq!(io.truncate(Some(4)), 4);
        assert_eq!(io.getvalue(), b"ab\0\0");

        // A read-only handle reports both as unsupported.
        let mut ro = Drip(BytesIO::new());
        assert!(matches!(
            ro.reserve_capacity(8),
            Err(IoError::Unsupported(_))
        ));
        assert!(matches!(
            Io::truncate(&mut ro, 0),
            Err(IoError::Unsupported(_))
        ));
    }

    #[test]
    fn io_stats_reports_size_and_kind() {
        let io = BytesIO::from_bytes(b"abcdef".to_vec());
        let stats = io.stats().unwrap();
        assert_eq!(stats.size(), 6);
        assert_eq!(stats.mtime(), None);
        // An in-memory handle is a File.
        assert_eq!(stats.kind(), Kind::File);
        assert!(stats.is_file());
        assert!(!stats.is_dir());
        assert!(stats.exists());
    }

    #[test]
    fn copy_uses_the_memory_fast_path() {
        let mut src = BytesIO::from_bytes(b"hello world".to_vec());
        src.seek(6, Whence::Start).unwrap();
        let mut dst = BytesIO::new();
        // Copies the tail from the cursor, then leaves the cursor at the end.
        assert_eq!(copy(&mut src, &mut dst).unwrap(), 5);
        assert_eq!(dst.getvalue(), b"world");
        assert_eq!(Io::stream_position(&src), 11);
    }

    #[test]
    fn mode_parses_string_forms() {
        for s in ["r", "rb", "read"] {
            assert_eq!(Mode::from_str(s).unwrap(), Mode::Read);
        }
        for s in ["w", "wb", "x", "write"] {
            assert_eq!(Mode::from_str(s).unwrap(), Mode::Write);
        }
        for s in ["a", "ab", "append"] {
            assert_eq!(Mode::from_str(s).unwrap(), Mode::Append);
        }
        for s in ["r+", "rb+", "w+", "a+", "rw", "read_write"] {
            assert_eq!(Mode::from_str(s).unwrap(), Mode::ReadWrite);
        }
        assert!(Mode::from_str("nope").is_err());
        // Predicates and canonical strings.
        assert!(Mode::Read.readable() && !Mode::Read.writable());
        assert!(Mode::Append.writable() && Mode::Append.appends());
        assert!(Mode::ReadWrite.readable() && Mode::ReadWrite.writable());
        assert_eq!(Mode::ReadWrite.to_string(), "r+");
        assert_eq!(Mode::default(), Mode::Read);
    }

    #[test]
    fn open_sets_parent_mode_and_stream() {
        let parent = BytesIO::from_bytes(b"hello".to_vec());
        // A root handle has no parent.
        assert!(parent.parent().is_none());
        // Default-ish open: read mode keeps the bytes, fresh cursor.
        let child = parent.open(Mode::Read, true);
        assert_eq!(child.mode(), Mode::Read);
        assert!(child.stream());
        assert_eq!(child.getvalue(), b"hello");
        // The parent is recorded as provenance.
        let parent_ref = child.parent().expect("child has a parent");
        assert_eq!(parent_ref.url().scheme(), "mem");

        // Write mode truncates; append positions at the end.
        let writer = BytesIO::from_bytes(b"abc".to_vec()).open(Mode::Write, false);
        assert_eq!(writer.mode(), Mode::Write);
        assert!(!writer.stream());
        assert!(writer.is_empty());

        let appender = BytesIO::from_bytes(b"abc".to_vec()).open(Mode::Append, true);
        assert_eq!(appender.getvalue(), b"abc");
        assert_eq!(appender.tell(), 3);

        // The trait `open` boxes a derived handle.
        let boxed = Box::new(BytesIO::from_bytes(b"x".to_vec()));
        let derived = Io::open(boxed, Mode::ReadWrite, true).unwrap();
        assert_eq!(derived.mode(), Mode::ReadWrite);
        assert!(derived.parent().is_some());

        // A backend without an `open` override (Drip) rejects it as unsupported.
        let ro: Box<dyn Io> = Box::new(Drip(BytesIO::new()));
        assert!(matches!(
            ro.open(Mode::Read, true),
            Err(IoError::Unsupported(_))
        ));
    }

    /// A read-only [`Io`] with no `as_slice`, to exercise the streamed (non
    /// zero-copy) fallbacks in `pread` / `copy_to`.
    #[derive(Debug)]
    struct Drip(BytesIO);

    impl Io for Drip {
        fn url(&self) -> Url {
            self.0.url()
        }
        fn stats(&self) -> Result<IoStats, IoError> {
            self.0.stats()
        }
        fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
            // Hand out at most one byte at a time, to stress the loops.
            let one = buf.len().min(1);
            Io::read(&mut self.0, &mut buf[..one])
        }
        fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
            Io::seek(&mut self.0, offset, whence)
        }
        fn stream_position(&self) -> u64 {
            Io::stream_position(&self.0)
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
        assert_eq!(Io::stream_position(&src), 0);
        // copy_to via the chunked loop.
        let mut dst = BytesIO::new();
        assert_eq!(copy(&mut src, &mut dst).unwrap(), 14);
        assert_eq!(dst.getvalue(), b"streamed bytes");
    }

    /// A read-only [`Io`] whose reads always error, to check that `pread`
    /// restores the cursor even when a positioned read fails.
    #[derive(Debug)]
    struct Boom {
        position: u64,
    }
    impl Io for Boom {
        fn read(&mut self, _buf: &mut [u8]) -> Result<usize, IoError> {
            Err(IoError::Io("boom".to_string()))
        }
        fn seek(&mut self, offset: i64, _whence: Whence) -> Result<u64, IoError> {
            self.position = offset as u64;
            Ok(self.position)
        }
        fn stream_position(&self) -> u64 {
            self.position
        }
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
        assert_eq!(Io::stream_position(&io), 5);
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
        let mut io = BytesIO::from_bytes(vec![0xFFu8, 0xFF, 0xFF, 0xFF]);
        assert_eq!(Frames.read(&mut io), Err(IoError::UnexpectedEof));
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
    fn read_exact_and_to_end_drain_a_handle() {
        let mut reader = BytesIO::from_bytes(vec![1u8, 2, 3, 4, 5]);
        let mut head = [0u8; 2];
        reader.read_exact(&mut head).unwrap();
        assert_eq!(head, [1, 2]);
        let mut rest = Vec::new();
        assert_eq!(reader.read_to_end(&mut rest).unwrap(), 3);
        assert_eq!(rest, vec![3, 4, 5]);
        assert_eq!(Io::read(&mut reader, &mut head).unwrap(), 0);
        assert_eq!(reader.read_exact(&mut head), Err(IoError::UnexpectedEof));
    }

    #[test]
    fn write_all_appends_to_a_handle() {
        let mut sink = BytesIO::new();
        sink.write_all(b"ab").unwrap();
        sink.write_all(b"cd").unwrap();
        Io::flush(&mut sink).unwrap();
        assert_eq!(sink.getvalue(), b"abcd");
    }

    #[test]
    fn frames_round_trip_one_value() {
        let value = b"payload".to_vec();
        let mut io = BytesIO::new();
        Frames.write(&mut io, &value).unwrap();
        assert_eq!(io.getvalue(), [&[0, 0, 0, 7][..], b"payload"].concat());
        io.seek(0, Whence::Start).unwrap();
        assert_eq!(Frames.read(&mut io).unwrap(), value);
    }

    #[test]
    fn stream_yields_every_frame_then_ends() {
        let mut sink = BytesIO::new();
        for value in [&b"one"[..], b"", b"three"] {
            Frames.write(&mut sink, &value.to_vec()).unwrap();
        }
        sink.seek(0, Whence::Start).unwrap();
        let items: Vec<Vec<u8>> = Frames.stream(sink).collect::<Result<_, _>>().unwrap();
        assert_eq!(items, vec![b"one".to_vec(), Vec::new(), b"three".to_vec()]);
    }

    #[test]
    fn truncated_frame_is_unexpected_eof() {
        let mut io = BytesIO::from_bytes(vec![0u8, 0, 0, 5, b'h', b'i']);
        assert_eq!(Frames.read(&mut io), Err(IoError::UnexpectedEof));
        let mut short = BytesIO::from_bytes(vec![0u8, 0]);
        assert_eq!(Frames.read(&mut short), Err(IoError::UnexpectedEof));
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
        LocalPath::open(&path).write(b"hello world").unwrap();

        let mut io = LocalPath::open(&path);
        assert_eq!(io.location(), path);
        assert!(io.exists());
        assert_eq!(io.url().scheme(), "file");
        assert_eq!(io.url().path(), path);
        let stats = io.stats().unwrap();
        assert_eq!(stats.size(), 11);
        assert_eq!(stats.kind(), Kind::File);
        assert!(stats.mtime().is_some());

        // Streamed read advances the cursor; positional pread does not.
        let mut head = [0u8; 5];
        io.read_exact(&mut head).unwrap();
        assert_eq!(&head, b"hello");
        let mut tail = [0u8; 5];
        assert_eq!(io.pread(&mut tail, 6, Whence::Start).unwrap(), 5);
        assert_eq!(&tail, b"world");
        assert_eq!(Io::stream_position(&io), 5);

        // Zero-copy transfer of the whole (lazily-mapped) file into memory.
        io.seek(0, Whence::Start).unwrap();
        let mut dst = BytesIO::new();
        assert_eq!(copy(&mut io, &mut dst).unwrap(), 11);
        assert_eq!(dst.getvalue(), b"hello world");
        assert_eq!(io.as_slice(), Some(&b"hello world"[..]));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn local_path_missing_reports_kind_and_reads_empty() {
        let mut io = LocalPath::open("/no/such/yggdryl/path");
        let stats = io.stats().unwrap();
        assert_eq!(stats.kind(), Kind::Missing);
        assert!(!stats.exists());
        assert!(!io.exists());
        // Reading a missing path yields nothing.
        let mut buf = [0u8; 4];
        assert_eq!(Io::read(&mut io, &mut buf).unwrap(), 0);
    }

    #[test]
    fn local_path_empty_file() {
        let path = temp_file("empty");
        LocalPath::open(&path).write(b"").unwrap();
        let mut io = LocalPath::open(&path);
        assert_eq!(io.stats().unwrap().size(), 0);
        assert_eq!(io.as_slice(), Some(&[][..]));
        let mut buf = [0u8; 4];
        assert_eq!(Io::read(&mut io, &mut buf).unwrap(), 0);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn local_path_stats_classify_the_kind() {
        // A missing path.
        let missing = temp_file("stat_missing");
        assert_eq!(
            LocalPath::open(&missing).stats().unwrap().kind(),
            Kind::Missing
        );

        // A regular file.
        let file = temp_file("stat_file");
        LocalPath::open(&file).write(b"hello").unwrap();
        let file_stats = LocalPath::open(&file).stats().unwrap();
        assert_eq!(file_stats.kind(), Kind::File);
        assert!(file_stats.is_file());
        assert_eq!(file_stats.size(), 5);
        assert!(file_stats.mtime().is_some());

        // A directory.
        let dir = temp_file("stat_dir");
        std::fs::create_dir_all(&dir).unwrap();
        let dir_stats = LocalPath::open(&dir).stats().unwrap();
        assert_eq!(dir_stats.kind(), Kind::Directory);
        assert!(dir_stats.is_dir());

        std::fs::remove_file(&file).ok();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn kind_renders_lowercase_names() {
        assert_eq!(Kind::Missing.as_str(), "missing");
        assert_eq!(Kind::File.to_string(), "file");
        assert_eq!(Kind::Directory.to_string(), "directory");
        assert_eq!(Kind::default(), Kind::File);
    }

    #[test]
    fn local_path_write_auto_creates_missing_parent_dirs() {
        let base = temp_file("autodir");
        let nested = format!("{base}/a/b/c.bin");
        // The parent directories do not exist yet; the write creates them.
        LocalPath::open(&nested).write(b"deep").unwrap();
        let mut io = LocalPath::open(&nested);
        let mut buf = [0u8; 4];
        assert_eq!(io.pread(&mut buf, 0, Whence::Start).unwrap(), 4);
        assert_eq!(&buf, b"deep");
        // A second write into the now-existing tree still succeeds.
        LocalPath::open(&nested).write(b"again").unwrap();
        std::fs::remove_dir_all(&base).ok();
    }

    // --- Parity: BytesIO and LocalPath behave the same for `stream` and `open` ---

    /// Asserts the Python-style `read` respects the `stream` flag the same way for
    /// any handle: streaming advances the cursor, non-streaming leaves it put.
    /// `$make` rebuilds a fresh handle over `b"abcdef"` each time it is used.
    macro_rules! assert_stream_parity {
        ($make:expr) => {{
            // Streaming (the default): each read advances the cursor.
            let mut io = $make;
            assert!(io.stream());
            assert_eq!(io.read(Some(3)), b"abc");
            assert_eq!(io.tell(), 3);
            assert_eq!(io.read(None), b"def");
            assert_eq!(io.tell(), 6);

            // Non-streaming: the cursor stays put, so reads repeat.
            let mut io = $make;
            io.set_stream(false);
            assert!(!io.stream());
            assert_eq!(io.read(Some(3)), b"abc");
            assert_eq!(io.read(Some(3)), b"abc");
            assert_eq!(io.tell(), 0);
            // read_line is governed by the same flag.
            assert_eq!(io.read_line(), b"abcdef");
            assert_eq!(io.tell(), 0);
        }};
    }

    /// Asserts `open` derives a child the same way for any handle: the mode shapes
    /// the bytes (Write truncates, Append seeks to the end, Read keeps them), and
    /// the child carries the `stream` flag and a parent.
    macro_rules! assert_open_parity {
        ($make:expr) => {{
            // Read open: child keeps the bytes, carries stream=false and a parent.
            let child = Io::open(Box::new($make), Mode::Read, false).unwrap();
            assert_eq!(child.mode(), Mode::Read);
            assert!(!child.stream());
            assert_eq!(child.as_slice(), Some(&b"abcdef"[..]));
            assert!(child.parent().is_some());

            // Write open truncates the child.
            let child = Io::open(Box::new($make), Mode::Write, true).unwrap();
            assert_eq!(child.mode(), Mode::Write);
            assert!(child.stream());
            assert_eq!(child.as_slice(), Some(&[][..]));

            // Append open keeps the bytes with the cursor at the end.
            let child = Io::open(Box::new($make), Mode::Append, true).unwrap();
            assert_eq!(child.mode(), Mode::Append);
            assert_eq!(Io::stream_position(&*child), 6);
            assert_eq!(child.as_slice(), Some(&b"abcdef"[..]));
        }};
    }

    #[test]
    fn bytesio_and_localpath_stream_parity() {
        assert_stream_parity!(BytesIO::from_bytes(b"abcdef".to_vec()));

        let path = temp_file("stream_parity");
        LocalPath::open(&path).write(b"abcdef").unwrap();
        assert_stream_parity!(LocalPath::open(&path));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn bytesio_and_localpath_open_parity() {
        assert_open_parity!(BytesIO::from_bytes(b"abcdef".to_vec()));

        let path = temp_file("open_parity");
        LocalPath::open(&path).write(b"abcdef").unwrap();
        assert_open_parity!(LocalPath::open(&path));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn close_is_a_noop_and_idempotent() {
        let mut io = BytesIO::from_bytes(b"abc".to_vec());
        assert!(io.close().is_ok());
        assert!(io.close().is_ok()); // idempotent
        assert_eq!(io.read(Some(3)), b"abc"); // still usable

        // Available through `dyn Io`, and on the default (read-only) backends.
        let mut boxed: Box<dyn Io> = Box::new(BytesIO::new());
        assert!(boxed.close().is_ok());
        let mut drip = Drip(BytesIO::new());
        assert!(drip.close().is_ok());
    }

    /// A mock [`RemotePath`] over a memory buffer, to check the trait composes as
    /// an [`Io`] handle and carries its remote URL through [`Io::url`].
    #[derive(Debug)]
    struct FakeRemote {
        inner: BytesIO,
    }
    impl Io for FakeRemote {
        fn url(&self) -> Url {
            Url::new("s3", "bucket").with_path("/key")
        }
        fn stats(&self) -> Result<IoStats, IoError> {
            self.inner.stats()
        }
        fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
            Io::seek(&mut self.inner, offset, whence)
        }
        fn stream_position(&self) -> u64 {
            Io::stream_position(&self.inner)
        }
        fn stream_len(&self) -> Option<u64> {
            Io::stream_len(&self.inner)
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
        LocalPath::open(&path).write(b"a,b,c\n1,2,3\n").unwrap();
        let io = LocalPath::open(&path);
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
