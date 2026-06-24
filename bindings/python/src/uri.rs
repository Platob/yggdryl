//! The `Uri` pyclass.

use pyo3::exceptions::PyKeyError;
use pyo3::prelude::*;
use yggdryl_url::{FromInput, Mapping, Params, ToOutput, Uri as CoreUri};

use crate::url::Url;
use crate::{hash_str, uri_err, url_err};

/// A generic RFC 3986 URI: ``scheme:[//authority]path[?query][#fragment]``.
#[pyclass(name = "Uri", module = "yggdryl")]
#[derive(Clone)]
pub struct Uri {
    pub(crate) inner: CoreUri,
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

    /// Render to a component ``dict`` (the inverse of ``from_mapping``).
    fn to_mapping(&self) -> Mapping {
        self.inner.to_mapping()
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
