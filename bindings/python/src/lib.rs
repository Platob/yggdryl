//! Python extension for **yggdryl**.
//!
//! Thin PyO3 wrappers around [`yggdryl_url::Uri`] and [`yggdryl_url::Url`]; all
//! parsing lives in the shared Rust core so the Python and Node bindings behave
//! identically.

// The `#[pymethods]` macro injects an `.into()` on returned errors; because our
// fallible methods already return `PyErr`, clippy flags it as a useless
// conversion. The lint fires on macro-generated code, so allow it crate-wide.
#![allow(clippy::useless_conversion)]

use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;
use pyo3::pyclass::CompareOp;
use pyo3::wrap_pyfunction;
use yggdryl_url::{
    percent_decode, percent_encode, FromInput, Mapping, Params, Uri as CoreUri, UriError,
    Url as CoreUrl, UrlError, Version as CoreVersion, VersionError,
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
    /// With ``safe=False`` the scheme and ``%XX`` escapes are not validated.
    #[new]
    #[pyo3(signature = (value, safe = true))]
    fn new(value: &str, safe: bool) -> PyResult<Self> {
        CoreUri::from_str(value, safe)
            .map(|inner| Uri { inner })
            .map_err(uri_err)
    }

    /// Alias for the constructor.
    #[staticmethod]
    #[pyo3(signature = (value, safe = true))]
    fn from_str(value: &str, safe: bool) -> PyResult<Self> {
        Uri::new(value, safe)
    }

    /// Build a :class:`Uri` from a dict of components (``scheme``, ``authority``,
    /// ``path``, ``query``, ``fragment``).
    #[staticmethod]
    #[pyo3(signature = (fields, safe = true))]
    fn from_mapping(fields: Mapping, safe: bool) -> PyResult<Self> {
        CoreUri::from_mapping(&fields, safe)
            .map(|inner| Uri { inner })
            .map_err(uri_err)
    }

    /// Build a :class:`Uri` directly from its parts (no string parsing).
    #[staticmethod]
    #[pyo3(signature = (scheme, path = String::new(), authority = None, query = None, fragment = None))]
    fn from_parts(
        scheme: String,
        path: String,
        authority: Option<String>,
        query: Option<String>,
        fragment: Option<String>,
    ) -> Self {
        Uri {
            inner: CoreUri::from_parts(scheme, authority, path, query, fragment),
        }
    }

    /// Return a copy, overriding any component passed and keeping the rest.
    /// ``copy()`` clones; ``copy(path="/x")`` clones with one field changed.
    #[pyo3(signature = (scheme = None, authority = None, path = None, query = None, fragment = None))]
    fn copy(
        &self,
        scheme: Option<String>,
        authority: Option<String>,
        path: Option<String>,
        query: Option<String>,
        fragment: Option<String>,
    ) -> Self {
        Uri {
            inner: self.inner.copy(scheme, authority, path, query, fragment),
        }
    }

    /// Return a copy with the scheme replaced.
    fn with_scheme(&self, scheme: String) -> Self {
        Uri {
            inner: self.inner.clone().with_scheme(scheme),
        }
    }

    /// Return a copy with the authority set.
    fn with_authority(&self, authority: String) -> Self {
        Uri {
            inner: self.inner.clone().with_authority(authority),
        }
    }

    /// Return a copy with the authority removed.
    fn without_authority(&self) -> Self {
        Uri {
            inner: self.inner.clone().without_authority(),
        }
    }

    /// Return a copy with the path replaced.
    fn with_path(&self, path: String) -> Self {
        Uri {
            inner: self.inner.clone().with_path(path),
        }
    }

    /// Return a copy with the query set.
    fn with_query(&self, query: String) -> Self {
        Uri {
            inner: self.inner.clone().with_query(query),
        }
    }

    /// Return a copy with the query removed.
    fn without_query(&self) -> Self {
        Uri {
            inner: self.inner.clone().without_query(),
        }
    }

    /// Return a copy with the fragment set.
    fn with_fragment(&self, fragment: String) -> Self {
        Uri {
            inner: self.inner.clone().with_fragment(fragment),
        }
    }

    /// Return a copy with the fragment removed.
    fn without_fragment(&self) -> Self {
        Uri {
            inner: self.inner.clone().without_fragment(),
        }
    }

    /// Return the query as a ``dict[str, list[str]]``; ``decode`` percent-decodes.
    #[pyo3(signature = (decode = true))]
    fn params(&self, decode: bool) -> Params {
        self.inner.params(decode)
    }

    /// Return a copy whose query is built from ``params``; ``encode`` percent-
    /// encodes each key and value.
    #[pyo3(signature = (params, encode = true))]
    fn with_params(&self, params: Params, encode: bool) -> Self {
        Uri {
            inner: self.inner.clone().with_params(&params, encode),
        }
    }

    /// Return a copy with ``key`` set to ``values``, adding or replacing it.
    #[pyo3(signature = (key, values, encode = true))]
    fn add_param(&self, key: String, values: Vec<String>, encode: bool) -> Self {
        Uri {
            inner: self.inner.add_param(key, values, encode),
        }
    }

    /// Base scheme before any ``+`` extension (e.g. ``https`` for ``https+zip``).
    #[getter]
    fn scheme_base(&self) -> &str {
        self.inner.scheme_base()
    }

    /// The ``+``-separated scheme extensions (e.g. ``["zip"]``).
    #[getter]
    fn scheme_ext(&self) -> Vec<&str> {
        self.inner.scheme_ext()
    }

    /// Build a :class:`Uri` from a :class:`Url`.
    #[staticmethod]
    fn from_url(url: &Url) -> Self {
        Uri {
            inner: CoreUri::from_url(&url.inner),
        }
    }

    /// Parse this URI into a :class:`Url` (requires an authority and host).
    fn to_url(&self) -> PyResult<Url> {
        self.inner
            .to_url()
            .map(|inner| Url { inner })
            .map_err(url_err)
    }

    /// Decoded values of one query parameter, or ``None``.
    fn get_param(&self, key: &str) -> Option<Vec<String>> {
        self.inner.get_param(key)
    }

    /// Whether the query has a parameter named ``key``.
    fn has_param(&self, key: &str) -> bool {
        self.inner.has_param(key)
    }

    fn __contains__(&self, key: &str) -> bool {
        self.inner.has_param(key)
    }

    /// ``url[key]`` -> the parameter's values (raises ``KeyError`` if absent).
    fn __getitem__(&self, key: &str) -> PyResult<Vec<String>> {
        self.inner
            .get_param(key)
            .ok_or_else(|| PyKeyError::new_err(key.to_string()))
    }

    /// ``url[key] = values`` -> set the parameter in place (percent-encoded).
    fn __setitem__(&mut self, key: String, values: Vec<String>) {
        self.inner = self.inner.set_param(key, values, true);
    }

    /// ``del url[key]`` -> remove the parameter in place.
    fn __delitem__(&mut self, key: &str) -> PyResult<()> {
        if !self.inner.has_param(key) {
            return Err(PyKeyError::new_err(key.to_string()));
        }
        self.inner = self.inner.remove_param(key, true);
        Ok(())
    }

    /// Return a copy with one parameter created or replaced (single update).
    #[pyo3(signature = (key, values, encode = true))]
    fn set_param(&self, key: String, values: Vec<String>, encode: bool) -> Self {
        Uri {
            inner: self.inner.set_param(key, values, encode),
        }
    }

    /// Return a copy with every entry of ``params`` set, others untouched (bulk).
    #[pyo3(signature = (params, encode = true))]
    fn set_params(&self, params: Params, encode: bool) -> Self {
        Uri {
            inner: self.inner.set_params(&params, encode),
        }
    }

    /// Return a copy with one parameter removed (single delete).
    #[pyo3(signature = (key, encode = true))]
    fn remove_param(&self, key: &str, encode: bool) -> Self {
        Uri {
            inner: self.inner.remove_param(key, encode),
        }
    }

    /// Return a copy with several parameters removed (bulk delete).
    #[pyo3(signature = (keys, encode = true))]
    fn remove_params(&self, keys: Vec<String>, encode: bool) -> Self {
        Uri {
            inner: self.inner.remove_params(&keys, encode),
        }
    }

    /// Return a copy with the entire query removed.
    fn clear_params(&self) -> Self {
        Uri {
            inner: self.inner.clear_params(),
        }
    }

    /// Render the URI; ``encode`` (default) percent-encodes, else decodes.
    #[pyo3(signature = (encode = true))]
    fn to_string(&self, encode: bool) -> String {
        self.inner.to_str(encode)
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
    /// With ``safe=False`` the scheme and ``%XX`` escapes are not validated.
    #[new]
    #[pyo3(signature = (value, safe = true))]
    fn new(value: &str, safe: bool) -> PyResult<Self> {
        CoreUrl::from_str(value, safe)
            .map(|inner| Url { inner })
            .map_err(url_err)
    }

    /// Alias for the constructor.
    #[staticmethod]
    #[pyo3(signature = (value, safe = true))]
    fn from_str(value: &str, safe: bool) -> PyResult<Self> {
        Url::new(value, safe)
    }

    /// Build a :class:`Url` from a dict of components (``scheme`` and ``host``
    /// required; ``username``, ``password``, ``port``, ``path``, ``query``,
    /// ``fragment``).
    #[staticmethod]
    #[pyo3(signature = (fields, safe = true))]
    fn from_mapping(fields: Mapping, safe: bool) -> PyResult<Self> {
        CoreUrl::from_mapping(&fields, safe)
            .map(|inner| Url { inner })
            .map_err(url_err)
    }

    /// Build a :class:`Url` directly from its parts (no string parsing).
    #[staticmethod]
    #[pyo3(signature = (scheme, host, port = None, username = None, password = None, path = String::new(), query = None, fragment = None))]
    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        scheme: String,
        host: String,
        port: Option<u16>,
        username: Option<String>,
        password: Option<String>,
        path: String,
        query: Option<String>,
        fragment: Option<String>,
    ) -> Self {
        Url {
            inner: CoreUrl::from_parts(
                scheme, username, password, host, port, path, query, fragment,
            ),
        }
    }

    /// Return a copy, overriding any component passed and keeping the rest.
    /// ``copy()`` clones; ``copy(port=443)`` clones with one field changed.
    #[pyo3(signature = (scheme = None, username = None, password = None, host = None, port = None, path = None, query = None, fragment = None))]
    #[allow(clippy::too_many_arguments)]
    fn copy(
        &self,
        scheme: Option<String>,
        username: Option<String>,
        password: Option<String>,
        host: Option<String>,
        port: Option<u16>,
        path: Option<String>,
        query: Option<String>,
        fragment: Option<String>,
    ) -> Self {
        Url {
            inner: self.inner.copy(
                scheme, username, password, host, port, path, query, fragment,
            ),
        }
    }

    /// Return a copy with the scheme replaced.
    fn with_scheme(&self, scheme: String) -> Self {
        Url {
            inner: self.inner.clone().with_scheme(scheme),
        }
    }

    /// Return a copy with the username set.
    fn with_username(&self, username: String) -> Self {
        Url {
            inner: self.inner.clone().with_username(username),
        }
    }

    /// Return a copy with the password set.
    fn with_password(&self, password: String) -> Self {
        Url {
            inner: self.inner.clone().with_password(password),
        }
    }

    /// Return a copy with username and password removed.
    fn without_userinfo(&self) -> Self {
        Url {
            inner: self.inner.clone().without_userinfo(),
        }
    }

    /// Return a copy with the host replaced.
    fn with_host(&self, host: String) -> Self {
        Url {
            inner: self.inner.clone().with_host(host),
        }
    }

    /// Return a copy with the port set.
    fn with_port(&self, port: u16) -> Self {
        Url {
            inner: self.inner.clone().with_port(port),
        }
    }

    /// Return a copy with the port removed.
    fn without_port(&self) -> Self {
        Url {
            inner: self.inner.clone().without_port(),
        }
    }

    /// Return a copy with the path replaced.
    fn with_path(&self, path: String) -> Self {
        Url {
            inner: self.inner.clone().with_path(path),
        }
    }

    /// Return a copy with the query set.
    fn with_query(&self, query: String) -> Self {
        Url {
            inner: self.inner.clone().with_query(query),
        }
    }

    /// Return a copy with the query removed.
    fn without_query(&self) -> Self {
        Url {
            inner: self.inner.clone().without_query(),
        }
    }

    /// Return a copy with the fragment set.
    fn with_fragment(&self, fragment: String) -> Self {
        Url {
            inner: self.inner.clone().with_fragment(fragment),
        }
    }

    /// Return a copy with the fragment removed.
    fn without_fragment(&self) -> Self {
        Url {
            inner: self.inner.clone().without_fragment(),
        }
    }

    /// Return the query as a ``dict[str, list[str]]``; ``decode`` percent-decodes.
    #[pyo3(signature = (decode = true))]
    fn params(&self, decode: bool) -> Params {
        self.inner.params(decode)
    }

    /// Return a copy whose query is built from ``params``; ``encode`` percent-
    /// encodes each key and value.
    #[pyo3(signature = (params, encode = true))]
    fn with_params(&self, params: Params, encode: bool) -> Self {
        Url {
            inner: self.inner.clone().with_params(&params, encode),
        }
    }

    /// Return a copy with ``key`` set to ``values``, adding or replacing it.
    #[pyo3(signature = (key, values, encode = true))]
    fn add_param(&self, key: String, values: Vec<String>, encode: bool) -> Self {
        Url {
            inner: self.inner.add_param(key, values, encode),
        }
    }

    /// Base scheme before any ``+`` extension (e.g. ``https`` for ``https+zip``).
    #[getter]
    fn scheme_base(&self) -> &str {
        self.inner.scheme_base()
    }

    /// The ``+``-separated scheme extensions (e.g. ``["zip"]``).
    #[getter]
    fn scheme_ext(&self) -> Vec<&str> {
        self.inner.scheme_ext()
    }

    /// Build a :class:`Url` from a :class:`Uri` (requires an authority and host).
    #[staticmethod]
    fn from_uri(uri: &Uri) -> PyResult<Self> {
        CoreUrl::from_uri(&uri.inner)
            .map(|inner| Url { inner })
            .map_err(url_err)
    }

    /// Decoded values of one query parameter, or ``None``.
    fn get_param(&self, key: &str) -> Option<Vec<String>> {
        self.inner.get_param(key)
    }

    /// Whether the query has a parameter named ``key``.
    fn has_param(&self, key: &str) -> bool {
        self.inner.has_param(key)
    }

    fn __contains__(&self, key: &str) -> bool {
        self.inner.has_param(key)
    }

    /// ``url[key]`` -> the parameter's values (raises ``KeyError`` if absent).
    fn __getitem__(&self, key: &str) -> PyResult<Vec<String>> {
        self.inner
            .get_param(key)
            .ok_or_else(|| PyKeyError::new_err(key.to_string()))
    }

    /// ``url[key] = values`` -> set the parameter in place (percent-encoded).
    fn __setitem__(&mut self, key: String, values: Vec<String>) {
        self.inner = self.inner.set_param(key, values, true);
    }

    /// ``del url[key]`` -> remove the parameter in place.
    fn __delitem__(&mut self, key: &str) -> PyResult<()> {
        if !self.inner.has_param(key) {
            return Err(PyKeyError::new_err(key.to_string()));
        }
        self.inner = self.inner.remove_param(key, true);
        Ok(())
    }

    /// Return a copy with one parameter created or replaced (single update).
    #[pyo3(signature = (key, values, encode = true))]
    fn set_param(&self, key: String, values: Vec<String>, encode: bool) -> Self {
        Url {
            inner: self.inner.set_param(key, values, encode),
        }
    }

    /// Return a copy with every entry of ``params`` set, others untouched (bulk).
    #[pyo3(signature = (params, encode = true))]
    fn set_params(&self, params: Params, encode: bool) -> Self {
        Url {
            inner: self.inner.set_params(&params, encode),
        }
    }

    /// Return a copy with one parameter removed (single delete).
    #[pyo3(signature = (key, encode = true))]
    fn remove_param(&self, key: &str, encode: bool) -> Self {
        Url {
            inner: self.inner.remove_param(key, encode),
        }
    }

    /// Return a copy with several parameters removed (bulk delete).
    #[pyo3(signature = (keys, encode = true))]
    fn remove_params(&self, keys: Vec<String>, encode: bool) -> Self {
        Url {
            inner: self.inner.remove_params(&keys, encode),
        }
    }

    /// Return a copy with the entire query removed.
    fn clear_params(&self) -> Self {
        Url {
            inner: self.inner.clear_params(),
        }
    }

    /// Render the URL; ``encode`` (default) percent-encodes, else decodes.
    #[pyo3(signature = (encode = true))]
    fn to_string(&self, encode: bool) -> String {
        self.inner.to_str(encode)
    }

    /// Return this URL viewed as a generic :class:`Uri`.
    fn to_uri(&self) -> Uri {
        Uri {
            inner: self.inner.to_uri(),
        }
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
    /// With ``safe=False`` extra components are ignored and junk becomes ``0``.
    #[staticmethod]
    #[pyo3(signature = (value, safe = true))]
    fn from_str(value: &str, safe: bool) -> PyResult<Self> {
        CoreVersion::from_str(value, safe)
            .map(|inner| Version { inner })
            .map_err(version_err)
    }

    /// Build a :class:`Version` from a dict of components (``major``, ``minor``,
    /// ``patch``).
    #[staticmethod]
    #[pyo3(signature = (fields, safe = true))]
    fn from_mapping(fields: Mapping, safe: bool) -> PyResult<Self> {
        CoreVersion::from_mapping(&fields, safe)
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
    m.add_function(wrap_pyfunction!(py_percent_encode, m)?)?;
    m.add_function(wrap_pyfunction!(py_percent_decode, m)?)?;
    Ok(())
}
