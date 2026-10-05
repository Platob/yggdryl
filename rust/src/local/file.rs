//! A lazy, auto-resizing memory-mapped local file [`IOBase`].
//!
//! # Concurrent readers
//!
//! A handle reads the file it mapped, so what another handle does to that
//! file decides whether the reader keeps reading:
//!
//! - a whole-value replace - [`IOBase::write_all_bytes`] and
//!   [`IOBase::clear`] - is safe: the value is written to a private sibling
//!   in the same folder and renamed over the path, so a handle that mapped
//!   the old file keeps reading the old bytes whole and a handle opened
//!   after the rename reads the new value;
//! - a create - [`IOBase::create_bytes`] - is safe: the whole value is
//!   linked at the path in one step, so a reader finds no file or all of it;
//! - an in-place [`IOBase::truncate`] that shortens the file is not: the
//!   flush that publishes it drops pages a reader may still touch;
//! - a [`IOBase::pwrite`] past the end - and so [`IOBase::append_bytes`] -
//!   is not: the flush trims the mapping's growth slack, which a reader that
//!   opened while it was on disk may still touch.
//!
//! A handle's writes follow the path: before [`IOBase::pwrite`],
//! [`IOBase::append_bytes`], [`IOBase::truncate`], [`IOBase::reserve`],
//! [`IOBase::flush`] or [`IOBase::close`] touches a file this handle already
//! holds, one `stat` asks the path which file it names, and a file another
//! handle replaced or removed is let go unpublished, so the write lands
//! where an in-place write would have; reads keep the file they mapped until
//! the handle closes. The device and inode say which file a descriptor is
//! on Unix; on Windows, where the standard library states no file identity,
//! nothing is asked and a handle keeps its file until it closes.
//!
//! On Unix the rename always replaces the name, and the old file lives on
//! for every handle that holds it. On Windows it replaces a file another
//! handle holds open where the system renames by POSIX semantics (current
//! Windows on NTFS), but a live mapping of that file can make the system
//! refuse: the replace then fails - the previous value standing, the
//! sibling removed - rather than faulting the reader.
//!
//! # Unsafe
//!
//! `unsafe` lives in four modules of Yggdryl, each under a paragraph like
//! this one: here, in `spill.rs`, and in the byte leaves `serie/bytes.rs`
//! and `serie/variant.rs`. Here it is used once, for `memmap2`'s mapping
//! constructor. That is `unsafe` for a reason no wrapper can remove: a
//! mapping aliases file bytes, so if another process - or another handle,
//! by one of the in-place operations above - shortens the file while a
//! mapping is live, touching the lost pages raises SIGBUS rather than
//! returning an error. Yggdryl cannot prevent that for a named file,
//! so [`LocalFile`] documents the hazard instead of pretending it away (a
//! spill file is unlinked before it is mapped, which is why `spill.rs` does
//! not carry it). Use [`super::Buffer`] when the file may change underneath
//! you.

#![allow(unsafe_code)]

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use memmap2::MmapMut;

use crate::{Error, MediaType, MimeType, Result, Uri, Url};

use crate::holder::Holder;
use crate::{IOBase, IOFile};

/// Growth is geometric so repeated appends do not remap on every write.
const MINIMUM_GROWTH: u64 = 64 * 1024;

/// The number the next whole-value replace names its sibling by, so no two
/// replaces of one process share one.
static SIBLINGS: AtomicU64 = AtomicU64::new(0);

/// How many sibling names a replace tries before reporting them all taken:
/// a name is taken only by a sibling a dead process of the same id left, or
/// by a process of the same id in another PID namespace.
const SIBLING_ATTEMPTS: u64 = 16;

/// How many symbolic links a whole-value write follows from the path to the
/// file it replaces, as Linux bounds a lookup.
const LINK_HOPS: usize = 40;

/// The file and its mapping, materialized on first use.
///
/// The mapping is optional because Windows refuses to resize a file while a
/// mapped section is open, so publishing the logical length must release it
/// first. Any later access re-establishes it.
struct Mapped {
    file: File,
    mapping: Option<MmapMut>,
    size: u64,
    dirty: bool,
    /// Which file `file` is - its device and inode on Unix, `None` where
    /// the platform states none - read from the `fstat` the open already
    /// makes, and compared with the path before a write.
    identity: Option<(u64, u64)>,
}

/// A lazily mapped local file addressed by offset.
///
/// Construction touches nothing: [`LocalFile::new`] only records the path.
/// The file is opened and mapped on the first operation that needs it, and
/// per the [`IOBase`] laziness contract a read of a missing file yields zero
/// bytes while a write creates it along with any missing parent directory.
///
/// The mapping covers the file's capacity, while [`IOBase::size`] tracks the
/// logical length. Writing past the mapping remaps at a larger capacity, so
/// appending does not remap on every write.
///
/// # Safety
///
/// See the module documentation: shortening the mapped file in place - by
/// another process, or by another handle's [`IOBase::truncate`] or trimming
/// flush - can raise SIGBUS in a handle reading it; a whole-value replace
/// cannot.
pub struct LocalFile {
    path: PathBuf,
    url: Url,
    /// An explicit media type overrides inference from the location.
    declared: Option<MediaType>,
    /// Inference from the path's compound filename, computed on demand.
    inferred: OnceLock<MediaType>,
    /// `None` until an operation materializes the mapping.
    state: Mutex<Option<Mapped>>,
}

impl LocalFile {
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
        let file = if create {
            Self::open_repairing(path, |path| Self::open_at(path, true))?
        } else {
            match Self::open_at(path, false) {
                Ok(file) => file,
                // Absence is emptiness on the read side.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(Error::from_io_at(error, "file", path.display())),
            }
        };
        let metadata = file.metadata()?;
        *state = Some(Mapped {
            file,
            mapping: None,
            size: metadata.len(),
            dirty: false,
            identity: identity(&metadata),
        });
        Ok(true)
    }

    /// Let go of a held file the path no longer names - another handle
    /// replaced or removed it - unpublished, so the write that follows lands
    /// in the file the path names, as an in-place write would have, and not
    /// in one nothing reads by the path any more.
    ///
    /// One `stat` of the path where the held file states an identity (Unix);
    /// elsewhere nothing is asked.
    fn drop_if_replaced(state: &mut Option<Mapped>, path: &Path) {
        let Some(held) = state.as_ref().and_then(|mapped| mapped.identity) else {
            return;
        };
        let named = std::fs::metadata(path)
            .ok()
            .and_then(|metadata| identity(&metadata));
        if named != Some(held) {
            *state = None;
        }
    }

    /// One open attempt, with no question asked first.
    fn open_at(path: &Path, create: bool) -> std::io::Result<File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(create)
            .truncate(false)
            .open(path)
    }

    /// One creating open, repairing a missing parent once.
    ///
    /// `create(true)` and `create_new(true)` both still fail when the
    /// *parent* is missing, so that is the only absence left to repair: the
    /// ancestry is created and `attempt` runs exactly once more, a second
    /// absence reported as it is, naming what the repair created. Every
    /// other failure is the attempt's own - `AlreadyExists` the conflict a
    /// create reports.
    fn open_repairing(
        path: &Path,
        attempt: impl Fn(&Path) -> std::io::Result<File>,
    ) -> Result<File> {
        let error = match attempt(path) {
            Ok(file) => return Ok(file),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => error,
            Err(error) => return Err(Error::from_io_at(error, "file", path.display())),
        };
        let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        else {
            return Err(Error::from_io_at(error, "file", path.display()));
        };
        std::fs::create_dir_all(parent)?;
        attempt(path).map_err(|retry| {
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
                Error::from_io_at(retry, "file", path.display())
            }
        })
    }

    /// Create the private sibling a write of `path` stages its bytes in: new,
    /// beside it, named `.{name}.{pid}.{n}.tmp` - private, so a listing
    /// passes it over - a name another sibling holds passed for the next one,
    /// and, where `repair` says, a missing parent created once.
    fn open_sibling(path: &Path, repair: bool) -> std::io::Result<(PathBuf, File)> {
        let name = path.file_name().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{} names no file", path.display()),
            )
        })?;
        let pid = std::process::id();
        let mut repaired = !repair;
        let mut taken = 0;
        loop {
            let mut sibling = OsString::from(".");
            sibling.push(name);
            sibling.push(format!(
                ".{pid}.{}.tmp",
                SIBLINGS.fetch_add(1, Ordering::Relaxed)
            ));
            let sibling = path.with_file_name(sibling);
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&sibling)
            {
                Ok(file) => return Ok((sibling, file)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    taken += 1;
                    if taken == SIBLING_ATTEMPTS {
                        // Not the path's conflict: nothing was asked of it.
                        return Err(std::io::Error::other(format!(
                            "{SIBLING_ATTEMPTS} sibling names of {} were taken",
                            path.display()
                        )));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && !repaired => {
                    repaired = true;
                    let Some(parent) = path
                        .parent()
                        .filter(|parent| !parent.as_os_str().is_empty())
                    else {
                        return Err(error);
                    };
                    std::fs::create_dir_all(parent)?;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// The file a whole-value write of `path` replaces, and what is there:
    /// each symbolic link the last component names followed, at most
    /// [`LINK_HOPS`] of them, so a link stays a link and the file it names
    /// takes the value; `None` where nothing is there yet. One `lstat`, and
    /// one more and a `readlink` per link.
    fn resolve_target(path: &Path) -> std::io::Result<(PathBuf, Option<std::fs::Metadata>)> {
        let mut target = path.to_path_buf();
        for _ in 0..=LINK_HOPS {
            let metadata = match std::fs::symlink_metadata(&target) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok((target, None));
                }
                Err(error) => return Err(error),
            };
            if !metadata.file_type().is_symlink() {
                return Ok((target, Some(metadata)));
            }
            // A relative link names a file beside itself.
            let link = std::fs::read_link(&target)?;
            target = match target.parent() {
                Some(folder) => folder.join(link),
                None => link,
            };
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "{} is more than {LINK_HOPS} symbolic links from a file",
                path.display()
            ),
        ))
    }

    /// Publish `bytes` as a new file at `path`: whole or not at all, never
    /// over a file there, and durable once this returns.
    ///
    /// The bytes are written to a private sibling and synced, the sibling is
    /// linked at `path` by one hard link - its `AlreadyExists` is the
    /// conflict, the file there left as it was - and its own name removed,
    /// then on Unix the folder is synced, best effort: a folder the system
    /// will not sync is passed over. A reader finds `path` absent or holding
    /// the whole value, and so does a crash. Where the store links no files,
    /// as a FAT volume, a network share or Windows off NTFS, the link's
    /// refusal falls back to an exclusive create, then the write and its
    /// sync: exclusive still, but whole only once this returns. `repair`
    /// creates a missing parent once, as a write does; without it the
    /// absence is the answer. The sibling never remains; on Windows its name
    /// goes at once where the system deletes by POSIX semantics, and when the
    /// answered descriptor closes otherwise. Answers that descriptor, of the
    /// file `path` now names.
    pub(crate) fn create_whole(path: &Path, bytes: &[u8], repair: bool) -> std::io::Result<File> {
        let (sibling, file) = Self::open_sibling(path, repair)?;
        if let Err(error) = (&file).write_all(bytes).and_then(|()| file.sync_data()) {
            drop(file);
            let _ = std::fs::remove_file(&sibling);
            return Err(error);
        }
        let linked = std::fs::hard_link(&sibling, path);
        let _ = std::fs::remove_file(&sibling);
        match linked {
            Ok(()) => {
                sync_folder(path);
                Ok(file)
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::Unsupported | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                drop(file);
                Self::create_unlinked(path, bytes)
            }
            Err(error) => Err(error),
        }
    }

    /// The exclusive create a store that links no files takes: `create_new`,
    /// the write and its sync, the file removed when either fails so the path
    /// is free for the next creator.
    fn create_unlinked(path: &Path, bytes: &[u8]) -> std::io::Result<File> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)?;
        if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_data()) {
            drop(file);
            let _ = std::fs::remove_file(path);
            return Err(error);
        }
        Ok(file)
    }

    /// Replace the file at `target` - the path, its links followed - with
    /// `bytes` through one rename of a private sibling.
    ///
    /// The sibling is created, written, given `mode` - the replaced file's
    /// permission mode - and synced first, so a failure to stage it leaves
    /// this handle as it was. This handle's own mapping is then released,
    /// what it staged flushed into the file it holds, because Windows refuses
    /// to replace a file the renaming process maps; its descriptor, length
    /// and staged state stay, so a refused rename leaves the handle as it
    /// was, the previous value standing. Once the sibling is renamed over the
    /// target, the descriptor that wrote it is this handle's state, mapped on
    /// the next read, and what the handle held before is let go unpublished.
    /// The sibling is removed on any failure after it was created.
    fn replace(
        &self,
        state: &mut Option<Mapped>,
        target: &Path,
        bytes: &[u8],
        mode: Option<std::fs::Permissions>,
    ) -> Result<()> {
        let at = |error: std::io::Error| Error::from_io_at(error, "file", self.path.display());
        let (sibling, file) = Self::open_sibling(target, true).map_err(at)?;
        let staged = (&file)
            .write_all(bytes)
            .and_then(|()| mode.map_or(Ok(()), |mode| file.set_permissions(mode)))
            .and_then(|()| file.sync_data())
            .and_then(|()| file.metadata())
            .map_err(Error::Io);
        let replaced = staged.and_then(|metadata| {
            state.as_mut().map_or(Ok(()), Mapped::release_mapping)?;
            std::fs::rename(&sibling, target).map_err(at)?;
            Ok(metadata)
        });
        let metadata = match replaced {
            Ok(metadata) => metadata,
            Err(error) => {
                drop(file);
                let _ = std::fs::remove_file(&sibling);
                return Err(error);
            }
        };
        *state = Some(Mapped {
            file,
            mapping: None,
            size: bytes.len() as u64,
            dirty: false,
            identity: identity(&metadata),
        });
        Ok(())
    }

    /// Run `read` over the value from `offset` to its logical end in the one
    /// file this handle holds, the length and the bytes read under one lock
    /// so they are never two files' answers; a missing file is `absent`, and
    /// nothing is mapped for an empty range.
    fn read_from<T>(&self, offset: u64, absent: T, read: impl FnOnce(&[u8]) -> T) -> Result<T> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        // A missing file reads as empty rather than failing.
        if !Self::materialize(&mut state, &self.path, false)? {
            return Ok(absent);
        }
        let mapped = state.as_mut().ok_or_else(poisoned)?;
        let size = usize::try_from(mapped.size).unwrap_or(usize::MAX);
        let Some(start) = usize::try_from(offset).ok().filter(|start| *start < size) else {
            return Ok(read(&[]));
        };
        let mapping = mapped.map_existing()?;
        // A file shortened in place before this handle mapped it answers the
        // bytes it still holds, never an index past them.
        let end = size.min(mapping.len());
        Ok(read(mapping.get(start..end).unwrap_or_default()))
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
        self.release_mapping()?;
        self.file.set_len(capacity)?;
        self.dirty = true;
        self.mapping = Some(map_file(&self.file)?);
        self.mapping.as_mut().ok_or_else(poisoned)
    }

    /// Unmap, flushing what the mapping staged first; the descriptor, the
    /// length and the staged state stay, and the next access maps again.
    fn release_mapping(&mut self) -> Result<()> {
        if let Some(mapping) = self.mapping.take()
            && self.dirty
        {
            mapping.flush()?;
        }
        Ok(())
    }

    /// Write `bytes` at `offset` into the mapping, growing it as needed and
    /// zero-filling any gap the offset leaves; staged until published.
    fn write_at(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| crate::iobase::oversized(u64::MAX))?;
        let previous = self.size;
        let mapping = self.remap(end)?;
        let start = usize::try_from(offset).map_err(|_| crate::iobase::oversized(offset))?;
        let finish = usize::try_from(end).map_err(|_| crate::iobase::oversized(end))?;
        // Zero-fill any gap the offset created before writing.
        if offset > previous {
            let gap = usize::try_from(previous).unwrap_or(usize::MAX);
            mapping[gap..start].fill(0);
        }
        mapping[start..finish].copy_from_slice(bytes);
        self.size = previous.max(end);
        self.dirty = true;
        Ok(bytes.len())
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
        let mapping = self.mapping.take();
        if !self.dirty {
            return Ok(());
        }
        if let Some(mapping) = mapping {
            mapping.flush()?;
        }
        self.file.set_len(self.size)?;
        self.dirty = false;
        Ok(())
    }
}

/// Map a file for shared read/write access.
fn map_file(file: &File) -> Result<MmapMut> {
    // SAFETY: `memmap2` requires this to be `unsafe` because the mapping
    // aliases file bytes that another process could truncate, which would turn
    // a later access into SIGBUS. Yggdryl cannot rule that out, so the hazard is
    // documented on `LocalFile` rather than hidden. Nothing else about the
    // call is unsound: the file handle is owned alongside the mapping and
    // outlives it.
    unsafe { MmapMut::map_mut(file) }.map_err(Error::Io)
}

/// The permission mode a replace gives its sibling: the replaced file's on
/// Unix, so a 0600 file stays 0600, and none elsewhere.
fn replaced_mode(metadata: &std::fs::Metadata) -> Option<std::fs::Permissions> {
    cfg!(unix).then(|| metadata.permissions())
}

/// Which file `metadata` describes: its device and inode on Unix, `None`
/// where the standard library states no identity.
#[cfg(unix)]
#[allow(clippy::unnecessary_wraps)]
fn identity(metadata: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt as _;
    Some((metadata.dev(), metadata.ino()))
}

/// Which file `metadata` describes: its device and inode on Unix, `None`
/// where the standard library states no identity.
#[cfg(not(unix))]
fn identity(_: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}

/// Sync the folder holding `path`, so a name a create linked there survives
/// a crash: on Unix, where a folder opens to be synced, and best effort - a
/// folder the system will not sync is passed over, the file's own bytes
/// already synced.
fn sync_folder(path: &Path) {
    #[cfg(unix)]
    {
        let folder = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        if let Ok(folder) = File::open(folder) {
            let _ = folder.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Report a poisoned lock without panicking a caller.
fn poisoned() -> Error {
    Error::Io(std::io::Error::other(
        "the memory-mapped file lock was poisoned by a panicking writer",
    ))
}

/// A memory-mapped file is the leaf role over the local file system.
impl IOFile for LocalFile {
    fn file_url(&self) -> &Url {
        &self.url
    }

    fn file_exists(&self) -> bool {
        self.path.exists()
    }

    /// Replace the file with the empty value as a whole-value write
    /// replaces it, without creating one that is not there.
    ///
    /// A rename creates its target, so one `lstat` asks first - the same one
    /// that follows a symbolic link to the file it names: nothing there is
    /// the no-op success, letting go of whatever this handle held, and a file
    /// there is replaced through a private sibling, keeping its mode on Unix,
    /// safe under a concurrent mapped reader, which keeps reading the old
    /// bytes whole. A staged write is dropped with the old file, never
    /// flushed back; a refused rename leaves the handle as it was. A file
    /// removed between the `lstat` and the rename comes back empty.
    fn clear_file(&mut self) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        let (target, replaced) = Self::resolve_target(&self.path)
            .map_err(|error| Error::from_io_at(error, "file", self.path.display()))?;
        let Some(replaced) = replaced else {
            *state = None;
            return Ok(());
        };
        self.replace(&mut state, &target, &[], replaced_mode(&replaced))
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
        crate::iobase::skip_absent(std::fs::remove_file(&self.path))
    }
}

impl crate::IOMedia for LocalFile {
    crate::impl_default_iomedia!();
}

impl IOBase for LocalFile {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.read_from(offset, 0, |available| {
            let count = available.len().min(buffer.len());
            buffer[..count].copy_from_slice(&available[..count]);
            count
        })
    }

    /// The whole value of the one file this handle holds: its length and
    /// its bytes read under one lock, so a handle opened while another
    /// replaces the file reads one value whole, never one's length over
    /// the other's bytes.
    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        self.read_from(0, Vec::new(), <[u8]>::to_vec)
    }

    /// The range of the one file this handle holds, clamped to its end
    /// under the same lock as [`Self::read_all_bytes`].
    fn read_range_bytes(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        self.read_from(offset, Vec::new(), |available| {
            available[..length.min(available.len())].to_vec()
        })
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        Self::drop_if_replaced(&mut state, &self.path);
        // A write creates the file and any parent it needs.
        Self::materialize(&mut state, &self.path, true)?;
        state.as_mut().ok_or_else(poisoned)?.write_at(offset, bytes)
    }

    /// Append after the end of the file the path names now, published when
    /// this returns: a held file another handle replaced is let go first, so
    /// the bytes follow the value the path holds, as an in-place append's
    /// would. The offset and the write are taken under one lock.
    fn append_bytes(&mut self, bytes: &[u8]) -> Result<u64> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        Self::drop_if_replaced(&mut state, &self.path);
        Self::materialize(&mut state, &self.path, true)?;
        let mapped = state.as_mut().ok_or_else(poisoned)?;
        let offset = mapped.size;
        mapped.write_at(offset, bytes)?;
        mapped.publish()?;
        Ok(offset)
    }

    /// Replace the value whole: `bytes` are written to a private sibling in
    /// the same folder - `.{name}.{pid}.{n}.tmp`, its missing parent
    /// repaired once - synced, and renamed over the file the path names.
    ///
    /// Safe under a concurrent mapped reader: a handle that mapped the old
    /// file keeps reading the old bytes whole, and a handle opened after the
    /// rename reads `bytes`. This handle ends holding the descriptor that
    /// wrote them, the file the rename put there, at `bytes.len()` with
    /// nothing staged; [`IOBase::mtime`] is the new file's. The sibling is
    /// removed on any failure after it was created, and a refused rename
    /// leaves this handle and the previous value as they were. On Windows
    /// another handle's live mapping of the file can make the system refuse
    /// the rename, which is then this failure.
    ///
    /// The value lands in a new file, so:
    ///
    /// - a symbolic link is followed, at most 40 deep, and the file it names
    ///   is replaced, the link staying a link; a link to nothing creates what
    ///   it names;
    /// - another hard link to the replaced file keeps the replaced value;
    /// - the new file is owned by the writing process, and on Unix takes the
    ///   replaced file's permission mode - a 0600 file stays 0600 - or the
    ///   default mode where nothing was there;
    /// - a folder the writer may not create a file in refuses the write, even
    ///   where the file itself is writable.
    ///
    /// Another handle holding the replaced file follows the path at its next
    /// write, as the module documentation says. One `lstat` of the path
    /// answers the link and the mode.
    fn write_all_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        let (target, replaced) = Self::resolve_target(&self.path)
            .map_err(|error| Error::from_io_at(error, "file", self.path.display()))?;
        let mode = replaced.as_ref().and_then(replaced_mode);
        self.replace(&mut state, &target, bytes, mode)
    }

    /// Create the file holding `bytes`: whole or not at all, never over a
    /// file there, and durable once this returns.
    ///
    /// The bytes are written to a private sibling and synced, and the sibling
    /// is linked at the path by one hard link: its `AlreadyExists` is the
    /// [`Error::Conflict`] naming the path, the file there left as it was,
    /// and the sibling's own name is removed either way, the folder synced on
    /// Unix where it allows. A reader finds the path absent or holding the
    /// whole value, and a crash leaves one of the two. The link is asked of
    /// the path whatever this handle holds, so a handle that already
    /// materialized the file still loses to the file there. Where the store
    /// links no files - a FAT volume, a network share, Windows off NTFS - the
    /// link's refusal falls back to an exclusive create, then the write and
    /// its sync: exclusive still, but whole only once this returns. A missing
    /// parent is created once. This handle ends holding the created file.
    fn create_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        let file = Self::create_whole(&self.path, bytes, true)
            .map_err(|error| Error::from_io_at(error, "file", self.path.display()))?;
        let metadata = file.metadata()?;
        *state = Some(Mapped {
            file,
            mapping: None,
            size: bytes.len() as u64,
            dirty: false,
            identity: identity(&metadata),
        });
        Ok(())
    }

    fn size(&self) -> u64 {
        if let Ok(state) = self.state.lock()
            && let Some(mapped) = state.as_ref()
        {
            return mapped.size;
        }
        // Not materialized: a missing file is empty, an existing one reports
        // its on-disk length without mapping it.
        std::fs::metadata(&self.path).map_or(0, |metadata| metadata.len())
    }

    /// The file's modification time, read without mapping it.
    ///
    /// A mapped handle still asks the filesystem: the mapping carries the
    /// bytes, not the stat the store keeps beside them.
    fn mtime(&self) -> Option<i64> {
        std::fs::metadata(&self.path)
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(crate::holder::system_time_ns)
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
        Self::drop_if_replaced(&mut state, &self.path);
        Self::materialize(&mut state, &self.path, true)?;
        state.as_mut().ok_or_else(poisoned)?.remap(capacity)?;
        Ok(())
    }

    /// Resize the value in place, zero-filling what an extension adds.
    ///
    /// Positional, like [`IOBase::pwrite`]: the length is staged in this
    /// handle's mapping and published by the next flush. Not safe under a
    /// concurrent mapped reader when it shortens the file - the flush drops
    /// pages that reader may still touch, raising SIGBUS there - so replace
    /// the value whole with [`IOBase::write_all_bytes`] or
    /// [`IOBase::clear`] where a reader may hold it.
    fn truncate(&mut self, size: u64) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        Self::drop_if_replaced(&mut state, &self.path);
        Self::materialize(&mut state, &self.path, true)?;
        let mapped = state.as_mut().ok_or_else(poisoned)?;
        if size > mapped.size {
            let previous = mapped.size;
            let mapping = mapped.remap(size)?;
            let from = usize::try_from(previous).unwrap_or(usize::MAX);
            let to = usize::try_from(size).map_err(|_| crate::iobase::oversized(size))?;
            mapping[from..to].fill(0);
        }
        mapped.size = size;
        mapped.dirty = true;
        Ok(())
    }

    fn uri(&self) -> Option<&Uri> {
        Some(self.url.as_ref())
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
        // What another handle replaced is not this handle's to publish.
        Self::drop_if_replaced(&mut state, &self.path);
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

    /// A local file is its path, which a move renames.
    fn local_url(&self) -> Option<&Url> {
        Some(&self.url)
    }

    fn close(&mut self) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        Self::drop_if_replaced(&mut state, &self.path);
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

    /// Replace the value with the empty one through a private sibling, as
    /// [`IOBase::write_all_bytes`] replaces it, creating nothing where no
    /// file is.
    ///
    /// Safe under a concurrent mapped reader: a handle that mapped the old
    /// file keeps reading the old bytes whole, and a handle opened after
    /// reads nothing. A staged write is dropped unpublished, never flushed
    /// back. On Windows another handle's live mapping of the file can make
    /// the system refuse the rename, which is then this failure.
    fn clear(&mut self) -> Result<()> {
        self.clear_file()
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.file_remove(recursive)
    }
}

impl Drop for LocalFile {
    fn drop(&mut self) {
        // Publish the logical length; a failure here cannot be reported, and
        // callers who care call `flush` explicitly.
        if let Ok(mut state) = self.state.lock()
            && let Some(mapped) = state.as_mut()
        {
            let _ = mapped.publish();
        }
    }
}

impl std::fmt::Debug for LocalFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalFile")
            .field("url", &self.url)
            .field("size", &self.size())
            .finish()
    }
}
