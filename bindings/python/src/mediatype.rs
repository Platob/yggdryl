//! The `yggdryl.mediatype` submodule — an **ordered list of [`MimeType`]s**: the layered type
//! description of a resource (a content type plus any encodings/wrappers, or the stack a
//! multi-extension file name implies, e.g. `archive.tar.gz` → `application/x-tar` then
//! `application/gzip`).
//!
//! Mirrors `yggdryl_core::mediatype::MediaType`. A value type — equal, hashable,
//! byte-serializable (the comma-joined essences), and picklable (through its
//! [`MimeType`](crate::mimetype::MimeType) list, so each entry's extensions/magic round-trip
//! faithfully). A bad mime item handed to `parse` / `deserialize_bytes` raises a guided
//! `ValueError`.

// `useless_conversion`: pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type
// `From`.
#![allow(clippy::useless_conversion)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::mimetype::MimeType;
use yggdryl_core::io::IoError;
use yggdryl_core::mediatype;

/// Maps an [`IoError`] to a Python `ValueError` carrying its guided text.
fn ioerr(error: IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// An ordered list of [`MimeType`]s describing a resource, **primary first**. A single-type
/// media (`application/json`) is a one-element list; a wrapped one (`.tar.gz`) lists the
/// content type then its encodings. A value type — equal, hashable, byte-serializable.
#[pyclass(module = "yggdryl.mediatype")]
#[derive(Clone)]
pub struct MediaType {
    pub(crate) inner: mediatype::MediaType,
}

#[pymethods]
impl MediaType {
    /// A media type from an ordered list of [`MimeType`]s (primary first), or an empty media
    /// type when `types` is omitted.
    #[new]
    #[pyo3(signature = (types = None))]
    fn new(types: Option<Vec<MimeType>>) -> Self {
        match types {
            Some(types) => Self {
                inner: mediatype::MediaType::from_types(types.into_iter().map(|m| m.inner)),
            },
            None => Self {
                inner: mediatype::MediaType::new(),
            },
        }
    }

    /// Parses a **comma-separated mime list** (like an HTTP `Accept` / `Content-Type` value),
    /// dropping each item's parameters (`;q=…`) and skipping empty items. Raises a guided
    /// `ValueError` if any non-empty item is not a `type/subtype` essence.
    #[staticmethod]
    fn parse(s: &str) -> PyResult<Self> {
        mediatype::MediaType::parse_str(s)
            .map(|inner| Self { inner })
            .map_err(ioerr)
    }

    /// A single-type media over `mime`.
    #[staticmethod]
    fn of(mime: &MimeType) -> Self {
        Self {
            inner: mediatype::MediaType::of(mime.inner.clone()),
        }
    }

    /// Builds a media type from a file's **extensions** (outermost-last): each known extension
    /// maps to its [`MimeType`], an unknown one is skipped (`["tar", "gz"]` →
    /// `[application/x-tar, application/gzip]`).
    #[staticmethod]
    fn from_extensions(exts: Vec<String>) -> Self {
        Self {
            inner: mediatype::MediaType::from_extensions(exts),
        }
    }

    /// The **primary** type (the first), or `None` when empty.
    fn primary(&self) -> Option<MimeType> {
        self.inner.primary().map(|inner| MimeType {
            inner: inner.clone(),
        })
    }

    /// The listed types, primary first.
    fn types(&self) -> Vec<MimeType> {
        self.inner
            .types()
            .iter()
            .map(|inner| MimeType {
                inner: inner.clone(),
            })
            .collect()
    }

    /// The listed essences, primary first (`["application/x-tar", "application/gzip"]`).
    fn essences(&self) -> Vec<String> {
        self.inner
            .essences()
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    /// Whether any listed type has the given `essence` (case-insensitive).
    fn contains(&self, essence: &str) -> bool {
        self.inner.contains(essence)
    }

    /// Appends a type to the list (in place).
    fn push(&mut self, mime: &MimeType) {
        self.inner.push(mime.inner.clone());
    }

    /// An explicit copy of this media type (equivalent to `copy.copy(media)`).
    fn copy(&self) -> Self {
        self.clone()
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// The value form — the comma-joined essences (the inverse of `parse`). Each entry's
    /// extensions/magic are dropped, like [`MimeType.serialize_bytes`](MimeType).
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(
            py,
            &yggdryl_core::io::Serializable::serialize_bytes(&self.inner),
        )
    }

    /// Reconstructs a media type from the comma-joined essence bytes produced by
    /// `serialize_bytes`, raising a guided `ValueError` on non-UTF-8 bytes or a bad item.
    #[staticmethod]
    fn deserialize_bytes(data: &[u8]) -> PyResult<Self> {
        <mediatype::MediaType as yggdryl_core::io::Serializable>::deserialize_bytes(data)
            .map(|inner| Self { inner })
            .map_err(ioerr)
    }

    /// The number of listed types.
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// Truthiness — `True` when the list has at least one type.
    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    /// Pickles through the constructor's [`MimeType`] list, so each entry's extensions/magic
    /// round-trip faithfully — unlike the essence-only byte codec.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Vec<MimeType>,))> {
        let ctor = py.get_type_bound::<MediaType>().into_any().unbind();
        let types = self
            .inner
            .types()
            .iter()
            .map(|inner| MimeType {
                inner: inner.clone(),
            })
            .collect();
        Ok((ctor, (types,)))
    }

    /// The comma-joined essences (`"application/x-tar, application/gzip"`).
    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("MediaType({:?})", self.inner.to_string())
    }
}

/// Populates the `mediatype` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<MediaType>()?;
    Ok(())
}
