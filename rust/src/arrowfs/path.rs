//! One foreign-filesystem location, whatever it turns out to be.

use std::sync::{Arc, Mutex};

use crate::generic::Holder;
use crate::io::{IOBase, IOPath};
use crate::{Error, IOKind, MediaType, MimeType, Result, Url};

use super::system::{ArrowFileSystem, filesystem_location};
use super::{File, Folder, location_url};

/// A foreign location that resolves to the implementation it turns out to need.
///
/// A caller often knows a location without knowing whether it names a
/// directory or a file - a listing entry, a configuration value, a
/// command-line argument. `Path` is that value: it answers [`IOBase::kind`]
/// with one [`ArrowFileSystem::file_info`], and every other operation runs
/// through [`Folder`] or [`File`] accordingly.
///
/// Resolution follows the laziness contract. Construction touches nothing.
/// A read of a location that does not exist yields nothing, and a write
/// creates a file, because a byte write is what distinguishes a leaf from a
/// container. Use [`Self::as_directory`] when the location must become a
/// container.
pub struct Path {
    filesystem: Arc<dyn ArrowFileSystem>,
    url: Url,
    /// The filesystem-relative spelling of `url`, derived once so every
    /// vtable call names the same path.
    location: String,
    /// The implementation this location resolved to, kept so a staged file's
    /// pending writes survive between calls.
    resolved: Mutex<Option<Resolved>>,
}

/// The specialized implementations a foreign location can resolve to.
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
    /// Describe a location on `filesystem` without touching it.
    pub fn new(filesystem: Arc<dyn ArrowFileSystem>, url: Url) -> Self {
        let location = filesystem_location(&url);
        Self {
            filesystem,
            url,
            location,
            resolved: Mutex::new(None),
        }
    }

    /// Describe a location named by a filesystem-relative spelling.
    ///
    /// # Errors
    ///
    /// Returns an error when `location` cannot form a canonical URL.
    pub fn from_location(filesystem: Arc<dyn ArrowFileSystem>, location: &str) -> Result<Self> {
        let url = location_url(filesystem.as_ref(), location)?;
        Ok(Self::new(filesystem, url))
    }

    /// Borrow the foreign filesystem this location lives on.
    pub fn filesystem(&self) -> &Arc<dyn ArrowFileSystem> {
        &self.filesystem
    }

    /// Borrow the described location.
    pub const fn url(&self) -> &Url {
        &self.url
    }

    /// Borrow the filesystem-relative spelling the vtable receives.
    pub fn location(&self) -> &str {
        &self.location
    }

    /// Return whether the location exists yet.
    pub fn exists(&self) -> bool {
        self.path_exists()
    }

    /// Treat this location as a directory, whether or not it exists yet.
    pub fn as_directory(&self) -> Folder {
        Folder::new(self.filesystem.clone(), self.url.clone())
    }

    /// Treat this location as a file, whether or not it exists yet.
    pub fn as_file(&self) -> File {
        File::new(self.filesystem.clone(), self.url.clone())
    }

    /// Ask the filesystem what is at this location right now.
    fn probe(&self) -> IOKind {
        match self.filesystem.file_info(&self.location) {
            Ok(info) if info.kind == IOKind::Directory => IOKind::Directory,
            Ok(info) if info.kind == IOKind::File => IOKind::File,
            // A prefix with entries is a directory even when the filesystem
            // reports no marker for it.
            _ => {
                if self
                    .filesystem
                    .list(&self.location, false)
                    .is_ok_and(|entries| !entries.is_empty())
                {
                    IOKind::Directory
                } else {
                    IOKind::Unknown
                }
            }
        }
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
                IOKind::Directory => Some(Resolved::Directory(self.as_directory())),
                IOKind::File => Some(Resolved::File(self.as_file())),
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
                IOKind::Directory => Resolved::Directory(self.as_directory()),
                _ => Resolved::File(self.as_file()),
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

/// A foreign location is the generic role over an Arrow filesystem.
impl IOPath for Path {
    fn path_url(&self) -> &Url {
        &self.url
    }

    fn is_folder(&self) -> bool {
        self.probe() == IOKind::Directory
    }

    fn is_file(&self) -> bool {
        self.probe() == IOKind::File
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
            return self.as_directory().truncate(size);
        }
        self.with_resolved_mut(|handle| handle.truncate(size))?
    }

    fn url(&self) -> Option<&Url> {
        Some(&self.url)
    }

    fn media_type(&self) -> &MediaType {
        // A location's type comes from its name, which is stable, so this
        // does not need the resolved implementation.
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
        // A location is named by its URL; its type follows from what is there.
    }

    fn kind(&self) -> IOKind {
        // A glob answers container before the filesystem is touched.
        if self.url.is_glob() {
            return IOKind::Directory;
        }
        self.probe()
    }

    fn is_atomic(&self) -> bool {
        self.path_is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.path_is_tabular()
    }

    fn flush(&mut self) -> Result<()> {
        self.with_resolved_mut(|handle| handle.flush())?
    }

    fn open(&mut self) -> Result<()> {
        self.with_resolved_mut(|handle| handle.open())?
    }

    fn is_open(&self) -> bool {
        self.with_resolved(false, |handle| handle.is_open())
            .unwrap_or(false)
    }

    fn close(&mut self) -> Result<()> {
        self.with_resolved_mut(|handle| handle.close())?
    }

    fn parent(&self) -> Option<Holder> {
        let parent = self.url.parent()?;
        Some(Holder::ArrowPath(Self::new(
            self.filesystem.clone(),
            parent,
        )))
    }

    fn child_by(&self, name: &str) -> Result<Holder> {
        // `name` is URI-path text, exactly as the reference backend and the
        // trait's own contract resolve it; see `Folder::child_by`.
        Ok(Holder::ArrowPath(Self::new(
            self.filesystem.clone(),
            self.url.joinpath(name)?,
        )))
    }

    fn ls(&self, recursive: bool, include_private: bool) -> Result<Vec<Holder>> {
        if !self.kind().is_container() {
            // A leaf contains nothing; that is not an error.
            return Ok(Vec::new());
        }
        self.as_directory().ls(recursive, include_private)
    }
}

impl std::fmt::Debug for Path {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Path")
            .field("filesystem", &self.filesystem.type_name())
            .field("url", &self.url)
            .finish()
    }
}
