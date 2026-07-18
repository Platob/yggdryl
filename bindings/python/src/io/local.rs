//! The `yggdryl.local` submodule — the **local-filesystem family**: the [`LocalIO`] single
//! access point and the raw memory-mapped [`Mmap`] file it builds on.
//!
//! Mirrors `yggdryl_core::io::local`. A [`LocalIO`] is one **lazy** handle over any path
//! (file, folder, or nothing yet) that decides per call how to serve reads and writes:
//! probing and navigating touch nothing, reads before any write open the file ad hoc (a
//! missing node reads as empty), and the first write auto-creates the missing parent
//! folders and the file, memory-maps it, and keeps the mapping so later access runs at
//! memory speed. The same handle carries the whole filesystem graph — `name` / `parent` /
//! `join` (and the `/` operator), streamed `ls` / collected `children` discovery, `mkdir`,
//! and the shape-checked `rm` / `rmfile` / `rmdir` (`IOBase` is the central access path; the
//! graph is part of the one byte contract). An [`Mmap`] is the raw mapping `LocalIO` builds
//! on, usable directly when a pre-existing file and explicit open/create control are wanted
//! — a **leaf** of the graph whose `rm` / `rmfile` really unlink.
//!
//! A **directory node is a memory tree**: `byte_size()` is the lazy streamed sum of its
//! subtree, and `pread_*` / `pwrite_*` serve the directory's name-sorted child file blocks
//! as one contiguous byte region (a middle block never grows; bytes past the end grow the
//! last block). DESIGN: the core's generic `tree_byte_size` / `blocks` /
//! `tree_pread_byte_array` / `tree_pwrite_byte_array` helpers are deliberately **not**
//! mirrored as named methods — they are the internal write-once pattern behind that
//! behavior, and the binding reaches it through the ordinary byte surface on a directory
//! node.
//!
//! Every method is one or two lines over `yggdryl_core`; a failing operation raises a guided
//! `ValueError` carrying the core error text unchanged.

// `useless_conversion`: pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type
// `From`.
#![allow(clippy::useless_conversion)]

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::headers::Headers;
use crate::io::kind::IOKind;
use crate::io::memory::{Heap, NoChildren, Whence};
use crate::io::mode::IOMode;
use crate::mediatype::MediaType;
use crate::mimetype::MimeType;
use crate::uri::Uri;
use yggdryl_core::io::local;
use yggdryl_core::io::memory::{IOBase, IoError};

/// Maps an [`IoError`] to a Python `ValueError` carrying its guided text.
fn ioerr(error: IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// The fail-fast bounds check shared by every bulk `pread_*_array` binding: the guided
/// [`IoError::UnexpectedEof`] when `count` elements of `width` bytes each would run past the
/// `available` bytes, else `None` — checked **before** the result list is allocated.
fn bulk_eof(offset: u64, available: u64, count: usize, width: usize) -> Option<IoError> {
    (count.saturating_mul(width) as u64 > available).then(|| IoError::UnexpectedEof {
        offset: offset + available,
        requested: count.saturating_mul(width),
        available: available as usize,
    })
}

/// Emits a `#[pymethods]` block of scalar positioned `pread_<t>` / `pwrite_<t>` pairs for the
/// `inner`-backed [`LocalIO`] handle — each a one-line delegation to `yggdryl_core`, completing
/// the native-width set alongside the hand-written `i32` / `i64` / byte accessors. The macro
/// emits the whole `#[pymethods] impl` block so pyo3 processes the expanded methods (the
/// `multiple-pymethods` feature allows the extra block).
macro_rules! scalar_methods {
    ($Ty:ty $(, ($t:ty, $pread:ident, $pwrite:ident))+ $(,)?) => {
        #[pymethods]
        impl $Ty {
            $(
                #[doc = concat!("Reads a little-endian `", stringify!($t),
                    "` at `offset`, raising `ValueError` on EOF.")]
                fn $pread(&self, offset: u64) -> PyResult<$t> {
                    self.inner.$pread(offset).map_err(ioerr)
                }
                #[doc = concat!("Writes `value` as a little-endian `", stringify!($t),
                    "` at `offset`, growing as needed.")]
                fn $pwrite(&mut self, offset: u64, value: $t) -> PyResult<()> {
                    self.inner.$pwrite(offset, value).map_err(ioerr)
                }
            )+
        }
    };
}

/// Emits a `#[pymethods]` block of cursor typed `read_<t>` / `write_<t>` pairs for the
/// `inner`-backed [`LocalIO`] handle — each reads/writes the positioned value at the cursor and
/// advances it, delegating to `yggdryl_core`.
macro_rules! cursor_typed_methods {
    ($Ty:ty $(, ($t:ty, $read:ident, $write:ident))+ $(,)?) => {
        #[pymethods]
        impl $Ty {
            $(
                #[doc = concat!("Reads a little-endian `", stringify!($t),
                    "` at the cursor, advancing it; raising `ValueError` on EOF.")]
                fn $read(&mut self) -> PyResult<$t> {
                    self.inner.$read().map_err(ioerr)
                }
                #[doc = concat!("Writes `value` as a little-endian `", stringify!($t),
                    "` at the cursor, advancing it.")]
                fn $write(&mut self, value: $t) -> PyResult<()> {
                    self.inner.$write(value).map_err(ioerr)
                }
            )+
        }
    };
}

/// Emits a `#[pymethods]` block of the bulk typed `pread_<t>_array` / `pwrite_<t>_array` /
/// `pwrite_<t>_repeat` methods for the `inner`-backed [`LocalIO`] handle — mirroring the
/// existing `u16` array binding, with the element `$width` feeding the fail-fast bounds check.
macro_rules! bulk_methods {
    ($Ty:ty $(, ($t:ty, $width:literal, $pread:ident, $pwrite:ident, $repeat:ident))+ $(,)?) => {
        #[pymethods]
        impl $Ty {
            $(
                #[doc = concat!("Bulk read of `count` little-endian `", stringify!($t),
                    "`s at `offset` (fail-fast bounds check before allocating).")]
                fn $pread(&self, offset: u64, count: usize) -> PyResult<Vec<$t>> {
                    let available = self.inner.byte_size().saturating_sub(offset);
                    if let Some(e) = bulk_eof(offset, available, count, $width) {
                        return Err(ioerr(e));
                    }
                    let mut values = vec![<$t>::default(); count];
                    self.inner.$pread(offset, &mut values).map_err(ioerr)?;
                    Ok(values)
                }
                #[doc = concat!("Bulk write of little-endian `", stringify!($t),
                    "`s at `offset`, growing as needed.")]
                fn $pwrite(&mut self, offset: u64, values: Vec<$t>) -> PyResult<()> {
                    self.inner.$pwrite(offset, &values).map_err(ioerr)
                }
                #[doc = concat!("Repeated-value fill of `count` little-endian `", stringify!($t),
                    "` copies of `value` at `offset` (no full array is built).")]
                fn $repeat(&mut self, offset: u64, value: $t, count: usize) -> PyResult<()> {
                    self.inner.$repeat(offset, value, count).map_err(ioerr)
                }
            )+
        }
    };
}

/// Emits a `#[pymethods]` block of scalar positioned `pread_<t>` / `pwrite_<t>` pairs for the
/// [`Mmap`] mapping — the [`Mmap`] counterpart of [`scalar_methods`], routing each call through
/// the closed-mapping guard [`Mmap::io`] / [`Mmap::io_mut`].
macro_rules! mmap_scalar_methods {
    ($(($t:ty, $pread:ident, $pwrite:ident)),+ $(,)?) => {
        #[pymethods]
        impl Mmap {
            $(
                #[doc = concat!("Reads a little-endian `", stringify!($t),
                    "` at `offset`, raising `ValueError` on EOF.")]
                fn $pread(&self, offset: u64) -> PyResult<$t> {
                    self.io()?.$pread(offset).map_err(ioerr)
                }
                #[doc = concat!("Writes `value` as a little-endian `", stringify!($t),
                    "` at `offset`, growing as needed.")]
                fn $pwrite(&mut self, offset: u64, value: $t) -> PyResult<()> {
                    self.io_mut()?.$pwrite(offset, value).map_err(ioerr)
                }
            )+
        }
    };
}

/// Emits a `#[pymethods]` block of cursor typed `read_<t>` / `write_<t>` pairs for the [`Mmap`]
/// mapping — the [`Mmap`] counterpart of [`cursor_typed_methods`].
macro_rules! mmap_cursor_typed_methods {
    ($(($t:ty, $read:ident, $write:ident)),+ $(,)?) => {
        #[pymethods]
        impl Mmap {
            $(
                #[doc = concat!("Reads a little-endian `", stringify!($t),
                    "` at the cursor, advancing it; raising `ValueError` on EOF.")]
                fn $read(&mut self) -> PyResult<$t> {
                    self.io_mut()?.$read().map_err(ioerr)
                }
                #[doc = concat!("Writes `value` as a little-endian `", stringify!($t),
                    "` at the cursor, advancing it.")]
                fn $write(&mut self, value: $t) -> PyResult<()> {
                    self.io_mut()?.$write(value).map_err(ioerr)
                }
            )+
        }
    };
}

/// Emits a `#[pymethods]` block of the bulk typed array/repeat methods for the [`Mmap`]
/// mapping — the [`Mmap`] counterpart of [`bulk_methods`], routing through the closed guard.
macro_rules! mmap_bulk_methods {
    ($(($t:ty, $width:literal, $pread:ident, $pwrite:ident, $repeat:ident)),+ $(,)?) => {
        #[pymethods]
        impl Mmap {
            $(
                #[doc = concat!("Bulk read of `count` little-endian `", stringify!($t),
                    "`s at `offset` (fail-fast bounds check before allocating).")]
                fn $pread(&self, offset: u64, count: usize) -> PyResult<Vec<$t>> {
                    let io = self.io()?;
                    if let Some(e) = bulk_eof(offset, io.byte_size().saturating_sub(offset), count, $width) {
                        return Err(ioerr(e));
                    }
                    let mut values = vec![<$t>::default(); count];
                    io.$pread(offset, &mut values).map_err(ioerr)?;
                    Ok(values)
                }
                #[doc = concat!("Bulk write of little-endian `", stringify!($t),
                    "`s at `offset`, growing as needed.")]
                fn $pwrite(&mut self, offset: u64, values: Vec<$t>) -> PyResult<()> {
                    self.io_mut()?.$pwrite(offset, &values).map_err(ioerr)
                }
                #[doc = concat!("Repeated-value fill of `count` little-endian `", stringify!($t),
                    "` copies of `value` at `offset` (no full array is built).")]
                fn $repeat(&mut self, offset: u64, value: $t, count: usize) -> PyResult<()> {
                    self.io_mut()?.$repeat(offset, value, count).map_err(ioerr)
                }
            )+
        }
    };
}

/// The one local-filesystem handle — a **lazy** node over any path (file, folder, or nothing
/// yet) that **decides per call how to serve reads and writes**: constructing, probing, and
/// navigating touch nothing; before any write a read opens the file ad hoc (a missing node
/// reads as empty, a directory reads as its **memory tree** — the name-sorted child file
/// blocks served as one contiguous byte region); the **first write auto-creates** the missing parent
/// folders and the file, memory-maps it, and keeps the mapping so later access runs at
/// memory speed. [`close`](LocalIO::close) — or the end of a `with LocalIO(path) as node:`
/// block — releases the mapped backing (truncating the file to its logical length) and the
/// handle **stays usable** — it simply returns to its lazy state;
/// [`is_mapped`](LocalIO::is_mapped) reports which state it is in.
///
/// The same handle is the filesystem **graph**: [`name`](LocalIO::name) /
/// [`parent`](LocalIO::parent) / [`join`](LocalIO::join) (and the `/` operator), the
/// streamed [`ls`](LocalIO::ls) / collected [`children`](LocalIO::children) discovery,
/// [`mkdir`](LocalIO::mkdir) when a folder itself is the goal, and the shape-checked
/// [`rm`](LocalIO::rm) / [`rmfile`](LocalIO::rmfile) / [`rmdir`](LocalIO::rmdir).
///
/// A `LocalIO` is a **live handle, not a value**: it compares by path (`==`), copies to a
/// fresh **lazy** handle (`copy()` / `copy.copy` — the mapping is deliberately never shared),
/// and is unhashable. It **pickles by its portable address** — its `file://` URI with a home /
/// temp path folded to a `~` / `$TMP` token ([`to_portable_str`](LocalIO::to_portable_str)) —
/// so a handle survives transport to another environment and reopens under that machine's home /
/// temp roots (the mapping and cursor are transient and not part of the pickled value).
#[pyclass(module = "yggdryl.local")]
#[derive(Clone)]
pub struct LocalIO {
    pub(crate) inner: local::LocalIO,
}

#[pymethods]
impl LocalIO {
    /// A lazy handle for `source` — nothing is touched or created. The generic,
    /// type-inferring entry point: a `str` path dispatches to the core `from_path`, a
    /// [`Uri`] (`file://…` or a plain-path URI) to `from_uri` (raising the core's guided
    /// `ValueError` for a non-file scheme or an empty path).
    #[new]
    fn new(source: &Bound<'_, PyAny>) -> PyResult<LocalIO> {
        if let Ok(path) = source.extract::<String>() {
            Ok(LocalIO {
                inner: local::LocalIO::from_path(path),
            })
        } else if let Ok(uri) = source.extract::<PyRef<'_, Uri>>() {
            local::LocalIO::from_uri(&uri.inner)
                .map(|inner| LocalIO { inner })
                .map_err(ioerr)
        } else {
            Err(PyTypeError::new_err(format!(
                "cannot open a LocalIO from {}: expected a str filesystem path or a \
                 yggdryl.uri.Uri (pass str(path) for a pathlib.Path)",
                source.repr()?
            )))
        }
    }

    /// A **lazy** handle to a temporary **file** in the system temp directory. `name` sets the
    /// file name; the default (`None`) is a process-unique name ending in `.tmp`. Like any
    /// `LocalIO` it is lazy — the file is created on the **first write** — so this only picks
    /// the path.
    #[staticmethod]
    #[pyo3(signature = (name = None))]
    fn tmpfile(name: Option<&str>) -> LocalIO {
        LocalIO {
            inner: local::LocalIO::tmpfile(name),
        }
    }

    /// A **lazy** handle to a temporary **folder** in the system temp directory. `name` sets
    /// the folder name; the default (`None`) is a process-unique name. Lazy — call
    /// [`mkdir`](LocalIO::mkdir) to create it, or just write a child (which auto-creates this
    /// folder as a parent).
    #[staticmethod]
    #[pyo3(signature = (name = None))]
    fn tmpfolder(name: Option<&str>) -> LocalIO {
        LocalIO {
            inner: local::LocalIO::tmpfolder(name),
        }
    }

    /// Alias of [`tmpfolder`](LocalIO::tmpfolder) under the familiar `tmpdir` name (mirroring
    /// Python's `tempfile` vocabulary) — a **lazy** handle to a temporary folder.
    #[staticmethod]
    #[pyo3(signature = (name = None))]
    fn tmpdir(name: Option<&str>) -> LocalIO {
        LocalIO {
            inner: local::LocalIO::tmpdir(name),
        }
    }

    // ---- lifecycle: close / is_mapped / flush / mkdir / path ---------------------------

    /// Releases the mapped backing eagerly (truncating the file to its logical length) —
    /// after which the handle is **still usable**: it simply returns to its lazy state.
    /// Idempotent. Call before removing a file this handle has written (Windows cannot
    /// delete a mapped file).
    fn close(&mut self) {
        self.inner.close();
    }

    /// Context-manager entry — returns the handle itself, so `with LocalIO(path) as node:`
    /// binds the lazy handle.
    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Context-manager exit — [`close`](LocalIO::close)s the mapped backing (the handle
    /// stays usable, back in its lazy state); exceptions propagate.
    fn __exit__(
        &mut self,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> bool {
        self.close();
        false
    }

    /// Whether the handle currently holds its optimized mapped backing (`True` after the
    /// first write, `False` while lazy and after [`close`](LocalIO::close)).
    #[getter]
    fn is_mapped(&self) -> bool {
        self.inner.is_mapped()
    }

    /// Flushes the mapped backing (if the handle has one) to disk; a lazy handle has
    /// nothing buffered and flushes trivially. Raises a guided `ValueError` on OS failure.
    fn flush(&self) -> PyResult<()> {
        self.inner.flush().map_err(ioerr)
    }

    /// Auto-creates the directory tree at this path (like `mkdir -p`) — the explicit form
    /// when a **folder** itself is the goal (file-bound writes auto-create their parents on
    /// their own).
    fn mkdir(&self) -> PyResult<()> {
        self.inner.mkdir().map_err(ioerr)
    }

    /// A standalone [`Mmap`] over this node's file, **reusing this handle's own parameters** —
    /// its path, its [`IOMode`] (a read-only handle maps read-only; a read-write one maps
    /// read-write and auto-creates the missing parents + file), and its
    /// [`headers`](LocalIO::headers) (copied onto the mapping). Independent of this handle's
    /// own lazy [`is_mapped`](LocalIO::is_mapped) backing — the direct front door to the
    /// memory-mapped source when the caller wants to hold it.
    fn mmap(&self) -> PyResult<Mmap> {
        self.inner
            .mmap()
            .map(|inner| Mmap { inner: Some(inner) })
            .map_err(ioerr)
    }

    /// The underlying filesystem path as a `str`.
    #[getter]
    fn path(&self) -> String {
        self.inner.as_std_path().to_string_lossy().into_owned()
    }

    /// The `os.PathLike` protocol — the filesystem path as a `str`, so a `LocalIO` can be
    /// passed straight to `open(...)`, `os.stat(...)`, `pathlib.Path(...)`, and the rest of the
    /// standard library.
    fn __fspath__(&self) -> String {
        self.inner.as_std_path().to_string_lossy().into_owned()
    }

    /// io-file-like: whether the handle's [`mode`](LocalIO::mode) permits reading
    /// (`Read` / `ReadWrite`).
    fn readable(&self) -> bool {
        self.inner.mode().is_readable()
    }

    /// io-file-like: whether the handle's [`mode`](LocalIO::mode) permits writing (everything
    /// except `Read`).
    fn writable(&self) -> bool {
        self.inner.mode().is_writable()
    }

    /// io-file-like: a `LocalIO` is always seekable (its positioned/cursor access reaches any
    /// offset).
    fn seekable(&self) -> bool {
        true
    }

    /// io-file-like: a `LocalIO` handle is **never** truly closed — [`close`](LocalIO::close)
    /// only releases the optimized mapped backing and the handle stays usable — so this is
    /// always `False` (use [`is_mapped`](LocalIO::is_mapped) to see the mapping state).
    #[getter]
    fn closed(&self) -> bool {
        false
    }

    // ---- size + capacity ---------------------------------------------------------------

    /// The **logical** length in bytes — the mapped backing's length once mapped, the
    /// on-disk file length while lazy, `0` for a missing node. A **directory** reports its
    /// memory-tree size: the lazy, streamed sum of its whole subtree (recomputed live per
    /// call, nothing cached).
    fn byte_size(&self) -> u64 {
        self.inner.byte_size()
    }

    /// The logical length in bytes (so `len(node)` works).
    fn __len__(&self) -> usize {
        self.inner.byte_size() as usize
    }

    /// The total length in bits — `byte_size() * 8`.
    fn bit_size(&self) -> u64 {
        self.inner.bit_size()
    }

    /// The mapped extent in bytes once the handle holds its backing (grows amortized,
    /// page-aligned, like `Heap`'s reallocation curve); `byte_size()` while lazy.
    fn capacity(&self) -> u64 {
        self.inner.capacity()
    }

    /// Reserves capacity for at least `additional` more bytes past the current size,
    /// materializing the mapped backing (auto-creating the file) on a writable handle.
    /// Best-effort — prefer [`try_reserve`](LocalIO::try_reserve) to see a failure.
    fn reserve(&mut self, additional: u64) {
        self.inner.reserve(additional);
    }

    /// The spare room already mapped — `capacity() - byte_size()`.
    fn spare_capacity(&self) -> u64 {
        self.inner.spare_capacity()
    }

    /// Reserves capacity for **exactly** `additional` more bytes — no amortized
    /// over-allocation, for a caller that knows the final size.
    fn reserve_exact(&mut self, additional: u64) {
        self.inner.reserve_exact(additional);
    }

    /// **Checked** reservation: raises a guided `ValueError` (overflow, a directory node,
    /// or the OS refusing to grow/remap the file) instead of failing silently.
    fn try_reserve(&mut self, additional: u64) -> PyResult<()> {
        self.inner.try_reserve(additional).map_err(ioerr)
    }

    /// **Checked exact** reservation — `try_reserve` without the amortized over-allocation.
    fn try_reserve_exact(&mut self, additional: u64) -> PyResult<()> {
        self.inner.try_reserve_exact(additional).map_err(ioerr)
    }

    /// Ensures the **total** capacity is at least `total` bytes (the absolute-target form of
    /// `reserve`); a no-op when already satisfied, never shrinks.
    fn ensure_capacity(&mut self, total: u64) {
        self.inner.ensure_capacity(total);
    }

    /// **Checked** `ensure_capacity` — raises a guided `ValueError` instead of failing
    /// silently.
    fn try_ensure_capacity(&mut self, total: u64) -> PyResult<()> {
        self.inner.try_ensure_capacity(total).map_err(ioerr)
    }

    /// Truncates the mapped backing toward the logical length, releasing the capacity
    /// padding on disk; a no-op while lazy.
    fn shrink_to_fit(&mut self) {
        self.inner.shrink_to_fit();
    }

    /// Shrinks the mapped extent toward `min_capacity` (never below `byte_size()`); a
    /// no-op while lazy.
    fn shrink_to(&mut self, min_capacity: u64) {
        self.inner.shrink_to(min_capacity);
    }

    /// Sets the file's byte length to exactly `length` — shrinking (dropping the tail) or
    /// extending (zero-filling), auto-creating + mapping the file — then syncs the size
    /// headers. Raises the guided `ValueError` on a read-only handle or a directory node.
    fn truncate(&mut self, length: u64) -> PyResult<()> {
        self.inner.truncate(length).map_err(ioerr)
    }

    /// The content length in bytes, **preferring the cached `Content-Length` header** when
    /// present and falling back to the live `byte_size()`.
    fn content_length(&self) -> u64 {
        self.inner.content_length()
    }

    /// The [`MemoryInfo`](crate::io::meminfo::MemoryInfo) capacity snapshot of the **disk
    /// volume** backing this path — total and free bytes (the same value type a
    /// `yggdryl.gpu.GpuDevice` reports for its memory). Reports the portable
    /// [`unknown`](crate::io::meminfo::MemoryInfo::unknown) snapshot where a native route is not
    /// yet wired.
    fn memory_info(&self) -> crate::io::meminfo::MemoryInfo {
        crate::io::meminfo::MemoryInfo {
            inner: self.inner.memory_info(),
        }
    }

    /// Whether the node holds no bytes (`byte_size() == 0`) — `True` for a missing node, and
    /// for a directory whose memory tree is empty (a populated directory reports its subtree
    /// sum, so it is **not** empty).
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Truthiness — `True` when the node holds at least one byte.
    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    // ---- positioned byte-array ---------------------------------------------------------

    /// **Positioned read.** Returns up to `length` bytes starting at `offset` as `bytes` —
    /// short near the end, empty at or past it (and empty on a missing node). A
    /// **directory** reads as its memory tree: the name-sorted child file blocks stitched
    /// into one contiguous region (child directories recurse). Never moves the cursor.
    /// Reads **directly** into the `bytes` allocation (one copy).
    fn pread_byte_array<'py>(
        &self,
        py: Python<'py>,
        offset: u64,
        length: usize,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let n = self
            .inner
            .byte_size()
            .saturating_sub(offset)
            .min(length as u64) as usize;
        PyBytes::new_bound_with(py, n, |dst| {
            self.inner.pread_byte_array(offset, dst);
            Ok(())
        })
    }

    /// **Positioned write.** Copies `data` (bytes / bytearray) in at `offset`, auto-creating
    /// parents + the file on the first write and zero-filling any gap; returns the number of
    /// bytes written (`0` on a read-only handle). A **directory** routes the write across
    /// its memory-tree blocks: a write inside a block stays capped at that block's end (a
    /// middle block never grows), bytes past the end grow the **last** block, and an empty
    /// directory writes nothing (the full/typed writes report the guided fix).
    fn pwrite_byte_array(&mut self, offset: u64, data: Vec<u8>) -> usize {
        self.inner.pwrite_byte_array(offset, &data)
    }

    // ---- positioned typed accessors ----------------------------------------------------

    /// Reads the single byte at `offset`, raising `ValueError` if it is past the end.
    fn pread_byte(&self, offset: u64) -> PyResult<u8> {
        self.inner.pread_byte(offset).map_err(ioerr)
    }

    /// Writes the single byte `value` at `offset`, growing the file as needed.
    fn pwrite_byte(&mut self, offset: u64, value: u8) -> PyResult<()> {
        self.inner.pwrite_byte(offset, value).map_err(ioerr)
    }

    /// Reads the bit at absolute **bit** `offset` (LSB-first: bit `offset % 8` of byte
    /// `offset / 8`), raising `ValueError` if its byte is past the end.
    fn pread_bit(&self, offset: u64) -> PyResult<bool> {
        self.inner.pread_bit(offset).map_err(ioerr)
    }

    /// Sets or clears the bit at absolute **bit** `offset` (LSB-first), growing the file
    /// (zero-filled) if the bit is past the end.
    fn pwrite_bit(&mut self, offset: u64, value: bool) -> PyResult<()> {
        self.inner.pwrite_bit(offset, value).map_err(ioerr)
    }

    /// Reads a little-endian `i32` (4 bytes) at `offset`, raising `ValueError` on EOF.
    fn pread_i32(&self, offset: u64) -> PyResult<i32> {
        self.inner.pread_i32(offset).map_err(ioerr)
    }

    /// Writes `value` as a little-endian `i32` (4 bytes) at `offset`, growing as needed.
    fn pwrite_i32(&mut self, offset: u64, value: i32) -> PyResult<()> {
        self.inner.pwrite_i32(offset, value).map_err(ioerr)
    }

    /// Reads a little-endian `i64` (8 bytes) at `offset`, raising `ValueError` on EOF.
    fn pread_i64(&self, offset: u64) -> PyResult<i64> {
        self.inner.pread_i64(offset).map_err(ioerr)
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at `offset`, growing as needed.
    fn pwrite_i64(&mut self, offset: u64, value: i64) -> PyResult<()> {
        self.inner.pwrite_i64(offset, value).map_err(ioerr)
    }

    /// Reads up to `length` **bytes** at `offset` and decodes them as UTF-8 text (clamped
    /// near the end), raising a guided `ValueError` on invalid UTF-8 — including a
    /// multi-byte character cut by the range.
    fn pread_utf8(&self, offset: u64, length: usize) -> PyResult<String> {
        self.inner.pread_utf8(offset, length).map_err(ioerr)
    }

    /// Writes `text`'s UTF-8 bytes at `offset` (auto-creating + growing as needed); returns
    /// the number of **bytes** written.
    fn pwrite_utf8(&mut self, offset: u64, text: &str) -> usize {
        self.inner.pwrite_utf8(offset, text)
    }

    // ---- bulk typed arrays + repeated fills ----------------------------------------------

    /// **Bulk typed read.** Returns `count` little-endian `i32`s starting at `offset` as a
    /// list, raising `ValueError` if fewer bytes remain — checked **before** the result is
    /// allocated, so a hostile `count` fails fast instead of allocating.
    fn pread_i32_array(&self, offset: u64, count: usize) -> PyResult<Vec<i32>> {
        let available = self.inner.byte_size().saturating_sub(offset);
        if count.saturating_mul(4) as u64 > available {
            return Err(ioerr(IoError::UnexpectedEof {
                offset: offset + available,
                requested: count.saturating_mul(4),
                available: available as usize,
            }));
        }
        let mut values = vec![0i32; count];
        self.inner
            .pread_i32_array(offset, &mut values)
            .map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write.** Writes all of `values` as little-endian `i32`s at `offset`,
    /// growing as needed.
    fn pwrite_i32_array(&mut self, offset: u64, values: Vec<i32>) -> PyResult<()> {
        self.inner.pwrite_i32_array(offset, &values).map_err(ioerr)
    }

    /// **Bulk typed read** of `count` little-endian `i64`s — the wide counterpart of
    /// [`pread_i32_array`](LocalIO::pread_i32_array), with the same fail-fast bounds check
    /// before the result is allocated.
    fn pread_i64_array(&self, offset: u64, count: usize) -> PyResult<Vec<i64>> {
        let available = self.inner.byte_size().saturating_sub(offset);
        if count.saturating_mul(8) as u64 > available {
            return Err(ioerr(IoError::UnexpectedEof {
                offset: offset + available,
                requested: count.saturating_mul(8),
                available: available as usize,
            }));
        }
        let mut values = vec![0i64; count];
        self.inner
            .pread_i64_array(offset, &mut values)
            .map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write** of little-endian `i64`s — the wide counterpart of
    /// [`pwrite_i32_array`](LocalIO::pwrite_i32_array).
    fn pwrite_i64_array(&mut self, offset: u64, values: Vec<i64>) -> PyResult<()> {
        self.inner.pwrite_i64_array(offset, &values).map_err(ioerr)
    }

    /// **Repeated-value fill.** Writes `count` copies of the byte `value` at `offset`
    /// (growing as needed) without ever materializing the full array — the `memset` of the
    /// family.
    fn pwrite_byte_repeat(&mut self, offset: u64, value: u8, count: usize) -> PyResult<()> {
        self.inner
            .pwrite_byte_repeat(offset, value, count)
            .map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `i32` copies of `value` at `offset` —
    /// no full array is built.
    fn pwrite_i32_repeat(&mut self, offset: u64, value: i32, count: usize) -> PyResult<()> {
        self.inner
            .pwrite_i32_repeat(offset, value, count)
            .map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `i64` copies of `value` at `offset` —
    /// the wide counterpart of [`pwrite_i32_repeat`](LocalIO::pwrite_i32_repeat).
    fn pwrite_i64_repeat(&mut self, offset: u64, value: i64, count: usize) -> PyResult<()> {
        self.inner
            .pwrite_i64_repeat(offset, value, count)
            .map_err(ioerr)
    }

    // ---- bulk unsigned + floating widths (u16/u32/u64/f32/f64) --------------------------

    /// **Bulk typed read** of `count` little-endian `u16`s — the `u16` counterpart of
    /// [`pread_i32_array`](LocalIO::pread_i32_array), with the same fail-fast bounds check.
    fn pread_u16_array(&self, offset: u64, count: usize) -> PyResult<Vec<u16>> {
        let available = self.inner.byte_size().saturating_sub(offset);
        if let Some(e) = bulk_eof(offset, available, count, 2) {
            return Err(ioerr(e));
        }
        let mut values = vec![0u16; count];
        self.inner
            .pread_u16_array(offset, &mut values)
            .map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write** of little-endian `u16`s at `offset`, growing as needed.
    fn pwrite_u16_array(&mut self, offset: u64, values: Vec<u16>) -> PyResult<()> {
        self.inner.pwrite_u16_array(offset, &values).map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `u16` copies of `value` at `offset`.
    fn pwrite_u16_repeat(&mut self, offset: u64, value: u16, count: usize) -> PyResult<()> {
        self.inner
            .pwrite_u16_repeat(offset, value, count)
            .map_err(ioerr)
    }

    /// **Bulk typed read** of `count` little-endian `u32`s (fail-fast bounds check).
    fn pread_u32_array(&self, offset: u64, count: usize) -> PyResult<Vec<u32>> {
        let available = self.inner.byte_size().saturating_sub(offset);
        if let Some(e) = bulk_eof(offset, available, count, 4) {
            return Err(ioerr(e));
        }
        let mut values = vec![0u32; count];
        self.inner
            .pread_u32_array(offset, &mut values)
            .map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write** of little-endian `u32`s at `offset`, growing as needed.
    fn pwrite_u32_array(&mut self, offset: u64, values: Vec<u32>) -> PyResult<()> {
        self.inner.pwrite_u32_array(offset, &values).map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `u32` copies of `value` at `offset`.
    fn pwrite_u32_repeat(&mut self, offset: u64, value: u32, count: usize) -> PyResult<()> {
        self.inner
            .pwrite_u32_repeat(offset, value, count)
            .map_err(ioerr)
    }

    /// **Bulk typed read** of `count` little-endian `u64`s (fail-fast bounds check).
    fn pread_u64_array(&self, offset: u64, count: usize) -> PyResult<Vec<u64>> {
        let available = self.inner.byte_size().saturating_sub(offset);
        if let Some(e) = bulk_eof(offset, available, count, 8) {
            return Err(ioerr(e));
        }
        let mut values = vec![0u64; count];
        self.inner
            .pread_u64_array(offset, &mut values)
            .map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write** of little-endian `u64`s at `offset`, growing as needed.
    fn pwrite_u64_array(&mut self, offset: u64, values: Vec<u64>) -> PyResult<()> {
        self.inner.pwrite_u64_array(offset, &values).map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `u64` copies of `value` at `offset`.
    fn pwrite_u64_repeat(&mut self, offset: u64, value: u64, count: usize) -> PyResult<()> {
        self.inner
            .pwrite_u64_repeat(offset, value, count)
            .map_err(ioerr)
    }

    /// **Bulk typed read** of `count` little-endian `f32`s (fail-fast bounds check).
    fn pread_f32_array(&self, offset: u64, count: usize) -> PyResult<Vec<f32>> {
        let available = self.inner.byte_size().saturating_sub(offset);
        if let Some(e) = bulk_eof(offset, available, count, 4) {
            return Err(ioerr(e));
        }
        let mut values = vec![0f32; count];
        self.inner
            .pread_f32_array(offset, &mut values)
            .map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write** of little-endian `f32`s at `offset`, growing as needed.
    fn pwrite_f32_array(&mut self, offset: u64, values: Vec<f32>) -> PyResult<()> {
        self.inner.pwrite_f32_array(offset, &values).map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `f32` copies of `value` at `offset`.
    fn pwrite_f32_repeat(&mut self, offset: u64, value: f32, count: usize) -> PyResult<()> {
        self.inner
            .pwrite_f32_repeat(offset, value, count)
            .map_err(ioerr)
    }

    /// **Bulk typed read** of `count` little-endian `f64`s (fail-fast bounds check).
    fn pread_f64_array(&self, offset: u64, count: usize) -> PyResult<Vec<f64>> {
        let available = self.inner.byte_size().saturating_sub(offset);
        if let Some(e) = bulk_eof(offset, available, count, 8) {
            return Err(ioerr(e));
        }
        let mut values = vec![0f64; count];
        self.inner
            .pread_f64_array(offset, &mut values)
            .map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write** of little-endian `f64`s at `offset`, growing as needed.
    fn pwrite_f64_array(&mut self, offset: u64, values: Vec<f64>) -> PyResult<()> {
        self.inner.pwrite_f64_array(offset, &values).map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `f64` copies of `value` at `offset`.
    fn pwrite_f64_repeat(&mut self, offset: u64, value: f64, count: usize) -> PyResult<()> {
        self.inner
            .pwrite_f64_repeat(offset, value, count)
            .map_err(ioerr)
    }

    // ---- cross-source copy -------------------------------------------------------------

    /// Overwrites this node with **all of `src`'s bytes** (a `yggdryl.memory.Heap`),
    /// truncating to match — a cross-source copy. Returns the byte count.
    fn copy_from(&mut self, src: &Heap) -> PyResult<u64> {
        self.inner.copy_from(&src.inner).map_err(ioerr)
    }

    /// **Positioned cross-source write**: copies `length` bytes of `src` (a
    /// `yggdryl.memory.Heap`) starting at `src_offset` into this node at `offset`. Returns
    /// the number of bytes transferred (short at the end of `src`).
    fn pwrite_from(
        &mut self,
        offset: u64,
        src: &Heap,
        src_offset: u64,
        length: u64,
    ) -> PyResult<u64> {
        self.inner
            .pwrite_from(offset, &src.inner, src_offset, length)
            .map_err(ioerr)
    }

    // ---- cursor ------------------------------------------------------------------------

    /// The current cursor position (bytes from the start). May sit past the end after a seek.
    #[getter]
    fn position(&self) -> u64 {
        self.inner.position()
    }

    /// io-file-like: the current cursor position — the same value as the
    /// [`position`](LocalIO::position) getter, under the `io` object's method name.
    fn tell(&self) -> u64 {
        self.inner.position()
    }

    /// Moves the cursor to an absolute `position` (past the end is allowed).
    fn set_position(&mut self, position: u64) {
        self.inner.set_position(position);
    }

    /// Seeks to `whence + offset` and returns the new position. A position past the end is
    /// allowed; seeking before the start raises `ValueError`.
    fn seek(&mut self, whence: Whence, offset: i64) -> PyResult<u64> {
        self.inner.seek(whence.into(), offset).map_err(ioerr)
    }

    /// Resets the cursor to the start.
    fn rewind(&mut self) {
        self.inner.rewind();
    }

    /// **Cursor read.** Returns up to `length` bytes from the current position (short near the
    /// end), advancing the cursor by the number read.
    fn read<'py>(&mut self, py: Python<'py>, length: usize) -> PyResult<Bound<'py, PyBytes>> {
        let position = self.inner.position();
        let n = self
            .inner
            .byte_size()
            .saturating_sub(position)
            .min(length as u64) as usize;
        let bytes = PyBytes::new_bound_with(py, n, |dst| {
            self.inner.pread_byte_array(position, dst);
            Ok(())
        })?;
        self.inner.set_position(position + n as u64);
        Ok(bytes)
    }

    /// **Cursor write.** Writes `data` (bytes / bytearray) at the current position, advancing
    /// the cursor by the number written (auto-creating + growing the file as needed); returns
    /// that count.
    fn write(&mut self, data: Vec<u8>) -> usize {
        self.inner.write(&data)
    }

    /// Reads the next byte at the cursor, advancing it by 1, raising `ValueError` at the end.
    fn read_byte(&mut self) -> PyResult<u8> {
        self.inner.read_byte().map_err(ioerr)
    }

    /// Writes the byte `value` at the cursor, advancing it by 1.
    fn write_byte(&mut self, value: u8) -> PyResult<()> {
        self.inner.write_byte(value).map_err(ioerr)
    }

    /// Reads a little-endian `i32` (4 bytes) at the cursor, advancing it by 4, raising
    /// `ValueError` on EOF.
    fn read_i32(&mut self) -> PyResult<i32> {
        self.inner.read_i32().map_err(ioerr)
    }

    /// Writes `value` as a little-endian `i32` (4 bytes) at the cursor, advancing it by 4.
    fn write_i32(&mut self, value: i32) -> PyResult<()> {
        self.inner.write_i32(value).map_err(ioerr)
    }

    /// Reads a little-endian `i64` (8 bytes) at the cursor, advancing it by 8, raising
    /// `ValueError` on EOF.
    fn read_i64(&mut self) -> PyResult<i64> {
        self.inner.read_i64().map_err(ioerr)
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at the cursor, advancing it by 8.
    fn write_i64(&mut self, value: i64) -> PyResult<()> {
        self.inner.write_i64(value).map_err(ioerr)
    }

    /// Reads up to `length` **bytes** from the cursor and decodes them as UTF-8 text (clamped
    /// near the end), advancing the cursor by the bytes read, raising a guided `ValueError`
    /// on invalid UTF-8 (leaving the cursor put).
    fn read_utf8(&mut self, length: usize) -> PyResult<String> {
        self.inner.read_utf8(length).map_err(ioerr)
    }

    /// Writes `text`'s UTF-8 bytes at the cursor, advancing it; returns the number of
    /// **bytes** written.
    fn write_utf8(&mut self, text: &str) -> usize {
        self.inner.write_utf8(text)
    }

    /// Reads from the current position **to the end** as `bytes`, advancing the cursor to the
    /// end.
    fn read_to_end<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let position = self.inner.position();
        let n = self.inner.byte_size().saturating_sub(position) as usize;
        let bytes = PyBytes::new_bound_with(py, n, |dst| {
            self.inner.pread_byte_array(position, dst);
            Ok(())
        })?;
        self.inner.set_position(self.inner.byte_size());
        Ok(bytes)
    }

    /// **Reads one line** from the cursor — the bytes through the next `\n` **inclusive** (or
    /// to the end if none), decoded as UTF-8 — advancing the cursor past it. Returns `""`
    /// **only** at the true end (a blank line keeps its `\n`).
    fn readline(&mut self) -> PyResult<String> {
        self.inner.readline().map_err(ioerr)
    }

    /// **Reads every remaining line** from the cursor into a list, advancing it to the end —
    /// each element keeps its trailing `\n` except possibly the last.
    fn readlines(&mut self) -> PyResult<Vec<String>> {
        self.inner.readlines().map_err(ioerr)
    }

    // ---- address (uri) -----------------------------------------------------------------

    /// The [`Uri`] that **addresses** this node — its file path (built with
    /// `Uri.from_path`, so back-slashes read as forward slashes).
    #[getter]
    fn uri(&self) -> Uri {
        Uri {
            inner: self.inner.uri(),
        }
    }

    // ---- metadata (headers / mode / kind) ------------------------------------------------

    /// The [`Headers`] metadata attached to this handle — returned as an owned **copy** (the
    /// binding cannot borrow into the Rust value); mutate the copy and write it back with
    /// [`set_headers`](LocalIO::set_headers).
    #[getter]
    fn headers(&self) -> Headers {
        Headers {
            inner: self.inner.headers().clone(),
        }
    }

    /// Replaces the whole [`Headers`] metadata map in place.
    fn set_headers(&mut self, headers: &Headers) {
        *self.inner.headers_mut() = headers.inner.clone();
    }

    /// How this handle may be accessed — [`IOMode.ReadWrite`](IOMode::ReadWrite) by default;
    /// writes check it before touching the disk.
    #[getter]
    fn mode(&self) -> IOMode {
        self.inner.mode().into()
    }

    /// Sets the access [`IOMode`] label in place (writes check it before touching the disk).
    fn set_mode(&mut self, mode: IOMode) {
        self.inner.set_mode(mode.into());
    }

    /// What this node **is right now** — a per-call disk probe:
    /// [`IOKind.File`](IOKind::File), [`IOKind.Directory`](IOKind::Directory), or
    /// [`IOKind.Missing`](IOKind::Missing).
    #[getter]
    fn kind(&self) -> IOKind {
        self.inner.kind().into()
    }

    // ---- predicates (is_file / is_dir / exists) ------------------------------------------

    /// Whether this node is a regular **file** — derived from [`kind`](LocalIO::kind), a
    /// per-call disk probe.
    fn is_file(&self) -> bool {
        self.inner.is_file()
    }

    /// Whether this node is a **directory** — derived from [`kind`](LocalIO::kind).
    fn is_dir(&self) -> bool {
        self.inner.is_dir()
    }

    /// Whether anything **exists** at this path — `is_file() or is_dir()`, asked of the
    /// disk per call.
    fn exists(&self) -> bool {
        self.inner.exists()
    }

    // ---- media type (declared headers, else the file address, else octet-stream) ---------

    /// The **primary** [`MimeType`](crate::mimetype::MimeType) of this node: the `Content-Type`
    /// its [`headers`](LocalIO::headers) declare, else inferred from the [`uri`](LocalIO::uri)'s
    /// file name (e.g. `report.pdf` → `application/pdf`), else the `application/octet-stream`
    /// fallback — always an answer.
    fn mime_type(&self) -> MimeType {
        MimeType {
            inner: self.inner.mime_type(),
        }
    }

    /// The full [`MediaType`](crate::mediatype::MediaType) of this node: the media the
    /// `Content-Type` / `Content-Encoding` [`headers`](LocalIO::headers) declare, else inferred
    /// from the file's extensions (`archive.tar.gz` → `application/x-tar, application/gzip`),
    /// else the single `application/octet-stream` fallback.
    fn media_type(&self) -> MediaType {
        MediaType {
            inner: self.inner.media_type(),
        }
    }

    /// Resolves the media type **and stores it** in this node's headers when `Content-Type` is
    /// not already set — memoizing the inference so later reads come straight from
    /// [`headers`](LocalIO::headers). Returns the effective
    /// [`MimeType`](crate::mimetype::MimeType).
    fn ensure_content_type(&mut self) -> MimeType {
        MimeType {
            inner: self.inner.ensure_content_type(),
        }
    }

    // ---- inference + compression (magic-inferred type; codec over the bytes) -------------

    /// The **primary** [`MimeType`](crate::mimetype::MimeType) inferred from this node's
    /// **magic bytes** — a positioned read of the head that **never moves the cursor**; falls
    /// back to the declared/address [`mime_type`](LocalIO::mime_type) when no magic matches.
    fn infer_mime_type(&self) -> MimeType {
        MimeType {
            inner: self.inner.infer_mime_type(),
        }
    }

    /// The full [`MediaType`](crate::mediatype::MediaType) inferred by **recursive magic** — the
    /// head's type, then the type inside each compression layer it can peel (a gzipped tar reads
    /// as `[application/gzip, application/x-tar]`). The head is read positioned (no cursor seek).
    fn infer_media_type(&self) -> MediaType {
        MediaType {
            inner: self.inner.infer_media_type(),
        }
    }

    /// The `yggdryl.compression` codec for this node's media type (headers, else the file
    /// address), or `None` when the type is not a supported compression.
    fn compression(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        crate::compression::codec_to_object(py, self.inner.compression())
    }

    /// This node **decompressed** with the codec inferred from its **media type**, as `bytes` —
    /// raises a guided `ValueError` when the node is not a supported compression.
    fn decompress<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let out = self.inner.decompress().map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// This node's whole content **compressed** with the explicit `codec` (a
    /// `yggdryl.compression` codec), as `bytes`.
    fn compress_with<'py>(
        &self,
        py: Python<'py>,
        codec: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let out = crate::compression::with_codec(codec, |c| self.inner.compressed_with(c))?
            .map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// This node's whole content **decompressed** with the explicit `codec`, as `bytes` —
    /// raises a guided `ValueError` on a corrupt stream.
    fn decompress_with<'py>(
        &self,
        py: Python<'py>,
        codec: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let out = crate::compression::with_codec(codec, |c| self.inner.decompressed_with(c))?
            .map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    // DESIGN: the byte-returning `decompress` / `compress_with` / `decompress_with` are the
    // ergonomic mirror of the core's compress/decompress family; the generic `compress_into` /
    // `decompress_into` (source-to-source) and `as_bytes` are deliberately not exposed.

    /// **Compresses this node in place** — replaces the file's bytes with the compressed form
    /// and updates the `Content-Type` / `Content-Length` / `mtime` headers. `codec` (a
    /// `yggdryl.compression` codec) defaults to the codec of the node's own media type (so a
    /// `.gz`-addressed file packs itself gzip); pass an explicit one to override. Raises a
    /// guided `ValueError` when no codec applies.
    #[pyo3(signature = (codec = None))]
    fn compress_in_place(&mut self, codec: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        match codec {
            Some(codec) => {
                crate::compression::with_codec(codec, |c| self.inner.compress_in_place(Some(c)))?
                    .map_err(ioerr)
            }
            None => self.inner.compress_in_place(None).map_err(ioerr),
        }
    }

    /// **Decompresses this node in place** — replaces the compressed bytes with the plain
    /// content (codec inferred from its media type) and updates the size/media/mtime headers.
    /// Raises a guided `ValueError` when the node is not a supported compression.
    fn decompress_in_place(&mut self) -> PyResult<()> {
        self.inner.decompress_in_place().map_err(ioerr)
    }

    // ---- graph: navigation + discovery + CRUD --------------------------------------------

    /// The last path segment — the node's own name (empty for a root).
    #[getter]
    fn name(&self) -> String {
        self.inner.name()
    }

    /// The parent node as a fresh lazy handle, or `None` at a root.
    fn parent(&self) -> Option<LocalIO> {
        self.inner.parent().map(|inner| LocalIO { inner })
    }

    /// This node's **ancestors** as a list of fresh lazy handles, nearest first — the repeated
    /// [`parent`](LocalIO::parent) chain up to the filesystem root (empty at a root). The
    /// node-graph counterpart of [`Uri.parents`](crate::uri::Uri::parents).
    fn parents(&self) -> Vec<LocalIO> {
        self.inner
            .parents()
            .map(|inner| LocalIO { inner })
            .collect()
    }

    /// The child node at `segment` (which may be a multi-segment relative path like
    /// `"a/b/c.txt"`) — **lazy**: nothing is touched or created.
    fn join(&self, segment: &str) -> LocalIO {
        LocalIO {
            inner: self.inner.join_str(segment),
        }
    }

    /// `node / "a/b.txt"` — the operator spelling of [`join`](LocalIO::join).
    fn __truediv__(&self, segment: &str) -> LocalIO {
        self.join(segment)
    }

    /// **Streams** the node's children as a [`LocalEntries`] iterator of lazy handles — the
    /// direct children by default, the **entire subtree** (depth-first) with
    /// `recursive=True`. Entries are produced as the caller pulls (`for entry in node.ls():`)
    /// — never a pre-collected tree; use [`children`](LocalIO::children) for the collected
    /// direct-children convenience. A file or missing node streams nothing. Raises a guided
    /// `ValueError` when the directory cannot be listed — up front for the node itself, or
    /// from the yielding step for an entry inside the walk.
    #[pyo3(signature = (recursive = false))]
    fn ls(&self, recursive: bool) -> PyResult<LocalEntries> {
        let inner = if recursive {
            Entries::Walk(self.inner.ls_recursive().map_err(ioerr)?)
        } else {
            Entries::Children(Box::new(self.inner.ls().map_err(ioerr)?))
        };
        Ok(LocalEntries { inner })
    }

    /// The direct children, collected — the same list as [`ls`](LocalIO::ls) without the
    /// `recursive` switch.
    fn children(&self) -> PyResult<Vec<LocalIO>> {
        self.inner
            .children()
            .map(|nodes| nodes.into_iter().map(|inner| LocalIO { inner }).collect())
            .map_err(ioerr)
    }

    /// Removes **whatever exists** at this node — a file is unlinked, a directory is removed
    /// with its whole subtree. `exist_ok` (default `True`) governs a **missing** node:
    /// `True` skips it (a no-op), `exist_ok=False` raises the guided `ValueError`. The
    /// generic form of [`rmfile`](LocalIO::rmfile) / [`rmdir`](LocalIO::rmdir).
    #[pyo3(signature = (exist_ok = true))]
    fn rm(&self, exist_ok: bool) -> PyResult<()> {
        self.inner.rm(exist_ok).map_err(ioerr)
    }

    /// Removes this node **as a file** — raises the guided `ValueError` when it is a
    /// directory (use [`rmdir`](LocalIO::rmdir)). `exist_ok` (default `True`) skips a
    /// missing node; `exist_ok=False` raises on one.
    #[pyo3(signature = (exist_ok = true))]
    fn rmfile(&self, exist_ok: bool) -> PyResult<()> {
        self.inner.rmfile(exist_ok).map_err(ioerr)
    }

    /// Removes this node **as a directory**, recursively — raises the guided `ValueError`
    /// when it is a file (use [`rmfile`](LocalIO::rmfile)). `exist_ok` (default `True`) skips
    /// a missing node; `exist_ok=False` raises on one.
    #[pyo3(signature = (exist_ok = true))]
    fn rmdir(&self, exist_ok: bool) -> PyResult<()> {
        self.inner.rmdir(exist_ok).map_err(ioerr)
    }

    // ---- live-handle dunders -----------------------------------------------------------

    /// A fresh **lazy** handle to the same path (equivalent to `copy.copy(node)`) — the
    /// mapped backing is deliberately not shared; path, headers, and mode are copied.
    fn copy(&self) -> Self {
        self.clone()
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// Handles compare by **path** (the value identity lives in [`uri`](LocalIO::uri)).
    /// Defining equality leaves `LocalIO` unhashable, like every mutable source.
    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    /// Line iteration — `for line in node:` yields each line from the cursor (like an open
    /// file object), via [`readline`](LocalIO::readline). Streamed *child* discovery stays on
    /// [`ls`](LocalIO::ls) / [`children`](LocalIO::children).
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// The next line from the cursor, or `StopIteration` at the true end.
    fn __next__(&mut self) -> PyResult<Option<String>> {
        let line = self.inner.readline().map_err(ioerr)?;
        Ok((!line.is_empty()).then_some(line))
    }

    /// This handle's **portable** address string — its `file://` URI with a home / temp path
    /// folded to a `~` / `$TMP` token (see [`yggdryl.uri.Uri.to_portable_str`](crate::uri::Uri)),
    /// so it relocates across environments. The form [`from_portable`](LocalIO::from_portable)
    /// and pickle use.
    fn to_portable_str(&self) -> String {
        self.inner.uri().to_portable_str()
    }

    /// Reconstructs a **lazy** handle from the [`to_portable_str`](LocalIO::to_portable_str)
    /// form — expanding a `~` / `$TMP` token against **this** environment's home / temp roots.
    /// The unpickling half: a handle addressing `~/data` on one machine reopens under this
    /// machine's home. The result is lazy (no mapping), like `copy()`.
    #[staticmethod]
    fn from_portable(portable: &str) -> PyResult<LocalIO> {
        let uri = yggdryl_core::uri::Uri::from_portable_str(portable)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        local::LocalIO::from_uri(&uri)
            .map(|inner| LocalIO { inner })
            .map_err(ioerr)
    }

    /// Pickles the handle by its **portable address** (`from_portable(to_portable_str())`), so a
    /// handle addressing a home / temp path is reconstructed against the receiving environment's
    /// home / temp roots — a live handle *is* its path identity (the mapping and cursor are
    /// transient and are not part of the pickled value).
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (String,))> {
        let ctor = py
            .get_type_bound::<LocalIO>()
            .getattr("from_portable")?
            .unbind();
        Ok((ctor, (self.inner.uri().to_portable_str(),)))
    }

    fn __repr__(&self) -> String {
        format!(
            "LocalIO({}, <{} bytes>)",
            self.inner.as_std_path().to_string_lossy(),
            self.inner.byte_size()
        )
    }
}

#[pymethods]
impl LocalIO {
    /// **Eagerly memory-maps** this node's file (when it is a regular file) so later reads run
    /// at memory speed and concurrent readers share one mapping — the explicit form of the
    /// self-optimizing backing that a write would otherwise create lazily. A no-op when the
    /// handle is already mapped, or when nothing exists yet at the path (reads stay ad-hoc /
    /// empty). Raises a guided `ValueError` on an OS mapping failure.
    fn load(&mut self) -> PyResult<()> {
        self.inner.load().map_err(ioerr)
    }

    /// **Moves** this node's whole content into `dst` (another `LocalIO`) and **removes this
    /// node** — a copy that consumes its origin (`mv` over the byte contract). Returns the
    /// number of bytes moved. A no-op when `self` and `dst` resolve to the same path (a file
    /// never moves onto itself); the mapped backing is released first so the source file can be
    /// unlinked even on platforms that refuse to delete a mapped file.
    fn move_into(&mut self, mut dst: PyRefMut<'_, LocalIO>) -> PyResult<u64> {
        self.inner.move_into(&mut dst.inner).map_err(ioerr)
    }
}

// The remaining native-width scalar, cursor-typed, and bulk-array accessors — completing the
// set alongside the hand-written `i32` / `i64` / byte forms in the main block above.
scalar_methods!(
    LocalIO,
    (i8, pread_i8, pwrite_i8),
    (u8, pread_u8, pwrite_u8),
    (i16, pread_i16, pwrite_i16),
    (u16, pread_u16, pwrite_u16),
    (u32, pread_u32, pwrite_u32),
    (u64, pread_u64, pwrite_u64),
    (i128, pread_i128, pwrite_i128),
    (u128, pread_u128, pwrite_u128),
    (f32, pread_f32, pwrite_f32),
    (f64, pread_f64, pwrite_f64),
);
cursor_typed_methods!(
    LocalIO,
    (i8, read_i8, write_i8),
    (u8, read_u8, write_u8),
    (i16, read_i16, write_i16),
    (u16, read_u16, write_u16),
    (u32, read_u32, write_u32),
    (u64, read_u64, write_u64),
    (i128, read_i128, write_i128),
    (u128, read_u128, write_u128),
    (f32, read_f32, write_f32),
    (f64, read_f64, write_f64),
);
bulk_methods!(
    LocalIO,
    (i8, 1, pread_i8_array, pwrite_i8_array, pwrite_i8_repeat),
    (i16, 2, pread_i16_array, pwrite_i16_array, pwrite_i16_repeat),
    (
        i128,
        16,
        pread_i128_array,
        pwrite_i128_array,
        pwrite_i128_repeat
    ),
    (
        u128,
        16,
        pread_u128_array,
        pwrite_u128_array,
        pwrite_u128_repeat
    ),
);

/// The core streamed iterator a [`LocalEntries`] wraps — one level
/// (`yggdryl_core::io::local::LocalChildren`) or the depth-first subtree walk
/// (`yggdryl_core::io::local::LocalWalk`); both are owned iterators, so the binding holds
/// them directly (the one-level iterator boxed — its OS `ReadDir` state dwarfs the walk's).
/// A **leaf** node ([`Mmap`]) instead streams the shared always-empty
/// [`yggdryl.memory.NoChildren`](crate::io::memory::NoChildren), the core's `NoChildren`.
enum Entries {
    Children(Box<local::LocalChildren>),
    Walk(local::LocalWalk),
}

/// The **streaming** iterator returned by [`ls`](LocalIO::ls) — entries are produced one at
/// a time as the caller pulls (house rule: discovery is streamed, never a pre-collected
/// tree). `__iter__` returns the iterator itself; `__next__` yields each child as a fresh
/// lazy [`LocalIO`], raises the guided `ValueError` (the core error text unchanged) for an
/// entry that cannot be produced, and `StopIteration` at the end.
#[pyclass(module = "yggdryl.local")]
pub struct LocalEntries {
    inner: Entries,
}

#[pymethods]
impl LocalEntries {
    /// The iterator protocol — `iter(entries) is entries`, like every Python iterator.
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// The next child as a fresh lazy [`LocalIO`]; `StopIteration` when the stream is
    /// exhausted, the guided `ValueError` for a failing entry.
    fn __next__(&mut self) -> PyResult<Option<LocalIO>> {
        let entry = match &mut self.inner {
            Entries::Children(children) => children.next(),
            Entries::Walk(walk) => walk.next(),
        };
        entry
            .map(|entry| entry.map(|inner| LocalIO { inner }).map_err(ioerr))
            .transpose()
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            Entries::Children(_) => "LocalEntries(<children>)".to_string(),
            Entries::Walk(_) => "LocalEntries(<recursive walk>)".to_string(),
        }
    }
}

/// A **memory-mapped file** — the on-disk implementor of the byte-access contract, sharing
/// [`Heap`](crate::io::memory::Heap)'s full surface (positioned + typed + bulk access, the
/// built-in cursor stream, capacity management, metadata) over a file instead of an owned
/// buffer. Opened from a `str` path or a [`Uri`] via [`open`](Mmap::open) /
/// [`open_readonly`](Mmap::open_readonly) / [`create`](Mmap::create); a write past the end
/// grows the file (amortized, page-aligned), and [`close`](Mmap::close) — or the end of a
/// `with` block, or garbage collection — unmaps the view and truncates the on-disk file back
/// to its exact logical length. On the IO graph a mapping is a **leaf**: [`name`](Mmap::name)
/// is its file name, [`ls`](Mmap::ls) / [`children`](Mmap::children) stream/collect nothing,
/// [`parent`](Mmap::parent) is `None`, and [`rm`](Mmap::rm) / [`rmfile`](Mmap::rmfile)
/// really unlink the file ([`rmdir`](Mmap::rmdir) is the guided file error).
///
/// Unlike [`Heap`](crate::io::memory::Heap), an `Mmap` is a **live OS resource, not a
/// value**: two independent mappings of one file would alias, so it is deliberately not
/// equatable, copyable, serializable, or picklable — no `__eq__`, no `copy`, no
/// `serialize_bytes` / pickle, and no `with_*` builders (each would need a copy). Use it as
/// a context manager (`with Mmap.create(path) as m:`) or call [`close`](Mmap::close)
/// explicitly; any access after closing raises a guided `ValueError`.
#[pyclass(module = "yggdryl.local")]
pub struct Mmap {
    /// `None` once closed — every access goes through [`Mmap::io`] / [`Mmap::io_mut`].
    pub(crate) inner: Option<local::Mmap>,
}

/// The guided error for any access to a closed mapping.
fn closed_err() -> PyErr {
    PyValueError::new_err(
        "the mapping is closed; reopen it with Mmap.open / Mmap.open_readonly / Mmap.create",
    )
}

impl Mmap {
    /// The live mapping, or the guided closed `ValueError`.
    fn io(&self) -> PyResult<&local::Mmap> {
        self.inner.as_ref().ok_or_else(closed_err)
    }

    /// The live mapping mutably, or the guided closed `ValueError`.
    fn io_mut(&mut self) -> PyResult<&mut local::Mmap> {
        self.inner.as_mut().ok_or_else(closed_err)
    }
}

/// Resolves the generic `source` (a `str` path or a [`Uri`]) through the matching pair of
/// explicit core constructors — the shared dispatch behind [`Mmap::open`] /
/// [`Mmap::open_readonly`] / [`Mmap::create`].
fn mmap_from(
    source: &Bound<'_, PyAny>,
    verb: &'static str,
    from_path: fn(&str) -> Result<local::Mmap, IoError>,
    from_uri: fn(&yggdryl_core::uri::Uri) -> Result<local::Mmap, IoError>,
) -> PyResult<Mmap> {
    if let Ok(path) = source.extract::<String>() {
        from_path(&path)
            .map(|inner| Mmap { inner: Some(inner) })
            .map_err(ioerr)
    } else if let Ok(uri) = source.extract::<PyRef<'_, Uri>>() {
        from_uri(&uri.inner)
            .map(|inner| Mmap { inner: Some(inner) })
            .map_err(ioerr)
    } else {
        Err(PyTypeError::new_err(format!(
            "cannot {verb} a mapping from {}: expected a str filesystem path or a \
             yggdryl.uri.Uri (pass str(path) for a pathlib.Path)",
            source.repr()?
        )))
    }
}

#[pymethods]
impl Mmap {
    // There is deliberately no `Mmap(...)` constructor — the explicit lifecycle verbs
    // `open` / `open_readonly` / `create` are the only entry points.

    // ---- constructors (generic dispatch over the explicit core pairs) ------------------

    /// Opens an **existing** file read-write — the generic, type-inferring entry point: a
    /// `str` path dispatches to the core `open_path`, a [`Uri`] (`file://…` or a plain path)
    /// to `open_uri`. Raises a guided `ValueError` naming the path if it is missing or
    /// inaccessible.
    #[staticmethod]
    fn open(source: &Bound<'_, PyAny>) -> PyResult<Mmap> {
        mmap_from(
            source,
            "open",
            |path| local::Mmap::open_path(path),
            local::Mmap::open_uri,
        )
    }

    /// Opens an **existing** file **read-only**: reads work, the write primitives write
    /// nothing (count `0`), and the full/typed writes raise the guided read-only error. Same
    /// `str` / [`Uri`] dispatch as [`open`](Mmap::open).
    #[staticmethod]
    fn open_readonly(source: &Bound<'_, PyAny>) -> PyResult<Mmap> {
        mmap_from(
            source,
            "open",
            |path| local::Mmap::open_path_readonly(path),
            local::Mmap::open_uri_readonly,
        )
    }

    /// Opens the file read-write, **creating it empty** if it does not exist (existing
    /// contents are kept — never truncated on open). Same `str` / [`Uri`] dispatch as
    /// [`open`](Mmap::open).
    #[staticmethod]
    fn create(source: &Bound<'_, PyAny>) -> PyResult<Mmap> {
        mmap_from(
            source,
            "create",
            |path| local::Mmap::create_path(path),
            local::Mmap::create_uri,
        )
    }

    // ---- lifecycle: close + context manager --------------------------------------------

    /// Closes the mapping: unmaps the view and truncates the on-disk file to its exact
    /// logical length. **Idempotent** — closing twice is a no-op; any other access after
    /// `close` raises the guided closed `ValueError`.
    fn close(&mut self) {
        self.inner = None;
    }

    /// Whether the mapping has been closed (like a file object's `closed`).
    #[getter]
    fn closed(&self) -> bool {
        self.inner.is_none()
    }

    /// Context-manager entry — returns the mapping itself, so `with Mmap.create(p) as m:`
    /// binds the open mapping.
    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Context-manager exit — [`close`](Mmap::close)s the mapping; exceptions propagate.
    fn __exit__(
        &mut self,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> bool {
        self.close();
        false
    }

    // ---- file inherent: path + flush ---------------------------------------------------

    /// The file path this mapping is backed by.
    #[getter]
    fn path(&self) -> PyResult<String> {
        Ok(self.io()?.path().to_string_lossy().into_owned())
    }

    /// Flushes the mapped bytes (and file metadata) to disk — `msync` / `FlushViewOfFile`
    /// plus an fsync. Raises a guided `ValueError` on OS failure.
    fn flush(&self) -> PyResult<()> {
        self.io()?.flush().map_err(ioerr)
    }

    // ---- size + capacity ---------------------------------------------------------------

    /// The **logical** length in bytes (the mapped file extent may be larger — see
    /// [`capacity`](Mmap::capacity)).
    fn byte_size(&self) -> PyResult<u64> {
        Ok(self.io()?.byte_size())
    }

    /// The logical length in bytes (so `len(mmap)` works).
    fn __len__(&self) -> PyResult<usize> {
        Ok(self.io()?.byte_size() as usize)
    }

    /// The total length in bits — `byte_size() * 8`.
    fn bit_size(&self) -> PyResult<u64> {
        Ok(self.io()?.bit_size())
    }

    /// The mapped (on-disk) extent in bytes — grows amortized (doubling, page-aligned) when
    /// a write lands past the end, exactly like `Heap`'s reallocation curve.
    fn capacity(&self) -> PyResult<u64> {
        Ok(self.io()?.capacity())
    }

    /// Reserves capacity for at least `additional` more bytes past the current size,
    /// amortizing later writes. Best-effort on a file — prefer
    /// [`try_reserve`](Mmap::try_reserve) to see a failure.
    fn reserve(&mut self, additional: u64) -> PyResult<()> {
        self.io_mut()?.reserve(additional);
        Ok(())
    }

    /// The spare room already mapped — `capacity() - byte_size()`, the bytes that can be
    /// appended before the next remap.
    fn spare_capacity(&self) -> PyResult<u64> {
        Ok(self.io()?.spare_capacity())
    }

    /// Reserves capacity for **exactly** `additional` more bytes — no amortized
    /// over-allocation, for a caller that knows the final size.
    fn reserve_exact(&mut self, additional: u64) -> PyResult<()> {
        self.io_mut()?.reserve_exact(additional);
        Ok(())
    }

    /// **Checked** reservation: raises a guided `ValueError` (overflow, or the OS refusing
    /// to grow/remap the file) instead of failing silently.
    fn try_reserve(&mut self, additional: u64) -> PyResult<()> {
        self.io_mut()?.try_reserve(additional).map_err(ioerr)
    }

    /// **Checked exact** reservation — `try_reserve` without the amortized over-allocation.
    fn try_reserve_exact(&mut self, additional: u64) -> PyResult<()> {
        self.io_mut()?.try_reserve_exact(additional).map_err(ioerr)
    }

    /// Ensures the **total** capacity is at least `total` bytes (the absolute-target form of
    /// `reserve`); a no-op when already satisfied, never shrinks.
    fn ensure_capacity(&mut self, total: u64) -> PyResult<()> {
        self.io_mut()?.ensure_capacity(total);
        Ok(())
    }

    /// **Checked** `ensure_capacity` — raises a guided `ValueError` instead of failing
    /// silently.
    fn try_ensure_capacity(&mut self, total: u64) -> PyResult<()> {
        self.io_mut()?.try_ensure_capacity(total).map_err(ioerr)
    }

    /// Truncates the mapped file back to the logical length, releasing the capacity padding
    /// on disk.
    fn shrink_to_fit(&mut self) -> PyResult<()> {
        self.io_mut()?.shrink_to_fit();
        Ok(())
    }

    /// Shrinks the mapped extent toward `min_capacity` (never below `byte_size()`).
    fn shrink_to(&mut self, min_capacity: u64) -> PyResult<()> {
        self.io_mut()?.shrink_to(min_capacity);
        Ok(())
    }

    /// Sets the mapped file's byte length to exactly `length` — shrinking (dropping the tail)
    /// or extending (zero-filling) — then syncs the size headers. Raises the guided
    /// `ValueError` on a read-only mapping.
    fn truncate(&mut self, length: u64) -> PyResult<()> {
        self.io_mut()?.truncate(length).map_err(ioerr)
    }

    /// The content length in bytes, **preferring the cached `Content-Length` header** when
    /// present and falling back to the live `byte_size()`.
    fn content_length(&self) -> PyResult<u64> {
        Ok(self.io()?.content_length())
    }

    /// Whether the file holds no bytes (`byte_size() == 0`).
    fn is_empty(&self) -> PyResult<bool> {
        Ok(self.io()?.is_empty())
    }

    /// Truthiness — `True` when the file holds at least one byte (like `bytearray`).
    fn __bool__(&self) -> PyResult<bool> {
        Ok(!self.io()?.is_empty())
    }

    // ---- positioned byte-array ---------------------------------------------------------

    /// **Positioned read.** Returns up to `length` bytes starting at `offset` as `bytes` —
    /// short near the end, empty at or past it. Never moves the cursor. Reads **directly**
    /// into the `bytes` allocation (one copy).
    fn pread_byte_array<'py>(
        &self,
        py: Python<'py>,
        offset: u64,
        length: usize,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let io = self.io()?;
        let n = io.byte_size().saturating_sub(offset).min(length as u64) as usize;
        PyBytes::new_bound_with(py, n, |dst| {
            io.pread_byte_array(offset, dst);
            Ok(())
        })
    }

    /// **Positioned write.** Copies `data` (bytes / bytearray) in at `offset`, growing the
    /// file and zero-filling any gap; returns the number of bytes written (`0` on a
    /// read-only mapping).
    fn pwrite_byte_array(&mut self, offset: u64, data: Vec<u8>) -> PyResult<usize> {
        Ok(self.io_mut()?.pwrite_byte_array(offset, &data))
    }

    // ---- positioned typed accessors ----------------------------------------------------

    /// Reads the single byte at `offset`, raising `ValueError` if it is past the end.
    fn pread_byte(&self, offset: u64) -> PyResult<u8> {
        self.io()?.pread_byte(offset).map_err(ioerr)
    }

    /// Writes the single byte `value` at `offset`, growing the file as needed.
    fn pwrite_byte(&mut self, offset: u64, value: u8) -> PyResult<()> {
        self.io_mut()?.pwrite_byte(offset, value).map_err(ioerr)
    }

    /// Reads the bit at absolute **bit** `offset` (LSB-first: bit `offset % 8` of byte
    /// `offset / 8`), raising `ValueError` if its byte is past the end.
    fn pread_bit(&self, offset: u64) -> PyResult<bool> {
        self.io()?.pread_bit(offset).map_err(ioerr)
    }

    /// Sets or clears the bit at absolute **bit** `offset` (LSB-first), growing the file
    /// (zero-filled) if the bit is past the end.
    fn pwrite_bit(&mut self, offset: u64, value: bool) -> PyResult<()> {
        self.io_mut()?.pwrite_bit(offset, value).map_err(ioerr)
    }

    /// Reads a little-endian `i32` (4 bytes) at `offset`, raising `ValueError` on EOF.
    fn pread_i32(&self, offset: u64) -> PyResult<i32> {
        self.io()?.pread_i32(offset).map_err(ioerr)
    }

    /// Writes `value` as a little-endian `i32` (4 bytes) at `offset`, growing as needed.
    fn pwrite_i32(&mut self, offset: u64, value: i32) -> PyResult<()> {
        self.io_mut()?.pwrite_i32(offset, value).map_err(ioerr)
    }

    /// Reads a little-endian `i64` (8 bytes) at `offset`, raising `ValueError` on EOF.
    fn pread_i64(&self, offset: u64) -> PyResult<i64> {
        self.io()?.pread_i64(offset).map_err(ioerr)
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at `offset`, growing as needed.
    fn pwrite_i64(&mut self, offset: u64, value: i64) -> PyResult<()> {
        self.io_mut()?.pwrite_i64(offset, value).map_err(ioerr)
    }

    /// Reads up to `length` **bytes** at `offset` and decodes them as UTF-8 text (clamped
    /// near the end), raising a guided `ValueError` on invalid UTF-8 — including a
    /// multi-byte character cut by the range.
    fn pread_utf8(&self, offset: u64, length: usize) -> PyResult<String> {
        self.io()?.pread_utf8(offset, length).map_err(ioerr)
    }

    /// Writes `text`'s UTF-8 bytes at `offset` (growing as needed); returns the number of
    /// **bytes** written.
    fn pwrite_utf8(&mut self, offset: u64, text: &str) -> PyResult<usize> {
        Ok(self.io_mut()?.pwrite_utf8(offset, text))
    }

    // ---- bulk typed arrays + repeated fills ----------------------------------------------

    /// **Bulk typed read.** Returns `count` little-endian `i32`s starting at `offset` as a
    /// list, raising `ValueError` if fewer bytes remain — checked **before** the result is
    /// allocated, so a hostile `count` fails fast instead of allocating.
    fn pread_i32_array(&self, offset: u64, count: usize) -> PyResult<Vec<i32>> {
        let io = self.io()?;
        let available = io.byte_size().saturating_sub(offset);
        if count.saturating_mul(4) as u64 > available {
            return Err(ioerr(IoError::UnexpectedEof {
                offset: offset + available,
                requested: count.saturating_mul(4),
                available: available as usize,
            }));
        }
        let mut values = vec![0i32; count];
        io.pread_i32_array(offset, &mut values).map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write.** Writes all of `values` as little-endian `i32`s at `offset`,
    /// growing as needed.
    fn pwrite_i32_array(&mut self, offset: u64, values: Vec<i32>) -> PyResult<()> {
        self.io_mut()?
            .pwrite_i32_array(offset, &values)
            .map_err(ioerr)
    }

    /// **Bulk typed read** of `count` little-endian `i64`s — the wide counterpart of
    /// [`pread_i32_array`](Mmap::pread_i32_array), with the same fail-fast bounds check
    /// before the result is allocated.
    fn pread_i64_array(&self, offset: u64, count: usize) -> PyResult<Vec<i64>> {
        let io = self.io()?;
        let available = io.byte_size().saturating_sub(offset);
        if count.saturating_mul(8) as u64 > available {
            return Err(ioerr(IoError::UnexpectedEof {
                offset: offset + available,
                requested: count.saturating_mul(8),
                available: available as usize,
            }));
        }
        let mut values = vec![0i64; count];
        io.pread_i64_array(offset, &mut values).map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write** of little-endian `i64`s — the wide counterpart of
    /// [`pwrite_i32_array`](Mmap::pwrite_i32_array).
    fn pwrite_i64_array(&mut self, offset: u64, values: Vec<i64>) -> PyResult<()> {
        self.io_mut()?
            .pwrite_i64_array(offset, &values)
            .map_err(ioerr)
    }

    /// **Repeated-value fill.** Writes `count` copies of the byte `value` at `offset`
    /// (growing as needed) without ever materializing the full array — the `memset` of the
    /// family.
    fn pwrite_byte_repeat(&mut self, offset: u64, value: u8, count: usize) -> PyResult<()> {
        self.io_mut()?
            .pwrite_byte_repeat(offset, value, count)
            .map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `i32` copies of `value` at `offset` —
    /// no full array is built.
    fn pwrite_i32_repeat(&mut self, offset: u64, value: i32, count: usize) -> PyResult<()> {
        self.io_mut()?
            .pwrite_i32_repeat(offset, value, count)
            .map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `i64` copies of `value` at `offset` —
    /// the wide counterpart of [`pwrite_i32_repeat`](Mmap::pwrite_i32_repeat).
    fn pwrite_i64_repeat(&mut self, offset: u64, value: i64, count: usize) -> PyResult<()> {
        self.io_mut()?
            .pwrite_i64_repeat(offset, value, count)
            .map_err(ioerr)
    }

    // ---- bulk unsigned + floating widths (u16/u32/u64/f32/f64) --------------------------

    /// **Bulk typed read** of `count` little-endian `u16`s — the `u16` counterpart of
    /// [`pread_i32_array`](Mmap::pread_i32_array), with the same fail-fast bounds check.
    fn pread_u16_array(&self, offset: u64, count: usize) -> PyResult<Vec<u16>> {
        let io = self.io()?;
        if let Some(e) = bulk_eof(offset, io.byte_size().saturating_sub(offset), count, 2) {
            return Err(ioerr(e));
        }
        let mut values = vec![0u16; count];
        io.pread_u16_array(offset, &mut values).map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write** of little-endian `u16`s at `offset`, growing as needed.
    fn pwrite_u16_array(&mut self, offset: u64, values: Vec<u16>) -> PyResult<()> {
        self.io_mut()?
            .pwrite_u16_array(offset, &values)
            .map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `u16` copies of `value` at `offset`.
    fn pwrite_u16_repeat(&mut self, offset: u64, value: u16, count: usize) -> PyResult<()> {
        self.io_mut()?
            .pwrite_u16_repeat(offset, value, count)
            .map_err(ioerr)
    }

    /// **Bulk typed read** of `count` little-endian `u32`s (fail-fast bounds check).
    fn pread_u32_array(&self, offset: u64, count: usize) -> PyResult<Vec<u32>> {
        let io = self.io()?;
        if let Some(e) = bulk_eof(offset, io.byte_size().saturating_sub(offset), count, 4) {
            return Err(ioerr(e));
        }
        let mut values = vec![0u32; count];
        io.pread_u32_array(offset, &mut values).map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write** of little-endian `u32`s at `offset`, growing as needed.
    fn pwrite_u32_array(&mut self, offset: u64, values: Vec<u32>) -> PyResult<()> {
        self.io_mut()?
            .pwrite_u32_array(offset, &values)
            .map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `u32` copies of `value` at `offset`.
    fn pwrite_u32_repeat(&mut self, offset: u64, value: u32, count: usize) -> PyResult<()> {
        self.io_mut()?
            .pwrite_u32_repeat(offset, value, count)
            .map_err(ioerr)
    }

    /// **Bulk typed read** of `count` little-endian `u64`s (fail-fast bounds check).
    fn pread_u64_array(&self, offset: u64, count: usize) -> PyResult<Vec<u64>> {
        let io = self.io()?;
        if let Some(e) = bulk_eof(offset, io.byte_size().saturating_sub(offset), count, 8) {
            return Err(ioerr(e));
        }
        let mut values = vec![0u64; count];
        io.pread_u64_array(offset, &mut values).map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write** of little-endian `u64`s at `offset`, growing as needed.
    fn pwrite_u64_array(&mut self, offset: u64, values: Vec<u64>) -> PyResult<()> {
        self.io_mut()?
            .pwrite_u64_array(offset, &values)
            .map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `u64` copies of `value` at `offset`.
    fn pwrite_u64_repeat(&mut self, offset: u64, value: u64, count: usize) -> PyResult<()> {
        self.io_mut()?
            .pwrite_u64_repeat(offset, value, count)
            .map_err(ioerr)
    }

    /// **Bulk typed read** of `count` little-endian `f32`s (fail-fast bounds check).
    fn pread_f32_array(&self, offset: u64, count: usize) -> PyResult<Vec<f32>> {
        let io = self.io()?;
        if let Some(e) = bulk_eof(offset, io.byte_size().saturating_sub(offset), count, 4) {
            return Err(ioerr(e));
        }
        let mut values = vec![0f32; count];
        io.pread_f32_array(offset, &mut values).map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write** of little-endian `f32`s at `offset`, growing as needed.
    fn pwrite_f32_array(&mut self, offset: u64, values: Vec<f32>) -> PyResult<()> {
        self.io_mut()?
            .pwrite_f32_array(offset, &values)
            .map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `f32` copies of `value` at `offset`.
    fn pwrite_f32_repeat(&mut self, offset: u64, value: f32, count: usize) -> PyResult<()> {
        self.io_mut()?
            .pwrite_f32_repeat(offset, value, count)
            .map_err(ioerr)
    }

    /// **Bulk typed read** of `count` little-endian `f64`s (fail-fast bounds check).
    fn pread_f64_array(&self, offset: u64, count: usize) -> PyResult<Vec<f64>> {
        let io = self.io()?;
        if let Some(e) = bulk_eof(offset, io.byte_size().saturating_sub(offset), count, 8) {
            return Err(ioerr(e));
        }
        let mut values = vec![0f64; count];
        io.pread_f64_array(offset, &mut values).map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write** of little-endian `f64`s at `offset`, growing as needed.
    fn pwrite_f64_array(&mut self, offset: u64, values: Vec<f64>) -> PyResult<()> {
        self.io_mut()?
            .pwrite_f64_array(offset, &values)
            .map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `f64` copies of `value` at `offset`.
    fn pwrite_f64_repeat(&mut self, offset: u64, value: f64, count: usize) -> PyResult<()> {
        self.io_mut()?
            .pwrite_f64_repeat(offset, value, count)
            .map_err(ioerr)
    }

    // ---- cross-source copy -------------------------------------------------------------

    /// Overwrites this mapping with **all of `src`'s bytes** (a `yggdryl.memory.Heap`),
    /// truncating to match — a cross-source copy. Returns the byte count.
    fn copy_from(&mut self, src: &Heap) -> PyResult<u64> {
        self.io_mut()?.copy_from(&src.inner).map_err(ioerr)
    }

    /// **Positioned cross-source write**: copies `length` bytes of `src` (a
    /// `yggdryl.memory.Heap`) starting at `src_offset` into this mapping at `offset`. Returns
    /// the number of bytes transferred (short at the end of `src`).
    fn pwrite_from(
        &mut self,
        offset: u64,
        src: &Heap,
        src_offset: u64,
        length: u64,
    ) -> PyResult<u64> {
        self.io_mut()?
            .pwrite_from(offset, &src.inner, src_offset, length)
            .map_err(ioerr)
    }

    // ---- cursor ------------------------------------------------------------------------

    /// The current cursor position (bytes from the start). May sit past the end after a seek.
    #[getter]
    fn position(&self) -> PyResult<u64> {
        Ok(self.io()?.position())
    }

    /// Moves the cursor to an absolute `position` (past the end is allowed).
    fn set_position(&mut self, position: u64) -> PyResult<()> {
        self.io_mut()?.set_position(position);
        Ok(())
    }

    /// Seeks to `whence + offset` and returns the new position. A position past the end is
    /// allowed; seeking before the start raises `ValueError`.
    fn seek(&mut self, whence: Whence, offset: i64) -> PyResult<u64> {
        self.io_mut()?.seek(whence.into(), offset).map_err(ioerr)
    }

    /// Resets the cursor to the start.
    fn rewind(&mut self) -> PyResult<()> {
        self.io_mut()?.rewind();
        Ok(())
    }

    /// **Cursor read.** Returns up to `length` bytes from the current position (short near the
    /// end), advancing the cursor by the number read.
    fn read<'py>(&mut self, py: Python<'py>, length: usize) -> PyResult<Bound<'py, PyBytes>> {
        let io = self.io_mut()?;
        let position = io.position();
        let n = io.byte_size().saturating_sub(position).min(length as u64) as usize;
        let bytes = PyBytes::new_bound_with(py, n, |dst| {
            io.pread_byte_array(position, dst);
            Ok(())
        })?;
        io.set_position(position + n as u64);
        Ok(bytes)
    }

    /// **Cursor write.** Writes `data` (bytes / bytearray) at the current position, advancing
    /// the cursor by the number written (growing the file as needed); returns that count.
    fn write(&mut self, data: Vec<u8>) -> PyResult<usize> {
        Ok(self.io_mut()?.write(&data))
    }

    /// Reads the next byte at the cursor, advancing it by 1, raising `ValueError` at the end.
    fn read_byte(&mut self) -> PyResult<u8> {
        self.io_mut()?.read_byte().map_err(ioerr)
    }

    /// Writes the byte `value` at the cursor, advancing it by 1.
    fn write_byte(&mut self, value: u8) -> PyResult<()> {
        self.io_mut()?.write_byte(value).map_err(ioerr)
    }

    /// Reads a little-endian `i32` (4 bytes) at the cursor, advancing it by 4, raising
    /// `ValueError` on EOF.
    fn read_i32(&mut self) -> PyResult<i32> {
        self.io_mut()?.read_i32().map_err(ioerr)
    }

    /// Writes `value` as a little-endian `i32` (4 bytes) at the cursor, advancing it by 4.
    fn write_i32(&mut self, value: i32) -> PyResult<()> {
        self.io_mut()?.write_i32(value).map_err(ioerr)
    }

    /// Reads a little-endian `i64` (8 bytes) at the cursor, advancing it by 8, raising
    /// `ValueError` on EOF.
    fn read_i64(&mut self) -> PyResult<i64> {
        self.io_mut()?.read_i64().map_err(ioerr)
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at the cursor, advancing it by 8.
    fn write_i64(&mut self, value: i64) -> PyResult<()> {
        self.io_mut()?.write_i64(value).map_err(ioerr)
    }

    /// Reads up to `length` **bytes** from the cursor and decodes them as UTF-8 text (clamped
    /// near the end), advancing the cursor by the bytes read, raising a guided `ValueError`
    /// on invalid UTF-8 (leaving the cursor put).
    fn read_utf8(&mut self, length: usize) -> PyResult<String> {
        self.io_mut()?.read_utf8(length).map_err(ioerr)
    }

    /// Writes `text`'s UTF-8 bytes at the cursor, advancing it; returns the number of
    /// **bytes** written.
    fn write_utf8(&mut self, text: &str) -> PyResult<usize> {
        Ok(self.io_mut()?.write_utf8(text))
    }

    /// Reads from the current position **to the end** as `bytes`, advancing the cursor to the
    /// end.
    fn read_to_end<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let io = self.io_mut()?;
        let position = io.position();
        let n = io.byte_size().saturating_sub(position) as usize;
        let bytes = PyBytes::new_bound_with(py, n, |dst| {
            io.pread_byte_array(position, dst);
            Ok(())
        })?;
        io.set_position(io.byte_size());
        Ok(bytes)
    }

    // ---- address (uri) -----------------------------------------------------------------

    /// The [`Uri`] that **addresses** this mapping — the file path it is backed by (built
    /// with `Uri.from_path`, so back-slashes read as forward slashes).
    #[getter]
    fn uri(&self) -> PyResult<Uri> {
        Ok(Uri {
            inner: self.io()?.uri(),
        })
    }

    // ---- metadata (headers / mode / kind) ------------------------------------------------

    /// The [`Headers`] metadata attached to this mapping — returned as an owned **copy** (the
    /// binding cannot borrow into the Rust value); mutate the copy and write it back with
    /// [`set_headers`](Mmap::set_headers).
    #[getter]
    fn headers(&self) -> PyResult<Headers> {
        Ok(Headers {
            inner: self.io()?.headers().clone(),
        })
    }

    /// Replaces the whole [`Headers`] metadata map in place. There is deliberately no
    /// `with_headers` — it would need a copy, and a live mapping cannot be copied.
    fn set_headers(&mut self, headers: &Headers) -> PyResult<()> {
        *self.io_mut()?.headers_mut() = headers.inner.clone();
        Ok(())
    }

    /// How this mapping may be accessed — [`IOMode.ReadWrite`](IOMode::ReadWrite) from
    /// [`open`](Mmap::open) / [`create`](Mmap::create), [`IOMode.Read`](IOMode::Read) from
    /// [`open_readonly`](Mmap::open_readonly).
    #[getter]
    fn mode(&self) -> PyResult<IOMode> {
        Ok(self.io()?.mode().into())
    }

    /// Sets the access [`IOMode`] label in place (the physical protection is fixed at open:
    /// use [`open_readonly`](Mmap::open_readonly) for a truly unwritable mapping). No
    /// `with_mode` for the same reason as `with_headers`.
    fn set_mode(&mut self, mode: IOMode) -> PyResult<()> {
        self.io_mut()?.set_mode(mode.into());
        Ok(())
    }

    /// What this source **is** — always [`IOKind.File`](IOKind::File).
    #[getter]
    fn kind(&self) -> PyResult<IOKind> {
        Ok(self.io()?.kind().into())
    }

    // ---- predicates (is_file / is_dir / exists) ------------------------------------------

    /// Whether this source is a regular **file** — always `True` for a live mapping.
    fn is_file(&self) -> PyResult<bool> {
        Ok(self.io()?.is_file())
    }

    /// Whether this source is a **directory** — always `False` for a mapping.
    fn is_dir(&self) -> PyResult<bool> {
        Ok(self.io()?.is_dir())
    }

    /// Whether the source **exists** — a live mapping is by construction a live file
    /// (`True`).
    fn exists(&self) -> PyResult<bool> {
        Ok(self.io()?.exists())
    }

    // ---- media type (declared headers, else the file address, else octet-stream) ---------

    /// The **primary** [`MimeType`](crate::mimetype::MimeType) of the mapped file: the
    /// `Content-Type` its [`headers`](Mmap::headers) declare, else inferred from the file name,
    /// else the `application/octet-stream` fallback.
    fn mime_type(&self) -> PyResult<MimeType> {
        Ok(MimeType {
            inner: self.io()?.mime_type(),
        })
    }

    /// The full [`MediaType`](crate::mediatype::MediaType) of the mapped file (headers, else
    /// the file's extensions, else the single `application/octet-stream` fallback).
    fn media_type(&self) -> PyResult<MediaType> {
        Ok(MediaType {
            inner: self.io()?.media_type(),
        })
    }

    /// Resolves the media type **and stores it** in the mapping's headers when `Content-Type`
    /// is unset; returns the effective [`MimeType`](crate::mimetype::MimeType).
    fn ensure_content_type(&mut self) -> PyResult<MimeType> {
        Ok(MimeType {
            inner: self.io_mut()?.ensure_content_type(),
        })
    }

    // ---- inference + compression (magic-inferred type; codec over the bytes) -------------

    /// The **primary** [`MimeType`](crate::mimetype::MimeType) inferred from the mapped file's
    /// **magic bytes** — a positioned read of the head that **never moves the cursor**; falls
    /// back to the declared/address mime when no magic matches.
    fn infer_mime_type(&self) -> PyResult<MimeType> {
        Ok(MimeType {
            inner: self.io()?.infer_mime_type(),
        })
    }

    /// The full [`MediaType`](crate::mediatype::MediaType) inferred by **recursive magic** over
    /// the mapped file's head (peeling each compression layer it can).
    fn infer_media_type(&self) -> PyResult<MediaType> {
        Ok(MediaType {
            inner: self.io()?.infer_media_type(),
        })
    }

    /// The `yggdryl.compression` codec for the mapped file's media type, or `None` when it is
    /// not a supported compression.
    fn compression(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        crate::compression::codec_to_object(py, self.io()?.compression())
    }

    /// The mapped file **decompressed** with the codec inferred from its media type, as
    /// `bytes` — raises a guided `ValueError` when it is not a supported compression.
    fn decompress<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let out = self.io()?.decompress().map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// The mapped file's content **compressed** with the explicit `codec`, as `bytes`.
    fn compress_with<'py>(
        &self,
        py: Python<'py>,
        codec: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let io = self.io()?;
        let out =
            crate::compression::with_codec(codec, |c| io.compressed_with(c))?.map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// The mapped file's content **decompressed** with the explicit `codec`, as `bytes`.
    fn decompress_with<'py>(
        &self,
        py: Python<'py>,
        codec: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let io = self.io()?;
        let out =
            crate::compression::with_codec(codec, |c| io.decompressed_with(c))?.map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// **Compresses this mapping in place** — replaces the file's bytes with the compressed
    /// form and updates the `Content-Type` / `Content-Length` / `mtime` headers. `codec` (a
    /// `yggdryl.compression` codec) defaults to the codec of the file's own media type; pass
    /// an explicit one to override. Raises a guided `ValueError` when no codec applies.
    #[pyo3(signature = (codec = None))]
    fn compress_in_place(&mut self, codec: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        match codec {
            Some(codec) => {
                let io = self.io_mut()?;
                crate::compression::with_codec(codec, |c| io.compress_in_place(Some(c)))?
                    .map_err(ioerr)
            }
            None => self.io_mut()?.compress_in_place(None).map_err(ioerr),
        }
    }

    /// **Decompresses this mapping in place** — replaces the compressed bytes with the plain
    /// content (codec inferred from its media type) and updates the size/media/mtime headers.
    /// Raises a guided `ValueError` when the file is not a supported compression.
    fn decompress_in_place(&mut self) -> PyResult<()> {
        self.io_mut()?.decompress_in_place().map_err(ioerr)
    }

    // ---- graph: navigation + discovery + CRUD (a mapping is a leaf) ---------------------

    /// The node's own name — the mapped **file name** (the last path segment).
    #[getter]
    fn name(&self) -> PyResult<String> {
        Ok(self.io()?.name())
    }

    /// The parent node, or `None` — a raw mapping is a **leaf** of the IO graph (navigate
    /// with [`LocalIO`] when the surrounding tree matters).
    fn parent(&self) -> PyResult<Option<Mmap>> {
        Ok(self.io()?.parent().map(|inner| Mmap { inner: Some(inner) }))
    }

    /// This node's ancestors, nearest first — empty for a leaf/root.
    fn parents(&self) -> PyResult<Vec<Mmap>> {
        Ok(self
            .io()?
            .parents()
            .map(|inner| Mmap { inner: Some(inner) })
            .collect())
    }

    /// Streams this node's children — always the shared **empty**
    /// [`yggdryl.memory.NoChildren`](crate::io::memory::NoChildren) stream (a raw mapping is a
    /// leaf: it streams nothing, with or without `recursive=True`), still satisfying the
    /// iterator protocol like [`LocalIO.ls`](LocalIO::ls).
    #[pyo3(signature = (recursive = false))]
    fn ls(&self, recursive: bool) -> PyResult<NoChildren> {
        let io = self.io()?;
        let _ = if recursive {
            io.ls_recursive()
        } else {
            io.ls()
        }
        .map_err(ioerr)?;
        Ok(NoChildren {})
    }

    /// The direct children, collected — always the empty list (a leaf has none).
    fn children(&self) -> PyResult<Vec<Mmap>> {
        let nodes = self.io()?.children().map_err(ioerr)?;
        Ok(nodes
            .into_iter()
            .map(|inner| Mmap { inner: Some(inner) })
            .collect())
    }

    /// Removes the mapped file — [`rmfile`](Mmap::rmfile). Removing an **open** mapping is
    /// OS-dependent (Windows refuses to delete a mapped file, Unix unlinks it); close the
    /// writing mapping first for portable removal. `exist_ok` (default `True`) skips a
    /// missing file; `exist_ok=False` raises on one.
    #[pyo3(signature = (exist_ok = true))]
    fn rm(&self, exist_ok: bool) -> PyResult<()> {
        self.io()?.rm(exist_ok).map_err(ioerr)
    }

    /// Removes the mapped file from disk — really unlinks it. `exist_ok` (default `True`)
    /// skips an already-missing file; `exist_ok=False` raises on one. Raises the OS's guided
    /// `ValueError` when the file cannot be removed.
    #[pyo3(signature = (exist_ok = true))]
    fn rmfile(&self, exist_ok: bool) -> PyResult<()> {
        self.io()?.rmfile(exist_ok).map_err(ioerr)
    }

    /// A mapping is never a directory — raises the guided `ValueError` naming the fix (use
    /// [`rmfile`](Mmap::rmfile)).
    #[pyo3(signature = (exist_ok = true))]
    fn rmdir(&self, exist_ok: bool) -> PyResult<()> {
        self.io()?.rmdir(exist_ok).map_err(ioerr)
    }

    // DESIGN: no `cursor()` / `window()` / `slice()` here — the binding's `Cursor` / `Slice`
    // classes are monomorphic over `Heap`, and the core builders consume (or clone) their
    // source, which a live OS mapping deliberately cannot do (`Mmap` is not `Clone`). Use the
    // built-in cursor stream and the positioned accessors instead.
    // DESIGN: likewise no `__eq__` / `copy` / `__copy__` / `serialize_bytes` / pickle and no
    // `with_headers` / `with_mode` — `Mmap` is a live OS resource, not a value (two
    // independent mappings of one file would alias), and each `with_*` would need a copy.

    fn __repr__(&self) -> String {
        match &self.inner {
            Some(io) => format!(
                "Mmap({}, <{} bytes>)",
                io.path().to_string_lossy(),
                io.byte_size()
            ),
            None => "Mmap(<closed>)".to_string(),
        }
    }
}

#[pymethods]
impl Mmap {
    /// **Moves** this mapping's whole content into `dst` (another `Mmap`) and **removes this
    /// mapping's file** — a copy that consumes its origin (`mv` over the byte contract).
    /// Returns the number of bytes moved. A no-op when `self` and `dst` map the same path.
    /// Raises the guided closed `ValueError` if either mapping is closed.
    fn move_into(&mut self, mut dst: PyRefMut<'_, Mmap>) -> PyResult<u64> {
        let dst = dst.io_mut()?;
        self.io_mut()?.move_into(dst).map_err(ioerr)
    }
}

// The remaining native-width scalar, cursor-typed, and bulk-array accessors — completing the
// set alongside the hand-written `i32` / `i64` / byte forms in the main `Mmap` block above.
mmap_scalar_methods!(
    (i8, pread_i8, pwrite_i8),
    (u8, pread_u8, pwrite_u8),
    (i16, pread_i16, pwrite_i16),
    (u16, pread_u16, pwrite_u16),
    (u32, pread_u32, pwrite_u32),
    (u64, pread_u64, pwrite_u64),
    (i128, pread_i128, pwrite_i128),
    (u128, pread_u128, pwrite_u128),
    (f32, pread_f32, pwrite_f32),
    (f64, pread_f64, pwrite_f64),
);
mmap_cursor_typed_methods!(
    (i8, read_i8, write_i8),
    (u8, read_u8, write_u8),
    (i16, read_i16, write_i16),
    (u16, read_u16, write_u16),
    (u32, read_u32, write_u32),
    (u64, read_u64, write_u64),
    (i128, read_i128, write_i128),
    (u128, read_u128, write_u128),
    (f32, read_f32, write_f32),
    (f64, read_f64, write_f64),
);
mmap_bulk_methods!(
    (i8, 1, pread_i8_array, pwrite_i8_array, pwrite_i8_repeat),
    (i16, 2, pread_i16_array, pwrite_i16_array, pwrite_i16_repeat),
    (
        i128,
        16,
        pread_i128_array,
        pwrite_i128_array,
        pwrite_i128_repeat
    ),
    (
        u128,
        16,
        pread_u128_array,
        pwrite_u128_array,
        pwrite_u128_repeat
    ),
);

/// Populates the `local` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<LocalIO>()?;
    module.add_class::<LocalEntries>()?;
    module.add_class::<Mmap>()?;
    Ok(())
}
