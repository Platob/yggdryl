//! The `BytesIO` pyclass.

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use yggdryl_io::{BytesIO as CoreBytesIO, Io, Mode};

use crate::iostats::IoStats;
use crate::url::Url;
use crate::{io_err, whence_from};

/// A simple in-memory byte buffer with a cursor, modelled on Python's
/// :class:`io.BytesIO`: :meth:`read` / :meth:`write` / :meth:`seek` /
/// :meth:`tell` / :meth:`getvalue` / :meth:`truncate`, plus a :attr:`stream`
/// flag that toggles whether the cursor advances on reads and writes.
#[pyclass(name = "BytesIO", module = "yggdryl")]
pub struct BytesIO {
    pub(crate) inner: CoreBytesIO,
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

    /// Create an empty buffer preallocated to hold ``capacity`` bytes.
    #[staticmethod]
    fn with_capacity(capacity: usize) -> Self {
        BytesIO {
            inner: CoreBytesIO::with_capacity(capacity),
        }
    }

    /// The reserved capacity (bytes the buffer can hold before reallocating).
    #[getter]
    fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Reserve room for ``additional`` more bytes beyond the current length.
    fn reserve_capacity(&mut self, additional: usize) -> PyResult<()> {
        self.inner.reserve_capacity(additional).map_err(io_err)
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

    /// The resource address as a :class:`Url` (``mem://<address>``).
    #[getter]
    fn url(&self) -> Url {
        Url {
            inner: self.inner.url(),
        }
    }

    /// Discover this handle's metadata (see :class:`IoStats`): ``kind == "file"``
    /// and the buffer ``size``.
    fn stats(&self) -> PyResult<IoStats> {
        self.inner
            .stats()
            .map(|inner| IoStats { inner })
            .map_err(io_err)
    }

    /// The access mode: ``"r"``, ``"w"``, ``"a"`` or ``"r+"``.
    #[getter]
    fn mode(&self) -> &'static str {
        self.inner.mode().as_str()
    }

    /// Open a new :class:`BytesIO` derived from this one (a snapshot of the
    /// current bytes), applying ``mode`` (default ``"r"``) and ``stream``
    /// (default ``True``). ``mode`` accepts the Python forms (``r`` / ``w`` /
    /// ``a`` / ``r+`` / ``rb`` / ``a+`` / …): ``w`` truncates, ``a`` appends.
    #[pyo3(signature = (mode = "r", stream = true))]
    fn open(&self, mode: &str, stream: bool) -> PyResult<BytesIO> {
        let mode = Mode::from_str(mode).map_err(io_err)?;
        let parent = CoreBytesIO::from_bytes(self.inner.getvalue().to_vec());
        Ok(BytesIO {
            inner: parent.open(mode, stream),
        })
    }

    /// Positional read of up to ``size`` bytes at ``offset`` relative to
    /// ``whence`` (``0`` start, ``1`` current, ``2`` end). With ``0``/``2`` the
    /// cursor is untouched; with ``1`` it is used and advanced.
    #[pyo3(signature = (size, offset = 0, whence = 0))]
    fn pread<'py>(
        &mut self,
        py: Python<'py>,
        size: usize,
        offset: i64,
        whence: i64,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let mut buf = vec![0u8; size];
        let count = self
            .inner
            .pread(&mut buf, offset, whence_from(whence)?)
            .map_err(io_err)?;
        buf.truncate(count);
        Ok(PyBytes::new_bound(py, &buf))
    }

    /// Positional write of ``data`` at ``offset`` relative to ``whence``,
    /// returning the count written. With ``0``/``2`` the cursor is untouched;
    /// with ``1`` it is used and advanced.
    #[pyo3(signature = (data, offset = 0, whence = 0))]
    fn pwrite(&mut self, data: Vec<u8>, offset: i64, whence: i64) -> PyResult<usize> {
        self.inner
            .pwrite(&data, offset, whence_from(whence)?)
            .map_err(io_err)
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

    /// No-op close, present for the ``io`` API; the buffer is freed when the
    /// object is dropped.
    fn close(&self) {}

    /// Enter a ``with`` block, returning the handle itself.
    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Exit a ``with`` block. Returns ``False`` so exceptions propagate.
    #[pyo3(signature = (_exc_type = None, _exc_value = None, _traceback = None))]
    fn __exit__(
        &self,
        _exc_type: Option<PyObject>,
        _exc_value: Option<PyObject>,
        _traceback: Option<PyObject>,
    ) -> bool {
        self.close();
        false
    }

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
