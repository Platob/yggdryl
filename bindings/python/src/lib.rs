//! Python extension for **yggdryl**.
//!
//! Thin PyO3 wrappers around [`yggdryl_core::Uri`] and [`yggdryl_core::Url`]; all
//! parsing lives in the shared Rust core so the Python and Node bindings behave
//! identically.

// The `#[pymethods]` macro injects an `.into()` on returned errors; because our
// fallible methods already return `PyErr`, clippy flags it as a useless
// conversion. The lint fires on macro-generated code, so allow it crate-wide.
#![allow(clippy::useless_conversion)]

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::pyclass::CompareOp;
use yggdryl_core::{
    Uri as CoreUri, UriError, Url as CoreUrl, UrlError, Version as CoreVersion, VersionError,
};

fn uri_err(err: UriError) -> PyErr {
    PyValueError::new_err(err.to_string())
}

fn url_err(err: UrlError) -> PyErr {
    PyValueError::new_err(err.to_string())
}

fn version_err(err: VersionError) -> PyErr {
    PyValueError::new_err(err.to_string())
}

/// A generic RFC 3986 URI: ``scheme:[//authority]path[?query][#fragment]``.
#[pyclass(name = "Uri", module = "yggdryl")]
#[derive(Clone)]
struct Uri {
    inner: CoreUri,
}

#[pymethods]
impl Uri {
    /// Parse ``value`` into a :class:`Uri`, raising ``ValueError`` on failure.
    #[new]
    fn new(value: &str) -> PyResult<Self> {
        CoreUri::parse(value)
            .map(|inner| Uri { inner })
            .map_err(uri_err)
    }

    /// Alias for the constructor.
    #[staticmethod]
    fn parse(value: &str) -> PyResult<Self> {
        Uri::new(value)
    }

    #[getter]
    fn scheme(&self) -> &str {
        self.inner.scheme()
    }

    #[getter]
    fn authority(&self) -> Option<&str> {
        self.inner.authority()
    }

    #[getter]
    fn path(&self) -> &str {
        self.inner.path()
    }

    #[getter]
    fn query(&self) -> Option<&str> {
        self.inner.query()
    }

    #[getter]
    fn fragment(&self) -> Option<&str> {
        self.inner.fragment()
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Uri('{}')", self.inner)
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        hash_str(&self.inner.to_string())
    }
}

/// A URL: a URI that always has an authority, split into ``username``,
/// ``password``, ``host`` and ``port``.
#[pyclass(name = "Url", module = "yggdryl")]
#[derive(Clone)]
struct Url {
    inner: CoreUrl,
}

#[pymethods]
impl Url {
    /// Parse ``value`` into a :class:`Url`, raising ``ValueError`` on failure.
    #[new]
    fn new(value: &str) -> PyResult<Self> {
        CoreUrl::parse(value)
            .map(|inner| Url { inner })
            .map_err(url_err)
    }

    /// Alias for the constructor.
    #[staticmethod]
    fn parse(value: &str) -> PyResult<Self> {
        Url::new(value)
    }

    #[getter]
    fn scheme(&self) -> &str {
        self.inner.scheme()
    }

    #[getter]
    fn username(&self) -> Option<&str> {
        self.inner.username()
    }

    #[getter]
    fn password(&self) -> Option<&str> {
        self.inner.password()
    }

    #[getter]
    fn host(&self) -> &str {
        self.inner.host()
    }

    #[getter]
    fn port(&self) -> Option<u16> {
        self.inner.port()
    }

    #[getter]
    fn path(&self) -> &str {
        self.inner.path()
    }

    #[getter]
    fn query(&self) -> Option<&str> {
        self.inner.query()
    }

    #[getter]
    fn fragment(&self) -> Option<&str> {
        self.inner.fragment()
    }

    #[getter]
    fn authority(&self) -> String {
        self.inner.authority()
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Url('{}')", self.inner)
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        hash_str(&self.inner.to_string())
    }
}

/// A generic ``major.minor.patch`` version, ordered numerically.
#[pyclass(name = "Version", module = "yggdryl")]
#[derive(Clone)]
struct Version {
    inner: CoreVersion,
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
    #[staticmethod]
    fn parse(value: &str) -> PyResult<Self> {
        CoreVersion::parse(value)
            .map(|inner| Version { inner })
            .map_err(version_err)
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
}

/// Stable hash of a string for `__hash__`.
fn hash_str(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// The ``yggdryl`` Python module.
#[pymodule]
fn yggdryl(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_class::<Uri>()?;
    m.add_class::<Url>()?;
    m.add_class::<Version>()?;
    Ok(())
}
