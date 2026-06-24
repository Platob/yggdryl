//! Python extension for **yggdryl**.
//!
//! Thin PyO3 wrappers around [`yggdryl_url::Uri`]/[`yggdryl_url::Url`] and
//! [`yggdryl_version::Version`]; each type lives in its own module, mirroring the
//! Rust crates. All logic lives in the shared core, so the Python and Node
//! bindings behave identically.

// The `#[pymethods]` macro injects an `.into()` on returned errors; because our
// fallible methods already return `PyErr`, clippy flags it as a useless
// conversion. The lint fires on macro-generated code, so allow it crate-wide.
#![allow(clippy::useless_conversion)]

mod uri;
mod url;
mod version;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::wrap_pyfunction;
use yggdryl_url::{percent_decode, percent_encode, UriError, UrlError};
use yggdryl_version::VersionError;

use crate::uri::Uri;
use crate::url::Url;
use crate::version::Version;

pub(crate) fn uri_err(err: UriError) -> PyErr {
    PyValueError::new_err(err.to_string())
}

pub(crate) fn url_err(err: UrlError) -> PyErr {
    PyValueError::new_err(err.to_string())
}

pub(crate) fn version_err(err: VersionError) -> PyErr {
    PyValueError::new_err(err.to_string())
}

/// Stable hash of a string for `__hash__`.
pub(crate) fn hash_str(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// URL-safe percent-encode ``value`` (e.g. a space becomes ``%20``).
#[pyfunction]
#[pyo3(name = "percent_encode")]
fn py_percent_encode(value: &str) -> String {
    percent_encode(value)
}

/// Percent-decode ``value``, raising ``ValueError`` on a malformed escape.
#[pyfunction]
#[pyo3(name = "percent_decode")]
fn py_percent_decode(value: &str) -> PyResult<String> {
    percent_decode(value).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// The ``yggdryl`` Python module.
#[pymodule]
fn yggdryl(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_class::<Uri>()?;
    m.add_class::<Url>()?;
    m.add_class::<Version>()?;
    m.add_function(wrap_pyfunction!(py_percent_encode, m)?)?;
    m.add_function(wrap_pyfunction!(py_percent_decode, m)?)?;
    Ok(())
}
