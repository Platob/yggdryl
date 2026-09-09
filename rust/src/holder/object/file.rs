//! One S3 object as a byte leaf.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use super::answer::ObjectMeta;
use super::client::Client;
use super::folder::Folder;
use crate::holder::Holder;
use crate::{Error, IOBase, IOFile, Listing, MediaType, MimeType, Result, Url};

/// An S3 object addressed by offset.
///
/// # What each operation costs
///
/// The whole point of this handle is that the answer is short and knowable:
///
/// | operation | requests |
/// | --- | --- |
/// | building the handle | none |
/// | [`IOBase::pread`] | one ranged `GET` |
/// | [`IOBase::read_all_bytes`] | one `GET` |
/// | [`IOBase::read_range_bytes`] | one ranged `GET` |
/// | [`IOBase::pstream_bytes`] to the end | one `GET` |
/// | [`IOBase::read_digest`] | one `GET` |
/// | [`IOBase::read_range_digest`] | one ranged `GET`, of that range |
/// | [`IOBase::size`] | one `HEAD`, or none while open |
/// | [`IOBase::write_all_bytes`] | one `PUT`, or a multipart upload above the threshold |
/// | [`IOBase::append_bytes`] | one `GET` and one `PUT` |
/// | [`IOBase::pwrite`] then [`IOBase::flush`] | one `GET` and one `PUT` |
/// | [`IOBase::remove`] | one `DELETE` |
///
/// A ranged read transfers the range, never the object: this is what makes a
/// Parquet footer read cost a few kilobytes rather than the file. A read also
/// *learns* the object's length from the `Content-Range` it comes back with,
/// so a scan that reads and then asks the size pays nothing for the answer.
///
/// # Laziness
///
/// Construction touches nothing. A missing object reads as empty and sizes as
/// zero; the first published write creates it. [`IOBase::open`] caches the
/// object's metadata - not its bytes - so repeated size and kind questions
/// stop asking, and [`IOBase::close`] publishes and drops that cache.
///
/// # Writing
///
/// S3 replaces whole objects and has no positional write, so
/// [`IOBase::pwrite`] stages: the stored value is loaded once, mutated in
/// memory, and published as one upload on [`IOBase::flush`] or
/// [`IOBase::close`]. Whole-value writes skip the load, because nothing of the
/// old value survives them.
pub struct File {
    client: Arc<Client>,
    /// The location, with any credentials the caller wrote into it removed.
    url: Url,
    bucket: String,
    /// The object key, decoded - what the store names the object.
    key: String,
    /// An explicit media type overrides inference from the location.
    declared: Option<MediaType>,
    /// Inference from the key's compound filename, computed on demand.
    inferred: OnceLock<MediaType>,
    state: Mutex<State>,
}

/// What the handle knows and what it has not published yet.
#[derive(Default)]
struct State {
    /// The stored object's metadata: `None` when unknown, `Some(None)` when
    /// known absent. Held only between `open` and `close`, plus whatever a
    /// read learned along the way.
    meta: Option<Option<ObjectMeta>>,
    /// Positional writes waiting to be published.
    stage: Option<Stage>,
    /// Whether metadata learned along the way is kept.
    opened: bool,
}

/// The staged value and whether the store has seen it.
struct Stage {
    bytes: Vec<u8>,
    dirty: bool,
}

impl File {
    /// Describe the object `url` names on `client`, touching nothing.
    pub(super) fn new(client: Arc<Client>, url: Url) -> Result<Self> {
        let (bucket, key) = super::split_location(&url)?;
        Ok(Self {
            client,
            url: super::without_credentials(url),
            bucket,
            key,
            declared: None,
            inferred: OnceLock::new(),
            state: Mutex::new(State::default()),
        })
    }

    /// Retain a length a listing already reported.
    ///
    /// A listing states every entry's size, so a handle built from one starts
    /// out knowing it: asking a listed object for its size costs nothing,
    /// which is what lets a partition scan weigh a lake without a `HEAD` per
    /// file. The value is what the listing saw, so it is dropped as soon as
    /// anything writes through this handle.
    #[must_use]
    pub(super) fn with_known_size(self, size: u64) -> Self {
        if let Ok(mut state) = self.state.lock() {
            state.meta = Some(Some(ObjectMeta {
                size,
                ..ObjectMeta::default()
            }));
        }
        self
    }

    /// Borrow the described location.
    pub const fn url(&self) -> &Url {
        &self.url
    }

    /// The bucket the object lives in.
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// The object key, as the store names it.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// How many requests this handle's client has sent, by shape.
    pub fn stats(&self) -> super::StatsSnapshot {
        self.client.snapshot()
    }

    /// Return whether the object exists, per the store right now.
    ///
    /// One `HEAD`, or none while the handle is open.
    pub fn exists(&self) -> bool {
        self.file_exists()
    }

    /// Lock the state, reporting a poisoned lock rather than panicking.
    fn state(&self) -> Result<MutexGuard<'_, State>> {
        self.state.lock().map_err(|_| poisoned())
    }

    /// The stored object's metadata, from the cache or from one `HEAD`.
    fn meta(&self, state: &mut State) -> Result<Option<ObjectMeta>> {
        if let Some(known) = state.meta.as_ref() {
            return Ok(known.clone());
        }
        let meta = self.client.head_object(&self.bucket, &self.key)?;
        // A closed handle keeps nothing: the next read must see the store as
        // it is, per the open/close contract.
        if state.opened {
            state.meta = Some(meta.clone());
        }
        Ok(meta)
    }

    /// Record what a read has just learned about the object's length.
    fn learn_size(state: &mut State, size: u64) {
        if !state.opened {
            return;
        }
        match state.meta.as_mut() {
            Some(Some(meta)) => meta.size = size,
            Some(slot @ None) => {
                *slot = Some(ObjectMeta {
                    size,
                    ..ObjectMeta::default()
                });
            }
            None => {
                state.meta = Some(Some(ObjectMeta {
                    size,
                    ..ObjectMeta::default()
                }));
            }
        }
    }

    /// Load the stored value into the stage so positional writes can land.
    ///
    /// One `GET`, and only the first time: a sequence of positional writes
    /// loads once and publishes once.
    fn materialize<'state>(
        &self,
        mut state: MutexGuard<'state, State>,
    ) -> Result<MutexGuard<'state, State>> {
        if state.stage.is_none() {
            let bytes = self.client.get_all(&self.bucket, &self.key)?;
            state.stage = Some(Stage {
                bytes,
                dirty: false,
            });
        }
        Ok(state)
    }

    /// Publish the staged value, as one upload or as a multipart one.
    ///
    /// What was published belongs to the store, so a closed handle lets go of
    /// it: keeping it would hold the whole payload in memory for the life of
    /// the handle and, worse, answer later reads from a copy the store may
    /// have moved on from. An open handle keeps it, because a scope that
    /// opened the object is a scope that asked for a coherent view of it.
    fn publish(&self, state: &mut State) -> Result<()> {
        let Some(stage) = state.stage.as_ref() else {
            return Ok(());
        };
        if !stage.dirty {
            return Ok(());
        }
        let content_type = self.media_type().to_string();
        let size = stage.bytes.len() as u64;
        let etag = self.upload(&stage.bytes, &content_type)?;
        if state.opened {
            if let Some(stage) = state.stage.as_mut() {
                stage.dirty = false;
            }
            state.meta = Some(Some(ObjectMeta {
                size,
                etag,
                content_type: Some(content_type),
            }));
        } else {
            state.stage = None;
            state.meta = None;
        }
        Ok(())
    }

    /// Upload `bytes` whole, in parts when they are large enough to warrant it.
    ///
    /// One `PUT` below the threshold. Above it, a multipart upload of
    /// `parts + 2` requests, which is what bounds the cost of a failure: a
    /// retried part re-sends one part rather than the whole object.
    fn upload(&self, bytes: &[u8], content_type: &str) -> Result<Option<String>> {
        let options = self.client.options();
        // A multipart upload of no parts is not a thing S3 will complete, so
        // an empty value is one `PUT` whatever the threshold says.
        if bytes.is_empty() || (bytes.len() as u64) < options.multipart_threshold() {
            return self
                .client
                .put_object(&self.bucket, &self.key, bytes, Some(content_type));
        }
        let part_size = usize::try_from(options.part_size())
            .map_err(|_| crate::iobase::oversized(options.part_size()))?;
        let upload = self
            .client
            .create_multipart(&self.bucket, &self.key, Some(content_type))?;
        let mut parts = Vec::with_capacity(bytes.len().div_ceil(part_size));
        for (index, chunk) in bytes.chunks(part_size).enumerate() {
            let number = u32::try_from(index + 1).map_err(|_| too_many_parts())?;
            match self
                .client
                .upload_part(&self.bucket, &self.key, &upload, number, chunk)
            {
                Ok(etag) => parts.push((number, etag)),
                Err(error) => {
                    // Abandon the upload so its parts are not billed forever;
                    // the original failure is what the caller hears about.
                    let _ = self
                        .client
                        .abort_multipart(&self.bucket, &self.key, &upload);
                    return Err(error);
                }
            }
        }
        match self
            .client
            .complete_multipart(&self.bucket, &self.key, &upload, &parts)
        {
            Ok(etag) => Ok(etag),
            Err(error) => {
                let _ = self
                    .client
                    .abort_multipart(&self.bucket, &self.key, &upload);
                Err(error)
            }
        }
    }

    /// Drop the stage without publishing it.
    ///
    /// The lifecycle pair uses this: a pending write on its way to being
    /// deleted must not be flushed, or the removal would race its own
    /// resurrection.
    pub(super) fn discard(&self) -> Result<()> {
        let mut state = self.state()?;
        state.stage = None;
        state.meta = None;
        Ok(())
    }

    /// The staged value from `position`, when a write is waiting to publish.
    ///
    /// A caller must read what it just wrote, and a stream cannot borrow
    /// through this handle's lock, so the pending bytes are copied out. `None`
    /// says nothing is staged and the store is what to read.
    pub(super) fn staged_from(&self, position: u64) -> Result<Option<Vec<u8>>> {
        let state = self.state()?;
        let Some(stage) = state.stage.as_ref() else {
            return Ok(None);
        };
        let start = usize::try_from(position)
            .unwrap_or(usize::MAX)
            .min(stage.bytes.len());
        Ok(Some(stage.bytes[start..].to_vec()))
    }
}

/// An S3 object is the leaf role over the store.
impl IOFile for File {
    fn file_url(&self) -> &Url {
        &self.url
    }

    fn file_exists(&self) -> bool {
        self.state()
            .and_then(|mut state| self.meta(&mut state))
            .is_ok_and(|meta| meta.is_some())
    }

    /// Empty the object, which on a store without a truncate is writing no
    /// bytes to it.
    ///
    /// One `PUT`. This is the one place an S3 leaf departs from
    /// [`IOBase::clear`]'s "clearing is not a write": an object store offers
    /// no way to empty a value that does not create one, and the only way to
    /// find out first is a probe the no-pre-call rule forbids. The sibling
    /// Arrow filesystem backend resolves it the same way.
    fn clear_file(&mut self) -> Result<()> {
        self.discard()?;
        let content_type = self.media_type().to_string();
        self.client
            .put_object(&self.bucket, &self.key, &[], Some(&content_type))?;
        Ok(())
    }

    /// Delete the object, dropping any staged write first.
    ///
    /// One `DELETE`, issued without asking whether the key is there: S3
    /// answers the same either way, which is exactly the removal contract.
    fn delete_file(&mut self) -> Result<()> {
        self.discard()?;
        self.client.delete_object(&self.bucket, &self.key)
    }
}

impl crate::IOMedia for File {
    crate::impl_default_iomedia!();
}

impl IOBase for File {
    /// Read into `buffer` from `offset` with one ranged `GET`.
    ///
    /// A staged write answers from memory instead, because it is what a later
    /// flush will publish and a caller must read what they just wrote.
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        {
            let state = self.state()?;
            if let Some(stage) = state.stage.as_ref() {
                let Ok(offset) = usize::try_from(offset) else {
                    return Ok(0);
                };
                if offset >= stage.bytes.len() {
                    return Ok(0);
                }
                let available = &stage.bytes[offset..];
                let count = available.len().min(buffer.len());
                buffer[..count].copy_from_slice(&available[..count]);
                return Ok(count);
            }
            // A length this scope already established bounds the read without
            // asking, so a scan that runs off the end costs nothing. Only
            // while open: a closed handle keeps nothing, and a size a listing
            // reported is what the store held then, not what it holds now.
            if state.opened {
                if let Some(Some(meta)) = state.meta.as_ref() {
                    if offset >= meta.size {
                        return Ok(0);
                    }
                }
            }
        }
        // The lock is released across the request, so two threads reading one
        // handle overlap on the wire instead of taking turns.
        let (read, total) = self
            .client
            .get_range(&self.bucket, &self.key, offset, buffer)?;
        if let Some(total) = total {
            Self::learn_size(&mut *self.state()?, total);
        }
        Ok(read)
    }

    /// Stream from `position` with one `GET` for the whole drain.
    ///
    /// Not one request per chunk: the response body is read in bounded pieces
    /// from a single connection, which is what makes a compressed or record
    /// scan over an object cost one round trip.
    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<crate::ByteStream<'_>> {
        if batch_size == 0 {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "byte stream batch_size must be greater than zero",
            )));
        }
        let state = self.state()?;
        if state.stage.is_some() {
            drop(state);
            // A staged value is already in memory; stream what will be
            // published rather than what is stored.
            return crate::ByteStream::from_handle(self, position, batch_size);
        }
        drop(state);
        // Resuming, because this is the long transfer: a stream drained over
        // minutes outlives more connections than one request does.
        let reader = self
            .client
            .open_resuming_reader(&self.bucket, &self.key, position, None)?;
        crate::ByteStream::from_reader(reader, batch_size)
    }

    /// Read the whole object with one `GET`.
    ///
    /// The inherited default would ask for the size first and then read it;
    /// here the one request answers both.
    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        if let Some(stage) = self.state()?.stage.as_ref() {
            return Ok(stage.bytes.clone());
        }
        let bytes = self.client.get_all(&self.bucket, &self.key)?;
        Self::learn_size(&mut *self.state()?, bytes.len() as u64);
        Ok(bytes)
    }

    /// Read `length` bytes from `offset` with one ranged `GET`.
    ///
    /// The inherited default clamps against [`IOBase::size`] first, which on a
    /// store is a second round trip; the range answers its own bound. Nothing
    /// allocates `length` on the caller's word either - asking for the rest of
    /// an object of unknown length is ordinary, and a store that holds sixteen
    /// bytes must not be able to cost eight gigabytes of memory to read.
    fn read_range_bytes(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        let bound = {
            let state = self.state()?;
            match state.stage.as_ref() {
                Some(stage) => {
                    let start = usize::try_from(offset)
                        .unwrap_or(usize::MAX)
                        .min(stage.bytes.len());
                    let end = start.saturating_add(length).min(stage.bytes.len());
                    return Ok(stage.bytes[start..end].to_vec());
                }
                None if state.opened => match state.meta.as_ref() {
                    Some(Some(meta)) => Some(meta.size.saturating_sub(offset)),
                    Some(None) => return Ok(Vec::new()),
                    None => None,
                },
                None => None,
            }
        };
        if let Some(bound) = bound {
            let want = usize::try_from(bound).unwrap_or(usize::MAX).min(length);
            if want == 0 {
                return Ok(Vec::new());
            }
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(want)
                .map_err(|_| crate::iobase::oversized(want as u64))?;
            bytes.resize(want, 0);
            let (read, _) = self
                .client
                .get_range(&self.bucket, &self.key, offset, &mut bytes)?;
            bytes.truncate(read);
            return Ok(bytes);
        }
        let (bytes, total) =
            self.client
                .get_range_vec(&self.bucket, &self.key, offset, length as u64)?;
        if let Some(total) = total {
            Self::learn_size(&mut *self.state()?, total);
        }
        Ok(bytes)
    }

    /// Hash `length` bytes from `offset` with one `GET` of exactly that range.
    ///
    /// The inherited default streams from `offset` to the end of the object
    /// and stops reading once it has enough, which on a store means asking for
    /// a gigabyte to hash sixteen bytes of it.
    fn read_range_digest(
        &self,
        offset: u64,
        length: usize,
        algorithm: crate::DigestAlgorithm,
    ) -> Result<crate::Digest> {
        if self.state()?.stage.is_some() {
            // A staged value is already in memory; the inherited walk reads it
            // there, out of `pstream_bytes`, without asking the store.
            return crate::xxhash::stream::read_range_digest(self, offset, length, algorithm);
        }
        let mut digester = algorithm.digester();
        if length == 0 {
            return Ok(digester.as_digest());
        }
        let last = offset.saturating_add(length as u64 - 1);
        let mut reader =
            self.client
                .open_resuming_reader(&self.bucket, &self.key, offset, Some(last))?;
        let mut window = vec![0_u8; crate::DEFAULT_STREAM_BATCH_SIZE.min(length)];
        loop {
            let read = std::io::Read::read(&mut reader, &mut window).map_err(Error::Io)?;
            if read == 0 {
                break;
            }
            digester.write_bytes(&window[..read]);
        }
        Ok(digester.as_digest())
    }

    /// Stage `bytes` at `offset`, loading the stored value once.
    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        let state = self.state()?;
        let mut state = self.materialize(state)?;
        let stage = state.stage.as_mut().ok_or_else(poisoned)?;
        let offset = usize::try_from(offset).map_err(|_| crate::iobase::oversized(offset))?;
        let end = offset
            .checked_add(bytes.len())
            .ok_or_else(|| crate::iobase::oversized(u64::MAX))?;
        if end > stage.bytes.len() {
            resize(&mut stage.bytes, end)?;
        }
        stage.bytes[offset..end].copy_from_slice(bytes);
        stage.dirty = true;
        Ok(bytes.len())
    }

    /// Replace the whole object with one upload.
    ///
    /// Nothing of the stored value survives, so nothing is loaded first: this
    /// is one `PUT`, where staging through [`IOBase::pwrite`] would be a
    /// `GET` and a `PUT`.
    fn write_all_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        let mut state = self.state()?;
        state.stage = Some(Stage {
            bytes: bytes.to_vec(),
            dirty: true,
        });
        self.publish(&mut state)
    }

    /// Append after the current end, answering the offset the bytes start at.
    ///
    /// One `GET` and one `PUT`: S3 has no append, so the value is read,
    /// extended, and replaced.
    fn append_bytes(&mut self, bytes: &[u8]) -> Result<u64> {
        let state = self.state()?;
        let mut state = self.materialize(state)?;
        let stage = state.stage.as_mut().ok_or_else(poisoned)?;
        let offset = stage.bytes.len() as u64;
        stage
            .bytes
            .try_reserve(bytes.len())
            .map_err(|_| crate::iobase::oversized((stage.bytes.len() + bytes.len()) as u64))?;
        stage.bytes.extend_from_slice(bytes);
        stage.dirty = true;
        self.publish(&mut state)?;
        Ok(offset)
    }

    /// The object's byte length: cached while open, otherwise one `HEAD`.
    fn size(&self) -> u64 {
        let Ok(mut state) = self.state() else {
            return 0;
        };
        if let Some(stage) = state.stage.as_ref() {
            return stage.bytes.len() as u64;
        }
        self.meta(&mut state)
            .ok()
            .flatten()
            .map_or(0, |meta| meta.size)
    }

    /// The staged allocation, or the stored length when nothing is staged.
    fn capacity(&self) -> u64 {
        match self.state() {
            Ok(state) => match state.stage.as_ref() {
                Some(stage) => stage.bytes.capacity() as u64,
                None => {
                    drop(state);
                    self.size()
                }
            },
            Err(_) => 0,
        }
    }

    /// Grow the staged buffer so a later positional write does not have to.
    ///
    /// No request, and no object: a store has no allocation to reserve, and
    /// reserving never changes a length, so nothing is staged that was not
    /// staged already and nothing is published. Reserving on a handle that has
    /// written nothing yet is a hint with nowhere to land, which is success.
    fn reserve(&mut self, capacity: u64) -> Result<()> {
        let mut state = self.state()?;
        let Some(stage) = state.stage.as_mut() else {
            return Ok(());
        };
        let capacity = usize::try_from(capacity).map_err(|_| crate::iobase::oversized(capacity))?;
        if capacity > stage.bytes.capacity() {
            stage
                .bytes
                .try_reserve_exact(capacity - stage.bytes.len())
                .map_err(|_| crate::iobase::oversized(capacity as u64))?;
        }
        Ok(())
    }

    /// Set the staged length, publishing nothing until a flush.
    ///
    /// Truncating to zero costs no request at all: nothing of the stored value
    /// survives it, so there is nothing to load.
    fn truncate(&mut self, size: u64) -> Result<()> {
        let size = usize::try_from(size).map_err(|_| crate::iobase::oversized(size))?;
        if size == 0 {
            let mut state = self.state()?;
            state.stage = Some(Stage {
                bytes: Vec::new(),
                dirty: true,
            });
            return Ok(());
        }
        let state = self.state()?;
        let mut state = self.materialize(state)?;
        let stage = state.stage.as_mut().ok_or_else(poisoned)?;
        resize(&mut stage.bytes, size)?;
        stage.dirty = true;
        Ok(())
    }

    fn url(&self) -> Option<&Url> {
        Some(&self.url)
    }

    fn media_type(&self) -> &MediaType {
        if let Some(declared) = &self.declared {
            return declared;
        }
        // Inferred from the key rather than the bytes: the object may not
        // exist yet, and its name is the only free evidence there is.
        self.inferred.get_or_init(|| {
            if self.url.extension().is_none() {
                return MediaType::from(MimeType::FILE);
            }
            self.url.media_type()
        })
    }

    fn set_media_type(&mut self, media_type: MediaType) {
        self.declared = Some(media_type);
    }

    fn kind(&self) -> crate::IOKind {
        // A staged write has already decided this location is an object, even
        // though publication waits for a flush or a close.
        if let Ok(state) = self.state() {
            if state.stage.as_ref().is_some_and(|stage| stage.dirty) {
                return crate::IOKind::File;
            }
        }
        self.file_kind()
    }

    /// A leaf never contains anything, which needs no request to establish.
    ///
    /// The inherited default would settle this from [`IOBase::kind`], and on a
    /// store that kind is a `HEAD`; the role is static, so the answer is too.
    fn is_container(&self) -> bool {
        false
    }

    fn is_atomic(&self) -> bool {
        self.file_is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.file_is_tabular()
    }

    fn flush(&mut self) -> Result<()> {
        let mut state = self.state()?;
        self.publish(&mut state)
    }

    /// Cache the object's metadata for this scope.
    ///
    /// One `HEAD`, and never the bytes: an opened multi-gigabyte object costs
    /// one small request, not a download. Opening one that does not exist yet
    /// succeeds without creating it.
    fn open(&mut self) -> Result<()> {
        let mut state = self.state()?;
        // Opening what is already open asks nothing: a caller that opens
        // defensively before each of a hundred reads pays for one `HEAD`.
        if state.opened && state.meta.is_some() {
            return Ok(());
        }
        state.opened = true;
        let meta = self.client.head_object(&self.bucket, &self.key)?;
        state.meta = Some(meta);
        Ok(())
    }

    fn opened(&self) -> bool {
        self.state().is_ok_and(|state| state.opened)
    }

    /// Publish anything staged and drop what was cached.
    fn close(&mut self) -> Result<()> {
        let mut state = self.state()?;
        self.publish(&mut state)?;
        state.stage = None;
        state.meta = None;
        state.opened = false;
        Ok(())
    }

    fn parent(&self) -> Option<Holder> {
        let parent = self.url.parent()?;
        Folder::new(self.client.clone(), parent)
            .ok()
            .map(Holder::ObjectFolder)
    }

    fn clear(&mut self) -> Result<()> {
        self.clear_file()
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.file_remove(recursive)
    }

    fn child_by_path(&self, name: &str) -> Result<Holder> {
        self.file_child_by_path(name)
    }

    fn ls(&self, _recursive: bool, _include_private: bool) -> Listing {
        self.file_ls()
    }
}

impl Drop for File {
    fn drop(&mut self) {
        // Publish a staged write; a failure here cannot be reported, and
        // callers who care call `flush` or `close` explicitly.
        if let Ok(mut state) = self.state.lock() {
            let _ = self.publish(&mut state);
        }
    }
}

impl std::fmt::Debug for File {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("File")
            .field("url", &self.url)
            .finish()
    }
}

/// Resize a staged value, refusing rather than aborting when it will not fit.
fn resize(bytes: &mut Vec<u8>, size: usize) -> Result<()> {
    if size <= bytes.len() {
        bytes.truncate(size);
        return Ok(());
    }
    bytes
        .try_reserve(size - bytes.len())
        .map_err(|_| crate::iobase::oversized(size as u64))?;
    bytes.resize(size, 0);
    Ok(())
}

/// Report a value needing more parts than S3 allows.
fn too_many_parts() -> Error {
    Error::Io(std::io::Error::other(
        "expected a value of at most 10,000 multipart parts; raise the part size to upload it",
    ))
}

/// Report a poisoned state lock without panicking a caller.
fn poisoned() -> Error {
    Error::Io(std::io::Error::other(
        "the S3 object lock was poisoned by a panicking writer",
    ))
}
