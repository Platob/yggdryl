//! The `Version` pyclass.

use pyo3::prelude::*;
use pyo3::pyclass::CompareOp;
use pyo3::types::PyType;
use yggdryl_core::{Mapping, Version as CoreVersion};

use crate::{hash_str, version_err};

/// A generic ``major.minor.patch`` version, ordered numerically.
#[pyclass(name = "Version", module = "yggdryl")]
#[derive(Clone)]
pub struct Version {
    pub(crate) inner: CoreVersion,
}

#[pymethods]
impl Version {
    /// Construct from components; ``minor`` and ``patch`` default to ``0``.
    #[new]
    #[pyo3(signature = (major, minor = 0, patch = 0))]
    fn new(major: u64, minor: u64, patch: u64) -> Self {
        Version {
            inner: CoreVersion::new(major, minor, patch),
        }
    }

    /// Parse a ``major[.minor[.patch]]`` string, raising ``ValueError`` on failure.
    /// Components must be non-negative integers (at most three).
    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        CoreVersion::from_str(value)
            .map(|inner| Version { inner })
            .map_err(version_err)
    }

    /// Build a :class:`Version` from a dict of components (``major``, ``minor``,
    /// ``patch``).
    #[staticmethod]
    fn from_mapping(fields: Mapping) -> PyResult<Self> {
        CoreVersion::from_mapping(&fields)
            .map(|inner| Version { inner })
            .map_err(version_err)
    }

    /// Return a copy, overriding any component passed and keeping the rest.
    #[pyo3(signature = (major = None, minor = None, patch = None))]
    fn copy(&self, major: Option<u64>, minor: Option<u64>, patch: Option<u64>) -> Self {
        Version {
            inner: self.inner.copy(major, minor, patch),
        }
    }

    /// Return a copy with the major component replaced.
    fn with_major(&self, major: u64) -> Self {
        Version {
            inner: self.inner.with_major(major),
        }
    }

    /// Return a copy with the minor component replaced.
    fn with_minor(&self, minor: u64) -> Self {
        Version {
            inner: self.inner.with_minor(minor),
        }
    }

    /// Return a copy with the patch component replaced.
    fn with_patch(&self, patch: u64) -> Self {
        Version {
            inner: self.inner.with_patch(patch),
        }
    }

    /// Render to a component ``dict`` (the inverse of ``from_mapping``).
    fn to_mapping(&self) -> Mapping {
        self.inner.to_mapping()
    }

    #[getter]
    fn major(&self) -> u64 {
        self.inner.major()
    }

    #[getter]
    fn minor(&self) -> u64 {
        self.inner.minor()
    }

    #[getter]
    fn patch(&self) -> u64 {
        self.inner.patch()
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Version('{}')", self.inner)
    }

    fn __richcmp__(&self, other: &Self, op: CompareOp) -> bool {
        op.matches(self.inner.cmp(&other.inner))
    }

    fn __hash__(&self) -> u64 {
        hash_str(&self.inner.to_string())
    }

    /// Support ``pickle`` / ``copy`` by reconstructing through the constructor.
    fn __reduce__<'py>(&self, py: Python<'py>) -> (Bound<'py, PyType>, (u64, u64, u64)) {
        (
            py.get_type_bound::<Self>(),
            (self.inner.major(), self.inner.minor(), self.inner.patch()),
        )
    }
}
