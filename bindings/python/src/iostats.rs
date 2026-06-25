//! The `IoStats` pyclass.

use std::time::UNIX_EPOCH;

use pyo3::prelude::*;
use yggdryl_core::IoStats as CoreIoStats;

use crate::media::MediaType;

/// Lazily-discovered metadata for an IO handle: ``size`` / ``mtime`` /
/// ``content_type`` / ``etag`` are cheap and eager, while ``media_type`` is
/// discovered only when the handle is asked for it.
#[pyclass(name = "IoStats", module = "yggdryl")]
#[derive(Clone)]
pub struct IoStats {
    pub(crate) inner: CoreIoStats,
}

#[pymethods]
impl IoStats {
    /// What the resource is: ``"missing"``, ``"file"``, ``"directory"`` or
    /// ``"other"``.
    #[getter]
    fn kind(&self) -> &'static str {
        self.inner.kind().as_str()
    }

    /// Whether the resource exists (its :attr:`kind` is not ``"missing"``).
    #[getter]
    fn exists(&self) -> bool {
        self.inner.exists()
    }

    /// Whether the resource is a regular file (or in-memory blob).
    #[getter]
    fn is_file(&self) -> bool {
        self.inner.is_file()
    }

    /// Whether the resource is a directory.
    #[getter]
    fn is_dir(&self) -> bool {
        self.inner.is_dir()
    }

    /// The size in bytes.
    #[getter]
    fn size(&self) -> u64 {
        self.inner.size()
    }

    /// The last-modified time as a Unix timestamp (seconds), or ``None``.
    #[getter]
    fn mtime(&self) -> Option<f64> {
        self.inner
            .mtime()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|since| since.as_secs_f64())
    }

    /// The transport content type (e.g. a cloud ``Content-Type``), or ``None``.
    #[getter]
    fn content_type(&self) -> Option<&str> {
        self.inner.content_type()
    }

    /// The backend entity tag (e.g. an object-store ``ETag``), or ``None``.
    #[getter]
    fn etag(&self) -> Option<&str> {
        self.inner.etag()
    }

    /// The discovered :class:`MediaType`, if any has been filled in.
    #[getter]
    fn media_type(&self) -> Option<MediaType> {
        self.inner.media_type().map(|media| MediaType {
            inner: media.clone(),
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "IoStats(kind='{}', size={})",
            self.inner.kind(),
            self.inner.size()
        )
    }
}
