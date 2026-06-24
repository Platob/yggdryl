//! The `LocalPath` pyclass.

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use yggdryl_io::{BytesIO as CoreBytesIO, Io, LocalPath as CoreLocalPath, Mode, Path};

use crate::bytesio::BytesIO;
use crate::iostats::IoStats;
use crate::media::MediaType;
use crate::url::Url;
use crate::{io_err, whence_from};

/// A local filesystem path opened as a byte-IO handle, memory-mapped lazily.
/// Positional (:meth:`pread`) and streamed (:meth:`read`) access share one
/// cursor; the :attr:`stream` flag toggles whether :meth:`read` advances it (as
/// in :class:`BytesIO`). :meth:`stats` / :meth:`media_type` expose metadata.
#[pyclass(name = "LocalPath", module = "yggdryl")]
pub struct LocalPath {
    pub(crate) inner: CoreLocalPath,
}

#[pymethods]
impl LocalPath {
    /// Open a handle for ``location``, statting it up front (so :attr:`url` /
    /// :meth:`stats` are ready). Never raises — a missing path yields a handle
    /// whose :meth:`stats` report ``kind == "missing"``.
    #[new]
    fn new(location: &str) -> Self {
        LocalPath {
            inner: CoreLocalPath::open(location),
        }
    }

    /// Write ``data`` to this path on disk, auto-creating missing parent
    /// directories (lazily, only on a missing-parent failure).
    fn write(&self, data: Vec<u8>) -> PyResult<()> {
        self.inner.write(&data).map_err(io_err)
    }

    /// The resource address as a :class:`Url` (``file://`` over the path).
    #[getter]
    fn url(&self) -> Url {
        Url {
            inner: self.inner.url(),
        }
    }

    /// The access mode (always ``"r"`` — the mapped handle is read-only).
    #[getter]
    fn mode(&self) -> &'static str {
        self.inner.mode().as_str()
    }

    /// Whether :meth:`read` advances the cursor (the same flag as
    /// :attr:`BytesIO.stream`).
    #[getter]
    fn stream(&self) -> bool {
        self.inner.stream()
    }

    #[setter]
    fn set_stream(&mut self, value: bool) {
        self.inner.set_stream(value);
    }

    /// Open an in-memory :class:`BytesIO` over this file's bytes, applying
    /// ``mode`` (default ``"r"``) and ``stream`` (default ``True``) — a
    /// :class:`LocalPath` and a :class:`BytesIO` open the same way.
    #[pyo3(signature = (mode = "r", stream = true))]
    fn open(&self, mode: &str, stream: bool) -> PyResult<BytesIO> {
        let mode = Mode::from_str(mode).map_err(io_err)?;
        let parent = CoreBytesIO::from_bytes(self.inner.getvalue().to_vec());
        Ok(BytesIO {
            inner: parent.open(mode, stream),
        })
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

    /// Move the cursor to ``offset`` relative to ``whence`` (``0`` start, ``1``
    /// current, ``2`` end), returning the new position.
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

    /// The capacity in bytes (the mapped file size; the handle is read-only, so
    /// :meth:`reserve_capacity` / :meth:`truncate` are unsupported).
    #[getter]
    fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Return the entire file contents as ``bytes``, ignoring the cursor.
    fn getvalue<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.as_slice().unwrap_or(&[]))
    }

    /// Discover this file's metadata (see :class:`IoStats`).
    fn stats(&self) -> PyResult<IoStats> {
        self.inner
            .stats()
            .map(|inner| IoStats { inner })
            .map_err(io_err)
    }

    /// The lazily-inferred :class:`MediaType` of this file, or ``None``.
    fn media_type(&self) -> Option<MediaType> {
        self.inner.media_type().map(|inner| MediaType { inner })
    }

    /// The file location.
    #[getter]
    fn location(&self) -> &str {
        self.inner.location()
    }

    /// Whether the file currently exists.
    fn exists(&self) -> bool {
        self.inner.exists()
    }

    /// No-op close, present for the ``io`` API; the mapping is released when the
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

    fn __len__(&self) -> usize {
        self.inner.as_slice().map_or(0, <[u8]>::len)
    }

    fn __repr__(&self) -> String {
        format!("LocalPath({:?})", self.inner.location())
    }
}
