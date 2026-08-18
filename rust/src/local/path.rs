//! One local location, whatever it turns out to be.

use std::sync::Mutex;

use super::File;
use super::Folder;
use crate::generic::Holder;
use crate::io::{IOBase, IOPath};
use crate::{Error, IOKind, MediaType, MimeType, Result, Url};

/// A local location that resolves to the implementation it turns out to need.
///
/// A caller often knows a path without knowing whether it names a directory or
/// a file - a listing entry, a configuration value, a command-line argument.
/// `Path` is that value: it answers [`IOBase::kind`] by looking, and every
/// other operation runs through [`Folder`] or [`File`] accordingly.
///
/// Resolution follows the laziness contract. Construction touches nothing.
/// A read of a location that does not exist yields nothing, and a write creates
/// a file, because a byte write is what distinguishes a leaf from a container.
/// Use [`Self::as_directory`] when the location must become a container.
///
/// ```no_run
/// use yggdryl::io::IOBase;
/// use yggdryl::local::Path;
/// use yggdryl::IOKind;
///
/// # fn main() -> yggdryl::Result<()> {
/// let path = Path::new(std::env::temp_dir())?;
/// assert_eq!(path.kind(), IOKind::Directory);
///
/// // The same type addresses a leaf, and reading a missing one is empty.
/// let leaf = Path::new(std::env::temp_dir().join("yggdryl-absent.arrows"))?;
/// assert_eq!(leaf.kind(), IOKind::Unknown);
/// assert!(leaf.read_all()?.is_empty());
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct Path {
    url: Url,
    /// The implementation this location resolved to, kept so a mapped file's
    /// state survives between calls.
    resolved: Mutex<Option<Resolved>>,
}

/// The specialized implementations a local location can resolve to.
///
/// This is deliberately not [`Holder`]: a `Holder` can hold a `Path`, and a
/// `Path` that could hold a `Holder` would be a type of unbounded size.
#[derive(Debug)]
enum Resolved {
    Directory(Folder),
    File(File),
}

impl Resolved {
    fn as_io(&self) -> &dyn IOBase {
        match self {
            Self::Directory(inner) => inner,
            Self::File(inner) => inner,
        }
    }

    fn as_io_mut(&mut self) -> &mut dyn IOBase {
        match self {
            Self::Directory(inner) => inner,
            Self::File(inner) => inner,
        }
    }
}

impl Path {
    /// Describe a local location without touching it.
    ///
    /// # Errors
    ///
    /// Returns an error only when the path cannot be expressed as a canonical
    /// `file:` URL.
    pub fn new(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Ok(Self {
            url: Url::from_path(path)?,
            resolved: Mutex::new(None),
        })
    }

    /// Describe a local location named by an existing URL.
    ///
    /// # Errors
    ///
    /// Returns an error when the URL is not local.
    pub fn from_url(url: Url) -> Result<Self> {
        url.to_path()?;
        Ok(Self {
            url,
            resolved: Mutex::new(None),
        })
    }

    /// Borrow the described location.
    pub const fn url(&self) -> &Url {
        &self.url
    }

    /// Return the described platform path.
    ///
    /// # Errors
    ///
    /// Returns an error when the URL cannot be expressed as a platform path.
    pub fn path(&self) -> Result<std::path::PathBuf> {
        self.url.to_path()
    }

    /// Return whether the location exists yet.
    pub fn exists(&self) -> bool {
        self.url.exists()
    }

    /// Treat this location as a directory, whether or not it exists yet.
    ///
    /// # Errors
    ///
    /// Returns an error when the URL is not local.
    pub fn as_directory(&self) -> Result<Folder> {
        Folder::from_url(self.url.clone())
    }

    /// Treat this location as a file, whether or not it exists yet.
    ///
    /// # Errors
    ///
    /// Returns an error when the URL is not local, or when this build has no
    /// local leaf implementation.
    pub fn as_file(&self) -> Result<File> {
        File::new(self.path()?)
    }

    /// Build the leaf implementation this build provides.
    fn leaf(&self) -> Result<Resolved> {
        Ok(Resolved::File(self.as_file()?))
    }

    /// Run `read` against the resolved implementation, or report absence.
    fn with_resolved<T>(&self, absent: T, read: impl FnOnce(&dyn IOBase) -> T) -> Result<T> {
        let mut slot = self.resolved.lock().map_err(|_| {
            Error::Io(std::io::Error::other(
                "the resolved handle lock was poisoned",
            ))
        })?;
        if slot.is_none() {
            *slot = match self.kind() {
                IOKind::Directory => Some(Resolved::Directory(self.as_directory()?)),
                IOKind::File => Some(self.leaf()?),
                _ => None,
            };
        }
        Ok(match slot.as_ref() {
            Some(resolved) => read(resolved.as_io()),
            None => absent,
        })
    }

    /// Run `write` against the resolved implementation, creating a leaf when
    /// the location does not exist yet.
    ///
    /// A write is what decides an undecided location: it becomes a file.
    fn with_resolved_mut<T>(&self, write: impl FnOnce(&mut dyn IOBase) -> T) -> Result<T> {
        let mut slot = self.resolved.lock().map_err(|_| {
            Error::Io(std::io::Error::other(
                "the resolved handle lock was poisoned",
            ))
        })?;
        if slot.is_none() {
            *slot = Some(match self.kind() {
                IOKind::Directory => Resolved::Directory(self.as_directory()?),
                _ => self.leaf()?,
            });
        }
        let resolved = slot.as_mut().ok_or_else(|| {
            Error::Io(std::io::Error::other(
                "the resolved handle was not retained",
            ))
        })?;
        Ok(write(resolved.as_io_mut()))
    }
}

/// A local location is the generic role over the file system.
impl IOPath for Path {
    fn path_url(&self) -> &Url {
        &self.url
    }

    fn is_folder(&self) -> bool {
        self.url.is_dir()
    }

    fn is_file(&self) -> bool {
        self.url.is_file()
    }
}

impl IOBase for Path {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.with_resolved(Ok(0), |handle| handle.pread(offset, buffer))?
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.with_resolved_mut(|handle| handle.pwrite(offset, bytes))?
    }

    fn size(&self) -> u64 {
        self.with_resolved(0, |handle| handle.size()).unwrap_or(0)
    }

    fn capacity(&self) -> u64 {
        self.with_resolved(0, |handle| handle.capacity())
            .unwrap_or(0)
    }

    fn reserve(&mut self, capacity: u64) -> Result<()> {
        self.with_resolved_mut(|handle| handle.reserve(capacity))?
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        // Truncating to zero is how a directory is brought into being, so an
        // existing container keeps its meaning here.
        if self.kind().is_container() {
            return self.as_directory()?.truncate(size);
        }
        self.with_resolved_mut(|handle| handle.truncate(size))?
    }

    fn url(&self) -> Option<&Url> {
        Some(&self.url)
    }

    fn media_type(&self) -> &MediaType {
        // A location's type comes from its name, which is stable, so this does
        // not need the resolved implementation.
        static FILE: std::sync::LazyLock<MediaType> =
            std::sync::LazyLock::new(|| MediaType::from(MimeType::FILE));
        static DIRECTORY: std::sync::LazyLock<MediaType> =
            std::sync::LazyLock::new(|| MediaType::from(MimeType::DIRECTORY));
        if self.kind().is_container() {
            return &DIRECTORY;
        }
        &FILE
    }

    fn set_media_type(&mut self, _media_type: MediaType) {
        // A location is named by its path; its type follows from what is there.
    }

    fn kind(&self) -> IOKind {
        self.path_kind()
    }

    fn flush(&mut self) -> Result<()> {
        self.with_resolved_mut(|handle| handle.flush())?
    }

    fn open(&mut self) -> Result<()> {
        self.with_resolved_mut(|handle| handle.open())?
    }

    fn opened(&self) -> bool {
        self.with_resolved(false, |handle| handle.opened())
            .unwrap_or(false)
    }

    fn close(&mut self) -> Result<()> {
        self.with_resolved_mut(|handle| handle.close())?
    }

    fn parent(&self) -> Option<Holder> {
        let parent = self.url.parent()?;
        Self::from_url(parent).ok().map(Holder::Path)
    }

    fn child_by(&self, name: &str) -> Result<Holder> {
        Ok(Holder::Path(Self::from_url(self.url.joinpath(name)?)?))
    }

    fn ls(&self, recursive: bool, include_private: bool) -> Result<Vec<Holder>> {
        if !self.kind().is_container() {
            // A leaf contains nothing; that is not an error.
            return Ok(Vec::new());
        }
        self.as_directory()?.ls(recursive, include_private)
    }
}
