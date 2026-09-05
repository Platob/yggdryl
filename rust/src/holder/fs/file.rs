//! A staged whole-value file over a foreign filesystem.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::holder::Holder;
use crate::{Error, MediaType, MimeType, Result, Url};
use crate::{IOBase, IOFile, Listing};

use super::system::{FileSystem, filesystem_location};
use super::{Folder, location_url};

/// A leaf on a foreign filesystem, staged in memory between writes.
///
/// Reads map straight through: [`IOBase::pread`] is one
/// [`FileSystem::read_range`] - an S3 range GET - and [`IOBase::size`]
/// one [`FileSystem::file_info`], so asking for a range transfers that
/// range rather than the value. What a reader above does with that is its
/// own business: the Parquet reader still fetches whole, as it documents.
///
/// Writes cannot map through, because an Arrow filesystem
/// replaces whole files while [`IOBase::pwrite`] is positional; they stage,
/// and one [`FileSystem::write_full`] publishes the staged value on
/// [`IOBase::flush`] or [`IOBase::close`]. Until then the stored value is
/// unchanged, which is why a file meant for another reader is written inside
/// an open/close pair.
///
/// Like every handle it is lazy: construction touches nothing, a missing file
/// reads as empty, and the first published write creates the file and its
/// parents.
pub struct File {
    filesystem: Arc<dyn FileSystem>,
    url: Url,
    /// The filesystem-relative spelling of `url`, derived once so every
    /// vtable call names the same path.
    location: String,
    /// An explicit media type overrides inference from the location.
    declared: Option<MediaType>,
    /// Inference from the location's compound filename, computed on demand.
    inferred: OnceLock<MediaType>,
    /// The staged whole value. An Arrow filesystem replaces a file rather
    /// than writing a range, so positional mutations must land in memory -
    /// loaded from the stored value on first write when one exists - and
    /// publish as exactly one `write_full` on flush or close.
    stage: Mutex<Option<Stage>>,
}

/// The staged bytes and whether the filesystem has seen them.
struct Stage {
    bytes: Vec<u8>,
    dirty: bool,
}

/// Report a poisoned stage lock without panicking a caller.
fn poisoned() -> Error {
    Error::Io(std::io::Error::other(
        "the staged file lock was poisoned by a panicking writer",
    ))
}

/// Resize the staged value to `size` bytes, zero-filling any growth.
///
/// The growth is a *fallible* reservation rather than [`Vec::resize`] alone,
/// because `resize` aborts the process when the allocation fails - and the
/// size here comes from a caller's offset or a foreign filesystem's reported
/// length, so an over-budget value has to be a typed refusal rather than a
/// crash. [`crate::iobase::oversized`] is the shared spelling of that refusal.
fn resize_stage(bytes: &mut Vec<u8>, size: usize) -> Result<()> {
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

impl File {
    /// Describe a leaf on `filesystem` without touching it.
    pub fn new(filesystem: Arc<dyn FileSystem>, url: Url) -> Self {
        let location = filesystem_location(&url);
        Self {
            filesystem,
            url,
            location,
            declared: None,
            inferred: OnceLock::new(),
            stage: Mutex::new(None),
        }
    }

    /// Describe a leaf named by a filesystem-relative location.
    ///
    /// # Errors
    ///
    /// Returns an error when `location` cannot form a canonical URL.
    pub fn from_location(filesystem: Arc<dyn FileSystem>, location: &str) -> Result<Self> {
        let url = location_url(filesystem.as_ref(), location)?;
        Ok(Self::new(filesystem, url))
    }

    /// Borrow the foreign filesystem this leaf lives on.
    pub fn filesystem(&self) -> &Arc<dyn FileSystem> {
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

    /// Return whether the file exists on the filesystem yet.
    ///
    /// A staged write that has not been published does not count: existence
    /// is what the filesystem reports right now.
    pub fn exists(&self) -> bool {
        self.file_exists()
    }

    /// Lock the stage, materializing the stored value into it once.
    ///
    /// The whole value is loaded so positional writes have something to land
    /// in; the shared size budget refuses a value the address space cannot
    /// hold rather than attempting the allocation.
    /// Drop the staged value *without* publishing it.
    ///
    /// The lifecycle pair uses this rather than `close`: a pending write on its
    /// way to being deleted must not be flushed, or the removal would race its
    /// own resurrection.
    fn discard(&self) -> Result<()> {
        let mut stage = self.stage.lock().map_err(|_| poisoned())?;
        *stage = None;
        Ok(())
    }

    fn materialize(&self) -> Result<MutexGuard<'_, Option<Stage>>> {
        let mut stage = self.stage.lock().map_err(|_| poisoned())?;
        if stage.is_none() {
            let info = self.filesystem.file_info(&self.location)?;
            let bytes = if info.kind == crate::IOKind::File && info.size > 0 {
                let size =
                    usize::try_from(info.size).map_err(|_| crate::iobase::oversized(info.size))?;
                // The reported length is the foreign filesystem's, so a value
                // too large for this process is refused rather than attempted.
                let mut bytes = Vec::new();
                resize_stage(&mut bytes, size)?;
                let mut filled = 0;
                while filled < bytes.len() {
                    let read = self.filesystem.read_range(
                        &self.location,
                        filled as u64,
                        &mut bytes[filled..],
                    )?;
                    if read == 0 {
                        break;
                    }
                    filled += read;
                }
                bytes.truncate(filled);
                bytes
            } else {
                Vec::new()
            };
            *stage = Some(Stage {
                bytes,
                dirty: false,
            });
        }
        Ok(stage)
    }

    /// Publish the staged value when it holds unseen changes.
    fn publish(
        stage: &mut Option<Stage>,
        filesystem: &dyn FileSystem,
        location: &str,
    ) -> Result<()> {
        if let Some(stage) = stage.as_mut() {
            if stage.dirty {
                filesystem.write_full(location, &stage.bytes)?;
                stage.dirty = false;
            }
        }
        Ok(())
    }
}

/// A staged foreign leaf is the leaf role over a filesystem.
impl IOFile for File {
    fn file_url(&self) -> &Url {
        &self.url
    }

    fn file_exists(&self) -> bool {
        self.filesystem
            .file_info(&self.location)
            .is_ok_and(|info| info.kind == crate::IOKind::File)
    }

    /// Replace the object with an empty one, dropping any staged write.
    ///
    /// An Arrow filesystem replaces whole objects rather than resizing them, so
    /// emptying one *is* writing zero bytes to it. Nothing is probed first: the
    /// write either lands or reports the backend's own failure.
    fn clear_file(&mut self) -> Result<()> {
        self.discard()?;
        self.filesystem.write_full(&self.location, &[])
    }

    /// Delete the object, dropping any staged write first.
    ///
    /// Discarding the stage is part of the removal: an unflushed write must not
    /// survive to recreate the object on a later close. The delete is issued
    /// unconditionally and the backend's own not-found answer is the success.
    fn delete_file(&mut self) -> Result<()> {
        self.discard()?;
        crate::iobase::skip_absent(
            self.filesystem
                .delete_file(&self.location)
                .map_err(std::io::Error::other),
        )
    }
}

impl crate::IOMedia for File {
    crate::impl_default_iomedia!();
}

impl IOBase for File {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        let stage = self.stage.lock().map_err(|_| poisoned())?;
        let Some(stage) = stage.as_ref() else {
            // No stage: the read maps straight onto one ranged fetch - never
            // a whole-value download to serve one range.
            return self.filesystem.read_range(&self.location, offset, buffer);
        };
        let Ok(offset) = usize::try_from(offset) else {
            return Ok(0);
        };
        if offset >= stage.bytes.len() {
            return Ok(0);
        }
        let available = &stage.bytes[offset..];
        let count = available.len().min(buffer.len());
        buffer[..count].copy_from_slice(&available[..count]);
        Ok(count)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        let mut stage = self.materialize()?;
        let stage = stage.as_mut().ok_or_else(poisoned)?;
        let offset = usize::try_from(offset).map_err(|_| crate::iobase::oversized(offset))?;
        let end = offset
            .checked_add(bytes.len())
            .ok_or_else(|| crate::iobase::oversized(u64::MAX))?;
        if end > stage.bytes.len() {
            // Growing zero-fills any gap the offset created.
            resize_stage(&mut stage.bytes, end)?;
        }
        stage.bytes[offset..end].copy_from_slice(bytes);
        stage.dirty = true;
        Ok(bytes.len())
    }

    fn size(&self) -> u64 {
        if let Ok(stage) = self.stage.lock() {
            if let Some(stage) = stage.as_ref() {
                return stage.bytes.len() as u64;
            }
        }
        // Not staged: one metadata call, and a missing file is empty.
        self.filesystem
            .file_info(&self.location)
            .map_or(0, |info| info.size)
    }

    fn capacity(&self) -> u64 {
        if let Ok(stage) = self.stage.lock() {
            if let Some(stage) = stage.as_ref() {
                return stage.bytes.capacity() as u64;
            }
        }
        self.size()
    }

    fn reserve(&mut self, capacity: u64) -> Result<()> {
        let mut stage = self.materialize()?;
        let stage = stage.as_mut().ok_or_else(poisoned)?;
        let capacity = usize::try_from(capacity).map_err(|_| crate::iobase::oversized(capacity))?;
        if capacity > stage.bytes.capacity() {
            stage
                .bytes
                .try_reserve_exact(capacity - stage.bytes.len())
                .map_err(|error| {
                    Error::Io(std::io::Error::other(format!(
                        "unable to reserve {capacity} staged bytes: {error}"
                    )))
                })?;
        }
        // Reserving is a write: it creates the resource on publication.
        stage.dirty = true;
        Ok(())
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        let mut stage = self.materialize()?;
        let stage = stage.as_mut().ok_or_else(poisoned)?;
        let size = usize::try_from(size).map_err(|_| crate::iobase::oversized(size))?;
        // Extending zero-fills rather than leaving stale bytes visible, and
        // refuses loudly rather than aborting on an over-budget length.
        resize_stage(&mut stage.bytes, size)?;
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
        // Inferred from the location rather than the content: the file may
        // not exist yet, and its name is the only evidence available.
        self.inferred.get_or_init(|| {
            // A name that says nothing still says this is a stored file.
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
        // A staged write already decided this location is a file, even though
        // publication waits for flush or close.
        if let Ok(stage) = self.stage.lock() {
            if stage.as_ref().is_some_and(|stage| stage.dirty) {
                return crate::IOKind::File;
            }
        }
        self.file_kind()
    }

    fn is_atomic(&self) -> bool {
        self.file_is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.file_is_tabular()
    }

    fn flush(&mut self) -> Result<()> {
        let mut stage = self.stage.lock().map_err(|_| poisoned())?;
        Self::publish(&mut stage, self.filesystem.as_ref(), &self.location)
    }

    fn open(&mut self) -> Result<()> {
        // Opening materializes the staged value so repeated reads stop paying
        // one ranged fetch each; it never creates - publication still waits
        // for a write and a flush or close.
        drop(self.materialize()?);
        Ok(())
    }

    fn opened(&self) -> bool {
        self.stage.lock().is_ok_and(|stage| stage.is_some())
    }

    fn close(&mut self) -> Result<()> {
        let mut stage = self.stage.lock().map_err(|_| poisoned())?;
        Self::publish(&mut stage, self.filesystem.as_ref(), &self.location)?;
        // Release the stage; a later operation refetches, never a stale copy.
        *stage = None;
        Ok(())
    }

    fn parent(&self) -> Option<Holder> {
        let parent = self.url.parent()?;
        Some(Holder::FsFolder(Folder::new(
            self.filesystem.clone(),
            parent,
        )))
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
        // Publish any staged write; a failure here cannot be reported, and
        // callers who care call `flush` or `close` explicitly.
        if let Ok(mut stage) = self.stage.lock() {
            let _ = Self::publish(&mut stage, self.filesystem.as_ref(), &self.location);
        }
    }
}

impl std::fmt::Debug for File {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("File")
            .field("filesystem", &self.filesystem.type_name())
            .field("url", &self.url)
            .finish()
    }
}
