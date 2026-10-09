//! One S3 location, whatever it turns out to be.

use std::sync::{Arc, Mutex};

use super::client::Client;
use super::file::S3File;
use super::folder::S3Folder;
use crate::holder::Holder;
use crate::warned;
use crate::{Error, IOBase, IOKind, IOPath, Listing, MediaType, MimeType, Result, Uri, Url};

/// An S3 location that resolves to the implementation it turns out to need.
///
/// A caller often knows a location without knowing whether it names an object
/// or a prefix - a listing entry, a configuration value, a command-line
/// argument. `S3Path` is that value: it answers [`IOBase::kind`] by asking,
/// and every other operation runs through [`S3Folder`] or [`S3File`]
/// accordingly.
///
/// # What resolving costs
///
/// **A location spelled with a trailing slash costs nothing**: `lake/` is a
/// container by its spelling, and nothing is asked. Anything else costs *one*
/// listing of a single key, which settles it whenever the location is an
/// object or the only thing sharing its name is under it - and a second one
/// otherwise, because `.` sorts below `/`, so `lake/part.parquet` stands
/// between `lake/part` and the keys under `lake/part/` and hides them from the
/// first answer. The listing that finds an object states its size beside its
/// key, and the object the location resolves to keeps it: its
/// [`IOBase::size`] asks nothing more, no `HEAD` after the listing.
///
/// That is what its kind costs. Whether anything is there ([`Self::exists`])
/// is asked even of a location spelled as a container, because the spelling
/// settles the role and never the presence: one listing of one key under
/// `lake/`, one `HEAD` for the bucket, and for a glob its listing up to the
/// first match.
///
/// Resolution follows the laziness contract: construction touches nothing, a
/// read of a location that does not exist yields nothing, and a write creates
/// an object, because a byte write is what distinguishes a leaf from a
/// container. Use [`Self::as_directory`] when the location must be a prefix.
pub struct S3Path {
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
    /// What the store last said is at this location - its role and, for an
    /// object, the size the listing stated - kept for the same reason and
    /// dropped by the same operations.
    probed: Mutex<Option<Probed>>,
}

/// What one probe of a location found: its role, and the size the listing
/// stated beside an object's key, which the object handle it resolves to is
/// built knowing - so asking it for its size costs nothing more.
#[derive(Clone, Copy, Debug)]
struct Probed {
    kind: IOKind,
    size: Option<u64>,
}

impl Probed {
    /// A role that states no size: a container, or nothing at all.
    const fn role(kind: IOKind) -> Self {
        Self { kind, size: None }
    }
}

/// The specialized implementations an S3 location can resolve to.
///
/// Deliberately not [`Holder`]: a `Holder` can hold an `S3Path`, and an
/// `S3Path` that could hold a `Holder` would be a type of unbounded size.
#[derive(Debug)]
enum Resolved {
    Directory(S3Folder),
    File(S3File),
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

impl S3Path {
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
            probed: Mutex::new(None),
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
        self.client.snapshot()
    }

    /// Return whether anything is at this location yet - a glob whether its
    /// pattern selects an object.
    pub fn exists(&self) -> bool {
        self.path_exists()
    }

    /// Treat this location as a prefix, whether or not anything is under it.
    ///
    /// # Errors
    ///
    /// Returns a refusal when the location names no bucket.
    pub fn as_directory(&self) -> Result<S3Folder> {
        S3Folder::new(self.client.clone(), self.url.clone())
    }

    /// Treat this location as an object, whether or not it exists yet.
    ///
    /// # Errors
    ///
    /// Returns a refusal when the location names no bucket.
    pub fn as_file(&self) -> Result<S3File> {
        let mut file = S3File::new(self.client.clone(), self.url.clone())?;
        if let Some(media_type) = &self.declared {
            file.set_media_type(media_type.clone());
        }
        Ok(file)
    }

    /// The object handle a probe resolved this location to, knowing the size
    /// the listing stated beside its key - as a listed child of an
    /// [`S3Folder`] is - so the resolved object answers its size without a
    /// `HEAD`.
    fn resolved_file(&self, probed: Probed) -> Result<S3File> {
        let file = self.as_file()?;
        Ok(match probed.size {
            Some(size) => file.with_known_size(size),
            None => file,
        })
    }

    /// Ask the store what is at this location.
    ///
    /// One listing bounded to a single key answers most of it: the key itself
    /// is the smallest string that starts with the key, so an object there is
    /// the first entry, and a key under `key/` coming back instead says this
    /// is a prefix.
    ///
    /// A second listing is needed only when neither holds, and one case makes
    /// that unavoidable rather than merely careful: `.` is `0x2E` and `/` is
    /// `0x2F`, so `lake/part.parquet` sorts *between* `lake/part` and
    /// `lake/part/000.parquet`. A sibling spelled that way is what comes back
    /// first, and it says nothing about whether the prefix exists, so the
    /// prefix is asked for by name.
    ///
    /// An object found this way comes back with the size its listing entry
    /// states, which the object handle is then built knowing.
    ///
    /// A refusal is not an answer: it propagates rather than reading as
    /// absence, because a caller who cannot see a location must hear so.
    fn probe(&self) -> Result<Probed> {
        let page = self
            .client
            .list_objects(&self.bucket, &self.key, None, None, 1)?;
        let under = format!("{}/", self.key);
        match page.objects.first() {
            Some(first) if first.key == self.key => {
                return Ok(Probed {
                    kind: IOKind::File,
                    size: Some(first.size),
                });
            }
            Some(first) if first.key.starts_with(&under) => {
                return Ok(Probed::role(IOKind::Directory));
            }
            // Nothing shares the name at all, so nothing is under it either.
            None => return Ok(Probed::role(IOKind::Unknown)),
            Some(_) => {}
        }
        let page = self
            .client
            .list_objects(&self.bucket, &under, None, None, 1)?;
        if page.objects.is_empty() {
            // A sibling merely shares a textual prefix, like `lake/partial`
            // for `lake/part`: nothing is at this location.
            Ok(Probed::role(IOKind::Unknown))
        } else {
            Ok(Probed::role(IOKind::Directory))
        }
    }

    /// The role this location has, from what it already resolved to.
    ///
    /// Asking the store is the last resort, and a retained handle answers
    /// without asking at all.
    fn current_kind(&self) -> Result<IOKind> {
        if let Ok(slot) = self.resolved.lock() {
            match slot.as_ref() {
                Some(Resolved::Directory(_)) => return Ok(IOKind::Directory),
                Some(Resolved::File(_)) => return Ok(IOKind::File),
                None => {}
            }
        }
        self.unresolved_kind()
    }

    /// Resolve a role before a specialized handle has been retained.
    ///
    /// What the store says is kept, on the same terms as the resolved handle
    /// beside it: a caller that asks three questions of one location - is it a
    /// container, what encoding does it hold, how many columns - would
    /// otherwise pay for three listings to hear one answer three times. It is
    /// dropped by the operations that can change the answer.
    fn unresolved(&self) -> Result<Probed> {
        // A glob or a trailing slash says what this is, so nothing is asked.
        if self.url.is_glob() || self.url.has_trailing_slash() || self.key.is_empty() {
            return Ok(Probed::role(IOKind::Directory));
        }
        if let Some(known) = *self.probed.lock().map_err(|_| poisoned())? {
            return Ok(known);
        }
        let probed = self.probe()?;
        *self.probed.lock().map_err(|_| poisoned())? = Some(probed);
        Ok(probed)
    }

    /// The role [`Self::unresolved`] settles.
    fn unresolved_kind(&self) -> Result<IOKind> {
        Ok(self.unresolved()?.kind)
    }

    /// What an infallible verb answers when asking the store failed: `absent`,
    /// after a warning naming this location and the refusal - logged once per
    /// location, later ones counted - so a refused key or a region the store
    /// answers `400` for is never a silent empty read. A store saying nothing
    /// is there is an answer, and warns nothing.
    fn heard<T>(&self, outcome: Result<T>, absent: T) -> T {
        outcome.unwrap_or_else(|error| {
            if !error.is_absent() {
                warned!(
                    "an S3 location could not be read and answers as empty",
                    &self.url.to_string(),
                    "{error}"
                );
            }
            absent
        })
    }

    /// Forget what the store said, because something changed it.
    fn forget(&self) -> Result<()> {
        *self.probed.lock().map_err(|_| poisoned())? = None;
        Ok(())
    }

    /// Run `read` against the resolved implementation, or report absence.
    fn with_resolved<T>(&self, absent: T, read: impl FnOnce(&dyn IOBase) -> T) -> Result<T> {
        let mut slot = self.resolved.lock().map_err(|_| poisoned())?;
        if slot.is_none() {
            let probed = self.unresolved()?;
            *slot = match probed.kind {
                IOKind::Directory => Some(Resolved::Directory(self.as_directory()?)),
                IOKind::File => Some(Resolved::File(self.resolved_file(probed)?)),
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
            let probed = self.unresolved()?;
            *slot = Some(match probed.kind {
                IOKind::Directory => Resolved::Directory(self.as_directory()?),
                _ => Resolved::File(self.resolved_file(probed)?),
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
impl IOPath for S3Path {
    fn path_url(&self) -> &Url {
        &self.url
    }

    /// Whether a container is there: a location spelled as one - `lake/`, the
    /// bucket, a glob - asks its prefix (one listing of one key, the bucket one
    /// `HEAD`, a glob its listing up to the first match), because the spelling
    /// settles the role and never the presence; any other asks what it
    /// resolved to.
    fn is_folder(&self) -> bool {
        if self.url.is_glob() || self.url.has_trailing_slash() || self.key.is_empty() {
            return self.as_directory().is_ok_and(|folder| folder.exists());
        }
        self.current_kind()
            .is_ok_and(|kind| kind == IOKind::Directory)
    }

    fn is_file(&self) -> bool {
        self.current_kind().is_ok_and(|kind| kind == IOKind::File)
    }
}

impl crate::IOMedia for S3Path {
    crate::impl_default_iomedia!();
}

impl IOBase for S3Path {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.with_resolved(Ok(0), |handle| handle.pread(offset, buffer))?
    }

    /// The stream the object at this location owns: the resolved handle's
    /// where the location resolved already - a staged value included - else
    /// the key's own, opened on its first read with no kind probe before it,
    /// since a `GET` of a prefix or of nothing reads as empty.
    fn owned_stream_bytes(
        &self,
        position: u64,
    ) -> Result<Option<Box<dyn std::io::Read + Send + 'static>>> {
        let slot = self.resolved.lock().map_err(|_| poisoned())?;
        if let Some(resolved) = slot.as_ref() {
            return resolved.as_io().owned_stream_bytes(position);
        }
        drop(slot);
        Ok(Some(super::file::lazy_object_stream(
            Arc::clone(&self.client),
            self.bucket.clone(),
            self.key.clone(),
            position,
        )))
    }

    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<crate::ByteStream<'_>> {
        // A stream borrows the handle it reads from, and the resolved one
        // lives behind a lock, so each role answers the stream that owns what
        // it reads: a container the objects beneath it, an object its staged
        // write copied out - what makes a write this handle has not published
        // yet visible to a read of it - or else the store's.
        {
            let slot = self.resolved.lock().map_err(|_| poisoned())?;
            match slot.as_ref() {
                Some(Resolved::Directory(folder)) => {
                    return crate::ByteStream::from_container(folder, position, batch_size);
                }
                Some(Resolved::File(file)) => return file.byte_stream(position, batch_size),
                None => {}
            }
        }
        if self.unresolved_kind()? == IOKind::Directory {
            return crate::ByteStream::from_container(&self.as_directory()?, position, batch_size);
        }
        self.as_file()?.byte_stream(position, batch_size)
    }

    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        self.with_resolved(Ok(Vec::new()), |handle| handle.read_all_bytes())?
    }

    fn read_range_bytes(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        self.with_resolved(Ok(Vec::new()), |handle| {
            handle.read_range_bytes(offset, length)
        })?
    }

    /// The resolved handle's tail read: the one listing that settles the
    /// role, then an object's one suffix-ranged `GET`; nothing at the
    /// location is no bytes and a total of `0`.
    fn read_tail_bytes(&self, length: usize) -> Result<(Vec<u8>, u64)> {
        self.with_resolved(Ok((Vec::new(), 0)), |handle| handle.read_tail_bytes(length))?
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.with_resolved_mut(|handle| handle.pwrite(offset, bytes))?
    }

    fn write_all_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.with_resolved_mut(|handle| handle.write_all_bytes(bytes))?
    }

    fn create_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.with_resolved_mut(|handle| handle.create_bytes(bytes))?
    }

    fn append_bytes(&mut self, bytes: &[u8]) -> Result<u64> {
        self.with_resolved_mut(|handle| handle.append_bytes(bytes))?
    }

    /// The resolved handle's size; `0` where nothing is, and `0` with a
    /// warning where the store refused to say what is (`heard`).
    fn size(&self) -> u64 {
        self.heard(self.with_resolved(0, |handle| handle.size()), 0)
    }

    /// The resolved handle's capacity, on the terms of [`Self::size`].
    fn capacity(&self) -> u64 {
        self.heard(self.with_resolved(0, |handle| handle.capacity()), 0)
    }

    fn reserve(&mut self, capacity: u64) -> Result<()> {
        self.with_resolved_mut(|handle| handle.reserve(capacity))?
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        // One resolution, not two: `with_resolved_mut` already routes a
        // container to the prefix handle, and asking the kind first would be
        // a second listing for the same answer.
        self.with_resolved_mut(|handle| handle.truncate(size))?
    }

    fn uri(&self) -> Option<&Uri> {
        Some(self.url.as_ref())
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
        if let Ok(mut resolved) = self.resolved.lock()
            && let Some(resolved) = resolved.as_mut()
        {
            resolved.as_io_mut().set_media_type(media_type.clone());
        }
        self.declared = Some(media_type);
    }

    /// The role this location has; `Unknown` where nothing is, and
    /// `Unknown` with a warning where the store refused to say what is
    /// (`heard`).
    fn kind(&self) -> IOKind {
        self.heard(self.current_kind(), IOKind::Unknown)
    }

    fn is_atomic(&self) -> bool {
        self.path_is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.path_is_tabular()
    }

    /// Publish whatever was resolved, and nothing when nothing was.
    ///
    /// No request when this location has resolved to nothing: there is nothing
    /// staged to publish, and asking the store what the location is in order
    /// to find that out would be the round trip this avoids.
    fn flush(&mut self) -> Result<()> {
        let mut slot = self.resolved.lock().map_err(|_| poisoned())?;
        match slot.as_mut() {
            Some(resolved) => resolved.as_io_mut().flush(),
            None => Ok(()),
        }
    }

    fn open(&mut self) -> Result<()> {
        self.with_resolved_mut(|handle| handle.open())?
    }

    /// Whether this handle is open, which is a question about the handle.
    ///
    /// No request: a location nothing has resolved yet has opened nothing, and
    /// asking the store what it is would not change that answer.
    fn opened(&self) -> bool {
        self.resolved
            .lock()
            .is_ok_and(|slot| slot.as_ref().is_some_and(|held| held.as_io().opened()))
    }

    /// Publish and let go of everything this scope learned.
    ///
    /// Costs nothing when nothing resolved, for the reason
    /// [`Self::flush`] gives, and drops the role along with the handle so the
    /// next read sees the store as it is.
    fn close(&mut self) -> Result<()> {
        let held = self.resolved.lock().map_err(|_| poisoned())?.take();
        self.forget()?;
        match held {
            Some(mut resolved) => resolved.as_io_mut().close(),
            None => Ok(()),
        }
    }

    fn parent(&self) -> Option<Holder> {
        let parent = self.url.parent()?;
        S3Folder::new(self.client.clone(), parent)
            .ok()
            .map(Holder::S3Folder)
    }

    /// Name a descendant without asking the store anything.
    ///
    /// A child of a location is a location, and naming one settles nothing
    /// about either: the child resolves itself if and when it is used. Only a
    /// handle that has *already* resolved answers through what it resolved to.
    fn child_by_path(&self, name: &str) -> Result<Holder> {
        {
            let slot = self.resolved.lock().map_err(|_| poisoned())?;
            if let Some(held) = slot.as_ref() {
                return held.as_io().child_by_path(name);
            }
        }
        self.as_directory()?.child_by_path(name)
    }

    /// Empty whichever of the two the resolved kind names.
    ///
    /// Routing on the kind this handle already resolves is the one documented
    /// exception to the no-pre-call rule, and it is the resolution `S3Path`
    /// performs anyway rather than a second probe.
    fn clear(&mut self) -> Result<()> {
        {
            let mut resolved = self.resolved.lock().map_err(|_| poisoned())?;
            if let Some(resolved) = resolved.as_mut() {
                return resolved.as_io_mut().clear();
            }
        }
        match self.unresolved_kind()? {
            IOKind::Directory => self.as_directory()?.clear(),
            IOKind::Unknown => Ok(()),
            _ => self.as_file()?.clear(),
        }
    }

    /// Delete whichever of the two the resolved kind names.
    fn remove(&mut self, recursive: bool) -> Result<()> {
        // Abandon what was resolved before deleting: dropping a leaf publishes
        // whatever it staged, so a pending write on its way to being deleted
        // has to be discarded first or the removal would race its own
        // resurrection. A later operation then re-resolves from scratch.
        let held = self.resolved.lock().map_err(|_| poisoned())?.take();
        let kind = match &held {
            Some(Resolved::Directory(_)) => IOKind::Directory,
            Some(Resolved::File(file)) => {
                file.discard()?;
                IOKind::File
            }
            None => self.unresolved_kind()?,
        };
        drop(held);
        // Whatever was there is about to not be.
        self.forget()?;
        match kind {
            IOKind::Directory => self.as_directory()?.remove(recursive),
            // An undecided location may still hold an object this handle has
            // not looked for; the delete is issued and absence is success.
            _ => self.as_file()?.remove(recursive),
        }
    }

    fn ls(&self, recursive: bool, include_private: bool) -> Listing {
        match self.current_kind() {
            // A leaf contains nothing; that is not an error.
            Ok(kind) if !kind.is_container() => return Listing::empty(),
            Err(error) => return Listing::failing(error),
            Ok(_) => {}
        }
        match self.as_directory() {
            Ok(directory) => directory.ls(recursive, include_private),
            Err(error) => Listing::failing(error),
        }
    }
}

impl std::fmt::Debug for S3Path {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("S3Path")
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
