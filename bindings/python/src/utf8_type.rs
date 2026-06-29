//! Python wrapper for the string [`yggdryl_dtype::Utf8Type`] data type.

use std::collections::BTreeMap;

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use yggdryl_core::Jsonable;
use yggdryl_dtype::{BinaryBased, DataType, Utf8Type as CoreUtf8Type};

use crate::{hash_of, py_bool, value_err};

/// Arrow's variable-length UTF-8 string type (`string` / `large_string`). The
/// in-memory string *value* is `Utf8`. `from_str` also accepts the aliases
/// `"utf8"` / `"large_utf8"`.
#[pyclass(module = "yggdryl", name = "Utf8Type", frozen)]
#[derive(Clone)]
pub struct Utf8Type {
    pub(crate) inner: CoreUtf8Type,
}

#[pymethods]
impl Utf8Type {
    #[new]
    #[pyo3(signature = (large = false))]
    fn new(large: bool) -> Self {
        Utf8Type {
            inner: if large {
                CoreUtf8Type::large()
            } else {
                CoreUtf8Type::new()
            },
        }
    }

    /// The canonical type name (`"string"` or `"large_string"`).
    #[getter]
    fn name(&self) -> String {
        self.inner.type_name().to_string()
    }

    /// Whether offsets are 64-bit (`large_string`).
    #[getter]
    fn is_large(&self) -> bool {
        self.inner.is_large()
    }

    /// Whether the bytes are guaranteed UTF-8 (always `True` for strings).
    #[getter]
    fn is_utf8(&self) -> bool {
        self.inner.is_utf8()
    }

    /// The canonical string form.
    fn to_str(&self) -> String {
        self.inner.to_str().into_owned()
    }

    /// Reconstructs the type from its canonical string (accepts the aliases).
    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        CoreUtf8Type::from_str(value)
            .map(|inner| Utf8Type { inner })
            .map_err(value_err)
    }

    /// The component map `{"type": …}`.
    fn to_mapping(&self) -> BTreeMap<String, String> {
        self.inner.to_mapping()
    }

    /// Reconstructs the type from its component map.
    #[staticmethod]
    fn from_mapping(mapping: BTreeMap<String, String>) -> PyResult<Self> {
        CoreUtf8Type::from_mapping(&mapping)
            .map(|inner| Utf8Type { inner })
            .map_err(value_err)
    }

    /// The canonical byte form.
    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.to_bytes())
    }

    /// Reconstructs the type from its byte form.
    #[staticmethod]
    fn from_bytes(data: &[u8]) -> PyResult<Self> {
        CoreUtf8Type::from_bytes(data)
            .map(|inner| Utf8Type { inner })
            .map_err(value_err)
    }

    /// The JSON form.
    fn to_json(&self) -> String {
        self.inner.to_json()
    }

    /// Reconstructs the type from its JSON form.
    #[staticmethod]
    fn from_json(value: &str) -> PyResult<Self> {
        CoreUtf8Type::from_json(value)
            .map(|inner| Utf8Type { inner })
            .map_err(value_err)
    }

    /// The JSON bytes (JSON text encoded with the global charset).
    fn to_bson<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.to_bson())
    }

    /// Reconstructs the type from its JSON bytes.
    #[staticmethod]
    fn from_bson(data: &[u8]) -> PyResult<Self> {
        CoreUtf8Type::from_bson(data)
            .map(|inner| Utf8Type { inner })
            .map_err(value_err)
    }

    fn __str__(&self) -> String {
        self.inner.to_str().into_owned()
    }

    fn __repr__(&self) -> String {
        format!("Utf8Type(large={})", py_bool(self.inner.is_large()))
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .extract::<Utf8Type>()
            .is_ok_and(|o| self.inner == o.inner)
    }

    fn __hash__(&self) -> u64 {
        hash_of(&self.inner)
    }

    fn __getnewargs__(&self) -> (bool,) {
        (self.inner.is_large(),)
    }
}
