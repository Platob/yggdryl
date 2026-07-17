//! The `yggdryl.memory` submodule — the in-heap byte source and the seek anchor.
//!
//! Mirrors `yggdryl_core::memory`'s [`Heap`](yggdryl_core::memory::Heap) source and
//! [`Whence`](yggdryl_core::memory::Whence) enum. A [`Heap`] is an owned byte buffer with a
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

use yggdryl_core::memory::{self, IOBase, IOCursor, IOSlice, IoError};

/// Maps an [`IoError`] to a Python `ValueError` carrying its guided text.
fn ioerr(error: IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Where a seek offset is measured from — the POSIX `lseek` `whence`. Mirrors
/// [`yggdryl_core::memory::Whence`]: the **start** of the data (`SEEK_SET`), the **current**
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

    // ---- value semantics + dunders -----------------------------------------------------

    /// The stored bytes as a `bytes` copy.
    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.as_slice())
    }

    /// The stored bytes as a `bytes` copy (so `bytes(heap)` works).
    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.as_slice())
    }

    /// An explicit copy of this buffer (equivalent to `copy.copy(heap)`).
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

/// Populates the `memory` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Heap>()?;
    module.add_class::<Whence>()?;
    Ok(())
}
