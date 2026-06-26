//! The `IoStats` pyclass.

use std::time::{Duration, UNIX_EPOCH};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyType;
use yggdryl_core::{IoStats as CoreIoStats, Kind};

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
    /// Construct stats explicitly. ``kind`` is one of ``"missing"`` / ``"file"`` /
    /// ``"directory"`` / ``"other"``; ``mtime`` is Unix-epoch seconds. The lazily-
    /// discovered ``media_type`` is not set here.
    #[new]
    #[pyo3(signature = (size = 0, kind = "file", mtime = None, content_type = None, etag = None))]
    fn new(
        size: u64,
        kind: &str,
        mtime: Option<f64>,
        content_type: Option<String>,
        etag: Option<String>,
    ) -> PyResult<Self> {
        let kind = match kind {
            "missing" => Kind::Missing,
            "file" => Kind::File,
            "directory" => Kind::Directory,
            "other" => Kind::Other,
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown kind {other:?}, expected 'missing', 'file', 'directory' or 'other'"
                )))
            }
        };
        let mut inner = CoreIoStats::new(size).with_kind(kind);
        if let Some(seconds) = mtime {
            inner = inner.with_mtime(UNIX_EPOCH + Duration::from_secs_f64(seconds));
        }
        if let Some(content_type) = content_type {
            inner = inner.with_content_type(content_type);
        }
        if let Some(etag) = etag {
            inner = inner.with_etag(etag);
        }
        Ok(IoStats { inner })
    }

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

    /// Support ``pickle`` / ``copy`` by reconstructing through the constructor
    /// (the lazily-discovered ``media_type`` is not carried).
    #[allow(clippy::type_complexity)]
    fn __reduce__<'py>(
        &self,
        py: Python<'py>,
    ) -> (
        Bound<'py, PyType>,
        (
            u64,
            &'static str,
            Option<f64>,
            Option<String>,
            Option<String>,
        ),
    ) {
        let mtime = self
            .inner
            .mtime()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|since| since.as_secs_f64());
        (
            py.get_type_bound::<Self>(),
            (
                self.inner.size(),
                self.inner.kind().as_str(),
                mtime,
                self.inner.content_type().map(str::to_string),
                self.inner.etag().map(str::to_string),
            ),
        )
    }
}
