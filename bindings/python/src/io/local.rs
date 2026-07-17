//! The `yggdryl.local` submodule — the **local-filesystem family**: the [`LocalIO`] single
//! access point and the raw memory-mapped [`Mmap`] file it builds on.
//!
//! Mirrors `yggdryl_core::io::local`. A [`LocalIO`] is one **lazy** handle over any path
//! (file, folder, or nothing yet) that decides per call how to serve reads and writes:
//! probing and navigating touch nothing, reads before any write open the file ad hoc (a
//! missing or directory node reads as empty), and the first write auto-creates the missing
//! parent folders and the file, memory-maps it, and keeps the mapping so later access runs
//! at memory speed. The same handle carries the whole filesystem graph — `name` / `parent` /
//! `join` (and the `/` operator), streamed `ls` / collected `children` discovery, `mkdir`,
//! and the shape-checked `rm` / `rmfile` / `rmdir`. An [`Mmap`] is the raw mapping `LocalIO`
//! builds on, usable directly when a pre-existing file and explicit open/create control are
//! wanted.
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
use crate::io::memory::Whence;
use crate::io::mode::IOMode;
use crate::uri::Uri;
use yggdryl_core::io::local;
use yggdryl_core::io::memory::{IOBase, IoError};
use yggdryl_core::io::Path;

/// Maps an [`IoError`] to a Python `ValueError` carrying its guided text.
fn ioerr(error: IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// The one local-filesystem handle — a **lazy** node over any path (file, folder, or nothing
/// yet) that **decides per call how to serve reads and writes**: constructing, probing, and
/// navigating touch nothing; before any write a read opens the file ad hoc (a missing or
/// directory node reads as empty); the **first write auto-creates** the missing parent
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
/// fresh **lazy** handle (`copy()` / `copy.copy` — the mapping is deliberately never
/// shared), and is unhashable and unpicklable (no byte codec — the path value lives in
/// [`uri`](LocalIO::uri)).
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

    /// The underlying filesystem path as a `str`.
    #[getter]
    fn path(&self) -> String {
        self.inner.as_std_path().to_string_lossy().into_owned()
    }

    // ---- size + capacity ---------------------------------------------------------------

    /// The **logical** length in bytes — the mapped backing's length once mapped, the
    /// on-disk file length while lazy, `0` for a missing or directory node.
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

    /// Whether the node holds no bytes (`byte_size() == 0` — also `True` for a missing or
    /// directory node).
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Truthiness — `True` when the node holds at least one byte.
    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    // ---- positioned byte-array ---------------------------------------------------------

    /// **Positioned read.** Returns up to `length` bytes starting at `offset` as `bytes` —
    /// short near the end, empty at or past it (and empty on a missing or directory node).
    /// Never moves the cursor. Reads **directly** into the `bytes` allocation (one copy).
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
    /// bytes written (`0` on a read-only handle or a directory node).
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

    // ---- cursor ------------------------------------------------------------------------

    /// The current cursor position (bytes from the start). May sit past the end after a seek.
    #[getter]
    fn position(&self) -> u64 {
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
    /// with its whole subtree; a missing node is a no-op. The generic form of
    /// [`rmfile`](LocalIO::rmfile) / [`rmdir`](LocalIO::rmdir).
    fn rm(&self) -> PyResult<()> {
        self.inner.rm().map_err(ioerr)
    }

    /// Removes this node **as a file** — raises the guided `ValueError` when it is a
    /// directory (use [`rmdir`](LocalIO::rmdir)); a no-op when missing.
    fn rmfile(&self) -> PyResult<()> {
        self.inner.rmfile().map_err(ioerr)
    }

    /// Removes this node **as a directory**, recursively — raises the guided `ValueError`
    /// when it is a file (use [`rmfile`](LocalIO::rmfile)); a no-op when missing.
    fn rmdir(&self) -> PyResult<()> {
        self.inner.rmdir().map_err(ioerr)
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

    // DESIGN: no `__reduce__` / pickle and no `serialize_bytes` — a live handle carries no
    // byte codec (the path value lives in `uri`); mirror of the Mmap precedent.

    fn __repr__(&self) -> String {
        format!(
            "LocalIO({}, <{} bytes>)",
            self.inner.as_std_path().to_string_lossy(),
            self.inner.byte_size()
        )
    }
}

/// The core streamed iterator a [`LocalEntries`] wraps — one level
/// (`yggdryl_core::io::local::LocalChildren`) or the depth-first subtree walk
/// (`yggdryl_core::io::local::LocalWalk`); both are owned iterators, so the binding holds
/// them directly (the one-level iterator boxed — its OS `ReadDir` state dwarfs the walk's).
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
/// to its exact logical length.
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

/// Populates the `local` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<LocalIO>()?;
    module.add_class::<LocalEntries>()?;
    module.add_class::<Mmap>()?;
    Ok(())
}
