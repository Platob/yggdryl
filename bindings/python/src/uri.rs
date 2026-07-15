//! The `yggdryl.uri` submodule — RFC 3986 URIs, absolute URLs, and authorities.
//!
//! Mirrors `yggdryl_core::io`'s root-level `Uri` / `Url` / `Authority`. A [`Uri`] is a generic
//! URI split into its components (any of which may be absent — a bare filesystem path is a
//! perfectly good `Uri`); a [`Url`] is an **absolute** URI (one that carries a scheme); an
//! [`Authority`] is the `[user[:password]@]host[:port]` component.
//!
//! Each has value semantics (equal iff their canonical strings are equal), and `Uri` /
//! `Url` round-trip through bytes (`serialize_bytes` /
//! `deserialize_bytes`, and pickle via `__reduce__`; `Authority` carries no byte codec in
//! the core, so it pickles through its four components instead).
//!
//! Paths are POSIX slash-normalized: a Windows drive path (`C:\Users\a.txt`), a UNC path
//! (`\\server\share`), or any back-slashed input has every `\` rewritten to `/` on the way
//! in. A single letter + `:` + slash is a **drive letter** kept in the path, never a
//! one-letter scheme, so examples use multi-letter schemes. Parse failures (a bad scheme,
//! an out-of-range port, a scheme-less string handed to `Url`, non-UTF-8 bytes) raise a
//! guided `ValueError`.

// `useless_conversion`: pyo3's `#[pymethods]` expansion wraps fallible returns in a
// same-type `From`. `wrong_self_convention`: the `into_url` / `into_uri` interchange keeps
// the core method names, but a binding wrapper cannot consume `self`, so it borrows.
#![allow(clippy::useless_conversion, clippy::wrong_self_convention)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use yggdryl_core::io::{self, UriError};

/// Maps a [`UriError`] to a Python `ValueError` carrying its guided text.
fn uri_err(error: UriError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// The IANA-registered default port for a well-known scheme (case-insensitive), or `None` if
/// the scheme has no registered default. Mirrors [`yggdryl_core::io::default_port`].
#[pyfunction]
fn default_port(scheme: &str) -> Option<u16> {
    io::default_port(scheme)
}

/// The constructor arguments an [`Authority`] pickles through: `(host, user, password, port)`.
type AuthorityParts = (String, Option<String>, Option<String>, Option<u16>);

/// The `[user[:password]@]host[:port]` authority component of a URI.
#[pyclass(module = "yggdryl.uri")]
#[derive(Clone)]
pub struct Authority {
    pub(crate) inner: io::Authority,
}

#[pymethods]
impl Authority {
    /// Builds an authority from its parts (`host` required; the rest optional).
    #[new]
    #[pyo3(signature = (host, user = None, password = None, port = None))]
    fn new(host: &str, user: Option<&str>, password: Option<&str>, port: Option<u16>) -> Self {
        Self {
            inner: io::Authority::new(user, password, host, port),
        }
    }

    /// Builds a bare `host`-only authority (no userinfo, no port).
    #[staticmethod]
    fn from_host(host: &str) -> Self {
        Self {
            inner: io::Authority::from_host(host),
        }
    }

    /// The userinfo user, if any.
    #[getter]
    fn user(&self) -> Option<String> {
        self.inner.user().map(str::to_string)
    }

    /// The userinfo password, if any.
    #[getter]
    fn password(&self) -> Option<String> {
        self.inner.password().map(str::to_string)
    }

    /// The host (an empty string for an empty authority such as `file:///path`; an IPv6
    /// literal keeps its brackets).
    #[getter]
    fn host(&self) -> String {
        self.inner.host().to_string()
    }

    /// Whether the host is a bracketed IPv6 literal (`"[::1]"`).
    #[getter]
    fn host_is_ipv6(&self) -> bool {
        self.inner.host_is_ipv6()
    }

    /// The host with any IPv6 brackets stripped (`"[::1]"` → `"::1"`); a reg-name/IPv4 host
    /// passes through verbatim.
    #[getter]
    fn host_unbracketed(&self) -> String {
        self.inner.host_unbracketed().to_string()
    }

    /// The port, if any.
    #[getter]
    fn port(&self) -> Option<u16> {
        self.inner.port()
    }

    /// Sets the userinfo user (pass `None` to clear it).
    #[pyo3(signature = (user = None))]
    fn set_user(&mut self, user: Option<&str>) {
        self.inner.set_user(user);
    }

    /// Sets the userinfo password (pass `None` to clear it).
    #[pyo3(signature = (password = None))]
    fn set_password(&mut self, password: Option<&str>) {
        self.inner.set_password(password);
    }

    /// Sets the host.
    fn set_host(&mut self, host: &str) {
        self.inner.set_host(host);
    }

    /// Sets the port (pass `None` to clear it).
    #[pyo3(signature = (port = None))]
    fn set_port(&mut self, port: Option<u16>) {
        self.inner.set_port(port);
    }

    /// An explicit copy of this authority.
    fn copy(&self) -> Self {
        self.clone()
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// Returns a copy with the userinfo user set (pass `None` to clear it).
    #[pyo3(signature = (user = None))]
    fn with_user(&self, user: Option<&str>) -> Self {
        Self {
            inner: self.inner.clone().with_user(user),
        }
    }

    /// Returns a copy with the userinfo password set (pass `None` to clear it).
    #[pyo3(signature = (password = None))]
    fn with_password(&self, password: Option<&str>) -> Self {
        Self {
            inner: self.inner.clone().with_password(password),
        }
    }

    /// Returns a copy with the host set.
    fn with_host(&self, host: &str) -> Self {
        Self {
            inner: self.inner.clone().with_host(host),
        }
    }

    /// Returns a copy with the port set (pass `None` to clear it).
    #[pyo3(signature = (port = None))]
    fn with_port(&self, port: Option<u16>) -> Self {
        Self {
            inner: self.inner.clone().with_port(port),
        }
    }

    /// Returns a copy overlaid by `other`: each field `other` sets wins, else this one's is kept.
    fn merge_with(&self, other: &Self) -> Self {
        Self {
            inner: self.inner.merge_with(&other.inner),
        }
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    /// Pickles through the four components (the core `Authority` has no byte codec).
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, AuthorityParts)> {
        let ctor = py.get_type_bound::<Authority>().into_any().unbind();
        Ok((
            ctor,
            (
                self.inner.host().to_string(),
                self.inner.user().map(str::to_string),
                self.inner.password().map(str::to_string),
                self.inner.port(),
            ),
        ))
    }

    /// The canonical authority string, `"[user[:password]@]host[:port]"`.
    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Authority({:?})", self.inner.to_string())
    }
}

/// A generic RFC 3986 URI split into its components, doubling as a filesystem path.
#[pyclass(module = "yggdryl.uri")]
#[derive(Clone)]
pub struct Uri {
    pub(crate) inner: io::Uri,
}

#[pymethods]
impl Uri {
    /// Parses `s` into its components, or normalizes a bare filesystem path, raising a
    /// guided `ValueError` on a malformed scheme or an out-of-range port.
    #[staticmethod]
    fn parse(s: &str) -> PyResult<Self> {
        io::Uri::parse(s)
            .map(|inner| Self { inner })
            .map_err(uri_err)
    }

    /// Builds a scheme-less, authority-less `Uri` from a filesystem path (back-slashes
    /// rewritten to forward slashes).
    #[staticmethod]
    fn from_path(path: &str) -> Self {
        Self {
            inner: io::Uri::from_path(path),
        }
    }

    /// The scheme, if any.
    #[getter]
    fn scheme(&self) -> Option<String> {
        self.inner.scheme().map(str::to_string)
    }

    /// The authority, if any.
    #[getter]
    fn authority(&self) -> Option<Authority> {
        self.inner
            .authority()
            .map(|a| Authority { inner: a.clone() })
    }

    /// The userinfo user, if any.
    #[getter]
    fn user(&self) -> Option<String> {
        self.inner.user().map(str::to_string)
    }

    /// The userinfo password, if any.
    #[getter]
    fn password(&self) -> Option<String> {
        self.inner.password().map(str::to_string)
    }

    /// The host, if this URI has an authority (an IPv6 literal keeps its brackets).
    #[getter]
    fn host(&self) -> Option<String> {
        self.inner.host().map(str::to_string)
    }

    /// Whether this URI's host is a bracketed IPv6 literal (`False` if it has no authority).
    #[getter]
    fn host_is_ipv6(&self) -> bool {
        self.inner.host_is_ipv6()
    }

    /// The host with any IPv6 brackets stripped, if this URI has an authority — the bare
    /// address to hand to a socket API.
    #[getter]
    fn host_unbracketed(&self) -> Option<String> {
        self.inner.host_unbracketed().map(str::to_string)
    }

    /// The port as written, if any (see `port_or_default` for the effective port).
    #[getter]
    fn port(&self) -> Option<u16> {
        self.inner.port()
    }

    /// The default port registered for this URI's scheme, or `None` if scheme-less or the
    /// scheme has no known default.
    #[getter]
    fn default_port(&self) -> Option<u16> {
        self.inner.default_port()
    }

    /// The effective port to connect to: the explicit `port`, else the scheme's
    /// `default_port`. `None` when neither is known. Derived on read — the URI is untouched.
    #[getter]
    fn port_or_default(&self) -> Option<u16> {
        self.inner.port_or_default()
    }

    /// The path, always POSIX slash-normalized (possibly empty).
    #[getter]
    fn path(&self) -> String {
        self.inner.path().to_string()
    }

    /// The query, if any (the text after `?`, without the `?`).
    #[getter]
    fn query(&self) -> Option<String> {
        self.inner.query().map(str::to_string)
    }

    /// The fragment, if any (the text after `#`, without the `#`).
    #[getter]
    fn fragment(&self) -> Option<String> {
        self.inner.fragment().map(str::to_string)
    }

    /// The last non-empty path segment (the filename), or `None` for a directory-like path.
    #[getter]
    fn name(&self) -> Option<String> {
        self.inner.name().map(str::to_string)
    }

    /// The filename without its last extension.
    #[getter]
    fn stem(&self) -> Option<String> {
        self.inner.stem().map(str::to_string)
    }

    /// The last extension of the filename (without the dot).
    #[getter]
    fn extension(&self) -> Option<String> {
        self.inner.extension().map(str::to_string)
    }

    /// Every extension of a multi-dot filename, outermost-last.
    #[getter]
    fn extensions(&self) -> Vec<String> {
        self.inner.extensions()
    }

    // ---- builder mutators (return a new `Uri`) -------------------------------------

    /// Returns a copy with the scheme set.
    fn with_scheme(&self, scheme: &str) -> Self {
        Self {
            inner: self.inner.clone().with_scheme(scheme),
        }
    }

    /// Returns a copy with the whole authority replaced (pass `None` to drop it).
    #[pyo3(signature = (authority = None))]
    fn with_authority(&self, authority: Option<&Authority>) -> Self {
        Self {
            inner: self
                .inner
                .clone()
                .with_authority(authority.map(|a| a.inner.clone())),
        }
    }

    /// Returns a copy with the host set (creating an authority if absent).
    fn with_host(&self, host: &str) -> Self {
        Self {
            inner: self.inner.clone().with_host(host),
        }
    }

    /// Returns a copy with the port set (creating an authority if absent).
    fn with_port(&self, port: u16) -> Self {
        Self {
            inner: self.inner.clone().with_port(port),
        }
    }

    /// Returns a copy with the userinfo user set (creating an authority if absent).
    fn with_user(&self, user: &str) -> Self {
        Self {
            inner: self.inner.clone().with_user(user),
        }
    }

    /// Returns a copy with the userinfo password set (creating an authority if absent).
    fn with_password(&self, password: &str) -> Self {
        Self {
            inner: self.inner.clone().with_password(password),
        }
    }

    /// Returns a copy with the path set, re-normalized to POSIX slashes.
    fn with_path(&self, path: &str) -> Self {
        Self {
            inner: self.inner.clone().with_path(path),
        }
    }

    /// Returns a copy with the query set.
    fn with_query(&self, query: &str) -> Self {
        Self {
            inner: self.inner.clone().with_query(query),
        }
    }

    /// Returns a copy with the fragment set.
    fn with_fragment(&self, fragment: &str) -> Self {
        Self {
            inner: self.inner.clone().with_fragment(fragment),
        }
    }

    // ---- in-place setters ----------------------------------------------------------

    /// Sets the scheme.
    fn set_scheme(&mut self, scheme: &str) {
        self.inner.set_scheme(scheme);
    }

    /// Replaces the whole authority (pass `None` to drop it).
    #[pyo3(signature = (authority = None))]
    fn set_authority(&mut self, authority: Option<&Authority>) {
        self.inner.set_authority(authority.map(|a| a.inner.clone()));
    }

    /// Sets the host, creating an authority if this URI had none.
    fn set_host(&mut self, host: &str) {
        self.inner.set_host(host);
    }

    /// Sets the port, creating an authority if this URI had none.
    fn set_port(&mut self, port: u16) {
        self.inner.set_port(port);
    }

    /// Sets the userinfo user, creating an authority if this URI had none.
    fn set_user(&mut self, user: &str) {
        self.inner.set_user(user);
    }

    /// Sets the userinfo password, creating an authority if this URI had none.
    fn set_password(&mut self, password: &str) {
        self.inner.set_password(password);
    }

    /// Sets the path, re-normalizing back-slashes to forward slashes.
    fn set_path(&mut self, path: &str) {
        self.inner.set_path(path);
    }

    /// Sets the query.
    fn set_query(&mut self, query: &str) {
        self.inner.set_query(query);
    }

    /// Sets the fragment.
    fn set_fragment(&mut self, fragment: &str) {
        self.inner.set_fragment(fragment);
    }

    // ---- byte codec + interchange --------------------------------------------------

    /// The canonical URI string as UTF-8 bytes.
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.serialize_bytes())
    }

    /// Reconstructs a URI from the bytes produced by `serialize_bytes` (the exact inverse),
    /// raising a guided `ValueError` on non-UTF-8 bytes or a parse failure.
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        io::Uri::deserialize_bytes(bytes)
            .map(|inner| Self { inner })
            .map_err(uri_err)
    }

    /// Converts to a [`Url`], raising a guided `ValueError` if this URI has no scheme.
    fn to_url(&self) -> PyResult<Url> {
        self.inner
            .to_url()
            .map(|inner| Url { inner })
            .map_err(uri_err)
    }

    /// Alias of [`to_url`](Uri::to_url) — converts to a [`Url`] (raises if scheme-less).
    fn into_url(&self) -> PyResult<Url> {
        self.to_url()
    }

    // ---- combinators (copy / joinpath / merge) -------------------------------------

    /// An explicit copy of this URI (equivalent to `copy.copy(uri)`).
    fn copy(&self) -> Self {
        self.clone()
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// Returns a copy with `path` joined lexically onto the path (one `/` at the seam, an
    /// absolute segment resets it, other components kept). Encoded like `set_path`.
    fn joinpath(&self, path: &str) -> Self {
        Self {
            inner: self.inner.joinpath(path),
        }
    }

    /// Returns a copy overlaid by `other`: each component `other` sets wins, else this URI's
    /// is kept.
    fn merge_with(&self, other: &Self) -> Self {
        Self {
            inner: self.inner.merge_with(&other.inner),
        }
    }

    // ---- query parameters (map access + CRUD) --------------------------------------

    /// The first value of query parameter `key`, **decoded** by default; pass
    /// `encoded=True` for the stored (percent-encoded) form. `None` if absent.
    #[pyo3(signature = (key, encoded = false))]
    fn query_param(&self, key: &str, encoded: bool) -> Option<String> {
        if encoded {
            self.inner.query_param(key).map(str::to_string)
        } else {
            self.inner
                .query_param_decoded(key)
                .map(|value| value.into_owned())
        }
    }

    /// Every value of query parameter `key`, in order, decoded by default
    /// (`encoded=True` for the stored form).
    #[pyo3(signature = (key, encoded = false))]
    fn query_param_all(&self, key: &str, encoded: bool) -> Vec<String> {
        if encoded {
            self.inner
                .query_param_all(key)
                .into_iter()
                .map(str::to_string)
                .collect()
        } else {
            self.inner
                .query_param_all_decoded(key)
                .into_iter()
                .map(|value| value.into_owned())
                .collect()
        }
    }

    /// All query parameters as ordered `(key, value)` pairs, decoded by default
    /// (`encoded=True` for stored) — `dict(uri.query_params())` builds a map.
    #[pyo3(signature = (encoded = false))]
    fn query_params(&self, encoded: bool) -> Vec<(String, String)> {
        if encoded {
            self.inner
                .query_params()
                .into_iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect()
        } else {
            self.inner
                .query_params_decoded()
                .into_iter()
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect()
        }
    }

    /// Whether query parameter `key` is present.
    fn has_query_param(&self, key: &str) -> bool {
        self.inner.has_query_param(key)
    }

    /// Sets query parameter `key` to `value` (first occurrence updated, later dupes dropped,
    /// or appended if absent). The value is stored verbatim.
    fn set_query_param(&mut self, key: &str, value: &str) {
        self.inner.set_query_param(key, value);
    }

    /// Returns a copy with query parameter `key` set.
    fn with_query_param(&self, key: &str, value: &str) -> Self {
        Self {
            inner: self.inner.clone().with_query_param(key, value),
        }
    }

    /// Removes every occurrence of query parameter `key`; returns whether any were removed.
    fn remove_query_param(&mut self, key: &str) -> bool {
        self.inner.remove_query_param(key)
    }

    /// Returns a copy with query parameter `key` removed.
    fn without_query_param(&self, key: &str) -> Self {
        Self {
            inner: self.inner.clone().without_query_param(key),
        }
    }

    /// Bulk-updates query parameters from `(key, value)` pairs in one pass (last value wins
    /// per key). Pass `list(mydict.items())` to apply a dict.
    fn set_query_params(&mut self, params: Vec<(String, String)>) {
        let refs: Vec<(&str, &str)> = params
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        self.inner.set_query_params(&refs);
    }

    /// Returns a copy with the bulk update applied.
    fn with_query_params(&self, params: Vec<(String, String)>) -> Self {
        let refs: Vec<(&str, &str)> = params
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        Self {
            inner: self.inner.clone().with_query_params(&refs),
        }
    }

    /// Normalizes the query: drops empty tokens and stable-sorts parameters by key.
    fn normalize_query(&mut self) {
        self.inner.normalize_query();
    }

    /// Returns a copy with the query normalized.
    fn with_normalized_query(&self) -> Self {
        Self {
            inner: self.inner.clone().with_normalized_query(),
        }
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<Uri>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    /// The canonical URI string.
    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Uri({:?})", self.inner.to_string())
    }
}

/// An **absolute** URI: a [`Uri`] guaranteed to carry a scheme.
#[pyclass(module = "yggdryl.uri")]
#[derive(Clone)]
pub struct Url {
    pub(crate) inner: io::Url,
}

#[pymethods]
impl Url {
    /// Parses `s` into an absolute URL, raising a guided `ValueError` if `s` has no scheme
    /// (or on any [`Uri.parse`](Uri::parse) failure).
    #[staticmethod]
    fn parse(s: &str) -> PyResult<Self> {
        io::Url::parse(s)
            .map(|inner| Self { inner })
            .map_err(uri_err)
    }

    /// Builds a [`Url`] from a [`Uri`], raising a guided `ValueError` if it has no scheme.
    #[staticmethod]
    fn from_uri(uri: &Uri) -> PyResult<Self> {
        uri.to_url()
    }

    /// The scheme (always present).
    #[getter]
    fn scheme(&self) -> String {
        self.inner.scheme().to_string()
    }

    /// The authority, if any.
    #[getter]
    fn authority(&self) -> Option<Authority> {
        self.inner
            .authority()
            .map(|a| Authority { inner: a.clone() })
    }

    /// The userinfo user, if any.
    #[getter]
    fn user(&self) -> Option<String> {
        self.inner.user().map(str::to_string)
    }

    /// The userinfo password, if any.
    #[getter]
    fn password(&self) -> Option<String> {
        self.inner.password().map(str::to_string)
    }

    /// The host, if this URL has an authority (an IPv6 literal keeps its brackets).
    #[getter]
    fn host(&self) -> Option<String> {
        self.inner.host().map(str::to_string)
    }

    /// Whether the host is a bracketed IPv6 literal (`False` if it has no authority).
    #[getter]
    fn host_is_ipv6(&self) -> bool {
        self.inner.host_is_ipv6()
    }

    /// The host with any IPv6 brackets stripped, if this URL has an authority.
    #[getter]
    fn host_unbracketed(&self) -> Option<String> {
        self.inner.host_unbracketed().map(str::to_string)
    }

    /// The port as written, if any (see `port_or_default` for the effective port).
    #[getter]
    fn port(&self) -> Option<u16> {
        self.inner.port()
    }

    /// The default port registered for this URL's scheme, or `None` if it has no known default.
    #[getter]
    fn default_port(&self) -> Option<u16> {
        self.inner.default_port()
    }

    /// The effective port to connect to: the explicit `port`, else the scheme's `default_port`.
    #[getter]
    fn port_or_default(&self) -> Option<u16> {
        self.inner.port_or_default()
    }

    /// The path, always POSIX slash-normalized.
    #[getter]
    fn path(&self) -> String {
        self.inner.path().to_string()
    }

    /// The query, if any.
    #[getter]
    fn query(&self) -> Option<String> {
        self.inner.query().map(str::to_string)
    }

    /// The fragment, if any.
    #[getter]
    fn fragment(&self) -> Option<String> {
        self.inner.fragment().map(str::to_string)
    }

    /// The last non-empty path segment (the filename), or `None` for a directory-like path.
    #[getter]
    fn name(&self) -> Option<String> {
        self.inner.name().map(str::to_string)
    }

    /// The filename without its last extension.
    #[getter]
    fn stem(&self) -> Option<String> {
        self.inner.stem().map(str::to_string)
    }

    /// The last extension of the filename (without the dot).
    #[getter]
    fn extension(&self) -> Option<String> {
        self.inner.extension().map(str::to_string)
    }

    /// Every extension of a multi-dot filename, outermost-last.
    #[getter]
    fn extensions(&self) -> Vec<String> {
        self.inner.extensions()
    }

    // ---- builder mutators (return a new `Url`) -------------------------------------

    /// Returns a copy with the scheme set.
    fn with_scheme(&self, scheme: &str) -> Self {
        Self {
            inner: self.inner.clone().with_scheme(scheme),
        }
    }

    /// Returns a copy with the whole authority replaced (pass `None` to drop it).
    #[pyo3(signature = (authority = None))]
    fn with_authority(&self, authority: Option<&Authority>) -> Self {
        Self {
            inner: self
                .inner
                .clone()
                .with_authority(authority.map(|a| a.inner.clone())),
        }
    }

    /// Returns a copy with the host set.
    fn with_host(&self, host: &str) -> Self {
        Self {
            inner: self.inner.clone().with_host(host),
        }
    }

    /// Returns a copy with the port set.
    fn with_port(&self, port: u16) -> Self {
        Self {
            inner: self.inner.clone().with_port(port),
        }
    }

    /// Returns a copy with the userinfo user set.
    fn with_user(&self, user: &str) -> Self {
        Self {
            inner: self.inner.clone().with_user(user),
        }
    }

    /// Returns a copy with the userinfo password set.
    fn with_password(&self, password: &str) -> Self {
        Self {
            inner: self.inner.clone().with_password(password),
        }
    }

    /// Returns a copy with the path set, re-normalized to POSIX slashes.
    fn with_path(&self, path: &str) -> Self {
        Self {
            inner: self.inner.clone().with_path(path),
        }
    }

    /// Returns a copy with the query set.
    fn with_query(&self, query: &str) -> Self {
        Self {
            inner: self.inner.clone().with_query(query),
        }
    }

    /// Returns a copy with the fragment set.
    fn with_fragment(&self, fragment: &str) -> Self {
        Self {
            inner: self.inner.clone().with_fragment(fragment),
        }
    }

    // ---- in-place setters ----------------------------------------------------------

    /// Sets the scheme.
    fn set_scheme(&mut self, scheme: &str) {
        self.inner.set_scheme(scheme);
    }

    /// Replaces the whole authority (pass `None` to drop it).
    #[pyo3(signature = (authority = None))]
    fn set_authority(&mut self, authority: Option<&Authority>) {
        self.inner.set_authority(authority.map(|a| a.inner.clone()));
    }

    /// Sets the host.
    fn set_host(&mut self, host: &str) {
        self.inner.set_host(host);
    }

    /// Sets the port.
    fn set_port(&mut self, port: u16) {
        self.inner.set_port(port);
    }

    /// Sets the userinfo user.
    fn set_user(&mut self, user: &str) {
        self.inner.set_user(user);
    }

    /// Sets the userinfo password.
    fn set_password(&mut self, password: &str) {
        self.inner.set_password(password);
    }

    /// Sets the path, re-normalizing back-slashes to forward slashes.
    fn set_path(&mut self, path: &str) {
        self.inner.set_path(path);
    }

    /// Sets the query.
    fn set_query(&mut self, query: &str) {
        self.inner.set_query(query);
    }

    /// Sets the fragment.
    fn set_fragment(&mut self, fragment: &str) {
        self.inner.set_fragment(fragment);
    }

    // ---- byte codec + interchange --------------------------------------------------

    /// The canonical URL string as UTF-8 bytes.
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.serialize_bytes())
    }

    /// Reconstructs a URL from the bytes produced by `serialize_bytes`, raising a guided
    /// `ValueError` on non-UTF-8 bytes, a missing scheme, or a parse failure.
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        io::Url::deserialize_bytes(bytes)
            .map(|inner| Self { inner })
            .map_err(uri_err)
    }

    /// The underlying [`Uri`] (infallible — a URL is always a URI).
    fn as_uri(&self) -> Uri {
        Uri {
            inner: self.inner.as_uri().clone(),
        }
    }

    /// Alias of [`as_uri`](Url::as_uri) — the underlying [`Uri`].
    fn into_uri(&self) -> Uri {
        self.as_uri()
    }

    // ---- combinators (copy / joinpath / merge) -------------------------------------

    /// An explicit copy of this URL (equivalent to `copy.copy(url)`).
    fn copy(&self) -> Self {
        self.clone()
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// Returns a copy with `path` joined lexically onto the path — see [`Uri.joinpath`]. The
    /// scheme is kept, so the result is still an absolute URL.
    fn joinpath(&self, path: &str) -> Self {
        Self {
            inner: self.inner.joinpath(path),
        }
    }

    /// Returns a copy overlaid by `other`: each component `other` sets wins, else this URL's
    /// is kept.
    fn merge_with(&self, other: &Self) -> Self {
        Self {
            inner: self.inner.merge_with(&other.inner),
        }
    }

    // ---- query parameters (map access + CRUD) --------------------------------------

    /// The first value of query parameter `key`, **decoded** by default; pass
    /// `encoded=True` for the stored (percent-encoded) form. `None` if absent.
    #[pyo3(signature = (key, encoded = false))]
    fn query_param(&self, key: &str, encoded: bool) -> Option<String> {
        if encoded {
            self.inner.query_param(key).map(str::to_string)
        } else {
            self.inner
                .query_param_decoded(key)
                .map(|value| value.into_owned())
        }
    }

    /// Every value of query parameter `key`, in order, decoded by default
    /// (`encoded=True` for the stored form).
    #[pyo3(signature = (key, encoded = false))]
    fn query_param_all(&self, key: &str, encoded: bool) -> Vec<String> {
        if encoded {
            self.inner
                .query_param_all(key)
                .into_iter()
                .map(str::to_string)
                .collect()
        } else {
            self.inner
                .query_param_all_decoded(key)
                .into_iter()
                .map(|value| value.into_owned())
                .collect()
        }
    }

    /// All query parameters as ordered `(key, value)` pairs, decoded by default
    /// (`encoded=True` for stored) — `dict(url.query_params())` builds a map.
    #[pyo3(signature = (encoded = false))]
    fn query_params(&self, encoded: bool) -> Vec<(String, String)> {
        if encoded {
            self.inner
                .query_params()
                .into_iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect()
        } else {
            self.inner
                .query_params_decoded()
                .into_iter()
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect()
        }
    }

    /// Whether query parameter `key` is present.
    fn has_query_param(&self, key: &str) -> bool {
        self.inner.has_query_param(key)
    }

    /// Sets query parameter `key` to `value` (first occurrence updated, later dupes dropped,
    /// or appended if absent). The value is stored verbatim.
    fn set_query_param(&mut self, key: &str, value: &str) {
        self.inner.set_query_param(key, value);
    }

    /// Returns a copy with query parameter `key` set.
    fn with_query_param(&self, key: &str, value: &str) -> Self {
        Self {
            inner: self.inner.clone().with_query_param(key, value),
        }
    }

    /// Removes every occurrence of query parameter `key`; returns whether any were removed.
    fn remove_query_param(&mut self, key: &str) -> bool {
        self.inner.remove_query_param(key)
    }

    /// Returns a copy with query parameter `key` removed.
    fn without_query_param(&self, key: &str) -> Self {
        Self {
            inner: self.inner.clone().without_query_param(key),
        }
    }

    /// Bulk-updates query parameters from `(key, value)` pairs in one pass (last value wins
    /// per key). Pass `list(mydict.items())` to apply a dict.
    fn set_query_params(&mut self, params: Vec<(String, String)>) {
        let refs: Vec<(&str, &str)> = params
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        self.inner.set_query_params(&refs);
    }

    /// Returns a copy with the bulk update applied.
    fn with_query_params(&self, params: Vec<(String, String)>) -> Self {
        let refs: Vec<(&str, &str)> = params
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        Self {
            inner: self.inner.clone().with_query_params(&refs),
        }
    }

    /// Normalizes the query: drops empty tokens and stable-sorts parameters by key.
    fn normalize_query(&mut self) {
        self.inner.normalize_query();
    }

    /// Returns a copy with the query normalized.
    fn with_normalized_query(&self) -> Self {
        Self {
            inner: self.inner.clone().with_normalized_query(),
        }
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<Url>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    /// The canonical URL string.
    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Url({:?})", self.inner.to_string())
    }
}

/// Populates the `uri` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Authority>()?;
    module.add_class::<Uri>()?;
    module.add_class::<Url>()?;
    module.add_function(wrap_pyfunction!(default_port, module)?)?;
    Ok(())
}
