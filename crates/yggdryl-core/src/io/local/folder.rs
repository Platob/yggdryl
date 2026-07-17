//! [`LocalFolder`] — the auto-created concrete directory of the local family.

use std::path::{Path as StdPath, PathBuf};

use super::{file_err, uri_to_path, LocalChildren, LocalPath, LocalWalk};
use crate::headers::Headers;
use crate::io::memory::IOBase;
use crate::io::{IOKind, IOMode, IoError, Path};
use crate::uri::Uri;

/// An **already-created** local directory — what a lazy [`LocalPath`](super::LocalPath)
/// [sub-instantiates](super::LocalPath::folder) for graph work. Construction
/// **auto-creates** the whole directory tree (`mkdir -p`), so a `LocalFolder` in hand always
/// exists; navigation and discovery ([`ls`](Path::ls) / [`ls_recursive`](Path::ls_recursive))
/// stream children lazily.
///
/// DESIGN: a folder has **no byte stream** — [`byte_size`](IOBase::byte_size) is `0`, reads
/// are empty, the write primitive writes nothing, and a *full* write reports the guided fix
/// (`join_str` a file name and write there instead).
///
/// ```
/// use yggdryl_core::io::local::LocalFolder;
/// use yggdryl_core::io::memory::IOBase;
/// use yggdryl_core::io::Path;
///
/// let dir = std::env::temp_dir().join("yggdryl_localfolder_doc/a/b");
/// let folder = LocalFolder::create_path(&dir).unwrap(); // the whole tree auto-created
/// assert!(folder.is_dir());
///
/// folder.join_str("note.txt").pwrite_utf8(0, "hi");
/// assert_eq!(folder.children().unwrap().len(), 1);
///
/// folder.rmdir().unwrap();
/// # std::fs::remove_dir_all(std::env::temp_dir().join("yggdryl_localfolder_doc")).ok();
/// ```
#[derive(Clone, Debug)]
pub struct LocalFolder {
    path: PathBuf,
    headers: Headers,
}

impl LocalFolder {
    /// Auto-creates the directory tree at `path` (like `mkdir -p`) and returns the handle.
    pub fn create_path(path: impl AsRef<StdPath>) -> Result<LocalFolder, IoError> {
        let path = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&path).map_err(|e| file_err("create", &path, &e))?;
        Ok(LocalFolder {
            path,
            headers: Headers::new(),
        })
    }

    /// Auto-creates the directory tree addressed by `uri`.
    pub fn create_uri(uri: &Uri) -> Result<LocalFolder, IoError> {
        Self::create_path(uri_to_path(uri)?)
    }

    /// The underlying filesystem path.
    pub fn as_std_path(&self) -> &StdPath {
        &self.path
    }

    /// The lazy [`LocalPath`] view of this folder's location.
    pub fn as_path(&self) -> LocalPath {
        LocalPath::from_path(&self.path)
    }
}

/// Handles compare by path (the value identity lives in `uri()`).
impl PartialEq for LocalFolder {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}
impl Eq for LocalFolder {}

impl IOBase for LocalFolder {
    fn byte_size(&self) -> u64 {
        0 // DESIGN: a folder has no byte stream through this API
    }

    fn kind(&self) -> IOKind {
        // Probed: the directory could have been removed behind the handle.
        if self.path.is_dir() {
            IOKind::Directory
        } else {
            IOKind::Missing
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
        IOMode::ReadWrite
    }

    fn pread_byte_array(&self, _offset: u64, _buf: &mut [u8]) -> usize {
        0 // a folder streams no bytes
    }

    fn pwrite_byte_array(&mut self, _offset: u64, _data: &[u8]) -> usize {
        0 // a folder accepts no bytes
    }

    fn pwrite_all(&mut self, _offset: u64, _data: &[u8]) -> Result<(), IoError> {
        Err(IoError::FileIo {
            op: "write",
            path: self.path.to_string_lossy().into_owned(),
            detail: "a folder has no byte stream; join_str a file name and write there".to_string(),
        })
    }
}

impl Path for LocalFolder {
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
        self.as_path().ls()
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
