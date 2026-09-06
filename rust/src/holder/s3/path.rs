//! One S3 location, whatever it turns out to be.

use std::sync::{Arc, Mutex};

use super::client::Client;
use super::file::File;
use super::folder::Folder;
use crate::holder::Holder;
use crate::{Error, IOBase, IOKind, IOPath, Listing, MediaType, MimeType, Result, Url};

/// An S3 location that resolves to the implementation it turns out to need.
///
/// A caller often knows a location without knowing whether it names an object
/// or a prefix - a listing entry, a configuration value, a command-line
/// argument. `Path` is that value: it answers [`IOBase::kind`] by asking, and
/// every other operation runs through [`Folder`] or [`File`] accordingly.
///
/// # What resolving costs
///
/// **A location spelled with a trailing slash costs nothing**: `lake/` is a
/// container by its spelling, and nothing is asked. Anything else costs *one*
/// listing of a single key, which settles both questions at once - S3 answers
/// keys in byte order, so the first key at or after this one is the exact key
/// when the object exists, and a key under it when a prefix does.
///
/// Resolution follows the laziness contract: construction touches nothing, a
/// read of a location that does not exist yields nothing, and a write creates
/// an object, because a byte write is what distinguishes a leaf from a
/// container. Use [`Self::as_directory`] when the location must be a prefix.
pub struct Path {
    client: Arc<Client>,
    /// The location, with any credentials the caller wrote into it removed.
    url: Url,
    bucket: String,
    /// The key, decoded, as the store names it.
    key: String,
    /// An explicit representation supplied by the caller.
    declared: Option<MediaType>,
    /// Inference from the key's compound filename, computed on demand.
    inferred: std::sync::OnceLock<MediaType>,
    /// The implementation this location resolved to, kept so a staged write
    /// survives between calls.
    resolved: Mutex<Option<Resolved>>,
}

/// The specialized implementations an S3 location can resolve to.
///
/// Deliberately not [`Holder`]: a `Holder` can hold a `Path`, and a `Path`
/// that could hold a `Holder` would be a type of unbounded size.
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
    /// Describe the location `url` names on `client`, touching nothing.
    pub(super) fn new(client: Arc<Client>, url: Url) -> Result<Self> {
        let (bucket, key) = super::split_location(&url)?;
        Ok(Self {
            client,
            url: super::without_credentials(url),
            bucket,
            key,
            declared: None,
            inferred: std::sync::OnceLock::new(),
            resolved: Mutex::new(None),
        })
    }

    /// Borrow the described location.
    pub const fn url(&self) -> &Url {
        &self.url
    }

    /// The bucket this location is in.
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// The key, as the store names it.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// How many requests this handle's client has sent, by shape.
    pub fn stats(&self) -> super::StatsSnapshot {
        self.client.stats().snapshot()
    }

    /// Return whether anything is at this location yet.
    pub fn exists(&self) -> bool {
        self.path_exists()
    }

    /// Treat this location as a prefix, whether or not anything is under it.
    ///
    /// # Errors
    ///
    /// Returns a refusal when the location names no bucket.
    pub fn as_directory(&self) -> Result<Folder> {
        Folder::new(self.client.clone(), self.url.clone())
    }

    /// Treat this location as an object, whether or not it exists yet.
    ///
    /// # Errors
    ///
    /// Returns a refusal when the location names no bucket.
    pub fn as_file(&self) -> Result<File> {
        let mut file = File::new(self.client.clone(), self.url.clone())?;
        if let Some(media_type) = &self.declared {
            file.set_media_type(media_type.clone());
        }
        Ok(file)
    }

    /// Ask the store what is at this location, with one listing.
    ///
    /// One request settles both questions. Keys come back in byte order, and
    /// `lake/part` sorts before `lake/part/x`, which sorts before
    /// `lake/part2`, so the single key at or after this one says which of the
    /// three cases holds.
    fn probe(&self) -> IOKind {
        let page = self
            .client
            .list_objects(&self.bucket, &self.key, None, None, 1);
        let Ok(page) = page else {
            return IOKind::Unknown;
        };
        let Some(first) = page.objects.first() else {
            return IOKind::Unknown;
        };
        if first.key == self.key {
            return IOKind::File;
        }
        let under = format!("{}/", self.key);
        if first.key.starts_with(&under) {
            return IOKind::Directory;
        }
        // The key merely shares a textual prefix, like `lake/partial` for
        // `lake/part`: nothing is at this location.
        IOKind::Unknown
    }

    /// The role this location has, from what it already resolved to.
    ///
    /// Asking the store is the last resort, and a retained handle answers
    /// without asking at all.
    fn current_kind(&self) -> IOKind {
        if let Ok(slot) = self.resolved.lock() {
            match slot.as_ref() {
                Some(Resolved::Directory(_)) => return IOKind::Directory,
                Some(Resolved::File(_)) => return IOKind::File,
                None => {}
            }
        }
        self.unresolved_kind()
    }

    /// Resolve a role before a specialized handle has been retained.
    fn unresolved_kind(&self) -> IOKind {
        // A glob or a trailing slash says what this is, so nothing is asked.
        if self.url.is_glob() || self.url.has_trailing_slash() || self.key.is_empty() {
            return IOKind::Directory;
        }
        self.probe()
    }

    /// Run `read` against the resolved implementation, or report absence.
    fn with_resolved<T>(&self, absent: T, read: impl FnOnce(&dyn IOBase) -> T) -> Result<T> {
        let mut slot = self.resolved.lock().map_err(|_| poisoned())?;
        if slot.is_none() {
            *slot = match self.unresolved_kind() {
                IOKind::Directory => Some(Resolved::Directory(self.as_directory()?)),
                IOKind::File => Some(Resolved::File(self.as_file()?)),
                _ => None,
            };
        }
        Ok(match slot.as_ref() {
            Some(resolved) => read(resolved.as_io()),
            None => absent,
        })
    }

    /// Run `write` against the resolved implementation, creating an object
    /// when the location has not been decided.
    ///
    /// A write is what decides an undecided location: it becomes an object.
    fn with_resolved_mut<T>(&self, write: impl FnOnce(&mut dyn IOBase) -> T) -> Result<T> {
        let mut slot = self.resolved.lock().map_err(|_| poisoned())?;
        if slot.is_none() {
            *slot = Some(match self.unresolved_kind() {
                IOKind::Directory => Resolved::Directory(self.as_directory()?),
                _ => Resolved::File(self.as_file()?),
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

/// An S3 location is the generic role over the store.
impl IOPath for Path {
    fn path_url(&self) -> &Url {
        &self.url
    }

    fn is_folder(&self) -> bool {
        self.current_kind() == IOKind::Directory
    }

    fn is_file(&self) -> bool {
        self.current_kind() == IOKind::File
    }
}

impl crate::IOMedia for Path {
    crate::impl_default_iomedia!();
}

impl IOBase for Path {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.with_resolved(Ok(0), |handle| handle.pread(offset, buffer))?
    }

    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<crate::ByteStream<'_>> {
        // A stream borrows the handle it reads from, and the resolved one
        // lives behind a lock, so this reads through the leaf directly.
        if self.unresolved_kind() == IOKind::Directory {
            return crate::ByteStream::from_reader(std::io::empty(), batch_size);
        }
        let reader = self.client.open_reader(&self.bucket, &self.key, position)?;
        crate::ByteStream::from_reader(reader, batch_size)
    }

    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        self.with_resolved(Ok(Vec::new()), |handle| handle.read_all_bytes())?
    }

    fn read_range_bytes(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        self.with_resolved(Ok(Vec::new()), |handle| {
            handle.read_range_bytes(offset, length)
        })?
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.with_resolved_mut(|handle| handle.pwrite(offset, bytes))?
    }

    fn write_all_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.with_resolved_mut(|handle| handle.write_all_bytes(bytes))?
    }

    fn append_bytes(&mut self, bytes: &[u8]) -> Result<u64> {
        self.with_resolved_mut(|handle| handle.append_bytes(bytes))?
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
        if self.current_kind().is_container() {
            return self.as_directory()?.truncate(size);
        }
        self.with_resolved_mut(|handle| handle.truncate(size))?
    }

    fn url(&self) -> Option<&Url> {
        Some(&self.url)
    }

    fn media_type(&self) -> &MediaType {
        // A location's type comes from its name, which is stable, so this
        // costs no request.
        static DIRECTORY: std::sync::LazyLock<MediaType> =
            std::sync::LazyLock::new(|| MediaType::from(MimeType::DIRECTORY));
        if self.url.has_trailing_slash() || self.key.is_empty() || self.url.is_glob() {
            return &DIRECTORY;
        }
        if let Some(media_type) = &self.declared {
            return media_type;
        }
        if self.url.extension().is_none() {
            static FILE: std::sync::LazyLock<MediaType> =
                std::sync::LazyLock::new(|| MediaType::from(MimeType::FILE));
            return &FILE;
        }
        self.inferred.get_or_init(|| self.url.media_type())
    }

    fn set_media_type(&mut self, media_type: MediaType) {
        if let Ok(mut resolved) = self.resolved.lock() {
            if let Some(resolved) = resolved.as_mut() {
                resolved.as_io_mut().set_media_type(media_type.clone());
            }
        }
        self.declared = Some(media_type);
    }

    fn kind(&self) -> IOKind {
        self.current_kind()
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

    fn opened(&self) -> bool {
        self.with_resolved(false, |handle| handle.opened())
            .unwrap_or(false)
    }

    fn close(&mut self) -> Result<()> {
        self.with_resolved_mut(|handle| handle.close())?
    }

    fn parent(&self) -> Option<Holder> {
        let parent = self.url.parent()?;
        Folder::new(self.client.clone(), parent)
            .ok()
            .map(Holder::S3Folder)
    }

    fn child_by_path(&self, name: &str) -> Result<Holder> {
        if self.current_kind() == IOKind::File {
            return self.as_file()?.child_by_path(name);
        }
        self.as_directory()?.child_by_path(name)
    }

    /// Empty whichever of the two the resolved kind names.
    ///
    /// Routing on the kind this handle already resolves is the one documented
    /// exception to the no-pre-call rule, and it is the resolution `Path`
    /// performs anyway rather than a second probe.
    fn clear(&mut self) -> Result<()> {
        {
            let mut resolved = self.resolved.lock().map_err(|_| poisoned())?;
            if let Some(resolved) = resolved.as_mut() {
                return resolved.as_io_mut().clear();
            }
        }
        match self.unresolved_kind() {
            IOKind::Directory => self.as_directory()?.clear(),
            IOKind::Unknown => Ok(()),
            _ => self.as_file()?.clear(),
        }
    }

    /// Delete whichever of the two the resolved kind names.
    fn remove(&mut self, recursive: bool) -> Result<()> {
        let kind = self.unresolved_kind();
        // Drop what was resolved before deleting, so no staged write survives
        // the removal and a later operation re-resolves from scratch.
        if let Ok(mut slot) = self.resolved.lock() {
            *slot = None;
        }
        match kind {
            IOKind::Directory => self.as_directory()?.remove(recursive),
            // An undecided location may still hold an object this handle has
            // not looked for; the delete is issued and absence is success.
            _ => self.as_file()?.remove(recursive),
        }
    }

    fn ls(&self, recursive: bool, include_private: bool) -> Listing {
        if !self.current_kind().is_container() {
            // A leaf contains nothing; that is not an error.
            return Listing::empty();
        }
        match self.as_directory() {
            Ok(directory) => directory.ls(recursive, include_private),
            Err(error) => Listing::failing(error),
        }
    }
}

impl std::fmt::Debug for Path {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Path")
            .field("url", &self.url)
            .finish()
    }
}

/// Report a poisoned resolution lock without panicking a caller.
fn poisoned() -> Error {
    Error::Io(std::io::Error::other(
        "the resolved S3 handle lock was poisoned",
    ))
}
