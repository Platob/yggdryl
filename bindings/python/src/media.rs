//! The `MediaType` pyclass: an ordered stack of :class:`MimeType`.

use pyo3::prelude::*;
use yggdryl_media::{FromInput, Mapping, MediaType as CoreMediaType, ToOutput};

use crate::mime::MimeType;
use crate::{hash_str, media_err};

/// An ordered stack of :class:`MimeType`, describing a layered file. Parsing
/// ``data.csv.gz`` yields ``MediaType([MimeType('text/csv'), MimeType('application/gzip')])``.
#[pyclass(name = "MediaType", module = "yggdryl")]
#[derive(Clone)]
pub struct MediaType {
    pub(crate) inner: CoreMediaType,
}

#[pymethods]
impl MediaType {
    /// Build a :class:`MediaType` from an ordered list of :class:`MimeType`.
    #[new]
    fn new(types: Vec<MimeType>) -> Self {
        MediaType {
            inner: CoreMediaType::new(types.into_iter().map(|t| t.inner).collect()),
        }
    }

    /// Build the stack from a path's file extensions (innermost content first).
    #[staticmethod]
    fn from_path(path: &str) -> Self {
        MediaType {
            inner: CoreMediaType::from_path(path),
        }
    }

    /// Parse a path or file name into its :class:`MimeType` stack.
    #[staticmethod]
    #[pyo3(signature = (value, safe = true))]
    fn from_str(value: &str, safe: bool) -> PyResult<Self> {
        CoreMediaType::from_str(value, safe)
            .map(|inner| MediaType { inner })
            .map_err(media_err)
    }

    /// Build the stack from a dict; reads the ``path`` key (or ``str``).
    #[staticmethod]
    #[pyo3(signature = (fields, safe = true))]
    fn from_mapping(fields: Mapping, safe: bool) -> PyResult<Self> {
        CoreMediaType::from_mapping(&fields, safe)
            .map(|inner| MediaType { inner })
            .map_err(media_err)
    }

    /// Build a single-type stack from one file ``extension`` (empty if unknown).
    #[staticmethod]
    fn from_extension(extension: &str) -> Self {
        MediaType {
            inner: CoreMediaType::from_extension(extension),
        }
    }

    /// Build the stack from an ordered list of file ``extensions``.
    #[staticmethod]
    fn from_extensions(extensions: Vec<String>) -> Self {
        let exts: Vec<&str> = extensions.iter().map(String::as_str).collect();
        MediaType {
            inner: CoreMediaType::from_extensions(&exts),
        }
    }

    /// The fallback stack, a single ``application/octet-stream`` — the default
    /// when no type can be inferred.
    #[staticmethod]
    #[allow(clippy::should_implement_trait)]
    fn default() -> Self {
        MediaType {
            inner: CoreMediaType::default(),
        }
    }

    /// Render to a component ``dict`` (the inverse of ``from_mapping``).
    fn to_mapping(&self) -> Mapping {
        self.inner.to_mapping()
    }

    /// The ordered :class:`MimeType` list, innermost content first.
    #[getter]
    fn types(&self) -> Vec<MimeType> {
        self.inner
            .types()
            .iter()
            .map(|inner| MimeType {
                inner: inner.clone(),
            })
            .collect()
    }

    /// The innermost (content) type, e.g. ``text/csv`` for ``data.csv.gz``.
    #[getter]
    fn first(&self) -> Option<MimeType> {
        self.inner.first().map(|inner| MimeType {
            inner: inner.clone(),
        })
    }

    /// The outermost (container) type, e.g. ``application/gzip`` for ``data.csv.gz``.
    #[getter]
    fn last(&self) -> Option<MimeType> {
        self.inner.last().map(|inner| MimeType {
            inner: inner.clone(),
        })
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    fn __getitem__(&self, index: usize) -> PyResult<MimeType> {
        self.inner
            .types()
            .get(index)
            .map(|inner| MimeType {
                inner: inner.clone(),
            })
            .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err("media type index out of range"))
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
        hash_str(&self.inner.to_string())
    }
}
