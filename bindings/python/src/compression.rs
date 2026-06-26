//! The `Compression` pyclass.

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyType};
use yggdryl_core::Compression as CoreCompression;

use crate::iostats::IoStats;
use crate::media::MediaType;
use crate::mime::MimeType;
use crate::{hash_str, io_err};

/// A byte-stream compression codec — ``gzip``, ``deflate`` (zlib), ``zstd``,
/// ``snappy`` or ``brotli`` (or ``none``, the identity codec) — that compresses
/// and decompresses bytes. The backends are optional Cargo features in the core,
/// so a codec may parse and name itself yet report :attr:`is_available` ``False``
/// when its backend was not compiled in.
#[pyclass(name = "Compression", module = "yggdryl")]
#[derive(Clone)]
pub struct Compression {
    pub(crate) inner: CoreCompression,
}

#[pymethods]
impl Compression {
    /// Parse a codec name — ``none`` / ``identity`` / ``store``, ``gzip`` /
    /// ``gz``, ``deflate`` / ``zlib`` / ``zz``, ``zstd`` / ``zst``, ``snappy`` /
    /// ``snap`` / ``sz``, ``brotli`` / ``br`` — raising ``ValueError`` on an
    /// unknown one.
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

    /// Infer the codec from a :class:`MimeType` (e.g. ``application/gzip`` →
    /// ``gzip``), or ``None`` if the MIME names no supported codec.
    #[staticmethod]
    fn from_mime(mime: &MimeType) -> Option<Self> {
        CoreCompression::from_mime(&mime.inner).map(|inner| Compression { inner })
    }

    /// Infer the codec from a layered :class:`MediaType` stack — its outermost
    /// (container) MIME, e.g. ``gzip`` for ``data.csv.gz`` — or ``None``.
    #[staticmethod]
    fn from_media(media: &MediaType) -> Option<Self> {
        CoreCompression::from_media(&media.inner).map(|inner| Compression { inner })
    }

    /// Infer the codec from an :class:`IoStats` — its discovered media type first,
    /// then its transport content type — or ``None`` if neither names a codec.
    #[staticmethod]
    fn from_stats(stats: &IoStats) -> Option<Self> {
        CoreCompression::from_stats(&stats.inner).map(|inner| Compression { inner })
    }

    /// The :class:`MimeType` this codec is carried as — the inverse of
    /// :meth:`from_mime`, used to add an encoding layer to a media type. ``None``
    /// for the identity codec and ``deflate`` / ``snappy`` (which have no
    /// registered MIME).
    fn mime(&self) -> Option<MimeType> {
        self.inner.mime().map(|inner| MimeType { inner })
    }

    /// The canonical codec name (``"none"`` / ``"gzip"`` / ``"deflate"`` /
    /// ``"zstd"`` / ``"snappy"`` / ``"brotli"``).
    #[getter]
    fn name(&self) -> &'static str {
        self.inner.as_str()
    }

    /// The conventional file extension (``"gz"`` / ``"zz"`` / ``"zst"`` /
    /// ``"sz"`` / ``"br"``), or ``None`` for the identity codec.
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

    /// Support ``pickle`` / ``copy`` by reconstructing from the codec name.
    fn __reduce__<'py>(&self, py: Python<'py>) -> (Bound<'py, PyType>, (String,)) {
        (
            py.get_type_bound::<Self>(),
            (self.inner.as_str().to_string(),),
        )
    }
}
