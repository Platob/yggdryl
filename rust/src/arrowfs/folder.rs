//! A foreign-filesystem directory as a container [`IOBase`].

use std::sync::Arc;

use crate::generic::Holder;
use crate::{IOBase, IOFolder, Listing};
use crate::{IOKind, MediaType, Result, Url};

use super::system::{ArrowFileSystem, encoded_relative, filesystem_location};
use super::{File, location_url};

/// A directory on a foreign Arrow filesystem, addressed as a container.
///
/// A directory holds no bytes of its own: [`IOBase::size`] is zero and reads
/// yield nothing. Its purpose is the hierarchy - [`IOBase::ls`],
/// [`IOBase::child_by_path`], and [`IOBase::parent`] - answered through one
/// [`ArrowFileSystem::list`] and [`ArrowFileSystem::file_info`] per ask.
///
/// On an object store a directory is a prefix, so existence here is what the
/// filesystem itself reports: the marker exists or the prefix has entries.
/// Nothing invents marker objects the filesystem would not create.
///
/// Like every handle it is lazy: construction touches nothing, listing a
/// directory that does not exist yields nothing, and [`IOBase::truncate`] to
/// zero is what brings it into being.
#[derive(Clone)]
pub struct Folder {
    filesystem: Arc<dyn ArrowFileSystem>,
    url: Url,
    /// The filesystem-relative spelling of `url`, derived once so every
    /// vtable call names the same path.
    location: String,
}

impl Folder {
    /// Describe a directory on `filesystem` without touching it.
    pub fn new(filesystem: Arc<dyn ArrowFileSystem>, url: Url) -> Self {
        let location = filesystem_location(&url);
        Self {
            filesystem,
            url,
            location,
        }
    }

    /// Describe a directory named by a filesystem-relative location.
    ///
    /// # Errors
    ///
    /// Returns an error when `location` cannot form a canonical URL.
    pub fn from_location(filesystem: Arc<dyn ArrowFileSystem>, location: &str) -> Result<Self> {
        let url = location_url(filesystem.as_ref(), location)?;
        Ok(Self::new(filesystem, url))
    }

    /// Borrow the foreign filesystem this directory lives on.
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

    /// Return whether the directory exists yet.
    pub fn exists(&self) -> bool {
        self.folder_exists()
    }

    /// Create the directory and every missing parent.
    ///
    /// # Errors
    ///
    /// Returns the filesystem's creation failure.
    pub fn create(&self) -> Result<()> {
        self.filesystem.create_dir(&self.location)
    }
}

/// The part of `entry` that lies below `base`, or `None` when it does not.
fn relative_to<'entry>(base: &str, entry: &'entry str) -> Option<&'entry str> {
    let base = base.trim_matches('/');
    let entry = entry.trim_matches('/');
    if base.is_empty() {
        return (!entry.is_empty()).then_some(entry);
    }
    entry
        .strip_prefix(base)?
        .strip_prefix('/')
        .filter(|rest| !rest.is_empty())
}

/// A foreign directory is the container role over an Arrow filesystem.
impl IOFolder for Folder {
    fn folder_url(&self) -> &Url {
        &self.url
    }

    fn folder_exists(&self) -> bool {
        // The marker exists, or the prefix has entries - exactly what the
        // filesystem itself would report for an object-store "directory".
        match self.filesystem.file_info(&self.location) {
            Ok(info) if info.kind == IOKind::Directory => true,
            Ok(info) if info.kind == IOKind::File => false,
            // One entry is enough, and the listing is lazy, so this asks the
            // filesystem for the first one and stops there.
            _ => self
                .filesystem
                .list(&self.location, false)
                .next()
                .is_some_and(|entry| entry.is_ok()),
        }
    }

    fn create_folder(&self) -> Result<()> {
        self.create()
    }

    fn list_folder(&self, recursive: bool, include_private: bool) -> Listing {
        // The vtable's own listing is already lazy and already ordered, so this
        // is a projection of each entry as it arrives and holds nothing.
        let filesystem = self.filesystem.clone();
        let root = self.url.clone();
        let location = self.location.clone();
        Listing::new(
            self.filesystem
                .list(&self.location, recursive)
                .filter_map(move |entry| {
                    let entry = match entry {
                        Ok(entry) => entry,
                        Err(error) => return Some(Err(error)),
                    };
                    let relative = relative_to(&location, &entry.path)?;
                    // A dot-prefixed name is private, and an entry inside a
                    // private directory stays private however deep the listing
                    // went.
                    if !include_private
                        && relative.split('/').any(|segment| segment.starts_with('.'))
                    {
                        return None;
                    }
                    let url = match root.joinpath(&encoded_relative(relative)) {
                        Ok(url) => url,
                        Err(error) => return Some(Err(error)),
                    };
                    Some(Ok(match entry.kind {
                        IOKind::Directory => {
                            Holder::ArrowFolder(Self::new(filesystem.clone(), url))
                        }
                        _ => Holder::ArrowFile(File::new(filesystem.clone(), url)),
                    }))
                }),
        )
    }

    /// Delete the directory marker itself, leaving its entries alone.
    ///
    /// An Arrow filesystem addresses a directory as one more path, so the
    /// vtable's own delete is what removes it and the backend's not-found
    /// answer is the no-op success. An object store that keeps no marker
    /// reports absence, which is the same success.
    fn delete_folder(&mut self) -> Result<()> {
        crate::iobase::skip_absent(
            self.filesystem
                .delete_file(&self.location)
                .map_err(std::io::Error::other),
        )
    }

    /// Refuse a non-recursive delete while entries remain.
    ///
    /// An Arrow filesystem's delete does not distinguish an empty prefix from a
    /// populated one, so the emptiness answer cannot come from the store's own
    /// failure here. The listing is lazy, so it costs the *first entry* and
    /// stops - and the recursive path never asks at all.
    fn folder_remove(&mut self, recursive: bool) -> Result<()> {
        if recursive {
            self.folder_clear()?;
        } else if let Some(entry) = self.list_folder(false, true).next() {
            entry?;
            return Err(crate::iobase::not_empty(&self.url));
        }
        self.delete_folder()
    }
}

impl crate::IOMedia for Folder {
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
        let parent = self.url.parent()?;
        Some(Holder::ArrowFolder(Self::new(
            self.filesystem.clone(),
            parent,
        )))
    }

    fn child_by_path(&self, name: &str) -> Result<Holder> {
        // `name` is URI-path text, not a raw object name: that is what the
        // reference backend resolves, what the trait documents (`.` and `..`
        // behave as `UriPath::joinpath` makes them), and what every generic
        // caller hands over - a generic media read reopens a leaf through
        // `parent().child_by_path(url.file_name())`, and a folder write routes
        // rows by the segments under its root. Encoding here would escape
        // those escapes and address a different object. A caller holding a
        // raw filesystem name reaches it through `from_location`, which is
        // where encoding belongs.
        let url = self.url.joinpath(name)?;
        let child = Self::new(self.filesystem.clone(), url);
        if matches!(
            self.filesystem.file_info(&child.location),
            Ok(info) if info.kind == IOKind::Directory
        ) {
            return Ok(Holder::ArrowFolder(child));
        }
        Ok(Holder::ArrowFile(File::new(
            self.filesystem.clone(),
            child.url,
        )))
    }

    fn ls(&self, recursive: bool, include_private: bool) -> Listing {
        self.folder_ls(recursive, include_private)
    }

    fn kind(&self) -> IOKind {
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

impl std::fmt::Debug for Folder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Folder")
            .field("filesystem", &self.filesystem.type_name())
            .field("url", &self.url)
            .finish()
    }
}
