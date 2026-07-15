//! The `yggdryl.io` submodule's [`Headers`] — the **centralized** string key/value metadata
//! holder: an ordered, case-insensitive, multi-value map that backs both HTTP headers and a
//! [`Field`](crate::types::Field)'s metadata (there is no separate `Metadata` type).
//!
//! Mirrors [`Headers`](yggdryl_core::io::Headers) method-for-method — each method is one or two
//! lines over the core. Like `dict`/`bytearray` it is a **mutable** container, so (matching that
//! idiom) it defines `__eq__` but is not hashable; a `Field` that embeds it still hashes, via the
//! core's byte-canonical field hash.

// pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type `From`.
#![allow(clippy::useless_conversion)]

use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};

use yggdryl_core::io::Headers as CoreHeaders;

/// A **case-insensitive, ordered, multi-value** string map — a `Field`'s metadata *and* an HTTP
/// header block. Dict-like (`h[key]`, `key in h`, `len(h)`), with multi-value `append`/`get_all`,
/// the HTTP text form (`to_http_bytes`/`parse_http`), and a lossless byte codec
/// (`serialize_bytes`/`deserialize_bytes`). Mutable, so — like `dict` — it is not hashable.
#[pyclass(module = "yggdryl.io")]
#[derive(Clone)]
pub struct Headers {
    pub(crate) inner: CoreHeaders,
}

impl Headers {
    /// Builds core [`Headers`](CoreHeaders) from an optional `Headers` or `dict[str, str]` — the
    /// shared adapter used by the constructor and by [`Field`](crate::types::Field).
    pub(crate) fn from_py(value: Option<&Bound<'_, PyAny>>) -> PyResult<CoreHeaders> {
        match value {
            None => Ok(CoreHeaders::new()),
            Some(any) => {
                if let Ok(headers) = any.extract::<Headers>() {
                    Ok(headers.inner)
                } else if let Ok(dict) = any.downcast::<PyDict>() {
                    let mut headers = CoreHeaders::new();
                    for (key, value) in dict.iter() {
                        headers.insert(&key.extract::<String>()?, &value.extract::<String>()?);
                    }
                    Ok(headers)
                } else {
                    Err(PyValueError::new_err(
                        "headers must be a Headers or a dict[str, str]",
                    ))
                }
            }
        }
    }
}

#[pymethods]
impl Headers {
    // ---- common HTTP header names (canonical casing; matched case-insensitively) -----------
    #[classattr]
    const CONTENT_TYPE: &'static str = CoreHeaders::CONTENT_TYPE;
    #[classattr]
    const CONTENT_LENGTH: &'static str = CoreHeaders::CONTENT_LENGTH;
    #[classattr]
    const CONTENT_ENCODING: &'static str = CoreHeaders::CONTENT_ENCODING;
    #[classattr]
    const HOST: &'static str = CoreHeaders::HOST;
    #[classattr]
    const ACCEPT: &'static str = CoreHeaders::ACCEPT;
    #[classattr]
    const ACCEPT_ENCODING: &'static str = CoreHeaders::ACCEPT_ENCODING;
    #[classattr]
    const AUTHORIZATION: &'static str = CoreHeaders::AUTHORIZATION;
    #[classattr]
    const USER_AGENT: &'static str = CoreHeaders::USER_AGENT;
    #[classattr]
    const LOCATION: &'static str = CoreHeaders::LOCATION;
    #[classattr]
    const CONNECTION: &'static str = CoreHeaders::CONNECTION;
    #[classattr]
    const CACHE_CONTROL: &'static str = CoreHeaders::CACHE_CONTROL;
    #[classattr]
    const COOKIE: &'static str = CoreHeaders::COOKIE;
    #[classattr]
    const SET_COOKIE: &'static str = CoreHeaders::SET_COOKIE;

    /// An empty map, or one seeded from a `dict[str, str]` (each entry `insert`ed).
    #[new]
    #[pyo3(signature = (entries = None))]
    fn new(entries: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        Ok(Self {
            inner: Self::from_py(entries)?,
        })
    }

    /// The first value for `key` (case-insensitive), or `default` (`None`) if absent or not UTF-8.
    #[pyo3(signature = (key, default = None))]
    fn get(&self, key: &str, default: Option<String>) -> Option<String> {
        self.inner.get(key).map(str::to_string).or(default)
    }

    /// Every value for `key`, in insertion order (non-UTF-8 values skipped).
    fn get_all(&self, key: &str) -> Vec<String> {
        self.inner
            .get_all(key)
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    /// Whether any entry matches `key` (case-insensitively).
    fn contains(&self, key: &str) -> bool {
        self.inner.contains(key)
    }

    /// **Sets** `key` to a single `value` — removes any existing entries with that name first.
    fn insert(&mut self, key: &str, value: &str) {
        self.inner.insert(key, value);
    }

    /// **Appends** a `key: value` entry, keeping any existing entries with that name (multi-value).
    fn append(&mut self, key: &str, value: &str) {
        self.inner.append(key, value);
    }

    /// Removes **every** entry matching `key`; returns how many were removed.
    fn remove(&mut self, key: &str) -> usize {
        self.inner.remove(key)
    }

    /// Removes all entries.
    fn clear(&mut self) {
        self.inner.clear();
    }

    /// The keys, in insertion order (a repeated name appears once per occurrence).
    fn keys(&self) -> Vec<String> {
        self.inner
            .iter()
            .map(|(key, _)| String::from_utf8_lossy(key).into_owned())
            .collect()
    }

    /// The values, in insertion order.
    fn values(&self) -> Vec<String> {
        self.inner
            .iter()
            .map(|(_, value)| String::from_utf8_lossy(value).into_owned())
            .collect()
    }

    /// The `(key, value)` pairs, in insertion order.
    fn items(&self) -> Vec<(String, String)> {
        self.inner
            .iter()
            .map(|(key, value)| {
                (
                    String::from_utf8_lossy(key).into_owned(),
                    String::from_utf8_lossy(value).into_owned(),
                )
            })
            .collect()
    }

    /// A fresh map with `key` set to a single `value` — the one-line, non-mutating builder.
    /// (`with` is a Python keyword, so the method is `with_entry`.)
    fn with_entry(&self, key: &str, value: &str) -> Self {
        Self {
            inner: self.inner.clone().with(key, value),
        }
    }

    /// A plain `dict[str, str]` copy (a repeated name keeps its last value, per `dict`).
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new_bound(py);
        for (key, value) in self.items() {
            dict.set_item(key, value)?;
        }
        Ok(dict)
    }

    // ---- HTTP conveniences -----------------------------------------------------------------

    /// The `Content-Type` value, if present and UTF-8.
    #[getter]
    fn content_type(&self) -> Option<String> {
        self.inner.content_type().map(str::to_string)
    }

    /// The `Content-Length` value parsed as an `int`, if present and numeric.
    #[getter]
    fn content_length(&self) -> Option<u64> {
        self.inner.content_length()
    }

    /// The header block in HTTP wire form — `Name: Value\r\n` per entry.
    fn to_http_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.to_http_bytes())
    }

    /// Parses an HTTP header block (`Name: Value` per line, `\r\n` or `\n`), stopping at the
    /// blank line and skipping colon-less lines (lenient).
    #[staticmethod]
    fn parse_http(bytes: &[u8]) -> Self {
        Self {
            inner: CoreHeaders::parse_http(bytes),
        }
    }

    // ---- lossless byte codec (round-trips arbitrary bytes + multi-value) --------------------

    /// The map serialized to `bytes` — the exact inverse of `deserialize_bytes`.
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.serialize_bytes())
    }

    /// Reconstructs a map from bytes produced by `serialize_bytes`. Raises `ValueError` on a
    /// truncated frame.
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        CoreHeaders::deserialize_bytes(bytes)
            .map(|inner| Self { inner })
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    /// An explicit copy.
    fn copy(&self) -> Self {
        self.clone()
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone() // `Headers` owns its data — no shared mutable state to deep-copy
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    fn __contains__(&self, key: &str) -> bool {
        self.inner.contains(key)
    }

    /// Iterates the keys, in insertion order (like `dict`).
    fn __iter__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(PyList::new_bound(py, self.keys())
            .call_method0("__iter__")?
            .unbind())
    }

    fn __getitem__(&self, key: &str) -> PyResult<String> {
        self.inner
            .get(key)
            .map(str::to_string)
            .ok_or_else(|| PyKeyError::new_err(key.to_string()))
    }

    fn __setitem__(&mut self, key: &str, value: &str) {
        self.inner.insert(key, value);
    }

    fn __delitem__(&mut self, key: &str) -> PyResult<()> {
        if self.inner.remove(key) == 0 {
            return Err(PyKeyError::new_err(key.to_string()));
        }
        Ok(())
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    /// Pickles through `Headers.deserialize_bytes(serialize_bytes())` — lossless (multi-value and
    /// arbitrary bytes survive, unlike a `dict` view).
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<Headers>()
            .getattr("deserialize_bytes")?
            .unbind();
        let bytes = self.serialize_bytes(py).into_any().unbind();
        Ok((ctor, (bytes,)))
    }

    fn __repr__(&self) -> String {
        let entries: Vec<String> = self
            .items()
            .into_iter()
            .map(|(key, value)| format!("{key:?}: {value:?}"))
            .collect();
        format!("Headers({{{}}})", entries.join(", "))
    }
}

/// Adds [`Headers`] to the `yggdryl.io` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Headers>()?;
    Ok(())
}
