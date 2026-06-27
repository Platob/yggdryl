//! The `BytesIO` pyclass.

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyType};
use yggdryl_core::{BytesIO as CoreBytesIO, Io, Mode};
use yggdryl_core::{CompressIo, Compression as CoreCompression};

use crate::iostats::IoStats;
use crate::media::MediaType;
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

/// Constructor input: a ``str`` (an existing file is read in, else the text is
/// UTF-8 encoded) or raw ``bytes``.
#[derive(FromPyObject)]
enum BytesInit {
    Str(String),
    Bytes(Vec<u8>),
}

#[pymethods]
impl BytesIO {
    /// Construct from optional ``initial`` contents — ``bytes`` taken verbatim, or
    /// a ``str`` resolved through :meth:`from_str` (an existing file is read in,
    /// else the text is UTF-8 encoded). ``stream`` (keyword-only, default ``True``)
    /// toggles cursor advancement. ``media_type`` (keyword-only) seeds the cached
    /// :attr:`media_type` so it is not inferred from the magic bytes.
    #[new]
    #[pyo3(signature = (initial = None, *, stream = true, media_type = None))]
    fn new(initial: Option<BytesInit>, stream: bool, media_type: Option<&MediaType>) -> Self {
        let mut inner = match initial {
            Some(BytesInit::Str(value)) => CoreBytesIO::from_str(&value),
            Some(BytesInit::Bytes(bytes)) => CoreBytesIO::from_bytes(bytes),
            None => CoreBytesIO::new(),
        };
        inner.set_stream(stream);
        if let Some(media_type) = media_type {
            inner = inner.with_media_type(media_type.inner.clone());
        }
        BytesIO { inner }
    }

    /// Build from a string: if ``value`` names an existing file, read its bytes;
    /// otherwise UTF-8-encode the string as the contents. ``stream`` (keyword-only,
    /// default ``True``) toggles cursor advancement.
    #[staticmethod]
    #[pyo3(signature = (value, *, stream = true))]
    fn from_str(value: &str, stream: bool) -> Self {
        let mut inner = CoreBytesIO::from_str(value);
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
    /// return the count written. Advances the cursor when :attr:`stream`. Raises
    /// ``ValueError`` if the write would extend the buffer past the addressable
    /// range (e.g. after seeking near the maximum past the end).
    fn write(&mut self, data: Vec<u8>) -> PyResult<usize> {
        self.inner.write(&data).map_err(io_err)
    }

    /// The resource address as a :class:`Url` (``mem://<address>``).
    #[getter]
    fn url(&self) -> Url {
        Url {
            inner: self.inner.url(),
        }
    }

    /// Discover this handle's metadata (see :class:`IoStats`): ``kind == "file"``
    /// and the buffer ``size``. The live byte count always wins; any
    /// :meth:`set_stats` override supplies the rest and the cached
    /// :attr:`media_type` is folded in.
    fn stats(&self) -> PyResult<IoStats> {
        self.inner
            .stats()
            .map(|inner| IoStats { inner })
            .map_err(io_err)
    }

    /// The :class:`MediaType` of this buffer — inferred from the magic bytes once
    /// and **cached**, or the one seeded via the ``media_type`` constructor
    /// argument. ``None`` when no type can be inferred.
    #[getter]
    fn media_type(&self) -> Option<MediaType> {
        self.inner.media_type().map(|inner| MediaType { inner })
    }

    /// The cached :class:`IoStats` if one has been installed with
    /// :meth:`set_stats`, else ``None`` — the *get* side of the stats cache.
    fn cached_stats(&self) -> Option<IoStats> {
        self.inner.cached_stats().map(|inner| IoStats { inner })
    }

    /// Install ``stats`` as this handle's cached metadata — the *set* side. The
    /// live byte count still wins in :meth:`stats`; the slot supplies the rest.
    fn set_stats(&mut self, stats: &IoStats) {
        self.inner.set_stats(stats.inner.clone());
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

    /// The current cursor position — the cross-language ``Io`` cursor accessor
    /// (same value as :meth:`tell`).
    fn stream_position(&self) -> u64 {
        self.inner.stream_position()
    }

    /// The total length in bytes when known without I/O, else ``None``.
    fn stream_len(&self) -> Option<u64> {
        self.inner.stream_len()
    }

    /// Return the entire buffer as ``bytes``, ignoring the cursor.
    fn getvalue<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.getvalue())
    }

    /// ``bytes(handle)`` — the whole buffer as native ``bytes`` (a copy across the
    /// FFI boundary; prefer staying on this handle for Rust-side work).
    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        self.getvalue(py)
    }

    /// Convert to a standard-library :class:`io.BytesIO`, for code that expects the
    /// native file-like object. Copies the bytes out of Rust.
    fn to_bytes_io<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let io = py.import_bound("io")?;
        let data = PyBytes::new_bound(py, self.inner.getvalue());
        io.getattr("BytesIO")?.call1((data,))
    }

    /// Parse the buffer's bytes as JSON (in Rust), returning the Python object.
    fn json(&mut self, py: Python<'_>) -> PyResult<PyObject> {
        let value = self.inner.json().map_err(io_err)?;
        Ok(crate::json_to_py(py, &value))
    }

    /// Compress this buffer's bytes (from the cursor) with ``codec`` — a name like
    /// ``"gzip"`` / ``"zstd"`` / ``"snappy"`` — into a new :class:`BytesIO`.
    fn compress(&mut self, codec: &str) -> PyResult<BytesIO> {
        let codec = CoreCompression::from_str(codec).map_err(io_err)?;
        let inner = self.inner.compress(codec).map_err(io_err)?;
        Ok(BytesIO { inner })
    }

    /// Decompress this buffer's bytes (from the cursor) into a new
    /// :class:`BytesIO`. ``codec`` names the codec; when ``None`` it is inferred
    /// from this buffer's magic bytes (e.g. a gzip header → ``gzip``).
    #[pyo3(signature = (codec = None))]
    fn decompress(&mut self, codec: Option<&str>) -> PyResult<BytesIO> {
        let codec = match codec {
            Some(name) => Some(CoreCompression::from_str(name).map_err(io_err)?),
            None => None,
        };
        let inner = self.inner.decompress(codec).map_err(io_err)?;
        Ok(BytesIO { inner })
    }

    /// Resize the buffer to ``size`` bytes (the current cursor when ``None``),
    /// returning the new length. The cursor is left where it is. Raises
    /// ``ValueError`` when growing past the addressable range.
    #[pyo3(signature = (size = None))]
    fn truncate(&mut self, size: Option<usize>) -> PyResult<usize> {
        self.inner.truncate(size).map_err(io_err)
    }

    /// No-op flush, present for parity with :class:`io.BytesIO`.
    fn flush(&self) {}

    /// Release the handle (a no-op for an in-memory buffer; the bytes are freed
    /// on drop). Idempotent.
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

    /// Support ``pickle`` / ``copy`` by reconstructing from the buffer's bytes
    /// (the cursor resets to the start, like a fresh :class:`BytesIO`).
    fn __reduce__<'py>(&self, py: Python<'py>) -> (Bound<'py, PyType>, (Bound<'py, PyBytes>,)) {
        (
            py.get_type_bound::<Self>(),
            (PyBytes::new_bound(py, self.inner.getvalue()),),
        )
    }
}
