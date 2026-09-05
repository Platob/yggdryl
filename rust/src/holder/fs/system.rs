//! The foreign-filesystem vtable and the two reference implementations.
//!
//! [`FileSystem`] is the minimal synchronous surface the three roles in
//! this module need, modeled on Arrow's `FileSystem` API - the contract
//! `pyarrow.fs`, Arrow C++, and Arrow Java already share - so an existing
//! implementation maps onto it method-for-method with no adaptation logic.
//! The trait is plain Rust and adds no dependency, which is why the whole
//! module is unconditional.
//!
//! Two implementations ship in-tree. [`MemoryFileSystem`] holds everything in
//! one map and is the substrate the tests and benchmarks run on;
//! [`LocalFileSystem`] is a thin `std::fs` mapping that proves the vtable
//! against a real OS filesystem. Neither replaces [`crate::holder::local`], whose
//! mapped [`File`](crate::holder::local::File) remains the local backend.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use crate::{Error, IOKind, Result, Scheme, Url};

/// One foreign filesystem: the minimal synchronous surface the three roles
/// need, modeled on Arrow's `FileSystem` API so an existing implementation
/// maps onto it without adaptation logic.
///
/// Implementations report failures as [`crate::Error`]; a transport failure
/// wraps as [`Error::Io`] with its source chain intact, so the foreign message
/// crosses unchanged.
pub trait FileSystem: Send + Sync {
    /// The filesystem's own name for diagnostics (`"s3"`, `"local"`,
    /// `"memory"`).
    fn type_name(&self) -> &str;

    /// What is at `path` right now: kind and size. `NotFound` - reported as
    /// [`IOKind::Unknown`] - is a normal answer, not an error.
    ///
    /// # Errors
    ///
    /// Returns the filesystem's own metadata failure; absence is not one.
    fn file_info(&self, path: &str) -> Result<FileInfo>;

    /// Every entry under `path` (`recursive` descends), one at a time. A
    /// missing directory lists empty.
    ///
    /// The listing is lazy at this seam too: a failure arrives as an entry and
    /// the iterator is fused after it, so a caller that takes three entries
    /// from a prefix of a hundred thousand pays for three. Order is
    /// deterministic: one directory's entries sorted by path, each container
    /// followed immediately by what is under it.
    fn list(&self, path: &str, recursive: bool) -> FileInfos;

    /// Bytes `[offset, offset + buffer.len())` of the file at `path` into
    /// `buffer`; short reads at end-of-file return the short count; a missing
    /// file reads 0 bytes.
    ///
    /// # Errors
    ///
    /// Returns the filesystem's own read failure; absence is not one.
    fn read_range(&self, path: &str, offset: u64, buffer: &mut [u8]) -> Result<usize>;

    /// Replace the file at `path` with exactly `bytes`, creating it and its
    /// parents. Whole-value replacement is the one write shape every Arrow
    /// filesystem supports (object stores have no random write).
    ///
    /// # Errors
    ///
    /// Returns the filesystem's own write failure, or a refusal when `path`
    /// names a directory.
    fn write_full(&self, path: &str, bytes: &[u8]) -> Result<()>;

    /// Create the directory at `path` and its parents; existing is success.
    ///
    /// # Errors
    ///
    /// Returns the filesystem's own creation failure, or a refusal when
    /// `path` names a file.
    fn create_dir(&self, path: &str) -> Result<()>;

    /// Remove the file at `path`; missing is success.
    ///
    /// # Errors
    ///
    /// Returns the filesystem's own removal failure, or a refusal when `path`
    /// names a directory.
    fn delete_file(&self, path: &str) -> Result<()>;
}

/// The entries of one foreign-filesystem listing, yielded one at a time.
///
/// The [`crate::Listing`] of the storage layer, one level down: the same
/// rules - lazy, `Result` per item, fused after the first failure - stated at
/// the seam where a foreign filesystem answers, so a backend never has to build
/// the listing it is describing. One named type, because there is one item kind.
pub struct FileInfos {
    /// The walk still running. `None` once the listing is spent.
    entries: Option<Box<dyn Iterator<Item = Result<FileInfo>> + Send + Sync>>,
}

impl FileInfos {
    /// A listing of nothing, which a missing directory answers with.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            entries: Some(Box::new(std::iter::empty())),
        }
    }

    /// Wrap a walk that is already lazy.
    pub fn new(entries: impl Iterator<Item = Result<FileInfo>> + Send + Sync + 'static) -> Self {
        Self {
            entries: Some(Box::new(entries)),
        }
    }

    /// A listing that reports one failure and then ends.
    #[must_use]
    pub fn failing(error: Error) -> Self {
        Self::new(std::iter::once(Err(error)))
    }
}

impl Iterator for FileInfos {
    type Item = Result<FileInfo>;

    fn next(&mut self) -> Option<Self::Item> {
        let entries = self.entries.as_mut()?;
        match entries.next() {
            Some(Ok(entry)) => Some(Ok(entry)),
            Some(Err(error)) => {
                self.entries = None;
                Some(Err(error))
            }
            None => {
                self.entries = None;
                None
            }
        }
    }
}

impl std::iter::FusedIterator for FileInfos {}

impl std::fmt::Debug for FileInfos {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FileInfos")
            .field("spent", &self.entries.is_none())
            .finish()
    }
}

impl Default for FileInfos {
    fn default() -> Self {
        Self::empty()
    }
}

/// What a foreign filesystem reports about one path.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FileInfo {
    /// Filesystem-relative location, forward slashes.
    pub path: String,
    /// `File` | `Directory` | `Unknown` (not found).
    pub kind: IOKind,
    /// Byte length; `0` unless `kind` is [`IOKind::File`].
    pub size: u64,
}

impl FileInfo {
    /// Describe a file of `size` bytes at `path`.
    pub fn file(path: impl Into<String>, size: u64) -> Self {
        Self {
            path: path.into(),
            kind: IOKind::File,
            size,
        }
    }

    /// Describe a directory at `path`.
    pub fn directory(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            kind: IOKind::Directory,
            size: 0,
        }
    }

    /// Describe a path where nothing exists yet.
    pub fn not_found(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            kind: IOKind::Unknown,
            size: 0,
        }
    }
}

/// Build the canonical identity [`Url`] for a filesystem-relative `location`.
///
/// A full URL (`s3://bucket/key`) passes through as it stands, an absolute
/// path becomes a `file:` URL, and a relative location is placed under the
/// filesystem's own [`FileSystem::type_name`] as its scheme - `"s3"` and
/// `"bucket/key"` spell `s3://bucket/key`. A type name that is not a valid
/// URL scheme falls back to the generic `fs` scheme, so a wrapped or
/// composite filesystem still gets a canonical identity.
///
/// # Errors
///
/// Returns an error when `location` is empty or cannot form a valid URL.
pub fn location_url(filesystem: &dyn FileSystem, location: &str) -> Result<Url> {
    if location.contains("://") {
        return Url::from_str(location);
    }
    if location.starts_with('/') || is_windows_drive(location) {
        return Url::from_path(location);
    }
    if location.is_empty() {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "expected a filesystem location with at least one path segment, got \"\"",
        )));
    }
    let scheme = Scheme::from_str(filesystem.type_name())
        .or_else(|_| Scheme::from_str("fs"))
        .map_err(|error| {
            Error::Io(std::io::Error::other(format!(
                "expected a filesystem type name usable as a URL scheme, got {:?}: {error}",
                filesystem.type_name()
            )))
        })?;
    // The first segment is the authority - which is exactly what an object
    // store means by it, so "bucket/key" spells s3://bucket/key - and the
    // rest is the path. The path is always present, even when empty, because
    // a URL whose path is absent cannot be joined onto: `s3://bucket` admits
    // no child, while `s3://bucket/` is the bucket root.
    let (authority, path) = location.split_once('/').unwrap_or((location, ""));
    Url::from_str(&format!(
        "{}://{}/{}",
        scheme.as_str(),
        encode_component(authority),
        encoded_relative(path)
    ))
}

/// Return whether `location` opens with a Windows drive designator.
fn is_windows_drive(location: &str) -> bool {
    let bytes = location.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_alphabetic) && bytes.get(1) == Some(&b':')
}

/// Percent-encode one path or authority component for a URL.
///
/// A filesystem names an object with arbitrary UTF-8, while a URL component
/// admits only the unreserved and sub-delimiter characters, so the identity a
/// handle carries has to escape the rest. The retained set is exactly URI
/// unreserved plus sub-delims - which keeps `year=2024` spelled the way a
/// Hive layout writes it - and everything else, the separator included,
/// becomes a percent escape.
fn encode_component(component: &str) -> String {
    let mut encoded = String::with_capacity(component.len());
    for byte in component.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'.'
                    | b'_'
                    | b'~'
                    | b'!'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b';'
                    | b'='
            )
        {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push_str(&format!("{byte:02X}"));
        }
    }
    encoded
}

/// Percent-encode a relative path, component by component.
///
/// The separators stay separators - only what sits between them is escaped -
/// so a caller-supplied `sub/../beside.bin` still resolves its dot segments
/// the way [`crate::UriPath::joinpath`] resolves them.
pub(super) fn encoded_relative(relative: &str) -> String {
    relative
        .split('/')
        .map(encode_component)
        .collect::<Vec<_>>()
        .join("/")
}

/// Decode the percent escapes of one URL component back to filesystem text.
///
/// Invalid UTF-8 or a malformed escape leaves the component as it stands,
/// because a location a filesystem gave us round-trips exactly and anything
/// else is better handed over verbatim than silently mangled.
fn decode_component(component: &str) -> String {
    if !component.contains('%') {
        return component.to_owned();
    }
    let bytes = component.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok());
            if let Some(byte) = hex {
                decoded.push(byte);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(decoded).unwrap_or_else(|_| component.to_owned())
}

/// The filesystem-relative spelling of `url` the vtable receives.
///
/// A `file:` URL becomes its platform path with forward slashes - the
/// spelling `pyarrow.fs.LocalFileSystem` and [`LocalFileSystem`] both read -
/// and any other URL joins its authority and path segments, so
/// `s3://bucket/key` hands `bucket/key` to the vtable, exactly the path an
/// S3 filesystem names its objects by.
pub(super) fn filesystem_location(url: &Url) -> String {
    if url.scheme() == &Scheme::FILE {
        if let Ok(path) = url.clone().into_path() {
            return path.to_string_lossy().replace('\\', "/");
        }
    }
    let mut text = String::new();
    if url.scheme() == &Scheme::FILE {
        text.push('/');
    } else {
        text.push_str(&decode_component(url.authority().as_str()));
    }
    for segment in url.path_segments() {
        if !text.ends_with('/') && !text.is_empty() {
            text.push('/');
        }
        // The URL escapes what a URL component cannot spell; the filesystem
        // wants the name it actually gave us.
        text.push_str(&decode_component(segment));
    }
    text
}

/// Report a poisoned filesystem lock without panicking a caller.
fn poisoned() -> Error {
    Error::Io(std::io::Error::other(
        "the memory filesystem lock was poisoned by a panicking writer",
    ))
}

/// The stored entries of one [`MemoryFileSystem`].
#[derive(Debug, Default)]
struct MemoryState {
    files: BTreeMap<String, Vec<u8>>,
    directories: BTreeSet<String>,
}

/// An in-memory [`FileSystem`], the reference "memory" filesystem.
///
/// One map of paths to byte values plus a set of explicitly created
/// directories. As on an object store, a directory is a prefix: a path with
/// entries beneath it reports [`IOKind::Directory`] whether or not
/// [`FileSystem::create_dir`] ever named it, and creating one stores
/// only the marker.
///
/// This is the test and benchmark substrate, and it gives Rust callers a
/// working backend with no foreign runtime in sight.
///
/// ```
/// use yggdryl::holder::fs::{FileSystem, MemoryFileSystem};
/// use yggdryl::IOKind;
///
/// # fn main() -> yggdryl::Result<()> {
/// let filesystem = MemoryFileSystem::new();
/// filesystem.write_full("lake/trades.bin", b"AAPL")?;
///
/// // The file exists, and its prefix is therefore a directory.
/// assert_eq!(filesystem.file_info("lake/trades.bin")?.size, 4);
/// assert_eq!(filesystem.file_info("lake")?.kind, IOKind::Directory);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Default)]
pub struct MemoryFileSystem {
    state: Mutex<MemoryState>,
}

impl MemoryFileSystem {
    /// Create an empty in-memory filesystem.
    pub fn new() -> Self {
        Self::default()
    }

    /// Trim the separators a caller-supplied path may carry.
    fn normalized(path: &str) -> &str {
        path.trim_matches('/')
    }

    /// Return `candidate`'s path relative to `prefix`, when it is beneath it.
    fn beneath<'a>(prefix: &str, candidate: &'a str) -> Option<&'a str> {
        if prefix.is_empty() {
            return (!candidate.is_empty()).then_some(candidate);
        }
        candidate
            .strip_prefix(prefix)?
            .strip_prefix('/')
            .filter(|rest| !rest.is_empty())
    }
}

impl FileSystem for MemoryFileSystem {
    fn type_name(&self) -> &str {
        "memory"
    }

    fn file_info(&self, path: &str) -> Result<FileInfo> {
        let path = Self::normalized(path);
        let state = self.state.lock().map_err(|_| poisoned())?;
        if path.is_empty() {
            // The filesystem root always exists.
            return Ok(FileInfo::directory(path));
        }
        if let Some(bytes) = state.files.get(path) {
            return Ok(FileInfo::file(path, bytes.len() as u64));
        }
        let is_prefix = state
            .directories
            .iter()
            .any(|directory| directory == path || Self::beneath(path, directory).is_some())
            || state
                .files
                .keys()
                .any(|file| Self::beneath(path, file).is_some());
        if is_prefix {
            return Ok(FileInfo::directory(path));
        }
        Ok(FileInfo::not_found(path))
    }

    fn list(&self, path: &str, recursive: bool) -> FileInfos {
        let prefix = Self::normalized(path);
        let Ok(state) = self.state.lock() else {
            return FileInfos::failing(poisoned());
        };
        // A directory is a prefix, so every intermediate prefix of a stored
        // file or marker is itself a listable directory.
        let mut directories: BTreeSet<&str> = BTreeSet::new();
        for stored in state
            .directories
            .iter()
            .map(String::as_str)
            .chain(state.files.keys().map(String::as_str))
        {
            let mut end = stored.len();
            loop {
                let ancestor = &stored[..end];
                if state.files.contains_key(stored) && end == stored.len() {
                    // The file itself is not a directory.
                } else if !ancestor.is_empty() {
                    directories.insert(ancestor);
                }
                match ancestor.rfind('/') {
                    Some(split) => end = split,
                    None => break,
                }
            }
        }
        let mut found = Vec::new();
        for directory in directories {
            let Some(rest) = Self::beneath(prefix, directory) else {
                continue;
            };
            if recursive || !rest.contains('/') {
                found.push(FileInfo::directory(directory));
            }
        }
        for (file, bytes) in &state.files {
            let Some(rest) = Self::beneath(prefix, file) else {
                continue;
            };
            if recursive || !rest.contains('/') {
                found.push(FileInfo::file(file.clone(), bytes.len() as u64));
            }
        }
        found.sort_by(|left, right| left.path.cmp(&right.path));
        // The store *is* memory, so its entry names are already held and this
        // snapshot is bounded by the store itself, not by the listing. Taking
        // it under one lock is also what makes the listing consistent: the
        // guard cannot outlive this call.
        drop(state);
        FileInfos::new(found.into_iter().map(Ok))
    }

    fn read_range(&self, path: &str, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        let path = Self::normalized(path);
        let state = self.state.lock().map_err(|_| poisoned())?;
        let Some(bytes) = state.files.get(path) else {
            // A missing file reads 0 bytes, per the vtable contract.
            return Ok(0);
        };
        let Ok(offset) = usize::try_from(offset) else {
            return Ok(0);
        };
        if offset >= bytes.len() {
            return Ok(0);
        }
        let available = &bytes[offset..];
        let count = available.len().min(buffer.len());
        buffer[..count].copy_from_slice(&available[..count]);
        Ok(count)
    }

    fn write_full(&self, path: &str, bytes: &[u8]) -> Result<()> {
        let path = Self::normalized(path);
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        if path.is_empty() {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "expected a file path to replace, got the filesystem root",
            )));
        }
        if state.directories.contains(path) {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::IsADirectory,
                format!("expected a file path to replace, got the directory {path:?}"),
            )));
        }
        state.files.insert(path.to_owned(), bytes.to_vec());
        Ok(())
    }

    fn create_dir(&self, path: &str) -> Result<()> {
        let path = Self::normalized(path);
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        if path.is_empty() {
            // The root already exists.
            return Ok(());
        }
        if state.files.contains_key(path) {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                format!("expected a directory path to create, got the file {path:?}"),
            )));
        }
        state.directories.insert(path.to_owned());
        Ok(())
    }

    fn delete_file(&self, path: &str) -> Result<()> {
        let path = Self::normalized(path);
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        if state.directories.contains(path) {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::IsADirectory,
                format!("expected a file path to remove, got the directory {path:?}"),
            )));
        }
        // Missing is success, per the vtable contract.
        state.files.remove(path);
        Ok(())
    }
}

/// A thin `std::fs` mapping of the vtable, the reference "local" filesystem.
///
/// It exists to prove [`FileSystem`] against a real OS filesystem and to
/// benchmark the wrapper against [`crate::holder::local::File`]; it does not replace
/// [`crate::holder::local`], whose memory-mapped `File` remains the local backend.
///
/// Publication is atomic: [`FileSystem::write_full`] writes a temporary
/// sibling and renames it into place, so a reader never observes a
/// half-written value.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalFileSystem;

impl LocalFileSystem {
    /// Create the local filesystem mapping; it holds no state of its own.
    pub fn new() -> Self {
        Self
    }
}

/// One local directory's entries, sorted, as a lazy listing.
///
/// `read_dir` is issued without asking whether the directory is there first: a
/// `NotFound` answer *is* "it contains nothing". The entry names are sorted
/// because `read_dir` order is platform-defined and a listing must be
/// deterministic, so one directory's entries are held and nothing else is.
fn level_of(directory: &str) -> FileInfos {
    // Deferred into the first `next`, so constructing a listing touches
    // nothing at all.
    let directory = directory.to_owned();
    FileInfos::new(std::iter::once(()).flat_map(move |()| read_level(&directory)))
}

/// The directory read itself, issued when the listing is first polled.
fn read_level(directory: &str) -> FileInfos {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        // A missing directory lists empty, per the vtable contract.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return FileInfos::empty(),
        Err(error) => return FileInfos::failing(Error::Io(error)),
    };
    let directory = directory.trim_end_matches('/').to_owned();
    let mut found: Vec<FileInfo> = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => return FileInfos::failing(Error::Io(error)),
        };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let child = format!("{directory}/{name}");
        // `DirEntry::metadata` is what the platform already read for the entry
        // on most systems, so the kind costs no extra call.
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => return FileInfos::failing(Error::Io(error)),
        };
        found.push(if metadata.is_dir() {
            FileInfo::directory(child)
        } else {
            FileInfo::file(child, metadata.len())
        });
    }
    found.sort_by(|left, right| left.path.cmp(&right.path));
    FileInfos::new(found.into_iter().map(Ok))
}

/// Distinguishes concurrent temporary files within one process.
static TEMPORARY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl FileSystem for LocalFileSystem {
    fn type_name(&self) -> &str {
        "local"
    }

    fn file_info(&self, path: &str) -> Result<FileInfo> {
        match std::fs::metadata(path) {
            Ok(metadata) if metadata.is_dir() => Ok(FileInfo::directory(path)),
            Ok(metadata) => Ok(FileInfo::file(path, metadata.len())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(FileInfo::not_found(path))
            }
            Err(error) => Err(Error::Io(error)),
        }
    }

    fn list(&self, path: &str, recursive: bool) -> FileInfos {
        let level = level_of(path);
        if !recursive {
            return level;
        }
        // Depth-first, pre-order, holding one directory's sorted entries per
        // open level - the frontier, never the result.
        let mut stack = std::collections::VecDeque::from([level]);
        let mut done = false;
        FileInfos::new(std::iter::from_fn(move || {
            if done {
                return None;
            }
            loop {
                let level = stack.front_mut()?;
                let Some(entry) = level.next() else {
                    stack.pop_front();
                    continue;
                };
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        done = true;
                        return Some(Err(error));
                    }
                };
                if entry.kind == IOKind::Directory {
                    stack.push_front(level_of(&entry.path));
                }
                return Some(Ok(entry));
            }
        }))
    }

    fn read_range(&self, path: &str, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        use std::io::{Read, Seek};

        let mut file = match std::fs::File::open(path) {
            Ok(file) => file,
            // A missing file reads 0 bytes, per the vtable contract.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(Error::Io(error)),
        };
        file.seek(std::io::SeekFrom::Start(offset))
            .map_err(Error::Io)?;
        let mut filled = 0;
        while filled < buffer.len() {
            let read = file.read(&mut buffer[filled..]).map_err(Error::Io)?;
            if read == 0 {
                break;
            }
            filled += read;
        }
        Ok(filled)
    }

    fn write_full(&self, path: &str, bytes: &[u8]) -> Result<()> {
        let target = std::path::Path::new(path);
        // Write beside the target and rename into place, so publication is
        // atomic and a concurrent reader never sees a half-written value.
        let tag = TEMPORARY.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let staged = target.with_file_name(format!(
            ".{}.yggdryl-{}-{tag}",
            target
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            std::process::id()
        ));
        // The staged write *is* the ancestry question: no directory is made in
        // advance. A missing parent is repaired once, and the original write is
        // retried exactly once; a second absence is reported naming what the
        // repair created.
        if let Err(error) = std::fs::write(&staged, bytes) {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(Error::from_io_at(error, "file", path));
            }
            let Some(parent) = target
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            else {
                return Err(Error::from_io_at(error, "file", path));
            };
            std::fs::create_dir_all(parent).map_err(Error::Io)?;
            std::fs::write(&staged, bytes).map_err(|retry| {
                if retry.kind() == std::io::ErrorKind::NotFound {
                    Error::absent(
                        "file",
                        format!("{path} (its parent {} was created)", parent.display()),
                    )
                } else {
                    Error::Io(retry)
                }
            })?;
        }
        std::fs::rename(&staged, target).map_err(|error| {
            let _ = std::fs::remove_file(&staged);
            Error::Io(error)
        })?;
        Ok(())
    }

    fn create_dir(&self, path: &str) -> Result<()> {
        std::fs::create_dir_all(path).map_err(Error::Io)
    }

    fn delete_file(&self, path: &str) -> Result<()> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            // Missing is success, per the vtable contract.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Error::Io(error)),
        }
    }
}
