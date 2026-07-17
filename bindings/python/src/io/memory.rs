//! The `yggdryl.memory` submodule — the in-heap byte source and the seek anchor.
//!
//! Mirrors `yggdryl_core::io::memory`'s [`Heap`](yggdryl_core::io::memory::Heap) source and
//! [`Whence`](yggdryl_core::io::memory::Whence) enum. A [`Heap`] is an owned byte buffer with a
//! read/write cursor and `Vec`-like capacity — the concrete in-memory implementor of the
//! byte-access traits (positioned `pread_*` / `pwrite_*`, the cursor stream, and bounded
//! [`slice`](Heap::slice) windows). It behaves like a `bytearray`: a mutable value that
//! compares by its stored bytes and is deliberately **unhashable**.
//!
//! Every method is one or two lines over `yggdryl_core`; a read with a hard length requirement
//! that runs off the end (a typed read, a slice past the end, a seek before the start) raises a
//! guided `ValueError` carrying the core error text unchanged.

// `useless_conversion`: pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type
// `From`.
#![allow(clippy::useless_conversion)]

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::io::uri::Uri;
use yggdryl_core::io::memory::{self, IOBase, IoError};

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
    /// short near the end, empty at or past it. Never moves the cursor.
    fn pread_byte_array<'py>(
        &self,
        py: Python<'py>,
        offset: u64,
        length: usize,
    ) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.pread_vec(offset, length))
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
    fn read<'py>(&mut self, py: Python<'py>, length: usize) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.read_vec(length))
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

    /// Reads from the current position **to the end** as `bytes`, advancing the cursor to the
    /// end.
    fn read_to_end<'py>(&mut self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.read_to_end())
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

    /// The [`Uri`] that **addresses** this heap — the empty (opaque) URI by default.
    #[getter]
    fn uri(&self) -> Uri {
        Uri {
            inner: self.inner.uri(),
        }
    }

    /// Sets the addressing [`Uri`] in place.
    fn set_uri(&mut self, uri: &Uri) {
        self.inner.set_uri(uri.inner.clone());
    }

    /// Returns a copy of this heap with its addressing [`Uri`] set.
    fn with_uri(&self, uri: &Uri) -> Heap {
        Heap {
            inner: self.inner.clone().with_uri(uri.inner.clone()),
        }
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

    /// An explicit copy of this buffer (equivalent to `copy.copy(heap)`); pass `uri` to
    /// override the copy's address (defaults to `None` = keep this heap's).
    #[pyo3(signature = (uri = None))]
    fn copy(&self, uri: Option<&Uri>) -> Self {
        let mut inner = self.inner.clone();
        if let Some(uri) = uri {
            inner.set_uri(uri.inner.clone());
        }
        Self { inner }
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
    fn read<'py>(&mut self, py: Python<'py>, length: usize) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.read_vec(length))
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

    /// Reads from the current position **to the end** as `bytes`, advancing the cursor to the
    /// end.
    fn read_to_end<'py>(&mut self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.read_to_end())
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

    // ---- address + source ---------------------------------------------------------------

    /// The [`Uri`] that **addresses** the wrapped source.
    #[getter]
    fn uri(&self) -> Uri {
        Uri {
            inner: self.inner.uri(),
        }
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
    /// window** as `bytes` — short near the window's end, empty at or past it.
    fn pread_byte_array<'py>(
        &self,
        py: Python<'py>,
        offset: u64,
        length: usize,
    ) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.pread_vec(offset, length))
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

/// Populates the `memory` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Heap>()?;
    module.add_class::<Whence>()?;
    module.add_class::<Cursor>()?;
    module.add_class::<Slice>()?;
    Ok(())
}
