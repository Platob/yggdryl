//! The `Url` pyclass.

use pyo3::exceptions::PyKeyError;
use pyo3::prelude::*;
use pyo3::types::PyType;
use std::collections::BTreeMap;
use yggdryl_core::{Params, Url as CoreUrl};

use crate::media::MediaType;
use crate::mime::MimeType;
use crate::uri::Uri;
use crate::{hash_str, url_err};

/// A reference accepted by :meth:`Url.join` — a path string, a list of segments,
/// or another :class:`Uri` / :class:`Url`. ``Str`` is tried first so a plain
/// string is never coerced to a one-character segment list.
#[derive(FromPyObject)]
enum JoinArg {
    Str(String),
    Segments(Vec<String>),
    Uri(Uri),
    Url(Url),
}

/// A URL: a URI that always has an authority, split into ``username``,
/// ``password``, ``host`` and ``port``.
#[pyclass(name = "Url", module = "yggdryl")]
#[derive(Clone)]
pub struct Url {
    pub(crate) inner: CoreUrl,
}

#[pymethods]
impl Url {
    /// Parse ``value`` into a :class:`Url`, raising ``ValueError`` on failure.
    #[new]
    fn new(value: &str) -> PyResult<Self> {
        CoreUrl::from_str(value)
            .map(|inner| Url { inner })
            .map_err(url_err)
    }

    /// Alias for the constructor.
    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        Url::new(value)
    }

    /// Build a :class:`Url` from a dict of components (``scheme`` and ``host``
    /// required; ``username``, ``password``, ``port``, ``path``, ``query``,
    /// ``fragment``).
    #[staticmethod]
    fn from_mapping(fields: BTreeMap<String, String>) -> PyResult<Self> {
        CoreUrl::from_mapping(&fields)
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

    /// The non-empty path segments; ``encode`` keeps the percent-encoded form.
    #[pyo3(signature = (encode = false))]
    fn parts(&self, encode: bool) -> Vec<String> {
        self.inner.parts(encode)
    }

    /// The file name (last path segment).
    #[pyo3(signature = (encode = false))]
    fn name(&self, encode: bool) -> String {
        self.inner.name(encode)
    }

    /// The file name without its extensions.
    #[pyo3(signature = (encode = false))]
    fn stem(&self, encode: bool) -> String {
        self.inner.stem(encode)
    }

    /// The file name's extensions, e.g. ``["tar", "gz"]``.
    #[pyo3(signature = (encode = false))]
    fn extensions(&self, encode: bool) -> Vec<String> {
        self.inner.extensions(encode)
    }

    /// The media type stack inferred from the path's file extensions, or ``None``.
    fn media_type(&self) -> Option<MediaType> {
        self.inner.media_type().map(|inner| MediaType { inner })
    }

    /// The outermost MIME type inferred from the path's last extension, or ``None``.
    fn mime_type(&self) -> Option<MimeType> {
        self.inner.mime_type().map(|inner| MimeType { inner })
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

    /// Join a relative reference onto the path (RFC 3986 dot-segment resolution).
    /// ``reference`` is a path string (``"a/b"``, ``"../x"``, ``"/abs"``), a list
    /// of segments (each percent-encoded and ``/``-joined), or another
    /// :class:`Uri` / :class:`Url` (its path is used). The query and fragment are
    /// dropped. The authority (userinfo / host / port) is preserved.
    fn join(&self, reference: JoinArg) -> Self {
        let inner = match reference {
            JoinArg::Str(value) => self.inner.join(value.as_str()),
            JoinArg::Segments(segments) => self.inner.join(segments.as_slice()),
            JoinArg::Uri(uri) => self.inner.join(&uri.inner),
            JoinArg::Url(url) => self.inner.join(&url.inner),
        };
        Url { inner }
    }

    /// Render the URL; ``encode`` (default) percent-encodes, else decodes.
    #[pyo3(signature = (encode = true))]
    fn to_string(&self, encode: bool) -> String {
        self.inner.to_str(encode)
    }

    /// Render to a component ``dict`` (the inverse of ``from_mapping``).
    fn to_mapping(&self) -> BTreeMap<String, String> {
        self.inner.to_mapping()
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

    /// Support ``pickle`` / ``copy`` by reconstructing from the encoded string.
    fn __reduce__<'py>(&self, py: Python<'py>) -> (Bound<'py, PyType>, (String,)) {
        (py.get_type_bound::<Self>(), (self.inner.to_string(),))
    }
}
