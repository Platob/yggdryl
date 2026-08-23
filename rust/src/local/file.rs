//! A lazy, auto-resizing memory-mapped local file [`IOBase`].
//!
//! # Unsafe
//!
//! This is the only module in Yggdryl that uses `unsafe`, and it uses it once,
//! for `memmap2`'s mapping constructor. That is `unsafe` for a reason no
//! wrapper can remove: a mapping aliases file bytes, so if another process
//! truncates the file while a mapping is live, touching the lost pages raises
//! SIGBUS rather than returning an error. Yggdryl cannot prevent that, so
//! [`File`] documents the hazard
//! instead of pretending it away. Use [`super::Buffer`] when the file may
//! change underneath you.

#![allow(unsafe_code)]

use std::fs::{File as StdFile, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use memmap2::MmapMut;

use crate::{Error, MediaType, MimeType, Result, Url};

use crate::generic::Holder;
use crate::io::{IOBase, IOFile};

/// Growth is geometric so repeated appends do not remap on every write.
const MINIMUM_GROWTH: u64 = 64 * 1024;

/// The file and its mapping, materialized on first use.
///
/// The mapping is optional because Windows refuses to resize a file while a
/// mapped section is open, so publishing the logical length must release it
/// first. Any later access re-establishes it.
struct Mapped {
    file: StdFile,
    mapping: Option<MmapMut>,
    size: u64,
}

/// A lazily mapped local file addressed by offset.
///
/// Construction touches nothing: [`File::new`] only records the path. The file
/// is opened and mapped on the first operation that needs it, and per the
/// [`IOBase`] laziness contract a read of a missing file yields zero bytes
/// while a write creates it along with any missing parent directory.
///
/// The mapping covers the file's capacity, while [`IOBase::size`] tracks the
/// logical length. Writing past the mapping remaps at a larger capacity, so
/// appending does not remap on every write.
///
/// # Safety
///
/// See the module documentation: a concurrent external truncation of the
/// mapped file can raise SIGBUS.
pub struct File {
    path: PathBuf,
    url: Url,
    /// An explicit media type overrides inference from the location.
    declared: Option<MediaType>,
    /// Inference from the path's compound filename, computed on demand.
    inferred: OnceLock<MediaType>,
    /// `None` until an operation materializes the mapping.
    state: Mutex<Option<Mapped>>,
}

impl File {
    /// Describe a mapped file without touching it.
    ///
    /// The path need not exist. Reads before it does yield nothing; the first
    /// write creates it.
    ///
    /// # Errors
    ///
    /// Returns an error only when the path cannot be expressed as a canonical
    /// `file:` URL.
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let url = Url::from_path(&path)?;
        Ok(Self {
            path,
            url,
            declared: None,
            inferred: OnceLock::new(),
            state: Mutex::new(None),
        })
    }

    /// Describe a mapped file whose existing contents are discarded on first use.
    ///
    /// Truncation is deferred like every other operation, so this still
    /// touches nothing until the handle is used.
    ///
    /// # Errors
    ///
    /// Returns an error only when the path cannot be expressed as a canonical
    /// `file:` URL.
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let mut mapped = Self::new(path)?;
        mapped.truncate(0)?;
        Ok(mapped)
    }

    /// Borrow the described path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return whether the file exists yet.
    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// Drop the descriptor and mapping *without* publishing anything.
    ///
    /// The lifecycle pair uses this rather than `close`: a pending write must
    /// not be flushed on its way to being deleted, or the removal would race
    /// its own resurrection.
    fn release(&mut self) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        *state = None;
        Ok(())
    }

    /// Materialize the mapping, creating the file when `create` is set.
    ///
    /// Returns `Ok(false)` when the file does not exist and creation was not
    /// requested, which is how a read of a missing file becomes empty rather
    /// than an error.
    ///
    /// The open *is* the existence question, per the existence contract: one
    /// attempt, then the failure is the answer. A read that finds nothing is
    /// empty; a write whose parent directory is missing repairs that ancestry
    /// once and retries the open exactly once, and a second absence is
    /// reported as it is, naming what the repair created.
    fn materialize(state: &mut Option<Mapped>, path: &Path, create: bool) -> Result<bool> {
        if state.is_some() {
            return Ok(true);
        }
        let file = match Self::open_at(path, create) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if !create {
                    // Absence is emptiness on the read side.
                    return Ok(false);
                }
                // `create(true)` still fails when the *parent* is missing, so
                // that is the only absence left to repair.
                let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                else {
                    return Err(Error::from_io_at(error, "file", path.display()));
                };
                std::fs::create_dir_all(parent)?;
                Self::open_at(path, create).map_err(|retry| {
                    if retry.kind() == std::io::ErrorKind::NotFound {
                        Error::absent(
                            "file",
                            format!(
                                "{} (its parent {} was created)",
                                path.display(),
                                parent.display()
                            ),
                        )
                    } else {
                        Error::Io(retry)
                    }
                })?
            }
            Err(error) => return Err(Error::from_io_at(error, "file", path.display())),
        };
        let size = file.metadata()?.len();
        *state = Some(Mapped {
            file,
            mapping: None,
            size,
        });
        Ok(true)
    }

    /// One open attempt, with no question asked first.
    fn open_at(path: &Path, create: bool) -> std::io::Result<StdFile> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(create)
            .truncate(false)
            .open(path)
    }
}

impl Mapped {
    /// Ensure a mapping exists covering at least `needed` bytes.
    fn remap(&mut self, needed: u64) -> Result<&mut MmapMut> {
        let current = self
            .mapping
            .as_ref()
            .map_or(0, |mapping| mapping.len() as u64);
        if self.mapping.is_some() && current >= needed {
            return self.mapping.as_mut().ok_or_else(poisoned);
        }
        // Double, so a sequence of appends remaps a logarithmic number of times.
        let capacity = needed.max(current * 2).max(MINIMUM_GROWTH);
        // Windows cannot resize a file with a live mapped section.
        if let Some(mapping) = self.mapping.take() {
            mapping.flush()?;
        }
        self.file.set_len(capacity)?;
        self.mapping = Some(map_file(&self.file)?);
        self.mapping.as_mut().ok_or_else(poisoned)
    }

    /// Map the file exactly as it is, without resizing it.
    ///
    /// Reading must never change a file's length, so this is the read path;
    /// [`Self::remap`] - which grows the file - is only for writes.
    fn map_existing(&mut self) -> Result<&mut MmapMut> {
        if self.mapping.is_none() {
            self.mapping = Some(map_file(&self.file)?);
        }
        self.mapping.as_mut().ok_or_else(poisoned)
    }

    /// Publish the logical length, releasing the mapping so the file can shrink.
    fn publish(&mut self) -> Result<()> {
        if let Some(mapping) = self.mapping.take() {
            mapping.flush()?;
        }
        self.file.set_len(self.size)?;
        Ok(())
    }
}

/// Map a file for shared read/write access.
fn map_file(file: &StdFile) -> Result<MmapMut> {
    // SAFETY: `memmap2` requires this to be `unsafe` because the mapping
    // aliases file bytes that another process could truncate, which would turn
    // a later access into SIGBUS. Yggdryl cannot rule that out, so the hazard is
    // documented on `File` rather than hidden. Nothing else about the call is unsound: the file
    // handle is owned alongside the mapping and outlives it.
    unsafe { MmapMut::map_mut(file) }.map_err(Error::Io)
}

/// Report a poisoned lock without panicking a caller.
fn poisoned() -> Error {
    Error::Io(std::io::Error::other(
        "the memory-mapped file lock was poisoned by a panicking writer",
    ))
}

/// A memory-mapped file is the leaf role over the local file system.
impl IOFile for File {
    fn file_url(&self) -> &Url {
        &self.url
    }

    fn file_exists(&self) -> bool {
        self.path.exists()
    }

    /// Truncate the file in place, without creating one that is not there.
    ///
    /// One `open` with `truncate(true)` and no `create`, so a missing file
    /// answers `NotFound` and that answer *is* the no-op success. Nothing
    /// probes first. The mapping is released beforehand, because a mapping
    /// must never outlive the length it was taken over.
    fn clear_file(&mut self) -> Result<()> {
        self.release()?;
        crate::io::skip_absent(
            OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&self.path)
                .map(|_| ()),
        )
    }

    /// Unlink the file, dropping the mapping first.
    ///
    /// Releasing the mapping is part of the removal, not housekeeping around
    /// it: an unpublished write held in the mapping must not survive to
    /// recreate the file on a later flush, and no mapping may outlive the file
    /// it maps. `std::fs::remove_file` is issued unconditionally and its own
    /// `NotFound` is the success answer.
    fn delete_file(&mut self) -> Result<()> {
        self.release()?;
        crate::io::skip_absent(std::fs::remove_file(&self.path))
    }
}

impl crate::io::IOMedia for File {
    crate::impl_default_iomedia!();
}

impl IOBase for File {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        // A missing file reads as empty rather than failing.
        if !Self::materialize(&mut state, &self.path, false)? {
            return Ok(0);
        }
        let Some(mapped) = state.as_mut() else {
            return Ok(0);
        };
        let Ok(offset) = usize::try_from(offset) else {
            return Ok(0);
        };
        let size = usize::try_from(mapped.size).unwrap_or(usize::MAX);
        if offset >= size {
            return Ok(0);
        }
        let mapping = mapped.map_existing()?;
        let available = &mapping[offset..size];
        let count = available.len().min(buffer.len());
        buffer[..count].copy_from_slice(&available[..count]);
        Ok(count)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        // A write creates the file and any parent it needs.
        Self::materialize(&mut state, &self.path, true)?;
        let mapped = state.as_mut().ok_or_else(poisoned)?;

        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| crate::io::oversized(u64::MAX))?;
        let previous = mapped.size;
        let mapping = mapped.remap(end)?;
        let start = usize::try_from(offset).map_err(|_| crate::io::oversized(offset))?;
        let finish = usize::try_from(end).map_err(|_| crate::io::oversized(end))?;
        // Zero-fill any gap the offset created before writing.
        if offset > previous {
            let gap = usize::try_from(previous).unwrap_or(usize::MAX);
            mapping[gap..start].fill(0);
        }
        mapping[start..finish].copy_from_slice(bytes);
        mapped.size = previous.max(end);
        Ok(bytes.len())
    }

    fn size(&self) -> u64 {
        if let Ok(state) = self.state.lock() {
            if let Some(mapped) = state.as_ref() {
                return mapped.size;
            }
        }
        // Not materialized: a missing file is empty, an existing one reports
        // its on-disk length without mapping it.
        std::fs::metadata(&self.path).map_or(0, |metadata| metadata.len())
    }

    fn capacity(&self) -> u64 {
        self.state.lock().map_or(0, |state| {
            state.as_ref().map_or(0, |mapped| {
                mapped
                    .mapping
                    .as_ref()
                    .map_or(mapped.size, |mapping| mapping.len() as u64)
            })
        })
    }

    fn reserve(&mut self, capacity: u64) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        Self::materialize(&mut state, &self.path, true)?;
        state.as_mut().ok_or_else(poisoned)?.remap(capacity)?;
        Ok(())
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        Self::materialize(&mut state, &self.path, true)?;
        let mapped = state.as_mut().ok_or_else(poisoned)?;
        if size > mapped.size {
            let previous = mapped.size;
            let mapping = mapped.remap(size)?;
            let from = usize::try_from(previous).unwrap_or(usize::MAX);
            let to = usize::try_from(size).map_err(|_| crate::io::oversized(size))?;
            mapping[from..to].fill(0);
        }
        mapped.size = size;
        Ok(())
    }

    fn url(&self) -> Option<&Url> {
        Some(&self.url)
    }

    fn kind(&self) -> crate::IOKind {
        // A path that does not exist yet has not decided what it is.
        self.file_kind()
    }

    fn is_atomic(&self) -> bool {
        self.file_is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.file_is_tabular()
    }

    fn media_type(&self) -> &MediaType {
        if let Some(declared) = &self.declared {
            return declared;
        }
        // Inferred from the location rather than the content: the file may not
        // exist yet, and its name is the only evidence available.
        self.inferred.get_or_init(|| {
            // A name that says nothing still says this is a local file.
            if self.url.extension().is_none() {
                return MediaType::from(MimeType::FILE);
            }
            self.url.media_type()
        })
    }

    fn set_media_type(&mut self, media_type: MediaType) {
        self.declared = Some(media_type);
    }

    fn flush(&mut self) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        match state.as_mut() {
            // Never materialized means nothing to publish.
            None => Ok(()),
            Some(mapped) => mapped.publish(),
        }
    }

    fn open(&mut self) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        // Opening never creates: a handle for a file that does not exist yet
        // stays unmaterialized until the first write.
        Self::materialize(&mut state, &self.path, false)?;
        Ok(())
    }

    fn opened(&self) -> bool {
        self.state.lock().is_ok_and(|state| state.is_some())
    }

    fn close(&mut self) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        if let Some(mapped) = state.as_mut() {
            mapped.publish()?;
        }
        // Drop the file handle and mapping; a later operation re-materializes.
        *state = None;
        Ok(())
    }

    fn parent(&self) -> Option<Holder> {
        let parent = self.path.parent()?;
        if parent.as_os_str().is_empty() {
            return None;
        }
        Holder::folder(parent).ok()
    }

    fn clear(&mut self) -> Result<()> {
        self.clear_file()
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.file_remove(recursive)
    }
}

impl Drop for File {
    fn drop(&mut self) {
        // Publish the logical length; a failure here cannot be reported, and
        // callers who care call `flush` explicitly.
        if let Ok(mut state) = self.state.lock() {
            if let Some(mapped) = state.as_mut() {
                let _ = mapped.publish();
            }
        }
    }
}

impl std::fmt::Debug for File {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("File")
            .field("url", &self.url)
            .field("size", &self.size())
            .finish()
    }
}
