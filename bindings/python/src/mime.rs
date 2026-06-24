//! The `MimeType` pyclass and the global registry hooks.

use pyo3::prelude::*;
use yggdryl_media::{FromInput, Mapping, MimeType as CoreMimeType, Signature, ToOutput};

use crate::{hash_str, media_err};

/// A single common media (MIME) type, parsed from a string or inferred from a
/// file extension or magic bytes. The extension/magic registry is global and can
/// be extended with :meth:`register` / :meth:`unregister`.
#[pyclass(name = "MimeType", module = "yggdryl")]
#[derive(Clone)]
pub struct MimeType {
    pub(crate) inner: CoreMimeType,
}

#[pymethods]
impl MimeType {
    /// Parse a ``type/subtype`` MIME string, raising ``ValueError`` on failure.
    /// Any ``;parameters`` are dropped; unknown but well-formed types are kept
    /// verbatim as ``Other``.
    #[new]
    fn new(value: &str) -> PyResult<Self> {
        CoreMimeType::from_str(value)
            .map(|inner| MimeType { inner })
            .map_err(media_err)
    }

    /// Alias for the constructor.
    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        MimeType::new(value)
    }

    /// Build a :class:`MimeType` from a dict of components (``type``, ``subtype``).
    #[staticmethod]
    fn from_mapping(fields: Mapping) -> PyResult<Self> {
        CoreMimeType::from_mapping(&fields)
            .map(|inner| MimeType { inner })
            .map_err(media_err)
    }

    /// Build a :class:`MimeType` straight from its ``type`` and ``subtype`` parts,
    /// without parsing a string, e.g. ``MimeType.from_parts("text", "csv")``.
    #[staticmethod]
    fn from_parts(type_: &str, subtype: &str) -> Self {
        MimeType {
            inner: CoreMimeType::from_parts(type_, subtype),
        }
    }

    /// Infer the MIME type from a file ``extension``, or ``None`` if unknown.
    #[staticmethod]
    fn from_extension(extension: &str) -> Option<Self> {
        CoreMimeType::from_extension(extension).map(|inner| MimeType { inner })
    }

    /// Infer the MIME type from a file's leading ``data`` bytes (magic bytes), or
    /// ``None`` if none match. Recognises Arrow IPC, Parquet, ZIP, gzip, etc.
    #[staticmethod]
    fn from_magic(data: Vec<u8>) -> Option<Self> {
        CoreMimeType::from_magic(&data).map(|inner| MimeType { inner })
    }

    /// Infer the outermost MIME type from a ``path``'s last known file extension,
    /// or ``None``. For the full layered view use :meth:`MediaType.from_path`.
    #[staticmethod]
    fn from_path(path: &str) -> Option<Self> {
        CoreMimeType::from_path(path).map(|inner| MimeType { inner })
    }

    /// The fallback MIME type, ``application/octet-stream`` — the conventional
    /// default when no more specific type is known.
    #[staticmethod]
    #[allow(clippy::should_implement_trait)]
    fn default() -> Self {
        MimeType {
            inner: CoreMimeType::default(),
        }
    }

    /// Register (or replace) a MIME type globally. ``magic`` is a list of byte
    /// prefixes matched at the start of a file. The change is process-wide.
    #[staticmethod]
    #[pyo3(signature = (mime, extensions, magic = Vec::new()))]
    fn register(mime: &str, extensions: Vec<String>, magic: Vec<Vec<u8>>) {
        let exts: Vec<&str> = extensions.iter().map(String::as_str).collect();
        let sigs: Vec<Signature> = magic.into_iter().map(Signature::prefix).collect();
        CoreMimeType::register(mime, &exts, &sigs);
    }

    /// Remove a MIME type from the global registry, returning whether it existed.
    #[staticmethod]
    fn unregister(mime: &str) -> bool {
        CoreMimeType::unregister(mime)
    }

    /// Restore the global registry to its built-in defaults.
    #[staticmethod]
    fn reset_registry() {
        CoreMimeType::reset_registry()
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
    fn extension(&self) -> Option<String> {
        self.inner.extension()
    }

    /// The file extensions registered for this type (the first is canonical).
    #[getter]
    fn extensions(&self) -> Vec<String> {
        self.inner.extensions()
    }

    /// Whether this is a built-in type rather than a fallback ``Other``.
    #[getter]
    fn is_known(&self) -> bool {
        self.inner.is_known()
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("MimeType('{}')", self.inner)
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        hash_str(self.inner.mime())
    }
}
