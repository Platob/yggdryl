//! The `yggdryl.io` submodule — the byte-I/O family: [`Bytes`] (an in-memory buffer with a
//! cursor) and the [`Whence`] seek origin.
//!
//! Mirrors `yggdryl_core::io`'s [`Bytes`](yggdryl_core::io::Bytes), which implements the
//! core's `IOBase` / `IOCursor` / `IOSlice` traits — positioned `pread` / `pwrite`, cursor
//! `read` / `write` with `seek(whence, offset)`, and zero-copy `slice` with copy-on-write
//! writes. Reads return a `bytes`; writes take any bytes-like and return the byte count. An
//! end-of-data `read_exact`, a seek before the start, or an out-of-bounds `slice` raise a
//! guided `ValueError`. Like `bytearray`, `Bytes` is mutable and so is not hashable.

// pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type `From`.
#![allow(clippy::useless_conversion)]

use pyo3::exceptions::{PyIndexError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PySlice};

use yggdryl_core::io::{self, IOBase, IOCursor, IOSlice, IoError};

/// Maps an [`IoError`] to a Python `ValueError` carrying its guided text.
fn io_err(error: IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Where a [`Bytes.seek`](Bytes::seek) offset is measured from — POSIX `SEEK_SET` /
/// `SEEK_CUR` / `SEEK_END`.
#[pyclass(eq, eq_int, module = "yggdryl.io")]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Whence {
    /// From the start of the data (absolute).
    Start,
    /// From the current cursor position.
    Current,
    /// From the end of the data.
    End,
}

impl Whence {
    fn to_core(self) -> io::Whence {
        match self {
            Whence::Start => io::Whence::Start,
            Whence::Current => io::Whence::Current,
            Whence::End => io::Whence::End,
        }
    }
}

/// An in-memory, growable byte buffer with a cursor — Arrow-backed, with zero-copy reads and
/// slices and copy-on-write writes.
#[pyclass(module = "yggdryl.io")]
#[derive(Clone)]
pub struct Bytes {
    pub(crate) inner: io::Bytes,
}

#[pymethods]
impl Bytes {
    /// Builds a buffer from a bytes-like (empty by default). The bytes are copied in; the
    /// cursor starts at `0`.
    #[new]
    #[pyo3(signature = (data = None))]
    fn new(data: Option<&[u8]>) -> Self {
        Self {
            inner: match data {
                Some(bytes) => io::Bytes::from_slice(bytes),
                None => io::Bytes::new(),
            },
        }
    }

    /// An empty buffer that can grow to `capacity` bytes before its first reallocation.
    #[staticmethod]
    fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: io::Bytes::with_capacity(capacity),
        }
    }

    /// The current cursor position (bytes from the start; may sit past the end after a seek).
    #[getter]
    fn position(&self) -> u64 {
        self.inner.position()
    }

    // ---- positioned (random-access) read/write -------------------------------------

    /// Reads up to `size` bytes starting at `offset` (short near the end), without moving the
    /// cursor.
    #[pyo3(signature = (offset, size))]
    fn pread<'py>(&self, py: Python<'py>, offset: u64, size: usize) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.pread_vec(offset, size))
    }

    /// Reads **exactly** `size` bytes at `offset`, raising `ValueError` if fewer remain.
    fn pread_exact<'py>(
        &self,
        py: Python<'py>,
        offset: u64,
        size: usize,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let mut buf = vec![0u8; size];
        self.inner.pread_exact(offset, &mut buf).map_err(io_err)?;
        Ok(PyBytes::new_bound(py, &buf))
    }

    /// Writes `data` at `offset`, growing (and zero-filling any gap) as needed; returns the
    /// number of bytes written. Does not move the cursor.
    fn pwrite(&mut self, offset: u64, data: &[u8]) -> usize {
        self.inner.pwrite(offset, data)
    }

    // ---- cursor read/write ---------------------------------------------------------

    /// Reads up to `size` bytes from the cursor, advancing it (short at the end).
    fn read<'py>(&mut self, py: Python<'py>, size: usize) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.read_vec(size))
    }

    /// Reads **exactly** `size` bytes from the cursor, advancing it; raises on end-of-data
    /// (leaving the cursor put).
    fn read_exact<'py>(&mut self, py: Python<'py>, size: usize) -> PyResult<Bound<'py, PyBytes>> {
        let mut buf = vec![0u8; size];
        self.inner.read_exact(&mut buf).map_err(io_err)?;
        Ok(PyBytes::new_bound(py, &buf))
    }

    /// Writes `data` at the cursor, advancing it; returns the number of bytes written.
    fn write(&mut self, data: &[u8]) -> usize {
        self.inner.write(data)
    }

    /// Reads from the cursor to the end, advancing it to the end.
    fn read_to_end<'py>(&mut self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.read_to_end())
    }

    // ---- seek ----------------------------------------------------------------------

    /// Seeks to `whence + offset` and returns the new position. A position past the end is
    /// allowed; seeking before the start raises `ValueError`.
    #[pyo3(signature = (whence, offset = 0))]
    fn seek(&mut self, whence: Whence, offset: i64) -> PyResult<u64> {
        self.inner.seek(whence.to_core(), offset).map_err(io_err)
    }

    /// Resets the cursor to the start.
    fn rewind(&mut self) {
        self.inner.rewind();
    }

    // ---- slice + interchange -------------------------------------------------------

    /// A bounded window `[offset, offset+length)` as a new `Bytes` — zero-copy, sharing the
    /// allocation until either side is written. Raises `ValueError` if it runs past the end.
    fn slice(&self, offset: u64, length: u64) -> PyResult<Self> {
        self.inner
            .slice(offset, length)
            .map(|inner| Self { inner })
            .map_err(io_err)
    }

    /// The whole content as a `bytes` (one copy).
    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.as_slice())
    }

    /// An explicit copy of this buffer (content and cursor).
    fn copy(&self) -> Self {
        self.clone()
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone() // copy-on-write storage — the copy is independent on the first write
    }

    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        self.to_bytes(py)
    }

    fn __len__(&self) -> usize {
        self.inner.len() as usize
    }

    fn __bool__(&self) -> bool {
        self.inner.len() != 0
    }

    /// Random access, like `bytearray` — `buf[i]` returns an `int` (negative indices allowed),
    /// `buf[a:b:step]` returns a `bytes`. Raises `IndexError` / `TypeError` on a bad key.
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<PyObject> {
        let data = self.inner.as_slice();
        let len = data.len() as isize;
        if let Ok(slice) = key.downcast::<PySlice>() {
            let indices = slice.indices(len)?;
            let mut out = Vec::with_capacity(indices.slicelength);
            let mut index = indices.start;
            for _ in 0..indices.slicelength {
                out.push(data[index as usize]);
                index += indices.step;
            }
            Ok(PyBytes::new_bound(py, &out).into_any().unbind())
        } else {
            let raw = key
                .extract::<isize>()
                .map_err(|_| PyTypeError::new_err("Bytes indices must be integers or slices"))?;
            let index = if raw < 0 { raw + len } else { raw };
            if index < 0 || index >= len {
                return Err(PyIndexError::new_err("Bytes index out of range"));
            }
            Ok(data[index as usize].into_py(py))
        }
    }

    /// Content equality (the cursor is not part of the value).
    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __repr__(&self) -> String {
        format!(
            "Bytes(len={}, position={})",
            self.inner.len(),
            self.inner.position()
        )
    }
}

/// Populates the `io` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Whence>()?;
    module.add_class::<Bytes>()?;
    Ok(())
}
