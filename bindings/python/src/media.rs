//! The `MediaType` pyclass.

use pyo3::prelude::*;
use yggdryl_media::{FromInput, Mapping, MediaType as CoreMediaType, ToOutput};

use crate::{hash_str, media_err};

/// A common media (MIME) type, parsed from a string or inferred from a file
/// extension or magic bytes.
#[pyclass(name = "MediaType", module = "yggdryl")]
#[derive(Clone)]
pub struct MediaType {
    pub(crate) inner: CoreMediaType,
}

#[pymethods]
impl MediaType {
    /// Parse a ``type/subtype`` MIME string, raising ``ValueError`` on failure.
    /// Any ``;parameters`` are dropped; with ``safe=False`` the input is taken
    /// as-is. Unknown but well-formed types are kept verbatim.
    #[new]
    #[pyo3(signature = (value, safe = true))]
    fn new(value: &str, safe: bool) -> PyResult<Self> {
        CoreMediaType::from_str(value, safe)
            .map(|inner| MediaType { inner })
            .map_err(media_err)
    }

    /// Alias for the constructor.
    #[staticmethod]
    #[pyo3(signature = (value, safe = true))]
    fn from_str(value: &str, safe: bool) -> PyResult<Self> {
        MediaType::new(value, safe)
    }

    /// Build a :class:`MediaType` from a dict of components (``type``, ``subtype``).
    #[staticmethod]
    #[pyo3(signature = (fields, safe = true))]
    fn from_mapping(fields: Mapping, safe: bool) -> PyResult<Self> {
        CoreMediaType::from_mapping(&fields, safe)
            .map(|inner| MediaType { inner })
            .map_err(media_err)
    }

    /// Infer the media type from a file ``extension``, or ``None`` if unknown.
    #[staticmethod]
    fn from_extension(extension: &str) -> Option<Self> {
        CoreMediaType::from_extension(extension).map(|inner| MediaType { inner })
    }

    /// Infer the media type from a file's leading ``data`` bytes (magic bytes),
    /// or ``None`` if none match. Recognises Arrow IPC, Parquet, ZIP, gzip, etc.
    #[staticmethod]
    fn from_magic(data: Vec<u8>) -> Option<Self> {
        CoreMediaType::from_magic(&data).map(|inner| MediaType { inner })
    }

    /// Infer the media type from a path's file extension, or ``None``.
    #[staticmethod]
    fn from_path(path: &str) -> Option<Self> {
        CoreMediaType::from_path(path).map(|inner| MediaType { inner })
    }

    /// Render to a component ``dict`` (the inverse of ``from_mapping``).
    fn to_mapping(&self) -> Mapping {
        self.inner.to_mapping()
    }

    /// The canonical ``type/subtype`` MIME string.
    #[getter]
    fn mime(&self) -> &str {
        self.inner.mime()
    }

    /// The top-level type, e.g. ``"image"`` for ``image/png``.
    #[getter]
    #[pyo3(name = "type")]
    fn type_(&self) -> &str {
        self.inner.type_()
    }

    /// The subtype, e.g. ``"png"`` for ``image/png``.
    #[getter]
    fn subtype(&self) -> &str {
        self.inner.subtype()
    }

    /// The canonical (first) file extension, if any.
    #[getter]
    fn extension(&self) -> Option<&str> {
        self.inner.extension()
    }

    /// The file extensions associated with this type (the first is canonical).
    #[getter]
    fn extensions(&self) -> Vec<&str> {
        self.inner.extensions().to_vec()
    }

    /// Whether this is a registry type rather than a fallback ``Other``.
    #[getter]
    fn is_known(&self) -> bool {
        self.inner.is_known()
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("MediaType('{}')", self.inner)
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        hash_str(self.inner.mime())
    }
}
