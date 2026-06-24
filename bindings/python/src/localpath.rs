//! The `LocalPath` pyclass.

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use yggdryl_io::{Io, LocalPath as CoreLocalPath, Path, Seek};

use crate::iostats::IoStats;
use crate::media::MediaType;
use crate::{io_err, whence_from};

/// A local filesystem path opened as a byte-IO handle, memory-mapped for
/// zero-copy direct access. Random (:meth:`read_at`) and streamed (:meth:`read`)
/// access share one cursor; :meth:`stats` and :meth:`media_type` expose metadata.
#[pyclass(name = "LocalPath", module = "yggdryl")]
pub struct LocalPath {
    pub(crate) inner: CoreLocalPath,
}

#[pymethods]
impl LocalPath {
    /// Open ``location`` for reading, raising ``ValueError`` if it is missing.
    #[new]
    fn new(location: &str) -> PyResult<Self> {
        CoreLocalPath::open(location)
            .map(|inner| LocalPath { inner })
            .map_err(io_err)
    }

    /// Alias for the constructor.
    #[staticmethod]
    fn open(location: &str) -> PyResult<Self> {
        LocalPath::new(location)
    }

    /// Write ``data`` to ``location`` on disk (creating or truncating it).
    #[staticmethod]
    fn write(location: &str, data: Vec<u8>) -> PyResult<()> {
        CoreLocalPath::write(location, &data).map_err(io_err)
    }

    /// Read up to ``size`` bytes from the cursor; ``None`` or a negative ``size``
    /// reads all remaining bytes. Advances the cursor.
    #[pyo3(signature = (size = None))]
    fn read<'py>(&mut self, py: Python<'py>, size: Option<i64>) -> PyResult<Bound<'py, PyBytes>> {
        let size = match size {
            Some(n) if n >= 0 => Some(n as usize),
            _ => None,
        };
        let bytes = self.inner.read_owned(size).map_err(io_err)?;
        Ok(PyBytes::new_bound(py, &bytes))
    }

    /// Read up to ``size`` bytes at absolute ``offset`` without moving the cursor.
    fn read_at<'py>(
        &mut self,
        py: Python<'py>,
        offset: u64,
        size: usize,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let mut buf = vec![0u8; size];
        let count = self.inner.read_at(offset, &mut buf).map_err(io_err)?;
        buf.truncate(count);
        Ok(PyBytes::new_bound(py, &buf))
    }

    /// Move the cursor to ``offset`` relative to ``whence`` (``0`` start, ``1``
    /// current, ``2`` end), returning the new position.
    #[pyo3(signature = (offset, whence = 0))]
    fn seek(&mut self, offset: i64, whence: i64) -> PyResult<u64> {
        self.inner
            .seek(offset, whence_from(whence)?)
            .map_err(io_err)
    }

    /// The current cursor position.
    fn tell(&self) -> u64 {
        self.inner.stream_position()
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

    fn __len__(&self) -> usize {
        self.inner.as_slice().map_or(0, <[u8]>::len)
    }

    fn __repr__(&self) -> String {
        format!("LocalPath({:?})", self.inner.location())
    }
}
