//! Python extension for **yggdryl**.
//!
//! Thin PyO3 wrappers around [`yggdryl_url::Uri`]/[`yggdryl_url::Url`],
//! [`yggdryl_version::Version`], [`yggdryl_media::MimeType`] and
//! [`yggdryl_media::MediaType`]; each type lives in its own module, mirroring the
//! Rust crates. All logic lives in the shared core, so the Python and Node
//! bindings behave identically.

// The `#[pymethods]` macro injects an `.into()` on returned errors; because our
// fallible methods already return `PyErr`, clippy flags it as a useless
// conversion. The lint fires on macro-generated code, so allow it crate-wide.
#![allow(clippy::useless_conversion)]

mod bytesio;
mod compression;
mod iostats;
mod localpath;
mod media;
mod mime;
mod uri;
mod url;
mod version;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::wrap_pyfunction;
use yggdryl_io::{IoError, Whence};
use yggdryl_media::MediaError;
use yggdryl_url::{percent_decode, percent_encode, UriError, UrlError};
use yggdryl_version::VersionError;

use crate::bytesio::BytesIO;
use crate::compression::Compression;
use crate::iostats::IoStats;
use crate::localpath::LocalPath;
use crate::media::MediaType;
use crate::mime::MimeType;
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

pub(crate) fn media_err(err: MediaError) -> PyErr {
    PyValueError::new_err(err.to_string())
}

pub(crate) fn io_err(err: IoError) -> PyErr {
    PyValueError::new_err(err.to_string())
}

/// Maps a Python ``whence`` integer (``SEEK_SET`` / ``SEEK_CUR`` / ``SEEK_END``)
/// to the core [`Whence`], raising ``ValueError`` on any other value. Shared by
/// the seekable IO types.
pub(crate) fn whence_from(whence: i64) -> PyResult<Whence> {
    match whence {
        0 => Ok(Whence::Start),
        1 => Ok(Whence::Current),
        2 => Ok(Whence::End),
        other => Err(PyValueError::new_err(format!(
            "invalid whence ({other}), expected 0, 1 or 2"
        ))),
    }
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
    percent_decode(value)
        .map(|decoded| decoded.into_owned())
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

/// The ``yggdryl`` Python module.
#[pymodule]
fn yggdryl(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_class::<Uri>()?;
    m.add_class::<Url>()?;
    m.add_class::<Version>()?;
    m.add_class::<MimeType>()?;
    m.add_class::<MediaType>()?;
    m.add_class::<BytesIO>()?;
    m.add_class::<IoStats>()?;
    m.add_class::<LocalPath>()?;
    m.add_class::<Compression>()?;
    m.add_function(wrap_pyfunction!(py_percent_encode, m)?)?;
    m.add_function(wrap_pyfunction!(py_percent_decode, m)?)?;
    Ok(())
}
