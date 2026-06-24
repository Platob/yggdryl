//! The `Compression` pyclass.

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use yggdryl_io::Compression as CoreCompression;

use crate::{hash_str, io_err};

/// A byte-stream compression codec — ``gzip``, ``zstd`` or ``snappy`` (or
/// ``none``, the identity codec) — that compresses and decompresses bytes. The
/// backends are optional Cargo features in the core, so a codec may parse and
/// name itself yet report :attr:`is_available` ``False`` when its backend was not
/// compiled in.
#[pyclass(name = "Compression", module = "yggdryl")]
#[derive(Clone)]
pub struct Compression {
    pub(crate) inner: CoreCompression,
}

#[pymethods]
impl Compression {
    /// Parse a codec name — ``none`` / ``identity`` / ``store``, ``gzip`` /
    /// ``gz``, ``zstd`` / ``zst``, ``snappy`` / ``snap`` / ``sz`` — raising
    /// ``ValueError`` on an unknown one.
    #[new]
    fn new(value: &str) -> PyResult<Self> {
        CoreCompression::from_str(value)
            .map(|inner| Compression { inner })
            .map_err(io_err)
    }

    /// Alias for the constructor.
    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        Compression::new(value)
    }

    /// Infer the codec from a file ``extension`` (``gz`` / ``zst`` / ``sz``, with
    /// or without a leading dot), or ``None`` if it names no known codec.
    #[staticmethod]
    fn from_extension(extension: &str) -> Option<Self> {
        CoreCompression::from_extension(extension).map(|inner| Compression { inner })
    }

    /// The canonical codec name (``"none"`` / ``"gzip"`` / ``"zstd"`` /
    /// ``"snappy"``).
    #[getter]
    fn name(&self) -> &'static str {
        self.inner.as_str()
    }

    /// The conventional file extension (``"gz"`` / ``"zst"`` / ``"sz"``), or
    /// ``None`` for the identity codec.
    #[getter]
    fn extension(&self) -> Option<&'static str> {
        self.inner.extension()
    }

    /// Whether this codec's backend is compiled in, so :meth:`compress` /
    /// :meth:`decompress` will work. ``none`` is always available.
    #[getter]
    fn is_available(&self) -> bool {
        self.inner.is_available()
    }

    /// Compress ``data`` in full and return the encoded ``bytes``. Raises
    /// ``ValueError`` if this codec's backend is not available.
    fn compress<'py>(&self, py: Python<'py>, data: Vec<u8>) -> PyResult<Bound<'py, PyBytes>> {
        let packed = self.inner.compress(&data).map_err(io_err)?;
        Ok(PyBytes::new_bound(py, &packed))
    }

    /// Decompress ``data`` in full and return the decoded ``bytes``. Raises
    /// ``ValueError`` if this codec's backend is not available or the data is
    /// malformed.
    fn decompress<'py>(&self, py: Python<'py>, data: Vec<u8>) -> PyResult<Bound<'py, PyBytes>> {
        let raw = self.inner.decompress(&data).map_err(io_err)?;
        Ok(PyBytes::new_bound(py, &raw))
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Compression('{}')", self.inner.as_str())
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        hash_str(self.inner.as_str())
    }
}
