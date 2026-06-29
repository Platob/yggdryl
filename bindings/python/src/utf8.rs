//! Python wrapper for the in-memory string value [`yggdryl_core::Utf8`].

use std::collections::BTreeMap;

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use yggdryl_core::{BinaryBased, Scalar, Utf8 as CoreUtf8, Utf8Type as CoreUtf8Type};

use crate::{anyscalar_to_py, anytype_to_py, hash_of, py_bool, py_to_anytype, value_err, Utf8Type};

/// A validated, in-memory UTF-8 string value. Equality and hashing are
/// content-based; `cast` converts to a `Binary` for byte IO.
#[pyclass(module = "yggdryl", name = "Utf8")]
#[derive(Clone)]
pub struct Utf8 {
    pub(crate) inner: CoreUtf8,
}

#[pymethods]
impl Utf8 {
    #[new]
    #[pyo3(signature = (value = "", large = false))]
    fn new(value: &str, large: bool) -> Self {
        let mut inner = CoreUtf8::from_str(value);
        if large {
            inner = inner.with_data_type(CoreUtf8Type::large());
        }
        Utf8 { inner }
    }

    /// The scalar's data type (a `Utf8Type` object).
    #[getter]
    fn data_type(&self, py: Python<'_>) -> PyResult<PyObject> {
        anytype_to_py(py, &self.inner.data_type())
    }

    /// The string value.
    #[getter]
    fn value(&self) -> String {
        self.inner.as_str().to_string()
    }

    /// Returns a copy carrying the given `string` type variant.
    fn with_data_type(&self, data_type: Utf8Type) -> Self {
        Utf8 {
            inner: self.inner.with_data_type(data_type.inner),
        }
    }

    /// The string's raw UTF-8 bytes.
    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.to_bytes())
    }

    /// A `string` value holding a copy of `data`, validating UTF-8.
    #[staticmethod]
    fn from_bytes(data: &[u8]) -> PyResult<Self> {
        CoreUtf8::from_bytes(data)
            .map(|inner| Utf8 { inner })
            .map_err(value_err)
    }

    /// The component map (`type`, plus the `value` text).
    fn to_mapping(&self) -> BTreeMap<String, String> {
        self.inner.to_mapping()
    }

    /// Reconstructs a value from its component map.
    #[staticmethod]
    fn from_mapping(mapping: BTreeMap<String, String>) -> PyResult<Self> {
        CoreUtf8::from_mapping(&mapping)
            .map(|inner| Utf8 { inner })
            .map_err(value_err)
    }

    /// The JSON form.
    fn to_json(&self) -> String {
        self.inner.to_json()
    }

    /// Reconstructs a value from its JSON form.
    #[staticmethod]
    fn from_json(value: &str) -> PyResult<Self> {
        CoreUtf8::from_json(value)
            .map(|inner| Utf8 { inner })
            .map_err(value_err)
    }

    /// Casts the value to `data_type` (a `BinaryType` or `Utf8Type`), returning a
    /// new `Binary` or `Utf8`.
    fn cast(&self, py: Python<'_>, data_type: &Bound<'_, PyAny>) -> PyResult<PyObject> {
        let data_type = py_to_anytype(data_type)?;
        let scalar = self.inner.cast(&data_type).map_err(value_err)?;
        anyscalar_to_py(py, scalar)
    }

    /// Sets the data type in place (same-family only).
    fn set_data_type(&mut self, data_type: &Bound<'_, PyAny>) -> PyResult<()> {
        let data_type = py_to_anytype(data_type)?;
        self.inner.set_data_type(&data_type).map_err(value_err)
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __str__(&self) -> String {
        self.inner.as_str().to_string()
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other.extract::<Utf8>().is_ok_and(|o| self.inner == o.inner)
    }

    fn __hash__(&self) -> u64 {
        hash_of(&self.inner)
    }

    fn __repr__(&self) -> String {
        format!(
            "Utf8({:?}, large={})",
            self.inner.as_str(),
            py_bool(self.inner.string_type().is_large()),
        )
    }

    fn __getnewargs__(&self) -> (String, bool) {
        (
            self.inner.as_str().to_string(),
            self.inner.string_type().is_large(),
        )
    }
}
