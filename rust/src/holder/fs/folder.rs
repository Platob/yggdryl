//! A directory over one bound Arrow filesystem location.

use crate::holder::Holder;
use crate::{IOBase, IOFolder, IOKind, Listing, MediaType, Result, Url};

use super::{BoundLocation, File, FileSelector, FileSystem, Path};

/// A directory whose lifecycle and listing are supplied by its filesystem.
#[derive(Clone)]
pub struct Folder {
    bound: BoundLocation,
}

impl Folder {
    /// Bind a known directory location without touching the filesystem.
    pub const fn new(bound: BoundLocation) -> Self {
        Self { bound }
    }

    /// Bind an injected raw filesystem path.
    pub fn from_path(
        filesystem: std::sync::Arc<dyn FileSystem>,
        path: impl Into<String>,
        uri: Option<String>,
    ) -> Result<Self> {
        BoundLocation::new(filesystem, path, uri).map(Self::new)
    }

    /// Borrow all bound-location facts.
    pub const fn bound(&self) -> &BoundLocation {
        &self.bound
    }

    /// Borrow the exact filesystem instance.
    pub fn filesystem(&self) -> &std::sync::Arc<dyn FileSystem> {
        self.bound.filesystem()
    }

    /// Borrow the exact opaque filesystem path.
    pub fn path(&self) -> &str {
        self.bound.path()
    }

    /// Borrow the safe diagnostic URL.
    pub const fn url(&self) -> &Url {
        self.bound.diagnostic_url()
    }

    /// Create this directory with an explicit recursive policy.
    pub fn create(&self, recursive: bool) -> Result<()> {
        self.filesystem().create_dir(self.path(), recursive)
    }

    /// Delete every filesystem-root child from an explicitly root-bound handle.
    pub fn delete_root_dir_contents(&self) -> Result<()> {
        if !self.path().is_empty() {
            return Err(crate::Error::unsupported(
                "delete_root_dir_contents from a non-root bound location",
                self.filesystem().type_name(),
            ));
        }
        self.filesystem().delete_root_dir_contents()
    }
}

fn relative_to<'path>(base: &str, path: &'path str) -> Option<&'path str> {
    crate::iobase::hierarchy::raw_relative(base, path).filter(|relative| !relative.is_empty())
}

impl IOFolder for Folder {
    fn folder_url(&self) -> &Url {
        self.url()
    }

    fn folder_exists(&self) -> bool {
        self.filesystem()
            .file_info(self.path())
            .is_ok_and(|info| info.kind == IOKind::Directory)
    }

    fn create_folder(&self) -> Result<()> {
        self.create(true)
    }

    fn list_folder(&self, recursive: bool, include_private: bool) -> Listing {
        let root = self.bound.clone();
        let base = self.path().to_owned();
        let selector = FileSelector::new(&base, recursive, true);
        Listing::new(self.filesystem().list(&selector).filter_map(move |entry| {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => return Some(Err(error)),
            };
            let relative = relative_to(&base, &entry.path)?;
            if !include_private && relative.split('/').any(|part| part.starts_with('.')) {
                return None;
            }
            let bound = match root.listed(entry.path) {
                Ok(bound) => bound,
                Err(error) => return Some(Err(error)),
            };
            Some(Ok(match entry.kind {
                IOKind::Directory => Holder::FsFolder(Folder::new(bound)),
                IOKind::File => Holder::FsFile(File::new(bound)),
                _ => Holder::FsPath(Path::new(bound)),
            }))
        }))
    }

    fn delete_folder(&mut self) -> Result<()> {
        match self.filesystem().delete_dir(self.path()) {
            Err(error) if error.is_absent() => Ok(()),
            result => result,
        }
    }

    fn folder_clear(&mut self) -> Result<()> {
        if self.path().is_empty() {
            return Err(crate::Error::unsupported(
                "clear at filesystem root; use delete_root_dir_contents",
                self.filesystem().type_name(),
            ));
        }
        self.filesystem().delete_dir_contents(self.path(), true)
    }

    fn folder_remove(&mut self, recursive: bool) -> Result<()> {
        if self.path().is_empty() {
            return Err(crate::Error::unsupported(
                "remove at filesystem root; use delete_root_dir_contents",
                self.filesystem().type_name(),
            ));
        }
        let result = if recursive {
            self.filesystem().delete_dir_recursive(self.path())
        } else {
            self.filesystem().delete_dir(self.path())
        };
        match result {
            Ok(()) => Ok(()),
            Err(error) if error.is_absent() => Ok(()),
            Err(error) => Err(error),
        }
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
        Ok(())
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        self.folder_truncate(size)
    }

    fn url(&self) -> Option<&Url> {
        Some(self.url())
    }

    fn bound_location(&self) -> Option<&BoundLocation> {
        Some(&self.bound)
    }

    fn media_type(&self) -> &MediaType {
        self.folder_media_type()
    }

    fn set_media_type(&mut self, _media_type: MediaType) {}

    fn parent(&self) -> Option<Holder> {
        self.bound
            .parent()?
            .ok()
            .map(Folder::new)
            .map(Holder::FsFolder)
    }

    fn child_by_path(&self, name: &str) -> Result<Holder> {
        self.bound.child(name).map(Path::new).map(Holder::FsPath)
    }

    fn ls(&self, recursive: bool, include_private: bool) -> Listing {
        self.folder_ls(recursive, include_private)
    }

    fn kind(&self) -> IOKind {
        IOKind::Directory
    }

    fn clear(&mut self) -> Result<()> {
        self.folder_clear()
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.folder_remove(recursive)
    }

    fn is_atomic(&self) -> bool {
        false
    }

    fn is_tabular(&self) -> bool {
        self.folder_is_tabular()
    }
}

impl std::fmt::Debug for Folder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_tuple("Folder").field(&self.bound).finish()
    }
}
