//! An unresolved resource over one bound Arrow filesystem location.

use std::sync::OnceLock;

use crate::holder::Holder;
use crate::{IOBase, IOKind, IOPath, Listing, MediaType, MimeType, Result, Url};

use super::{BoundLocation, File, FileSystem, Folder};

/// A bound location whose current file/directory role is resolved per action.
pub struct Path {
    bound: BoundLocation,
    declared: Option<MediaType>,
    inferred: OnceLock<MediaType>,
}

impl Path {
    /// Bind an unresolved location without touching the filesystem.
    pub fn new(bound: BoundLocation) -> Self {
        Self {
            bound,
            declared: None,
            inferred: OnceLock::new(),
        }
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

    /// Treat this location as a directory without probing it.
    pub fn as_directory(&self) -> Folder {
        Folder::new(self.bound.clone())
    }

    /// Treat this location as a file without probing it.
    pub fn as_file(&self) -> File {
        let mut file = File::new(self.bound.clone());
        if let Some(media_type) = &self.declared {
            file.set_media_type(media_type.clone());
        }
        file
    }

    fn current_kind(&self) -> Result<IOKind> {
        self.filesystem()
            .file_info(self.path())
            .map(|info| info.kind)
    }
}

impl IOPath for Path {
    fn path_url(&self) -> &Url {
        self.url()
    }

    fn is_folder(&self) -> bool {
        self.current_kind().is_ok_and(IOKind::is_container)
    }

    fn is_file(&self) -> bool {
        self.current_kind().is_ok_and(|kind| kind == IOKind::File)
    }
}

impl crate::IOMedia for Path {
    crate::impl_default_iomedia!();
}

impl IOBase for Path {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.as_file().pread(offset, buffer)
    }

    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<crate::ByteStream<'_>> {
        self.as_file().byte_stream(position, batch_size)
    }

    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        self.as_file().read_all_bytes()
    }

    fn read_range_bytes(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        self.as_file().read_range_bytes(offset, length)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.as_file().pwrite(offset, bytes)
    }

    fn write_all_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.as_file().write_all_bytes(bytes)
    }

    fn append_bytes(&mut self, bytes: &[u8]) -> Result<u64> {
        self.as_file().append_bytes(bytes)
    }

    fn size(&self) -> u64 {
        self.filesystem()
            .file_info(self.path())
            .ok()
            .and_then(|info| info.size)
            .unwrap_or(0)
    }

    fn capacity(&self) -> u64 {
        self.size()
    }

    fn reserve(&mut self, capacity: u64) -> Result<()> {
        self.as_file().reserve(capacity)
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        self.as_file().truncate(size)
    }

    fn url(&self) -> Option<&Url> {
        Some(self.url())
    }

    fn bound_location(&self) -> Option<&BoundLocation> {
        Some(&self.bound)
    }

    fn media_type(&self) -> &MediaType {
        static DIRECTORY: std::sync::LazyLock<MediaType> =
            std::sync::LazyLock::new(|| MediaType::from(MimeType::DIRECTORY));
        if self.current_kind().is_ok_and(IOKind::is_container) {
            return &DIRECTORY;
        }
        if let Some(media_type) = &self.declared {
            return media_type;
        }
        self.inferred.get_or_init(|| {
            if self.url().extension().is_none() {
                MediaType::from(MimeType::FILE)
            } else {
                self.url().media_type()
            }
        })
    }

    fn set_media_type(&mut self, media_type: MediaType) {
        self.declared = Some(media_type);
    }

    fn kind(&self) -> IOKind {
        self.current_kind().unwrap_or(IOKind::Unknown)
    }

    fn parent(&self) -> Option<Holder> {
        self.bound.parent()?.ok().map(Path::new).map(Holder::FsPath)
    }

    fn child_by_path(&self, name: &str) -> Result<Holder> {
        self.bound.child(name).map(Path::new).map(Holder::FsPath)
    }

    fn ls(&self, recursive: bool, include_private: bool) -> Listing {
        self.as_directory().ls(recursive, include_private)
    }

    fn clear(&mut self) -> Result<()> {
        match self.filesystem().delete_dir_contents(self.path(), true) {
            Ok(()) => Ok(()),
            Err(crate::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotADirectory => {
                self.as_file().clear()
            }
            Err(error) => Err(error),
        }
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        match self.filesystem().delete_file(self.path()) {
            Ok(()) => Ok(()),
            Err(error) if error.is_absent() => Ok(()),
            Err(crate::Error::Io(error)) if error.kind() == std::io::ErrorKind::IsADirectory => {
                self.as_directory().remove(recursive)
            }
            Err(error) => Err(error),
        }
    }

    fn is_atomic(&self) -> bool {
        self.path_is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.path_is_tabular()
    }
}

impl std::fmt::Debug for Path {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_tuple("Path").field(&self.bound).finish()
    }
}
