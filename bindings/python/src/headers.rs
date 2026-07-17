//! The `yggdryl.io` [`Headers`] map — the project's one metadata map.
//!
//! Mirrors [`yggdryl_core::headers::Headers`]: an ordered, ASCII case-insensitive, multi-value map
//! of byte-string names to byte-string values following HTTP header conventions, with `str`
//! conveniences over the byte storage. It behaves like a `dict`: `len` / `in` / `h[name]` /
//! `h[name] = value` / `del h[name]` / iteration over names — and, like `dict`, it is a
//! **mutable** map, so it is deliberately unhashable (`__eq__` without `__hash__`).
//!
//! Every method is one or two lines over `yggdryl_core`; a truncated byte frame handed to
//! `deserialize_bytes` raises a guided `ValueError` carrying the core error text unchanged.

// `useless_conversion`: pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type
// `From`.
#![allow(clippy::useless_conversion)]

use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use yggdryl_core::headers;
use yggdryl_core::io::IoError;

/// Maps an [`IoError`] to a Python `ValueError` carrying its guided text.
fn ioerr(error: IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// An ordered, case-insensitive, multi-value map of byte-string names to byte-string values —
/// HTTP headers, schema/field metadata, and source annotations all live here. `str` accessors
/// (`get` / `append` / `insert`) sit over the byte storage for the common textual case;
/// `*_bytes` accessors reach the raw bytes. Mutable like `dict`, so intentionally
/// **not** hashable.
#[pyclass(module = "yggdryl.headers")]
#[derive(Clone)]
pub struct Headers {
    pub(crate) inner: headers::Headers,
}

#[pymethods]
impl Headers {
    /// An empty header map (no allocation).
    #[new]
    fn new() -> Self {
        Self {
            inner: headers::Headers::new(),
        }
    }

    /// An empty map with room for `capacity` entries before its first reallocation.
    #[staticmethod]
    fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: headers::Headers::with_capacity(capacity),
        }
    }

    /// Parses an HTTP header block (bytes / bytearray): one `Name: Value` per line (`\r\n` or
    /// `\n`). **Lenient** — a blank line stops parsing and a line with no colon is skipped.
    #[staticmethod]
    fn parse_http(data: Vec<u8>) -> Self {
        Self {
            inner: headers::Headers::parse_http(&data),
        }
    }

    // ---- read (str + bytes) --------------------------------------------------------------

    /// The **first** value for `name` (case-insensitive), or `None` if absent or not valid
    /// UTF-8. Use [`get_bytes`](Headers::get_bytes) for the raw bytes.
    fn get(&self, name: &str) -> Option<String> {
        self.inner.get(name).map(str::to_string)
    }

    /// Every value for `name`, in insertion order (non-UTF-8 values are skipped).
    fn get_all(&self, name: &str) -> Vec<String> {
        self.inner
            .get_all(name)
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    /// The raw value of the **first** entry whose name matches `name` (case-insensitively).
    fn get_bytes<'py>(&self, py: Python<'py>, name: Vec<u8>) -> Option<Bound<'py, PyBytes>> {
        self.inner
            .get_bytes(&name)
            .map(|value| PyBytes::new_bound(py, value))
    }

    /// Every raw value for `name`, in insertion order.
    fn get_all_bytes<'py>(&self, py: Python<'py>, name: Vec<u8>) -> Vec<Bound<'py, PyBytes>> {
        self.inner
            .get_all_bytes(&name)
            .into_iter()
            .map(|value| PyBytes::new_bound(py, value))
            .collect()
    }

    /// Whether any entry matches `name` (case-insensitively).
    fn contains(&self, name: &str) -> bool {
        self.inner.contains(name)
    }

    /// The `(name, value)` entries as `bytes` pairs, in insertion order (like `dict.items()`,
    /// but multi-value: a repeated name appears once per occurrence).
    fn items<'py>(&self, py: Python<'py>) -> Vec<(Bound<'py, PyBytes>, Bound<'py, PyBytes>)> {
        self.inner
            .iter()
            .map(|(name, value)| (PyBytes::new_bound(py, name), PyBytes::new_bound(py, value)))
            .collect()
    }

    /// The entry names in insertion order (one per entry, so a repeated name appears once per
    /// occurrence; non-UTF-8 names are skipped, like the core's `str` accessors).
    fn keys(&self) -> Vec<String> {
        self.inner
            .iter()
            .filter_map(|(name, _)| std::str::from_utf8(name).ok().map(str::to_string))
            .collect()
    }

    // ---- write (str + bytes) -------------------------------------------------------------

    /// Appends a `name: value` entry, **keeping** any existing entries with the same name
    /// (multi-value append).
    fn append(&mut self, name: &str, value: &str) {
        self.inner.append(name, value);
    }

    /// [`append`](Headers::append) with raw byte-string arguments.
    fn append_bytes(&mut self, name: Vec<u8>, value: Vec<u8>) {
        self.inner.append_bytes(&name, &value);
    }

    /// **Sets** `name` to a single `value` — removes every existing entry with that name,
    /// then appends one (HTTP "replace" semantics, like `dict` assignment).
    fn insert(&mut self, name: &str, value: &str) {
        self.inner.insert(name, value);
    }

    /// [`insert`](Headers::insert) with raw byte-string arguments.
    fn insert_bytes(&mut self, name: Vec<u8>, value: Vec<u8>) {
        self.inner.insert_bytes(&name, &value);
    }

    /// A fresh map with `name` set to a single `value` — the one-line, non-mutating builder
    /// (`headers.with_("a", "1").with_("b", "2")`). Named `with_` because `with` is a Python
    /// keyword (mirrors the core's `with`).
    fn with_(&self, name: &str, value: &str) -> Self {
        Self {
            inner: self.inner.clone().with(name, value),
        }
    }

    /// Removes **every** entry matching `name` (case-insensitively); returns how many were
    /// removed.
    fn remove(&mut self, name: &str) -> usize {
        self.inner.remove(name)
    }

    /// [`remove`](Headers::remove) with a raw byte-string name — reaches entries whose name
    /// is not valid UTF-8.
    fn remove_bytes(&mut self, name: Vec<u8>) -> usize {
        self.inner.remove_bytes(&name)
    }

    /// Removes all entries.
    fn clear(&mut self) {
        self.inner.clear();
    }

    /// An explicit copy of this map (equivalent to `copy.copy(headers)`).
    fn copy(&self) -> Self {
        self.clone()
    }

    /// Returns a copy of this map overlaid by `other`: every name `other` carries **replaces**
    /// that name here (all occurrences), and names only this map carries are kept.
    fn merge_with(&self, other: &Self) -> Self {
        Self {
            inner: self.inner.merge_with(&other.inner),
        }
    }

    // ---- typed conveniences for common headers -------------------------------------------

    /// The `Content-Type` value, if present and UTF-8.
    fn content_type(&self) -> Option<String> {
        self.inner.content_type().map(str::to_string)
    }

    /// The `Content-Length` value parsed as an int, if present and numeric.
    fn content_length(&self) -> Option<u64> {
        self.inner.content_length()
    }

    // ---- HTTP text form + byte codec ------------------------------------------------------

    /// The header block in HTTP wire form — `Name: Value\r\n` per entry (no trailing blank
    /// line). The inverse of [`parse_http`](Headers::parse_http) for textual headers.
    fn to_http_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.to_http_bytes())
    }

    /// The map as a length-prefixed binary frame — unlike the HTTP text form this round-trips
    /// **arbitrary** bytes, insertion order, and multi-value entries;
    /// [`deserialize_bytes`](Headers::deserialize_bytes) is the exact inverse.
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.serialize_bytes())
    }

    /// Reconstructs a map from bytes produced by [`serialize_bytes`](Headers::serialize_bytes),
    /// raising a guided `ValueError` (naming the shortfall) if the frame is truncated.
    #[staticmethod]
    fn deserialize_bytes(data: &[u8]) -> PyResult<Self> {
        headers::Headers::deserialize_bytes(data)
            .map(|inner| Self { inner })
            .map_err(ioerr)
    }

    // ---- map protocol dunders --------------------------------------------------------------

    /// The number of entries (a repeated name counts once per occurrence).
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// Truthiness — `True` when the map has at least one entry (like `dict`).
    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    /// Membership: `name in headers` is true when any entry matches (case-insensitively).
    fn __contains__(&self, name: &str) -> bool {
        self.inner.contains(name)
    }

    /// Map access: `headers[name]` is the first value for `name`, raising `KeyError` when
    /// absent — like `dict` (`name in headers` implies `headers[name]` never raises
    /// `KeyError`). A present name whose first value is not valid UTF-8 raises a guided
    /// `ValueError` instead. Use `get(name)` for an `Optional` read.
    fn __getitem__(&self, name: &str) -> PyResult<String> {
        if !self.inner.contains(name) {
            return Err(PyKeyError::new_err(name.to_string()));
        }
        self.inner.get(name).map(str::to_string).ok_or_else(|| {
            PyValueError::new_err(format!(
                "the value of {name:?} is not valid UTF-8; use get_bytes for the raw bytes"
            ))
        })
    }

    /// Map write: `headers[name] = value` sets `name` to a single value (insert/replace,
    /// like `dict`).
    fn __setitem__(&mut self, name: &str, value: &str) {
        self.inner.insert(name, value);
    }

    /// Map delete: `del headers[name]` removes every entry matching `name`, raising
    /// `KeyError` when nothing was removed — like `dict`.
    fn __delitem__(&mut self, name: &str) -> PyResult<()> {
        if self.inner.remove(name) > 0 {
            Ok(())
        } else {
            Err(PyKeyError::new_err(name.to_string()))
        }
    }

    /// Iterates the entry names in insertion order (like `dict`; see
    /// [`keys`](Headers::keys)).
    fn __iter__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(self.keys().into_py(py).bind(py).iter()?.unbind().into_any())
    }

    // ---- value semantics -------------------------------------------------------------------

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// Pickles through the byte codec (`deserialize_bytes(serialize_bytes())`).
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<Headers>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    fn __repr__(&self) -> String {
        format!("Headers({:?})", self.inner)
    }
}
