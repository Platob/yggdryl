//! The `BytesIO` pyclass.

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use yggdryl_io::{BytesIO as CoreBytesIO, Whence};

use crate::io_err;

/// A simple in-memory byte buffer with a cursor, modelled on Python's
/// :class:`io.BytesIO`: :meth:`read` / :meth:`write` / :meth:`seek` /
/// :meth:`tell` / :meth:`getvalue` / :meth:`truncate`, plus a :attr:`stream`
/// flag that toggles whether the cursor advances on reads and writes.
#[pyclass(name = "BytesIO", module = "yggdryl")]
pub struct BytesIO {
    pub(crate) inner: CoreBytesIO,
}

/// Maps a Python ``whence`` integer (``SEEK_SET`` / ``SEEK_CUR`` / ``SEEK_END``)
/// to the core [`Whence`], raising ``ValueError`` on any other value.
fn whence_from(whence: i64) -> PyResult<Whence> {
    match whence {
        0 => Ok(Whence::Start),
        1 => Ok(Whence::Current),
        2 => Ok(Whence::End),
        other => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "invalid whence ({other}), expected 0, 1 or 2"
        ))),
    }
}

#[pymethods]
impl BytesIO {
    /// Construct from optional ``initial`` bytes, with the cursor at the start.
    /// ``stream`` (keyword-only, default ``True``) toggles cursor advancement.
    #[new]
    #[pyo3(signature = (initial = Vec::new(), *, stream = true))]
    fn new(initial: Vec<u8>, stream: bool) -> Self {
        let mut inner = CoreBytesIO::from_bytes(initial);
        inner.set_stream(stream);
        BytesIO { inner }
    }

    /// Read up to ``size`` bytes from the cursor; ``None`` or a negative ``size``
    /// reads all remaining bytes. Advances the cursor when :attr:`stream`.
    #[pyo3(signature = (size = None))]
    fn read<'py>(&mut self, py: Python<'py>, size: Option<i64>) -> Bound<'py, PyBytes> {
        let size = match size {
            Some(n) if n >= 0 => Some(n as usize),
            _ => None,
        };
        PyBytes::new_bound(py, &self.inner.read(size))
    }

    /// Read from the cursor through the next newline (inclusive), or to the end.
    /// Advances the cursor when :attr:`stream`.
    fn readline<'py>(&mut self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.read_line())
    }

    /// Write ``data`` at the cursor (overwriting and zero-filling as needed) and
    /// return the count written. Advances the cursor when :attr:`stream`.
    fn write(&mut self, data: Vec<u8>) -> usize {
        self.inner.write(&data)
    }

    /// Move the cursor to ``offset`` relative to ``whence`` (``0`` start, ``1``
    /// current, ``2`` end), returning the new position. Raises ``ValueError`` if
    /// it would land before the start.
    #[pyo3(signature = (offset, whence = 0))]
    fn seek(&mut self, offset: i64, whence: i64) -> PyResult<usize> {
        self.inner
            .seek(offset, whence_from(whence)?)
            .map_err(io_err)
    }

    /// The current cursor position.
    fn tell(&self) -> usize {
        self.inner.tell()
    }

    /// Return the entire buffer as ``bytes``, ignoring the cursor.
    fn getvalue<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.getvalue())
    }

    /// Resize the buffer to ``size`` bytes (the current cursor when ``None``),
    /// returning the new length. The cursor is left where it is.
    #[pyo3(signature = (size = None))]
    fn truncate(&mut self, size: Option<usize>) -> usize {
        self.inner.truncate(size)
    }

    /// No-op flush, present for parity with :class:`io.BytesIO`.
    fn flush(&self) {}

    /// Whether reads and writes advance the cursor (Python-stream semantics).
    #[getter]
    fn stream(&self) -> bool {
        self.inner.stream()
    }

    #[setter]
    fn set_stream(&mut self, value: bool) {
        self.inner.set_stream(value);
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__<'py>(&mut self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        let line = self.inner.read_line();
        if line.is_empty() {
            None
        } else {
            Some(PyBytes::new_bound(py, &line))
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "BytesIO(len={}, pos={}, stream={})",
            self.inner.len(),
            self.inner.tell(),
            self.inner.stream()
        )
    }
}
