//! Named byte resources: the [`Path`] / [`RemotePath`] traits and the local
//! filesystem [`LocalPath`] backend.

use std::fs;

use crate::io::{read_cursor, read_line_cursor, BytesIO, Io, IoError, IoStats, Kind, Mode, Whence};
#[allow(unused_imports)]
use crate::log_event;
use crate::Url;

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
/// this trait; nothing in `yggdryl-core` pulls them in.
pub trait RemotePath: Io {
    /// Whether the object currently exists (a metadata / `HEAD` probe).
    fn exists(&self) -> bool;
}

/// How a [`LocalPath`] holds its bytes: a memory map (zero-copy, `mmap` feature)
/// or an eagerly-read buffer.
///
/// Memory-mapping is **disabled on Windows** even with the `mmap` feature: a
/// Windows mapping holds a `user-mapped section` lock on the file, which blocks a
/// later truncate/overwrite ([`std::io::Error`] os 1224) and can hide a fresh
/// write from a subsequent read. Windows therefore always takes the buffered
/// (`fs::read`) path; other platforms map as usual.
#[derive(Debug)]
enum Backing {
    Buffered(Vec<u8>),
    #[cfg(all(feature = "mmap", not(windows)))]
    Mapped(memmap2::Mmap),
}

impl Backing {
    /// The backing bytes, however they are held.
    fn bytes(&self) -> &[u8] {
        match self {
            Backing::Buffered(buffer) => buffer,
            #[cfg(all(feature = "mmap", not(windows)))]
            Backing::Mapped(map) => map,
        }
    }
}

/// The OS-native filesystem path for `location`. URL parsing yields a Windows
/// drive path in the form `/C:/dir/file` (a leading `/` before the drive letter,
/// from `file:///C:/…`), but `/C:/…` is **not** a valid Windows path while `C:/…`
/// is — so on Windows the leading slash before a `X:` drive letter is stripped.
/// On other platforms, and for non-drive paths, the location is returned verbatim.
fn native_location(location: &str) -> &str {
    #[cfg(windows)]
    {
        let bytes = location.as_bytes();
        if bytes.len() >= 3
            && bytes[0] == b'/'
            && bytes[1].is_ascii_alphabetic()
            && bytes[2] == b':'
        {
            return &location[1..];
        }
    }
    location
}

/// Stats `location` into [`IoStats`] (kind / size / mtime), reporting
/// [`Kind::Missing`] when it is absent or unreachable. Never opens the file.
fn stat_path(location: &str) -> IoStats {
    match fs::metadata(native_location(location)) {
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
/// feature (except on Windows, see [`Backing`]), otherwise buffered. Non-files
/// (and any failure) yield empty bytes.
fn load_backing(location: &str, stats: &IoStats) -> Backing {
    if !stats.is_file() {
        return Backing::Buffered(Vec::new());
    }
    #[cfg(all(feature = "mmap", not(windows)))]
    {
        if stats.size() == 0 {
            return Backing::Buffered(Vec::new());
        }
        // SAFETY: we map a file we open read-only here. The standard mmap caveat
        // applies — external truncation while mapped is undefined — and is the
        // caller's responsibility for the paths they hand us.
        match fs::File::open(native_location(location))
            .and_then(|file| unsafe { memmap2::Mmap::map(&file) })
        {
            Ok(map) => Backing::Mapped(map),
            Err(_) => Backing::Buffered(Vec::new()),
        }
    }
    // Buffered path: no `mmap` feature, or on Windows where mapping would lock the
    // file against later writes.
    #[cfg(not(all(feature = "mmap", not(windows))))]
    {
        fs::read(native_location(location))
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
    media: std::sync::OnceLock<Option<crate::MediaType>>,
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

    /// Opens a handle from a parsed [`Uri`], folding a Windows drive-letter
    /// authority back into the filesystem location.
    ///
    /// A `file://C:/dir/file` URL parses the drive `C:` as the **authority** and
    /// `/dir/file` as the path, so opening [`path`](Uri::path) alone would lose the
    /// drive. This rejoins them into `C:/dir/file`. A well-formed `file:///C:/…`
    /// (or a POSIX `file:///path`) has an empty authority and opens
    /// [`path`](Uri::path) unchanged — the leading-slash drive form is handled by
    /// [`native_location`] at the filesystem boundary.
    pub fn from_uri(uri: &crate::Uri) -> LocalPath {
        let path = uri.path();
        let location = match uri.authority() {
            Some(drive)
                if drive.len() == 2
                    && drive.as_bytes()[0].is_ascii_alphabetic()
                    && drive.as_bytes()[1] == b':' =>
            {
                format!("{drive}{path}")
            }
            _ => path.to_string(),
        };
        LocalPath::open(location)
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
        let location = native_location(&self.location);
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
    ///
    /// Note: because the derived handle is an in-memory [`BytesIO`], opening for
    /// [`Read`](Mode::Read), [`Append`](Mode::Append) or [`ReadWrite`](Mode::ReadWrite)
    /// buffers the whole file in memory (O(size) RAM) — only [`Write`](Mode::Write),
    /// which truncates, avoids the copy.
    fn open(self: Box<Self>, mode: Mode, stream: bool) -> Result<Box<dyn Io>, IoError> {
        // `Mode::Write` truncates to empty, so mapping and copying the whole file
        // (potentially multi-GB) into a Vec would be pure waste — only the read /
        // append / read-write modes need the existing bytes.
        let bytes = if mode == Mode::Write {
            Vec::new()
        } else {
            self.as_slice().unwrap_or_default().to_vec()
        };
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
    fn media_type(&self) -> Option<crate::MediaType> {
        self.media
            .get_or_init(|| {
                let by_name = crate::MediaType::from_path(&self.location);
                if !by_name.is_empty() {
                    Some(by_name)
                } else {
                    let bytes = self
                        .backing
                        .get_or_init(|| load_backing(&self.location, &self.stats))
                        .bytes();
                    crate::MimeType::from_magic(bytes).map(|mime| crate::MediaType::new(vec![mime]))
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
        std::path::Path::new(native_location(&self.location)).exists()
    }
}
