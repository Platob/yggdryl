//! Python extension for **yggdryl**.
//!
//! Thin PyO3 wrappers around [`yggdryl_core::Uri`]/[`yggdryl_core::Url`],
//! [`yggdryl_core::Version`], [`yggdryl_core::MimeType`] and
//! [`yggdryl_core::MediaType`]; each type lives in its own module, mirroring the
//! Rust crates. All logic lives in the shared core, so the Python and Node
//! bindings behave identically.

// The `#[pymethods]` macro injects an `.into()` on returned errors; because our
// fallible methods already return `PyErr`, clippy flags it as a useless
// conversion. The lint fires on macro-generated code, so allow it crate-wide.
#![allow(clippy::useless_conversion)]

mod bytesio;
mod compression;
mod datatype;
mod date;
mod datetime;
mod duration;
mod field;
mod http;
mod iostats;
mod localpath;
mod media;
mod mime;
mod pytime;
mod timezone;
mod uri;
mod url;
mod version;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::wrap_pyfunction;
use yggdryl_core::MediaError;
use yggdryl_core::VersionError;
use yggdryl_core::{percent_decode, percent_encode, UriError, UrlError};
use yggdryl_core::{IoError, TimeError, TimeUnit, Whence};
use yggdryl_http::HttpError;

use crate::bytesio::BytesIO;
use crate::compression::Compression;
use crate::datatype::DataType;
use crate::date::Date;
use crate::datetime::DateTime;
use crate::duration::Duration;
use crate::field::Field;
use crate::http::{
    http_delete, http_get, http_head, http_patch, http_post, http_put, http_request, set_base_url,
    HttpRequest, HttpResponse, HttpSession,
};
use crate::iostats::IoStats;
use crate::localpath::LocalPath;
use crate::media::MediaType;
use crate::mime::MimeType;
use crate::pytime::Time;
use crate::timezone::Timezone;
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

pub(crate) fn http_err(err: HttpError) -> PyErr {
    PyValueError::new_err(err.to_string())
}

pub(crate) fn time_err(err: TimeError) -> PyErr {
    PyValueError::new_err(err.to_string())
}

/// Parses a time-unit string (``s`` / ``ms`` / ``us`` / ``ns``) to a [`TimeUnit`],
/// raising ``ValueError`` on a bad token. Shared by the schema / duration types.
pub(crate) fn time_unit_from(unit: &str) -> PyResult<TimeUnit> {
    TimeUnit::from_str(unit).map_err(time_err)
}

/// Converts a `serde_json::Value` (from [`yggdryl_core::Io::json`]) into the
/// matching Python object, so JSON is parsed in Rust and handed back natively.
pub(crate) fn json_to_py(py: Python<'_>, value: &serde_json::Value) -> PyObject {
    use pyo3::types::{PyDict, PyList};
    use serde_json::Value;
    match value {
        Value::Null => py.None(),
        Value::Bool(b) => b.into_py(py),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into_py(py)
            } else if let Some(u) = n.as_u64() {
                u.into_py(py)
            } else {
                n.as_f64().unwrap_or(f64::NAN).into_py(py)
            }
        }
        Value::String(s) => s.into_py(py),
        Value::Array(items) => {
            let list = PyList::empty_bound(py);
            for item in items {
                let _ = list.append(json_to_py(py, item));
            }
            list.into()
        }
        Value::Object(map) => {
            let dict = PyDict::new_bound(py);
            for (key, value) in map {
                let _ = dict.set_item(key, json_to_py(py, value));
            }
            dict.into()
        }
    }
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

/// Open a byte-IO handle for ``location``, dispatching on its URL scheme (the
/// core ``Io`` factory): a bare path or ``file://`` URL opens a
/// :class:`LocalPath`. Remote schemes (``http`` / ``https``) are served by
/// :class:`HttpSession`; any other scheme raises ``ValueError``.
#[pyfunction]
#[pyo3(name = "open")]
fn py_open(location: &str) -> PyResult<LocalPath> {
    let uri = yggdryl_core::Uri::from_str(location).map_err(uri_err)?;
    match uri.scheme() {
        "file" | "" => Ok(LocalPath {
            inner: yggdryl_core::LocalPath::from_uri(&uri),
        }),
        other => Err(PyValueError::new_err(format!(
            "no local Io handle for scheme {other:?}; use HttpSession for http/https"
        ))),
    }
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
    m.add_class::<DataType>()?;
    m.add_class::<Field>()?;
    m.add_class::<Date>()?;
    m.add_class::<Time>()?;
    m.add_class::<DateTime>()?;
    m.add_class::<Duration>()?;
    m.add_class::<Timezone>()?;
    m.add_class::<HttpSession>()?;
    m.add_class::<HttpRequest>()?;
    m.add_class::<HttpResponse>()?;
    m.add_function(wrap_pyfunction!(py_percent_encode, m)?)?;
    m.add_function(wrap_pyfunction!(py_percent_decode, m)?)?;
    m.add_function(wrap_pyfunction!(py_open, m)?)?;
    // Module-level HTTP verbs backed by the shared `HttpSession` singleton,
    // mirroring `requests.get(...)` and friends.
    m.add_function(wrap_pyfunction!(http_get, m)?)?;
    m.add_function(wrap_pyfunction!(http_head, m)?)?;
    m.add_function(wrap_pyfunction!(http_delete, m)?)?;
    m.add_function(wrap_pyfunction!(http_post, m)?)?;
    m.add_function(wrap_pyfunction!(http_put, m)?)?;
    m.add_function(wrap_pyfunction!(http_patch, m)?)?;
    m.add_function(wrap_pyfunction!(http_request, m)?)?;
    m.add_function(wrap_pyfunction!(set_base_url, m)?)?;
    Ok(())
}
