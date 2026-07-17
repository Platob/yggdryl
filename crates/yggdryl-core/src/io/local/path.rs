//! [`LocalPath`] — the **lazy** local-filesystem node: navigation, streamed discovery, CRUD,
//! and auto-creating byte access, without ever touching the disk until asked.

use std::fs::{self, File, OpenOptions};
use std::path::{Path as StdPath, PathBuf};

use super::{file_err, read_at, uri_to_path, write_at_all, LocalFile, LocalFolder};
use crate::headers::Headers;
use crate::io::memory::IOBase;
use crate::io::{IOKind, IOMode, IoError, Path};
use crate::uri::Uri;

/// A lazy handle to a local filesystem node — the family's uniform [`Path`] node type.
///
/// Constructing one **never touches the disk**: existence ([`kind`](IOBase::kind) /
/// [`exists`](IOBase::exists) / [`is_file`](IOBase::is_file) / [`is_dir`](IOBase::is_dir)) is
/// probed per call, reads on a missing node are simply empty, and a **write auto-creates**
/// the missing parent folders and the file itself — callers never pre-flight `mkdir`/`touch`.
///
/// A `LocalPath` opens the file **per operation** (positioned OS reads/writes, no state) —
/// ideal for one-off access and graph walking. For repeated byte access, **sub-instantiate**
/// the optimized concrete types: [`file()`](LocalPath::file) auto-creates and returns a
/// memory-mapped [`LocalFile`], [`folder()`](LocalPath::folder) auto-creates and returns a
/// [`LocalFolder`].
///
/// DESIGN: the path *value* lives in [`uri`](IOBase::uri) (equatable, hashable,
/// serializable there); `LocalPath` itself is a live handle — it is `Clone` and compares by
/// path for convenience, but carries no byte codec.
///
/// ```
/// use yggdryl_core::io::local::LocalPath;
/// use yggdryl_core::io::memory::IOBase;
/// use yggdryl_core::io::Path;
///
/// let root = LocalPath::from_path(std::env::temp_dir().join("yggdryl_localpath_doc"));
/// let mut note = root.join_str("deep/nested/note.txt");
/// assert!(!note.exists()); // lazy: nothing exists yet
///
/// note.pwrite_utf8(0, "hello"); // auto-creates deep/ and deep/nested/ and the file
/// assert!(note.is_file());
/// assert_eq!(note.pread_utf8(0, 5).unwrap(), "hello");
///
/// root.rmdir().unwrap(); // recursive cleanup
/// assert!(!root.exists());
/// ```
#[derive(Clone, Debug)]
pub struct LocalPath {
    path: PathBuf,
    headers: Headers,
    mode: IOMode,
}

impl LocalPath {
    /// A lazy handle for `path` — nothing is touched or created.
    pub fn from_path(path: impl AsRef<StdPath>) -> LocalPath {
        LocalPath {
            path: path.as_ref().to_path_buf(),
            headers: Headers::new(),
            mode: IOMode::ReadWrite,
        }
    }

    /// A lazy handle addressed by `uri` (`file://…` or a plain-path URI).
    pub fn from_uri(uri: &Uri) -> Result<LocalPath, IoError> {
        Ok(Self::from_path(uri_to_path(uri)?))
    }

    /// The underlying filesystem path.
    pub fn as_std_path(&self) -> &StdPath {
        &self.path
    }

    /// Sub-instantiates the **optimized file handle** at this path: auto-creates the missing
    /// parent folders and the file (empty if new), then memory-maps it — the fast handle for
    /// repeated byte access.
    pub fn file(&self) -> Result<LocalFile, IoError> {
        LocalFile::create_path(&self.path)
    }

    /// Sub-instantiates the **folder handle** at this path: auto-creates the directory tree
    /// (like `mkdir -p`) and returns it.
    pub fn folder(&self) -> Result<LocalFolder, IoError> {
        LocalFolder::create_path(&self.path)
    }

    /// Sets the access [`IOMode`] label in place (advisory for a lazy handle; writes check
    /// it before touching the disk).
    pub fn set_mode(&mut self, mode: IOMode) {
        self.mode = mode;
    }

    /// Opens the file for reading, or `None` when nothing readable exists at the path.
    fn open_read(&self) -> Option<File> {
        File::open(&self.path)
            .ok()
            .filter(|f| f.metadata().map(|m| m.is_file()).unwrap_or(false))
    }

    /// Opens (auto-creating parents + file) for writing — the auto-create write path.
    fn open_write(&self) -> Result<File, IoError> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| file_err("create", parent, &e))?;
            }
        }
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.path)
            .map_err(|e| file_err("open", &self.path, &e))
    }
}

/// Handles compare by path (the value identity lives in `uri()`).
impl PartialEq for LocalPath {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}
impl Eq for LocalPath {}

impl IOBase for LocalPath {
    fn byte_size(&self) -> u64 {
        fs::metadata(&self.path)
            .ok()
            .filter(|m| m.is_file())
            .map(|m| m.len())
            .unwrap_or(0)
    }

    fn kind(&self) -> IOKind {
        match fs::metadata(&self.path) {
            Ok(meta) if meta.is_dir() => IOKind::Directory,
            Ok(_) => IOKind::File,
            Err(_) => IOKind::Missing,
        }
    }

    fn uri(&self) -> Uri {
        Uri::from_path(&self.path.to_string_lossy())
    }

    #[inline]
    fn headers(&self) -> &Headers {
        &self.headers
    }

    #[inline]
    fn headers_mut(&mut self) -> &mut Headers {
        &mut self.headers
    }

    #[inline]
    fn mode(&self) -> IOMode {
        self.mode
    }

    fn pread_byte_array(&self, offset: u64, buf: &mut [u8]) -> usize {
        // Lazy: a missing (or directory) node reads as empty.
        let Some(file) = self.open_read() else {
            return 0;
        };
        read_at(&file, offset, buf).unwrap_or(0)
    }

    fn pwrite_byte_array(&mut self, offset: u64, data: &[u8]) -> usize {
        if data.is_empty() || !self.mode.is_writable() {
            return 0;
        }
        // Auto-create: parents + file come into being on the first write.
        let Ok(file) = self.open_write() else {
            return 0;
        };
        match write_at_all(&file, offset, data) {
            Ok(()) => data.len(),
            Err(_) => 0,
        }
    }

    fn pwrite_all(&mut self, offset: u64, data: &[u8]) -> Result<(), IoError> {
        if !self.mode.is_writable() {
            return Err(IoError::FileIo {
                op: "write",
                path: self.path.to_string_lossy().into_owned(),
                detail: "the handle is read-only (IOMode::Read); set_mode(ReadWrite) to write"
                    .to_string(),
            });
        }
        let file = self.open_write()?;
        write_at_all(&file, offset, data).map_err(|e| file_err("write", &self.path, &e))
    }
}

/// The streamed one-level child iterator of a [`LocalPath`] — lazy: entries are produced as
/// the caller pulls. A file or missing node streams nothing.
pub struct LocalChildren {
    read_dir: Option<fs::ReadDir>,
}

impl Iterator for LocalChildren {
    type Item = Result<LocalPath, IoError>;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.read_dir.as_mut()?.next()?;
        Some(
            entry
                .map(|e| LocalPath::from_path(e.path()))
                .map_err(|e| file_err("list", StdPath::new(""), &e)),
        )
    }
}

/// The streamed depth-first recursive walker of a [`LocalPath`] subtree.
pub struct LocalWalk {
    stack: Vec<fs::ReadDir>,
}

impl Iterator for LocalWalk {
    type Item = Result<LocalPath, IoError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let read_dir = self.stack.last_mut()?;
            match read_dir.next() {
                Some(Ok(entry)) => {
                    let path = entry.path();
                    if path.is_dir() {
                        if let Ok(inner) = fs::read_dir(&path) {
                            self.stack.push(inner);
                        }
                    }
                    return Some(Ok(LocalPath::from_path(path)));
                }
                Some(Err(e)) => {
                    return Some(Err(file_err("list", StdPath::new(""), &e)));
                }
                None => {
                    self.stack.pop();
                }
            }
        }
    }
}

impl Path for LocalPath {
    type Node = LocalPath;
    type Children = LocalChildren;
    type Walk = LocalWalk;

    fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn parent(&self) -> Option<LocalPath> {
        self.path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(LocalPath::from_path)
    }

    fn join_str(&self, segment: &str) -> LocalPath {
        LocalPath::from_path(self.path.join(segment))
    }

    fn ls(&self) -> Result<LocalChildren, IoError> {
        // Lazy semantics: a missing node or a file streams nothing; a real listing failure
        // (permissions) is a guided error.
        match fs::read_dir(&self.path) {
            Ok(read_dir) => Ok(LocalChildren {
                read_dir: Some(read_dir),
            }),
            Err(_) if !self.is_dir() => Ok(LocalChildren { read_dir: None }),
            Err(e) => Err(file_err("list", &self.path, &e)),
        }
    }

    fn ls_recursive(&self) -> Result<LocalWalk, IoError> {
        match fs::read_dir(&self.path) {
            Ok(read_dir) => Ok(LocalWalk {
                stack: vec![read_dir],
            }),
            Err(_) if !self.is_dir() => Ok(LocalWalk { stack: Vec::new() }),
            Err(e) => Err(file_err("list", &self.path, &e)),
        }
    }

    fn rm(&self) -> Result<(), IoError> {
        match self.kind() {
            IOKind::Directory => {
                fs::remove_dir_all(&self.path).map_err(|e| file_err("remove", &self.path, &e))
            }
            IOKind::Missing => Ok(()), // already gone — removing is idempotent
            _ => fs::remove_file(&self.path).map_err(|e| file_err("remove", &self.path, &e)),
        }
    }

    fn rmfile(&self) -> Result<(), IoError> {
        match self.kind() {
            IOKind::Directory => Err(IoError::FileIo {
                op: "remove",
                path: self.path.to_string_lossy().into_owned(),
                detail: "the node is a directory; use rmdir (recursive) instead of rmfile"
                    .to_string(),
            }),
            IOKind::Missing => Ok(()),
            _ => fs::remove_file(&self.path).map_err(|e| file_err("remove", &self.path, &e)),
        }
    }

    fn rmdir(&self) -> Result<(), IoError> {
        match self.kind() {
            IOKind::File => Err(IoError::FileIo {
                op: "remove",
                path: self.path.to_string_lossy().into_owned(),
                detail: "the node is a file; use rmfile instead of rmdir".to_string(),
            }),
            IOKind::Missing => Ok(()),
            _ => fs::remove_dir_all(&self.path).map_err(|e| file_err("remove", &self.path, &e)),
        }
    }
}
