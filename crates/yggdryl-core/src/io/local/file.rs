//! [`LocalFile`] — the auto-created, **memory-mapped** concrete file of the local family.

use std::path::{Path as StdPath, PathBuf};

use super::{uri_to_path, LocalChildren, LocalPath, LocalWalk, Mmap};
use crate::headers::Headers;
use crate::io::memory::{cursor_methods, IOBase, IoError, Whence};
use crate::io::{IOKind, IOMode, Path};
use crate::uri::Uri;

/// An **already-created**, memory-mapped local file — what a lazy
/// [`LocalPath`](super::LocalPath) [sub-instantiates](super::LocalPath::file) for repeated,
/// optimized byte access. Construction **auto-creates**: the missing parent folders come
/// into being (`mkdir -p`) and the file is created empty if absent — then it is mapped
/// ([`Mmap`]-backed), so every read/write is a zero-allocation memory access with the same
/// auto-resizing growth as the raw mapping.
///
/// It carries its own cursor (the same stream surface as `Heap`/`Mmap`) and implements the
/// [`Path`] graph contract (its `ls` streams nothing — a file has no children; `rm`/`rmfile`
/// delegate to the path).
///
/// ```
/// use yggdryl_core::io::local::LocalFile;
/// use yggdryl_core::io::memory::IOBase;
///
/// let path = std::env::temp_dir().join("yggdryl_localfile_doc/inner/data.bin");
/// let mut file = LocalFile::create_path(&path).unwrap(); // parents auto-created
/// file.write_utf8("mapped");
/// assert_eq!(file.pread_utf8(0, 6).unwrap(), "mapped");
/// assert!(file.is_file());
/// drop(file);
/// # std::fs::remove_dir_all(std::env::temp_dir().join("yggdryl_localfile_doc")).ok();
/// ```
#[derive(Debug)]
pub struct LocalFile {
    map: Mmap,
    /// The file's own cursor — independent of the inner mapping's.
    position: u64,
}

impl LocalFile {
    /// Auto-creates (parents + empty file if absent) and maps the file at `path`.
    pub fn create_path(path: impl AsRef<StdPath>) -> Result<LocalFile, IoError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| super::file_err("create", parent, &e))?;
            }
        }
        Ok(LocalFile {
            map: Mmap::create_path(path)?,
            position: 0,
        })
    }

    /// Auto-creates and maps the file addressed by `uri`.
    pub fn create_uri(uri: &Uri) -> Result<LocalFile, IoError> {
        Self::create_path(uri_to_path(uri)?)
    }

    /// Opens an **existing** file read-write (no auto-create) — errors if it is missing.
    pub fn open_path(path: impl AsRef<StdPath>) -> Result<LocalFile, IoError> {
        Ok(LocalFile {
            map: Mmap::open_path(path)?,
            position: 0,
        })
    }

    /// Opens an existing file addressed by `uri`.
    pub fn open_uri(uri: &Uri) -> Result<LocalFile, IoError> {
        Self::open_path(uri_to_path(uri)?)
    }

    /// The underlying filesystem path.
    pub fn as_std_path(&self) -> &StdPath {
        self.map.path()
    }

    /// Flushes the mapped bytes to disk — see [`Mmap::flush`].
    pub fn flush(&self) -> Result<(), IoError> {
        self.map.flush()
    }

    /// The lazy [`LocalPath`] view of this file's location.
    pub fn as_path(&self) -> LocalPath {
        LocalPath::from_path(self.map.path())
    }

    cursor_methods!();
}

impl IOBase for LocalFile {
    #[inline]
    fn byte_size(&self) -> u64 {
        self.map.byte_size()
    }

    #[inline]
    fn capacity(&self) -> u64 {
        self.map.capacity()
    }

    fn reserve(&mut self, additional: u64) {
        self.map.reserve(additional);
    }

    fn reserve_exact(&mut self, additional: u64) {
        self.map.reserve_exact(additional);
    }

    fn try_reserve(&mut self, additional: u64) -> Result<(), IoError> {
        self.map.try_reserve(additional)
    }

    fn try_reserve_exact(&mut self, additional: u64) -> Result<(), IoError> {
        self.map.try_reserve_exact(additional)
    }

    fn shrink_to_fit(&mut self) {
        self.map.shrink_to_fit();
    }

    fn shrink_to(&mut self, min_capacity: u64) {
        self.map.shrink_to(min_capacity);
    }

    fn uri(&self) -> Uri {
        self.map.uri()
    }

    #[inline]
    fn headers(&self) -> &Headers {
        self.map.headers()
    }

    #[inline]
    fn headers_mut(&mut self) -> &mut Headers {
        self.map.headers_mut()
    }

    #[inline]
    fn mode(&self) -> IOMode {
        self.map.mode()
    }

    #[inline]
    fn kind(&self) -> IOKind {
        IOKind::File
    }

    #[inline]
    fn pread_byte_array(&self, offset: u64, buf: &mut [u8]) -> usize {
        self.map.pread_byte_array(offset, buf)
    }

    #[inline]
    fn pwrite_byte_array(&mut self, offset: u64, data: &[u8]) -> usize {
        self.map.pwrite_byte_array(offset, data)
    }

    fn pwrite_all(&mut self, offset: u64, data: &[u8]) -> Result<(), IoError> {
        self.map.pwrite_all(offset, data)
    }
}

impl Path for LocalFile {
    type Node = LocalPath;
    type Children = LocalChildren;
    type Walk = LocalWalk;

    fn name(&self) -> String {
        self.as_path().name()
    }

    fn parent(&self) -> Option<LocalPath> {
        self.as_path().parent()
    }

    fn join_str(&self, segment: &str) -> LocalPath {
        self.as_path().join_str(segment)
    }

    fn ls(&self) -> Result<LocalChildren, IoError> {
        self.as_path().ls() // a file streams nothing
    }

    fn ls_recursive(&self) -> Result<LocalWalk, IoError> {
        self.as_path().ls_recursive()
    }

    fn rm(&self) -> Result<(), IoError> {
        self.as_path().rm()
    }

    fn rmfile(&self) -> Result<(), IoError> {
        self.as_path().rmfile()
    }

    fn rmdir(&self) -> Result<(), IoError> {
        self.as_path().rmdir()
    }
}

// Suppress an unused-import lint pathway on PathBuf in some cfg combinations.
#[allow(unused)]
type _PathBufUse = PathBuf;
