//! A local directory as a container [`IOBase`].

use std::path::PathBuf;

use crate::{Error, MediaType, Result, Url};

use crate::generic::Holder;
use crate::io::{IOBase, IOFolder, Listing};

/// A local directory addressed as a container rather than as bytes.
///
/// A directory holds no bytes of its own: [`IOBase::size`] is zero and reads
/// yield nothing. Its purpose is the hierarchy - [`IOBase::ls`],
/// [`IOBase::child_by_path`], and [`IOBase::parent`] - which resolve children as
/// further [`Folder`] values for subdirectories and mapped files for leaves.
///
/// Its whole state is one [`Url`]. The platform path is derived from it on
/// demand through [`Url::into_path`], so there is exactly one spelling of the
/// location and no way for a stored path and URL to disagree.
///
/// Like every handle it is lazy: construction touches nothing, listing a
/// directory that does not exist yields nothing rather than failing, and
/// [`IOBase::truncate`] or a write to a child creates the directory on demand.
///
/// ```
/// use yggdryl::io::IOBase;
/// use yggdryl::local::Folder;
///
/// # fn main() -> yggdryl::Result<()> {
/// let root = Folder::new(std::env::temp_dir())?;
/// assert!(root.is_container());
/// assert_eq!(root.size(), 0);
/// assert_eq!(root.media_type().base(), &yggdryl::MimeType::DIRECTORY);
///
/// // A directory that does not exist lists nothing instead of failing.
/// let missing = root.child_by_path("yggdryl-absent-directory")?;
/// assert_eq!(missing.ls(false, false).count(), 0);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Folder {
    url: Url,
}

impl Folder {
    /// Describe a local directory without touching it.
    ///
    /// # Errors
    ///
    /// Returns an error only when the path cannot be expressed as a canonical
    /// `file:` URL.
    pub fn new(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Ok(Self {
            url: Url::from_path(path)?,
        })
    }

    /// Describe a local directory named by an existing URL.
    ///
    /// # Errors
    ///
    /// Returns an error when the URL is not local, because a directory listing
    /// resolves through the file system.
    pub fn from_url(url: Url) -> Result<Self> {
        // Converting eagerly is the one check worth paying for: it rejects a
        // URL no later call could ever resolve.
        url.clone().into_path()?;
        Ok(Self { url })
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
    pub fn path(&self) -> Result<PathBuf> {
        self.url.clone().into_path()
    }

    /// Return whether the directory exists yet.
    pub fn exists(&self) -> bool {
        self.url.is_dir()
    }

    /// Create the directory and every missing parent.
    ///
    /// # Errors
    ///
    /// Returns the file system's creation failure.
    pub fn create(&self) -> Result<()> {
        std::fs::create_dir_all(self.path()?)?;
        Ok(())
    }

    /// Resolve one entry into the holder that fits it.
    fn hold(url: &Url) -> Result<Holder> {
        if url.is_dir() {
            return Ok(Holder::Folder(Self { url: url.clone() }));
        }
        Holder::file(url.clone().into_path()?)
    }

    /// One directory's entries, sorted, as a lazy listing.
    ///
    /// `read_dir` is issued without asking whether the directory is there
    /// first: a `NotFound` answer *is* "it contains nothing", which is what the
    /// contract says a listing of an absent container yields. The entry names
    /// are sorted, because `read_dir` order is platform-defined and a listing
    /// must be deterministic - so one directory's names are held, and nothing
    /// else is. That bound is the directory's own width, never the tree's.
    fn level(url: &Url, include_private: bool) -> Listing {
        // Deferred into the first `next`, so constructing a listing touches
        // nothing: a glob whose fixed prefix loses never reads a directory
        // under it.
        let url = url.clone();
        Listing::new(
            std::iter::once(()).flat_map(move |()| Self::read_level(&url, include_private)),
        )
    }

    /// The directory read itself, issued when the listing is first polled.
    fn read_level(url: &Url, include_private: bool) -> Listing {
        let path = match url.clone().into_path() {
            Ok(path) => path,
            Err(error) => return Listing::failing(error),
        };
        let read = match std::fs::read_dir(&path) {
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Listing::empty();
            }
            Err(error) => {
                return Listing::failing(Error::from_io_at(error, "directory", url));
            }
        };
        let mut entries: Vec<PathBuf> = Vec::new();
        for entry in read {
            match entry {
                Ok(entry) => entries.push(entry.path()),
                Err(error) => return Listing::failing(Error::Io(error)),
            }
        }
        entries.sort();
        Listing::new(entries.into_iter().filter_map(move |entry| {
            let entry = match Url::from_path(entry) {
                Ok(entry) => entry,
                Err(error) => return Some(Err(error)),
            };
            // A dot-prefixed name is private, and a private directory is not
            // descended into either.
            if !include_private && entry.is_private() {
                return None;
            }
            Some(Self::hold(&entry))
        }))
    }
}

/// A local directory is the container role over the file system.
impl IOFolder for Folder {
    fn folder_url(&self) -> &Url {
        &self.url
    }

    fn folder_exists(&self) -> bool {
        self.url.is_dir()
    }

    fn create_folder(&self) -> Result<()> {
        self.create()
    }

    fn list_folder(&self, recursive: bool, include_private: bool) -> Listing {
        let level = Self::level(&self.url, include_private);
        if recursive {
            // Depth-first, pre-order: an entry is yielded before the subtree
            // under it, and only the frontier is held.
            return level.descending(include_private);
        }
        level
    }

    /// Remove the directory itself, which `remove_dir` refuses when populated.
    ///
    /// Issued unconditionally: `NotFound` is the no-op success and
    /// `DirectoryNotEmpty` is what [`IOFolder::folder_remove`] turns into the
    /// refusal naming the location. Nothing is listed or stat-ed first.
    fn delete_folder(&mut self) -> Result<()> {
        crate::io::skip_absent(std::fs::remove_dir(self.url.clone().into_path()?))
    }

    /// Empty the directory in one call, keeping the directory itself.
    ///
    /// `remove_dir_all` then `create_dir` would change the inode; deleting the
    /// entries keeps the directory the caller already holds. The listing is the
    /// work rather than a probe - an absent directory lists nothing and the
    /// call does nothing, which is the contract.
    fn folder_clear(&mut self) -> Result<()> {
        for child in self.list_folder(false, true) {
            child?.remove(true)?;
        }
        Ok(())
    }

    /// Delete the whole tree in one call when `recursive` allows it.
    ///
    /// `remove_dir_all` is a single walk the platform performs, so a recursive
    /// removal never issues one delete per entry from here.
    fn folder_remove(&mut self, recursive: bool) -> Result<()> {
        if recursive {
            return crate::io::skip_absent(std::fs::remove_dir_all(self.url.clone().into_path()?));
        }
        match self.delete_folder() {
            Err(crate::Error::Io(error))
                if error.kind() == std::io::ErrorKind::DirectoryNotEmpty =>
            {
                Err(crate::io::not_empty(&self.url))
            }
            other => other,
        }
    }
}

impl crate::io::IOMedia for Folder {
    crate::impl_default_iomedia!();
}

impl IOBase for Folder {
    fn pread(&self, _offset: u64, _buffer: &mut [u8]) -> Result<usize> {
        self.folder_pread()
    }

    fn pwrite(&mut self, _offset: u64, bytes: &[u8]) -> Result<usize> {
        self.folder_pwrite(bytes.len())
    }

    fn size(&self) -> u64 {
        0
    }

    fn capacity(&self) -> u64 {
        0
    }

    fn reserve(&mut self, _capacity: u64) -> Result<()> {
        // Reserving space in a container is meaningless but harmless.
        Ok(())
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        // Truncating to zero is the write that brings a directory into being.
        self.folder_truncate(size)
    }

    fn url(&self) -> Option<&Url> {
        Some(&self.url)
    }

    fn media_type(&self) -> &MediaType {
        self.folder_media_type()
    }

    fn set_media_type(&mut self, _media_type: MediaType) {
        // A directory is a directory; it has no content type to declare.
    }

    fn parent(&self) -> Option<Holder> {
        self.url.parent().map(|url| Holder::Folder(Self { url }))
    }

    fn child_by_path(&self, name: &str) -> Result<Holder> {
        // Resolve through the URL so `.` and `..` behave as they do everywhere
        // else in the crate.
        Self::hold(&self.url.joinpath(name)?)
    }

    fn ls(&self, recursive: bool, include_private: bool) -> Listing {
        self.folder_ls(recursive, include_private)
    }

    fn kind(&self) -> crate::IOKind {
        self.folder_kind()
    }

    fn clear(&mut self) -> Result<()> {
        self.folder_clear()
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.folder_remove(recursive)
    }

    fn is_atomic(&self) -> bool {
        self.folder_is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.folder_is_tabular()
    }
}
