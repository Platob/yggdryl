//! The `LocalPath` pyclass.

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use yggdryl_core::{BytesIO as CoreBytesIO, Io, LocalPath as CoreLocalPath, Mode, Path};
use yggdryl_core::{CompressIo, Compression as CoreCompression};

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

    /// Read from the cursor through the next newline (inclusive), or to the end.
    /// Advances the cursor when :attr:`stream`.
    fn readline<'py>(&mut self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.read_line())
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

    /// Parse the file's bytes as JSON (in Rust), returning the Python object.
    fn json(&mut self, py: Python<'_>) -> PyResult<PyObject> {
        let value = self.inner.json().map_err(io_err)?;
        Ok(crate::json_to_py(py, &value))
    }

    /// Compress this file's bytes (from the cursor) with ``codec`` — a name like
    /// ``"gzip"`` / ``"zstd"`` / ``"snappy"`` — into a new :class:`BytesIO`.
    fn compress(&mut self, codec: &str) -> PyResult<BytesIO> {
        let codec = CoreCompression::from_str(codec).map_err(io_err)?;
        let inner = self.inner.compress(codec).map_err(io_err)?;
        Ok(BytesIO { inner })
    }

    /// Decompress this file's bytes (from the cursor) into a new :class:`BytesIO`.
    /// ``codec`` names the codec; when ``None`` it is inferred from this handle
    /// (its URL extension, e.g. ``data.csv.gz`` → gzip, then its stats' media
    /// type).
    #[pyo3(signature = (codec = None))]
    fn decompress(&mut self, codec: Option<&str>) -> PyResult<BytesIO> {
        let codec = match codec {
            Some(name) => Some(CoreCompression::from_str(name).map_err(io_err)?),
            None => None,
        };
        let inner = self.inner.decompress(codec).map_err(io_err)?;
        Ok(BytesIO { inner })
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

    /// No-op flush, present for the ``io`` API.
    fn flush(&self) {}

    /// Release the handle (a no-op; the mapping is released on drop). Idempotent.
    fn close(&mut self) -> PyResult<()> {
        self.inner.close().map_err(io_err)
    }

    /// Enter a ``with`` block, returning the handle itself.
    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Exit a ``with`` block: close the handle and return ``False`` so any
    /// exception propagates.
    #[pyo3(signature = (_exc_type = None, _exc_value = None, _traceback = None))]
    fn __exit__(
        &mut self,
        _exc_type: Option<PyObject>,
        _exc_value: Option<PyObject>,
        _traceback: Option<PyObject>,
    ) -> PyResult<bool> {
        self.close()?;
        Ok(false)
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
        format!("LocalPath({:?})", self.inner.location())
    }
}
