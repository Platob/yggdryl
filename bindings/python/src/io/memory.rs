//! The `yggdryl.memory` submodule — the in-heap and memory-mapped byte sources and the seek
//! anchor.
//!
//! Mirrors `yggdryl_core::io::memory`'s [`Heap`](yggdryl_core::io::memory::Heap) and
//! [`Mmap`](yggdryl_core::io::local::Mmap) sources and the
//! [`Whence`](yggdryl_core::io::memory::Whence) enum. A [`Heap`] is an owned byte buffer with a
//! read/write cursor and `Vec`-like capacity — the concrete in-memory implementor of the
//! byte-access traits (positioned `pread_*` / `pwrite_*` including UTF-8 text and the bulk
//! `i32`/`i64` arrays and repeated fills, the cursor stream, bounded [`slice`](Heap::slice)
//! windows, and the source metadata: an addressing `Uri`, a `Headers` map, an `IOMode`, and
//! an `IOKind`). It behaves like a `bytearray`: a mutable value that compares by its stored
//! bytes, round-trips through `serialize_bytes` / `deserialize_bytes` (and pickle), and is
//! deliberately **unhashable**. An [`Mmap`] is the same surface over a **file on disk** —
//! opened from a `str` path or a `Uri`, auto-growing on writes, truncated back to its logical
//! length on [`close`](Mmap::close) — but is a live OS resource, not a value.
//!
//! Every method is one or two lines over `yggdryl_core`; a read with a hard length requirement
//! that runs off the end (a typed read, a slice past the end, a seek before the start) raises a
//! guided `ValueError` carrying the core error text unchanged.

// `useless_conversion`: pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type
// `From`.
#![allow(clippy::useless_conversion)]

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::headers::Headers;
use crate::io::kind::IOKind;
use crate::io::mode::IOMode;
use crate::uri::Uri;
use yggdryl_core::io::local;
use yggdryl_core::io::memory::{self, IOBase, IoError};
use yggdryl_core::io::Serializable;

/// Maps an [`IoError`] to a Python `ValueError` carrying its guided text.
fn ioerr(error: IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Where a seek offset is measured from — the POSIX `lseek` `whence`. Mirrors
/// [`yggdryl_core::io::memory::Whence`]: the **start** of the data (`SEEK_SET`), the **current**
/// cursor position (`SEEK_CUR`), or the **end** (`SEEK_END`).
#[pyclass(module = "yggdryl.memory", eq, eq_int, hash, frozen)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Whence {
    /// From the start of the data (absolute) — POSIX `SEEK_SET`.
    Start,
    /// From the current cursor position — POSIX `SEEK_CUR`.
    Current,
    /// From the end of the data — POSIX `SEEK_END`.
    End,
}

impl From<Whence> for memory::Whence {
    fn from(whence: Whence) -> Self {
        match whence {
            Whence::Start => memory::Whence::Start,
            Whence::Current => memory::Whence::Current,
            Whence::End => memory::Whence::End,
        }
    }
}

/// An in-heap byte buffer with a read/write cursor and amortized capacity — the concrete
/// in-memory implementor of the byte-access contracts. Grows like a `bytearray`; compares by
/// its stored bytes (the cursor is transient) and is intentionally **not** hashable.
#[pyclass(module = "yggdryl.memory")]
#[derive(Clone)]
pub struct Heap {
    pub(crate) inner: memory::Heap,
}

#[pymethods]
impl Heap {
    /// Builds a buffer owning a copy of `data` (bytes / bytearray), or an empty buffer if
    /// `data` is omitted. The generic, type-inferring entry point (delegates to `from_vec`).
    #[new]
    #[pyo3(signature = (data = None))]
    fn new(data: Option<Vec<u8>>) -> Self {
        match data {
            Some(bytes) => Self {
                inner: memory::Heap::from_vec(bytes),
            },
            None => Self {
                inner: memory::Heap::new(),
            },
        }
    }

    /// An empty buffer that can hold `capacity` bytes before reallocating (like
    /// `bytearray` growth), cursor at `0`.
    #[staticmethod]
    fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: memory::Heap::with_capacity(capacity),
        }
    }

    // ---- size + capacity ---------------------------------------------------------------

    /// The total length in bytes.
    fn byte_size(&self) -> u64 {
        self.inner.byte_size()
    }

    /// The total length in bytes (so `len(heap)` works).
    fn __len__(&self) -> usize {
        self.inner.byte_size() as usize
    }

    /// The total length in bits — `byte_size() * 8`.
    fn bit_size(&self) -> u64 {
        self.inner.bit_size()
    }

    /// The number of bytes the buffer can hold before it must reallocate — like
    /// `list`/`Vec` capacity.
    fn capacity(&self) -> u64 {
        self.inner.capacity()
    }

    /// Reserves capacity for at least `additional` more bytes past the current size,
    /// amortizing later writes.
    fn reserve(&mut self, additional: u64) {
        self.inner.reserve(additional);
    }

    /// The spare room already allocated — `capacity() - byte_size()`, the bytes that can be
    /// appended before the next reallocation.
    fn spare_capacity(&self) -> u64 {
        self.inner.spare_capacity()
    }

    /// Reserves capacity for **exactly** `additional` more bytes — no amortized
    /// over-allocation, for a caller that knows the final size.
    fn reserve_exact(&mut self, additional: u64) {
        self.inner.reserve_exact(additional);
    }

    /// **Checked** reservation: where `reserve` would abort the process on overflow or
    /// allocator failure, this raises a guided `ValueError` instead.
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

    /// **Checked** `ensure_capacity` — raises a guided `ValueError` instead of aborting.
    fn try_ensure_capacity(&mut self, total: u64) -> PyResult<()> {
        self.inner.try_ensure_capacity(total).map_err(ioerr)
    }

    /// Releases spare capacity back to the allocator, shrinking toward `byte_size()`.
    fn shrink_to_fit(&mut self) {
        self.inner.shrink_to_fit();
    }

    /// Shrinks the allocation toward `min_capacity` (never below `byte_size()`).
    fn shrink_to(&mut self, min_capacity: u64) {
        self.inner.shrink_to(min_capacity);
    }

    /// Whether the buffer holds no bytes (`byte_size() == 0`).
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Truthiness — `True` when the buffer holds at least one byte (like `bytearray`).
    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
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

    /// **Positioned write.** Copies `data` (bytes / bytearray) in at `offset`, growing the
    /// buffer and zero-filling any gap; returns the number of bytes written.
    fn pwrite_byte_array(&mut self, offset: u64, data: Vec<u8>) -> usize {
        self.inner.pwrite_byte_array(offset, &data)
    }

    // ---- positioned typed accessors ----------------------------------------------------

    /// Reads the single byte at `offset`, raising `ValueError` if it is past the end.
    fn pread_byte(&self, offset: u64) -> PyResult<u8> {
        self.inner.pread_byte(offset).map_err(ioerr)
    }

    /// Writes the single byte `value` at `offset`, growing the buffer as needed.
    fn pwrite_byte(&mut self, offset: u64, value: u8) -> PyResult<()> {
        self.inner.pwrite_byte(offset, value).map_err(ioerr)
    }

    /// Reads the bit at absolute **bit** `offset` (LSB-first: bit `offset % 8` of byte
    /// `offset / 8`), raising `ValueError` if its byte is past the end.
    fn pread_bit(&self, offset: u64) -> PyResult<bool> {
        self.inner.pread_bit(offset).map_err(ioerr)
    }

    /// Sets or clears the bit at absolute **bit** `offset` (LSB-first), growing the buffer
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

    /// Writes `text`'s UTF-8 bytes at `offset` (growing as needed); returns the number of
    /// **bytes** written.
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
    /// [`pread_i32_array`](Heap::pread_i32_array), with the same fail-fast bounds check
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
    /// [`pwrite_i32_array`](Heap::pwrite_i32_array).
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
    /// the wide counterpart of [`pwrite_i32_repeat`](Heap::pwrite_i32_repeat).
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
    /// the cursor by the number written (growing the buffer as needed); returns that count.
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

    // ---- slice -------------------------------------------------------------------------

    /// The window `[offset, offset + length)` as a fresh, independent `Heap` addressed from
    /// its own `0`. Raises `ValueError` if it runs past the end.
    fn slice(&self, offset: u64, length: u64) -> PyResult<Heap> {
        self.inner
            .slice(offset, length)
            .map(|inner| Heap { inner })
            .map_err(ioerr)
    }

    // ---- address (uri) -----------------------------------------------------------------

    /// The [`Uri`] that **addresses** this heap — always the stable synthetic `mem://heap`
    /// (a heap stores no address; an anonymous in-memory buffer has no other identity).
    #[getter]
    fn uri(&self) -> Uri {
        Uri {
            inner: self.inner.uri(),
        }
    }

    // ---- metadata (headers / mode / kind) ------------------------------------------------

    /// The [`Headers`] metadata attached to this heap — returned as an owned **copy** (the
    /// binding cannot borrow into the Rust value); mutate the copy and write it back with
    /// [`set_headers`](Heap::set_headers).
    #[getter]
    fn headers(&self) -> Headers {
        Headers {
            inner: self.inner.headers().clone(),
        }
    }

    /// Replaces the whole [`Headers`] metadata map in place.
    fn set_headers(&mut self, headers: &Headers) {
        self.inner.set_headers(headers.inner.clone());
    }

    /// Returns a copy of this heap with its [`Headers`] metadata replaced.
    fn with_headers(&self, headers: &Headers) -> Heap {
        Heap {
            inner: self.inner.clone().with_headers(headers.inner.clone()),
        }
    }

    /// How this heap may be accessed — [`IOMode.ReadWrite`](IOMode::ReadWrite) by default
    /// (it is in-memory).
    #[getter]
    fn mode(&self) -> IOMode {
        self.inner.mode().into()
    }

    /// Sets the access [`IOMode`] in place.
    fn set_mode(&mut self, mode: IOMode) {
        self.inner.set_mode(mode.into());
    }

    /// Returns a copy of this heap with its access [`IOMode`] set.
    fn with_mode(&self, mode: IOMode) -> Heap {
        Heap {
            inner: self.inner.clone().with_mode(mode.into()),
        }
    }

    /// What this source **is** — always [`IOKind.Heap`](IOKind::Heap).
    #[getter]
    fn kind(&self) -> IOKind {
        self.inner.kind().into()
    }

    // ---- cursor / window views ---------------------------------------------------------

    /// A [`Cursor`] over an **independent copy** of this heap (the binding clones since it
    /// cannot consume `self`), positioned at the start.
    fn cursor(&self) -> Cursor {
        Cursor {
            inner: self.inner.clone().cursor(),
        }
    }

    /// A [`Slice`] — the bounded window `[offset, offset + length)` over an **independent
    /// copy** of this heap, addressed from its own `0`. Raises `ValueError` if it runs past
    /// the end.
    fn window(&self, offset: u64, length: u64) -> PyResult<Slice> {
        self.inner
            .clone()
            .window(offset, length)
            .map(|inner| Slice { inner })
            .map_err(ioerr)
    }

    // ---- value semantics + dunders -----------------------------------------------------

    /// The stored bytes as a `bytes` copy.
    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.as_slice())
    }

    /// The stored bytes as a `bytes` copy (so `bytes(heap)` works).
    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.as_slice())
    }

    /// The heap's value form — its stored bytes (the cursor, address, headers, and mode are
    /// transient metadata and are not serialized), matching the identity `__eq__` uses.
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.serialize_bytes())
    }

    /// Reconstructs a heap from bytes produced by [`serialize_bytes`](Heap::serialize_bytes)
    /// — the exact inverse (cursor at `0`, default address/metadata).
    #[staticmethod]
    fn deserialize_bytes(data: &[u8]) -> PyResult<Heap> {
        memory::Heap::deserialize_bytes(data)
            .map(|inner| Heap { inner })
            .map_err(ioerr)
    }

    /// Pickles through the byte codec (`deserialize_bytes(serialize_bytes())`).
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<Heap>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    /// An explicit copy of this buffer (equivalent to `copy.copy(heap)`) — bytes, cursor,
    /// headers, and mode all copied.
    fn copy(&self) -> Self {
        self.clone()
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __repr__(&self) -> String {
        format!("Heap(<{} bytes>)", self.inner.byte_size())
    }
}

/// A **cursor** — a moving read/write position over an owned [`Heap`] source. Mirrors
/// `yggdryl_core::io::memory::IOCursor<Heap>`: `read` / `write` advance it, `seek` moves relative
/// to a [`Whence`] anchor, and the positioned `pread_*` / `pwrite_*` accessors reach any offset
/// without moving it. A read with a hard length requirement that runs off the end raises a
/// guided `ValueError`.
#[pyclass(module = "yggdryl.memory")]
pub struct Cursor {
    pub(crate) inner: memory::IOCursor<memory::Heap>,
}

#[pymethods]
impl Cursor {
    /// A cursor over a fresh [`Heap`] owning a copy of `data` (bytes / bytearray), or over an
    /// empty heap if `data` is omitted; positioned at the start.
    #[new]
    #[pyo3(signature = (data = None))]
    fn new(data: Option<Vec<u8>>) -> Self {
        let heap = match data {
            Some(bytes) => memory::Heap::from_vec(bytes),
            None => memory::Heap::new(),
        };
        Self {
            inner: heap.cursor(),
        }
    }

    /// A cursor over an **independent copy** of `heap` (the binding clones since it cannot
    /// consume the source), positioned at the start.
    #[staticmethod]
    fn over(heap: &Heap) -> Self {
        Self {
            inner: heap.inner.clone().cursor(),
        }
    }

    // ---- cursor stream -----------------------------------------------------------------

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
    /// the cursor by the number written (growing the source as needed); returns that count.
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

    // ---- positioned (delegates to the wrapped source) ----------------------------------

    /// The total length in bytes of the wrapped source.
    fn byte_size(&self) -> u64 {
        self.inner.byte_size()
    }

    /// The total length in bytes (so `len(cursor)` works).
    fn __len__(&self) -> usize {
        self.inner.byte_size() as usize
    }

    /// The total length in bits — `byte_size() * 8`.
    fn bit_size(&self) -> u64 {
        self.inner.bit_size()
    }

    /// Reads the single byte at `offset`, raising `ValueError` if it is past the end. Never
    /// moves the cursor.
    fn pread_byte(&self, offset: u64) -> PyResult<u8> {
        self.inner.pread_byte(offset).map_err(ioerr)
    }

    /// Reads the bit at absolute **bit** `offset` (LSB-first), raising `ValueError` if its
    /// byte is past the end.
    fn pread_bit(&self, offset: u64) -> PyResult<bool> {
        self.inner.pread_bit(offset).map_err(ioerr)
    }

    /// Reads a little-endian `i32` (4 bytes) at `offset`, raising `ValueError` on EOF.
    fn pread_i32(&self, offset: u64) -> PyResult<i32> {
        self.inner.pread_i32(offset).map_err(ioerr)
    }

    /// Reads a little-endian `i64` (8 bytes) at `offset`, raising `ValueError` on EOF.
    fn pread_i64(&self, offset: u64) -> PyResult<i64> {
        self.inner.pread_i64(offset).map_err(ioerr)
    }

    /// Writes the single byte `value` at `offset`, growing the source as needed. Never moves
    /// the cursor.
    fn pwrite_byte(&mut self, offset: u64, value: u8) -> PyResult<()> {
        self.inner.pwrite_byte(offset, value).map_err(ioerr)
    }

    /// Sets or clears the bit at absolute **bit** `offset` (LSB-first), growing the source
    /// (zero-filled) if the bit is past the end.
    fn pwrite_bit(&mut self, offset: u64, value: bool) -> PyResult<()> {
        self.inner.pwrite_bit(offset, value).map_err(ioerr)
    }

    /// Writes `value` as a little-endian `i32` (4 bytes) at `offset`, growing as needed.
    fn pwrite_i32(&mut self, offset: u64, value: i32) -> PyResult<()> {
        self.inner.pwrite_i32(offset, value).map_err(ioerr)
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at `offset`, growing as needed.
    fn pwrite_i64(&mut self, offset: u64, value: i64) -> PyResult<()> {
        self.inner.pwrite_i64(offset, value).map_err(ioerr)
    }

    /// Reads up to `length` **bytes** at `offset` and decodes them as UTF-8 text (clamped
    /// near the end), raising a guided `ValueError` on invalid UTF-8. Never moves the cursor.
    fn pread_utf8(&self, offset: u64, length: usize) -> PyResult<String> {
        self.inner.pread_utf8(offset, length).map_err(ioerr)
    }

    /// Writes `text`'s UTF-8 bytes at `offset` (growing as needed); returns the number of
    /// **bytes** written. Never moves the cursor.
    fn pwrite_utf8(&mut self, offset: u64, text: &str) -> usize {
        self.inner.pwrite_utf8(offset, text)
    }

    // ---- address + source ---------------------------------------------------------------

    /// The [`Uri`] that **addresses** the wrapped source.
    #[getter]
    fn uri(&self) -> Uri {
        Uri {
            inner: self.inner.uri(),
        }
    }

    /// The wrapped source's [`Headers`] metadata — an owned **copy** (delegates to the
    /// source; edit the source and re-wrap to change it).
    #[getter]
    fn headers(&self) -> Headers {
        Headers {
            inner: self.inner.headers().clone(),
        }
    }

    /// How the wrapped source may be accessed (delegates to the source).
    #[getter]
    fn mode(&self) -> IOMode {
        self.inner.mode().into()
    }

    /// What the wrapped source **is** (delegates to the source).
    #[getter]
    fn kind(&self) -> IOKind {
        self.inner.kind().into()
    }

    /// An independent copy of the wrapped [`Heap`] source (the cursor position is discarded).
    fn inner(&self) -> Heap {
        Heap {
            inner: self.inner.inner().clone(),
        }
    }

    /// The wrapped source's bytes as a `bytes` copy.
    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.inner().as_slice())
    }

    /// The wrapped source's bytes as a `bytes` copy (so `bytes(cursor)` works).
    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.inner().as_slice())
    }

    fn __repr__(&self) -> String {
        format!(
            "Cursor(position={}, <{} bytes>)",
            self.inner.position(),
            self.inner.byte_size()
        )
    }
}

/// A **bounded window** over an owned [`Heap`] source — the range `[offset, offset + length)`
/// addressed from its own `0`. Mirrors `yggdryl_core::io::memory::IOSlice<Heap>`: it is
/// **fixed-length**, so a write past its end is clamped away (it never grows the source beyond
/// the window). A typed read that runs off the window's end raises a guided `ValueError`.
#[pyclass(module = "yggdryl.memory")]
pub struct Slice {
    pub(crate) inner: memory::IOSlice<memory::Heap>,
}

#[pymethods]
impl Slice {
    /// The window `[offset, offset + length)` over an **independent copy** of `heap`, addressed
    /// from its own `0`. Raises `ValueError` if it runs past the source's end.
    #[new]
    fn new(heap: &Heap, offset: u64, length: u64) -> PyResult<Self> {
        heap.inner
            .clone()
            .window(offset, length)
            .map(|inner| Self { inner })
            .map_err(ioerr)
    }

    /// A [`Slice`] over an **independent copy** of `heap` — the same as the constructor, as a
    /// factory (the spelling shared with [`Cursor.over`](Cursor::over)). Raises `ValueError`
    /// if the window runs past the source's end.
    #[staticmethod]
    fn over(heap: &Heap, offset: u64, length: u64) -> PyResult<Self> {
        Self::new(heap, offset, length)
    }

    /// The window length in bytes.
    fn byte_size(&self) -> u64 {
        self.inner.byte_size()
    }

    /// The window length in bytes (so `len(slice)` works).
    fn __len__(&self) -> usize {
        self.inner.byte_size() as usize
    }

    /// The window's start offset within the source.
    #[getter]
    fn offset(&self) -> u64 {
        self.inner.offset()
    }

    /// **Positioned read.** Returns up to `length` bytes starting at `offset` **within the
    /// window** as `bytes` — short near the window's end, empty at or past it. Reads
    /// **directly** into the `bytes` allocation (one copy).
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

    /// Reads the single byte at `offset` within the window, raising `ValueError` if it is past
    /// the window's end.
    fn pread_byte(&self, offset: u64) -> PyResult<u8> {
        self.inner.pread_byte(offset).map_err(ioerr)
    }

    /// Reads a little-endian `i32` (4 bytes) at `offset` within the window, raising
    /// `ValueError` on EOF.
    fn pread_i32(&self, offset: u64) -> PyResult<i32> {
        self.inner.pread_i32(offset).map_err(ioerr)
    }

    /// Reads a little-endian `i64` (8 bytes) at `offset` within the window, raising
    /// `ValueError` on EOF.
    fn pread_i64(&self, offset: u64) -> PyResult<i64> {
        self.inner.pread_i64(offset).map_err(ioerr)
    }

    /// Reads up to `length` **bytes** at `offset` **within the window** and decodes them as
    /// UTF-8 text (clamped to the window's end), raising a guided `ValueError` on invalid
    /// UTF-8 — including a multi-byte character cut by the window.
    fn pread_utf8(&self, offset: u64, length: usize) -> PyResult<String> {
        self.inner.pread_utf8(offset, length).map_err(ioerr)
    }

    /// **Positioned write**, clamped to the window. Copies `data` (bytes / bytearray) in at
    /// `offset`, writing only as far as the window's end; returns the number of bytes written.
    fn pwrite_byte_array(&mut self, offset: u64, data: Vec<u8>) -> usize {
        self.inner.pwrite_byte_array(offset, &data)
    }

    /// The [`Uri`] that **addresses** the wrapped source.
    #[getter]
    fn uri(&self) -> Uri {
        Uri {
            inner: self.inner.uri(),
        }
    }

    /// The wrapped source's [`Headers`] metadata — an owned **copy** (delegates to the
    /// source).
    #[getter]
    fn headers(&self) -> Headers {
        Headers {
            inner: self.inner.headers().clone(),
        }
    }

    /// How the wrapped source may be accessed (delegates to the source).
    #[getter]
    fn mode(&self) -> IOMode {
        self.inner.mode().into()
    }

    /// What the wrapped source **is** (delegates to the source).
    #[getter]
    fn kind(&self) -> IOKind {
        self.inner.kind().into()
    }

    /// An independent copy of the wrapped [`Heap`] source (the window bounds are discarded).
    fn inner(&self) -> Heap {
        Heap {
            inner: self.inner.inner().clone(),
        }
    }

    /// The window's bytes as a `bytes` copy.
    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(
            py,
            &self.inner.pread_vec(0, self.inner.byte_size() as usize),
        )
    }

    /// The window's bytes as a `bytes` copy (so `bytes(slice)` works).
    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(
            py,
            &self.inner.pread_vec(0, self.inner.byte_size() as usize),
        )
    }

    fn __repr__(&self) -> String {
        format!(
            "Slice(offset={}, <{} bytes>)",
            self.inner.offset(),
            self.inner.byte_size()
        )
    }
}

/// A **memory-mapped file** — the on-disk implementor of the byte-access contract, sharing
/// [`Heap`]'s full surface (positioned + typed + bulk access, the built-in cursor stream,
/// capacity management, metadata) over a file instead of an owned buffer. Opened from a `str`
/// path or a [`Uri`] via [`open`](Mmap::open) / [`open_readonly`](Mmap::open_readonly) /
/// [`create`](Mmap::create); a write past the end grows the file (amortized, page-aligned),
/// and [`close`](Mmap::close) — or the end of a `with` block, or garbage collection — unmaps
/// the view and truncates the on-disk file back to its exact logical length.
///
/// Unlike [`Heap`], an `Mmap` is a **live OS resource, not a value**: two independent mappings
/// of one file would alias, so it is deliberately not equatable, copyable, serializable, or
/// picklable — no `__eq__`, no `copy`, no `serialize_bytes` / pickle, and no `with_*` builders
/// (each would need a copy). Use it as a context manager (`with Mmap.create(path) as m:`) or
/// call [`close`](Mmap::close) explicitly; any access after closing raises a guided
/// `ValueError`.
#[pyclass(module = "yggdryl.memory")]
pub struct Mmap {
    /// `None` once closed — every access goes through [`Mmap::io`] / [`Mmap::io_mut`].
    pub(crate) inner: Option<local::Mmap>,
}

/// The guided error for any access to a closed mapping.
fn closed_err() -> PyErr {
    PyValueError::new_err("the mapping is closed; reopen it with Mmap.open / Mmap.create")
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

/// Populates the `memory` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Heap>()?;
    module.add_class::<Whence>()?;
    module.add_class::<Cursor>()?;
    module.add_class::<Slice>()?;
    module.add_class::<Mmap>()?;
    Ok(())
}
