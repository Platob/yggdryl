//! [`Mmap`] — the **memory-mapped file** source: a file on disk exposed through the same
//! [`IOBase`] contract as the in-heap [`Heap`](super::Heap), addressed by a
//! [`Uri`](crate::uri::Uri), with optimized page-backed read/write access and **auto-resizing**
//! writes (a write past the end grows the file and remaps, with the same amortized-doubling
//! capacity strategy as `Heap`).
//!
//! Dependency-free: the mapping uses the OS APIs directly (`mmap`/`munmap`/`msync` on Unix,
//! `CreateFileMappingW`/`MapViewOfFile` on Windows) through `std`'s raw handles — no external
//! crate.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use super::cursor::cursor_methods;
use super::{IOBase, IoError, Whence};
use crate::headers::Headers;
use crate::io::{IOKind, IOMode};
use crate::uri::Uri;

/// The minimum mapped capacity once a mapping exists — one typical page, so tiny files do not
/// remap on every small append.
const MIN_MAP: u64 = 4096;

/// A **memory-mapped file** source — the on-disk implementor of [`IOBase`], sharing `Heap`'s
/// full surface (positioned + typed + bulk access, the built-in cursor stream, capacity
/// management, metadata) over a file instead of an owned `Vec`.
///
/// # Addressing, size, and auto-resizing
///
/// A mapping is opened from a [`Uri`] (`file://…` or a plain path — the crate's `Uri` doubles
/// as a filesystem path) and [`uri`](IOBase::uri) reports that address back.
/// [`byte_size`](IOBase::byte_size) is the **logical** length; [`capacity`](IOBase::capacity)
/// is the mapped (file) extent, which grows **amortized** (doubling, page-aligned) when a
/// write lands past the end — so append streams remap `O(log n)` times, exactly like `Heap`'s
/// reallocation curve. On drop (or [`shrink_to_fit`](IOBase::shrink_to_fit)) the file is
/// truncated back to the logical length, so the on-disk file never keeps the capacity padding.
///
/// # Failure and safety model
///
/// Opening, growing, and flushing report the guided [`IoError::FileIo`] naming the operation,
/// path, and OS detail. A **read-only** mapping ([`open_uri_readonly`](Mmap::open_uri_readonly))
/// physically cannot be written: the write primitives write nothing (count `0`) and the *full*
/// writes report a guided error naming the fix. All access goes through the copying
/// [`IOBase`] methods — the mapping's pointer is never handed out, so a remap can never
/// invalidate a caller's view. Like any file mapping, bytes may change underneath if another
/// process writes the same file (DESIGN: single-writer is the caller's contract, as with every
/// mmap API).
///
/// `Mmap` is deliberately **not** `Clone`/`PartialEq`/serializable — it is a live OS resource
/// (two independent mappings of one file would alias), not a value.
///
/// ```
/// use yggdryl_core::io::memory::{IOBase, Mmap};
/// use yggdryl_core::io::IOKind;
///
/// let path = std::env::temp_dir().join("yggdryl_mmap_doc.bin");
/// let uri = yggdryl_core::uri::Uri::from_path(&path.to_string_lossy());
///
/// let mut map = Mmap::create_uri(&uri).unwrap();
/// map.write_utf8("hello mapped world");
/// assert_eq!(map.byte_size(), 18);
/// assert_eq!(map.kind(), IOKind::File);
/// assert_eq!(map.pread_utf8(6, 6).unwrap(), "mapped");
/// drop(map); // truncates the capacity padding back to 18 bytes on disk
///
/// let reopened = Mmap::open_uri(&uri).unwrap();
/// assert_eq!(reopened.byte_size(), 18);
/// assert_eq!(reopened.pread_utf8(0, 5).unwrap(), "hello");
/// # drop(reopened);
/// # std::fs::remove_file(&path).ok();
/// ```
#[derive(Debug)]
pub struct Mmap {
    file: File,
    path: PathBuf,
    /// The mapping base — null iff `map_len == 0` (an empty file maps nothing).
    ptr: *mut u8,
    /// The mapped extent — the file's on-disk length while open (the capacity).
    map_len: usize,
    /// The logical length — what `byte_size()` reports; `<= map_len`.
    len: u64,
    /// The built-in cursor — bytes from the start; may sit past the end after a seek.
    position: u64,
    mode: IOMode,
    headers: Headers,
}

// SAFETY: the mapping pointer is owned exclusively by this value; every `&self` method only
// READS mapped memory and every mutation (write, remap, truncate) requires `&mut self`, so
// Rust's borrow rules provide the synchronization a `Vec<u8>` would get for free.
unsafe impl Send for Mmap {}
// SAFETY: shared references only ever read the mapping (see above), which is safe from
// multiple threads; any remapping requires exclusive access.
unsafe impl Sync for Mmap {}

impl Mmap {
    // ---- constructors (explicit input types, per the naming rule) ----------------------

    /// Opens an **existing** file read-write, addressed by `uri` (`file://…` or a plain
    /// path). Errors with [`IoError::FileIo`] naming the path if it is missing or
    /// inaccessible.
    pub fn open_uri(uri: &Uri) -> Result<Mmap, IoError> {
        Self::open_path(&uri_to_path(uri)?)
    }

    /// Opens an **existing** file **read-only**: reads work, the write primitives write
    /// nothing, and full writes report a guided error.
    pub fn open_uri_readonly(uri: &Uri) -> Result<Mmap, IoError> {
        Self::open_path_readonly(&uri_to_path(uri)?)
    }

    /// Opens the file at `uri` read-write, **creating it empty** if it does not exist
    /// (existing contents are kept — never truncated on open).
    pub fn create_uri(uri: &Uri) -> Result<Mmap, IoError> {
        Self::create_path(&uri_to_path(uri)?)
    }

    /// [`open_uri`](Mmap::open_uri) with a plain path string.
    pub fn open_path(path: impl AsRef<Path>) -> Result<Mmap, IoError> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| file_err("open", &path, &error))?;
        Self::from_file(file, path, IOMode::ReadWrite)
    }

    /// [`open_uri_readonly`](Mmap::open_uri_readonly) with a plain path string.
    pub fn open_path_readonly(path: impl AsRef<Path>) -> Result<Mmap, IoError> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .read(true)
            .open(&path)
            .map_err(|error| file_err("open", &path, &error))?;
        Self::from_file(file, path, IOMode::Read)
    }

    /// [`create_uri`](Mmap::create_uri) with a plain path string.
    pub fn create_path(path: impl AsRef<Path>) -> Result<Mmap, IoError> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| file_err("open", &path, &error))?;
        Self::from_file(file, path, IOMode::ReadWrite)
    }

    fn from_file(file: File, path: PathBuf, mode: IOMode) -> Result<Mmap, IoError> {
        let len = file
            .metadata()
            .map_err(|error| file_err("open", &path, &error))?
            .len();
        let mut map = Mmap {
            file,
            path,
            ptr: std::ptr::null_mut(),
            map_len: 0,
            len,
            position: 0,
            mode,
            headers: Headers::new(),
        };
        if len > 0 {
            map.remap(len as usize)?;
        }
        Ok(map)
    }

    // ---- inherent accessors ------------------------------------------------------------

    /// The file path this mapping is backed by.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Flushes the mapped bytes (and file metadata) to disk — `msync` /
    /// `FlushViewOfFile` + an fsync. Errors with [`IoError::FileIo`] on OS failure.
    pub fn flush(&self) -> Result<(), IoError> {
        if !self.ptr.is_null() {
            // SAFETY: ptr/map_len describe a live mapping owned by self.
            unsafe { sys::flush(self.ptr, self.map_len) }
                .map_err(|error| file_err("flush", &self.path, &error))?;
        }
        self.file
            .sync_all()
            .map_err(|error| file_err("flush", &self.path, &error))
    }

    /// Sets the access [`IOMode`] label in place (the physical protection is fixed at open:
    /// use the `open_*_readonly` constructors for a truly unwritable mapping).
    pub fn set_mode(&mut self, mode: IOMode) {
        self.mode = mode;
    }

    // ---- mapping machinery ---------------------------------------------------------------

    /// Unmaps the current view (if any).
    fn unmap(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: ptr/map_len came from a successful `sys::map` and are unmapped once.
            unsafe { sys::unmap(self.ptr, self.map_len) };
            self.ptr = std::ptr::null_mut();
            self.map_len = 0;
        }
    }

    /// (Re)maps the file at exactly `new_cap` bytes, growing/shrinking the on-disk file to
    /// match (writable mappings only — a read-only mapping maps the file as it is). The
    /// logical `len` is untouched on success.
    ///
    /// Failure-safety: a failed grow/map can never leave a dangling state where `len > 0`
    /// but no view exists — the file is extended **before** the old view is dropped
    /// (growing a mapped file is legal on every platform), and any later failure restores
    /// a view over the file's current extent (or clamps `len` to `0` as the last resort).
    fn remap(&mut self, new_cap: usize) -> Result<(), IoError> {
        let growing = new_cap >= self.map_len;
        if self.mode.is_writable() && growing && new_cap != self.map_len {
            // Extend first: if this fails, the OLD view is still fully intact.
            self.file
                .set_len(new_cap as u64)
                .map_err(|error| file_err("grow", &self.path, &error))?;
        }
        self.unmap();
        if self.mode.is_writable() && !growing {
            // Shrinking needs the view gone first (Windows refuses to truncate a mapped
            // file); on failure, restore a view so the existing bytes stay readable.
            if let Err(error) = self.file.set_len(new_cap as u64) {
                self.restore_view();
                return Err(file_err("grow", &self.path, &error));
            }
        }
        if new_cap == 0 {
            return Ok(());
        }
        let writable = self.mode.is_writable();
        // SAFETY: the file is open with matching access and is `new_cap` bytes long.
        match unsafe { sys::map(&self.file, new_cap, writable) } {
            Ok(ptr) => {
                self.ptr = ptr;
                self.map_len = new_cap;
                Ok(())
            }
            Err(error) => {
                self.restore_view();
                Err(file_err("map", &self.path, &error))
            }
        }
    }

    /// Best-effort recovery after a failed remap: maps whatever extent the file currently
    /// has so the existing bytes stay readable; if even that fails, clamps the logical
    /// length to `0` so no read can ever touch a missing view.
    fn restore_view(&mut self) {
        let current = self.file.metadata().map(|m| m.len()).unwrap_or(0) as usize;
        if current > 0 {
            // SAFETY: mapping the file's own current extent with matching access.
            if let Ok(ptr) = unsafe { sys::map(&self.file, current, self.mode.is_writable()) } {
                self.ptr = ptr;
                self.map_len = current;
                self.len = self.len.min(current as u64);
                return;
            }
        }
        self.len = 0;
        self.map_len = 0; // ptr is already null from the unmap
    }

    /// Ensures the mapped capacity covers `needed` bytes, growing **amortized** (doubling,
    /// never below one page) so append streams remap `O(log n)` times.
    fn grow_capacity(&mut self, needed: u64) -> Result<(), IoError> {
        if needed <= self.map_len as u64 {
            return Ok(());
        }
        let target = needed
            .max(self.map_len as u64 * 2)
            .max(MIN_MAP)
            .min(usize::MAX as u64) as usize;
        self.remap(target)
    }
    // A read-only mapping never grows (`grow_capacity` is only reached from write paths,
    // which bail out first).

    cursor_methods!();
}

impl Drop for Mmap {
    fn drop(&mut self) {
        self.unmap();
        if self.mode.is_writable() {
            // Truncate the capacity padding so the on-disk file is exactly the logical
            // length. Best-effort: a failure here cannot be reported from `drop`.
            let _ = self.file.set_len(self.len);
        }
    }
}

impl IOBase for Mmap {
    #[inline]
    fn byte_size(&self) -> u64 {
        self.len
    }

    #[inline]
    fn capacity(&self) -> u64 {
        self.map_len as u64
    }

    fn reserve(&mut self, additional: u64) {
        // Best-effort (the signature is infallible): a failed grow leaves capacity unchanged
        // and later writes short-write into what exists. Prefer `try_reserve` on a file.
        if self.mode.is_writable() {
            let _ = self.grow_capacity(self.len.saturating_add(additional));
        }
    }

    fn reserve_exact(&mut self, additional: u64) {
        if self.mode.is_writable() {
            let needed = self.len.saturating_add(additional);
            if needed > self.map_len as u64 {
                let _ = self.remap(needed.min(usize::MAX as u64) as usize);
            }
        }
    }

    fn try_reserve(&mut self, additional: u64) -> Result<(), IoError> {
        if !self.mode.is_writable() {
            return Err(readonly_err(&self.path));
        }
        let needed = self
            .len
            .checked_add(additional)
            .ok_or(IoError::CapacityOverflow {
                additional,
                capacity: self.map_len as u64,
            })?;
        self.grow_capacity(needed)
    }

    fn try_reserve_exact(&mut self, additional: u64) -> Result<(), IoError> {
        if !self.mode.is_writable() {
            return Err(readonly_err(&self.path));
        }
        let needed = self
            .len
            .checked_add(additional)
            .ok_or(IoError::CapacityOverflow {
                additional,
                capacity: self.map_len as u64,
            })?;
        if needed > self.map_len as u64 {
            self.remap(needed.min(usize::MAX as u64) as usize)?;
        }
        Ok(())
    }

    fn shrink_to_fit(&mut self) {
        if self.mode.is_writable() && (self.map_len as u64) > self.len {
            let len = self.len as usize;
            let _ = self.remap(len);
        }
    }

    fn shrink_to(&mut self, min_capacity: u64) {
        if self.mode.is_writable() {
            let target = self.len.max(min_capacity).min(usize::MAX as u64) as usize;
            if target < self.map_len {
                let _ = self.remap(target);
            }
        }
    }

    fn uri(&self) -> Uri {
        Uri::from_path(&self.path.to_string_lossy())
    }

    #[inline]
    fn headers(&self) -> &Headers {
        &self.headers
    }

    #[inline]
    fn headers_mut(&mut self) -> &mut Headers {
        &mut self.headers
    }

    #[inline]
    fn mode(&self) -> IOMode {
        self.mode
    }

    #[inline]
    fn kind(&self) -> IOKind {
        IOKind::File
    }

    fn pread_byte_array(&self, offset: u64, buf: &mut [u8]) -> usize {
        // The null check is defensive belt-and-braces: `remap` guarantees `len > 0` implies
        // a live view, but a read must never be able to dereference null regardless.
        if offset >= self.len || self.ptr.is_null() {
            return 0;
        }
        let start = offset as usize;
        let n = buf.len().min((self.len - offset) as usize);
        // SAFETY: start + n <= len <= map_len — inside the live mapping; `&self` methods
        // never mutate or remap, so the borrow rules make this read race-free in safe code.
        unsafe {
            std::ptr::copy_nonoverlapping(self.ptr.add(start), buf.as_mut_ptr(), n);
        }
        n
    }

    fn pwrite_byte_array(&mut self, offset: u64, data: &[u8]) -> usize {
        if data.is_empty() || !self.mode.is_writable() {
            return 0; // a read-only mapping writes nothing (pwrite_all reports the fix)
        }
        let Some(end) = offset.checked_add(data.len() as u64) else {
            return 0;
        };
        if end > self.map_len as u64 && self.grow_capacity(end).is_err() {
            return 0; // could not grow: write nothing, let the full writes report it
        }
        let start = offset as usize;
        // SAFETY: start + data.len() <= map_len after the grow; exclusive `&mut self`.
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), self.ptr.add(start), data.len());
        }
        // The gap (old len .. offset), if any, is already zero — file growth zero-fills.
        self.len = self.len.max(end);
        data.len()
    }

    fn pwrite_all(&mut self, offset: u64, data: &[u8]) -> Result<(), IoError> {
        if !self.mode.is_writable() {
            return Err(readonly_err(&self.path));
        }
        let written = self.pwrite_byte_array(offset, data);
        if written == data.len() {
            Ok(())
        } else {
            Err(IoError::UnexpectedEof {
                offset: offset + written as u64,
                requested: data.len(),
                available: written,
            })
        }
    }
}

/// The guided error for a write attempted on a read-only mapping.
fn readonly_err(path: &Path) -> IoError {
    IoError::FileIo {
        op: "write",
        path: path.to_string_lossy().into_owned(),
        detail: "the mapping is read-only (IOMode::Read); reopen with open_uri / create_uri \
                 for write access"
            .to_string(),
    }
}

/// Builds the guided [`IoError::FileIo`] from an OS error.
fn file_err(op: &'static str, path: &Path, error: &std::io::Error) -> IoError {
    IoError::FileIo {
        op,
        path: path.to_string_lossy().into_owned(),
        detail: error.to_string(),
    }
}

/// Resolves a [`Uri`] to a filesystem path: a `file://` URL or a plain-path URI (no scheme).
/// A `file:///C:/x` path keeps its drive letter (the leading slash is stripped on Windows).
fn uri_to_path(uri: &Uri) -> Result<PathBuf, IoError> {
    match uri.scheme() {
        None | Some("file") => {}
        Some(other) => {
            return Err(IoError::FileIo {
                op: "open",
                path: uri.to_string(),
                detail: format!(
                    "unsupported scheme {other:?}: a mapping needs a file:// URL or a plain \
                     path URI"
                ),
            });
        }
    }
    let path = uri.path();
    // `file:///C:/x` parses to the path `/C:/x` — strip the leading slash before a drive.
    let path = match path.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => &path[1..],
        _ => path,
    };
    if path.is_empty() {
        return Err(IoError::FileIo {
            op: "open",
            path: uri.to_string(),
            detail: "the URI has an empty path; give it a file path to map".to_string(),
        });
    }
    Ok(PathBuf::from(path))
}

// -------------------------------------------------------------------------------------
// OS mapping primitives — the only platform-specific code in the crate. Dependency-free:
// both platforms link these symbols through the system libraries std already links.
// -------------------------------------------------------------------------------------

#[cfg(unix)]
mod sys {
    use std::fs::File;
    use std::io::{Error, Result};
    use std::os::unix::io::AsRawFd;

    use core::ffi::{c_int, c_void};

    const PROT_READ: c_int = 1;
    const PROT_WRITE: c_int = 2;
    const MAP_SHARED: c_int = 0x01;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    const MS_SYNC: c_int = 0x0010;
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    const MS_SYNC: c_int = 4;

    extern "C" {
        fn mmap(
            addr: *mut c_void,
            len: usize,
            prot: c_int,
            flags: c_int,
            fd: c_int,
            offset: isize,
        ) -> *mut c_void;
        fn munmap(addr: *mut c_void, len: usize) -> c_int;
        fn msync(addr: *mut c_void, len: usize, flags: c_int) -> c_int;
    }

    /// Maps `len` bytes of `file` shared, readable (+ writable when asked).
    pub unsafe fn map(file: &File, len: usize, writable: bool) -> Result<*mut u8> {
        let prot = PROT_READ | if writable { PROT_WRITE } else { 0 };
        let ptr = mmap(
            std::ptr::null_mut(),
            len,
            prot,
            MAP_SHARED,
            file.as_raw_fd(),
            0,
        );
        if ptr as isize == -1 {
            Err(Error::last_os_error())
        } else {
            Ok(ptr.cast())
        }
    }

    pub unsafe fn unmap(ptr: *mut u8, len: usize) {
        let _ = munmap(ptr.cast(), len);
    }

    pub unsafe fn flush(ptr: *mut u8, len: usize) -> Result<()> {
        if msync(ptr.cast(), len, MS_SYNC) == 0 {
            Ok(())
        } else {
            Err(Error::last_os_error())
        }
    }
}

#[cfg(windows)]
mod sys {
    use std::fs::File;
    use std::io::{Error, Result};
    use std::os::windows::io::AsRawHandle;

    use core::ffi::c_void;

    type Handle = *mut c_void;
    const PAGE_READONLY: u32 = 0x02;
    const PAGE_READWRITE: u32 = 0x04;
    const FILE_MAP_READ: u32 = 0x0004;
    const FILE_MAP_WRITE: u32 = 0x0002; // implies read access

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateFileMappingW(
            file: Handle,
            attributes: *mut c_void,
            protect: u32,
            max_size_high: u32,
            max_size_low: u32,
            name: *const u16,
        ) -> Handle;
        fn MapViewOfFile(
            mapping: Handle,
            desired_access: u32,
            offset_high: u32,
            offset_low: u32,
            size: usize,
        ) -> *mut c_void;
        fn UnmapViewOfFile(base: *const c_void) -> i32;
        fn FlushViewOfFile(base: *const c_void, size: usize) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }

    /// Maps `len` bytes of `file` shared, readable (+ writable when asked). The mapping
    /// handle is closed immediately — the view keeps the mapping alive until unmapped.
    pub unsafe fn map(file: &File, len: usize, writable: bool) -> Result<*mut u8> {
        let protect = if writable {
            PAGE_READWRITE
        } else {
            PAGE_READONLY
        };
        let mapping = CreateFileMappingW(
            file.as_raw_handle().cast(),
            std::ptr::null_mut(),
            protect,
            (len as u64 >> 32) as u32,
            len as u32,
            std::ptr::null(),
        );
        if mapping.is_null() {
            return Err(Error::last_os_error());
        }
        let access = if writable {
            FILE_MAP_WRITE | FILE_MAP_READ
        } else {
            FILE_MAP_READ
        };
        let view = MapViewOfFile(mapping, access, 0, 0, len);
        let _ = CloseHandle(mapping);
        if view.is_null() {
            Err(Error::last_os_error())
        } else {
            Ok(view.cast())
        }
    }

    pub unsafe fn unmap(ptr: *mut u8, _len: usize) {
        let _ = UnmapViewOfFile(ptr.cast());
    }

    pub unsafe fn flush(ptr: *mut u8, len: usize) -> Result<()> {
        if FlushViewOfFile(ptr.cast(), len) != 0 {
            Ok(())
        } else {
            Err(Error::last_os_error())
        }
    }
}
