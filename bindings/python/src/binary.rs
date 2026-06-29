//! Python wrapper for [`yggdryl_core::Binary`].

use std::collections::BTreeMap;

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use yggdryl_core::{Binary as CoreBinary, BinaryBased, DataType};

use crate::{hash_of, py_bool, value_err};

/// Arrow's variable-length binary type (`binary` / `large_binary`).
#[pyclass(module = "yggdryl", name = "Binary", frozen)]
#[derive(Clone)]
pub struct Binary {
    pub(crate) inner: CoreBinary,
}

#[pymethods]
impl Binary {
    #[new]
    #[pyo3(signature = (large = false))]
    fn new(large: bool) -> Self {
        Binary {
            inner: if large {
                CoreBinary::large()
            } else {
                CoreBinary::new()
            },
        }
    }

    /// The canonical type name (`"binary"` or `"large_binary"`).
    #[getter]
    fn name(&self) -> String {
        self.inner.type_name().to_string()
    }

    /// Whether offsets are 64-bit (`large_binary`).
    #[getter]
    fn is_large(&self) -> bool {
        self.inner.is_large()
    }

    /// Whether the bytes are guaranteed UTF-8 (always `False` for binary).
    #[getter]
    fn is_utf8(&self) -> bool {
        self.inner.is_utf8()
    }

    /// The canonical string form.
    fn to_str(&self) -> String {
        self.inner.to_str().into_owned()
    }

    /// The component map `{"type": …}`.
    fn to_mapping(&self) -> BTreeMap<String, String> {
        self.inner.to_mapping()
    }

    /// Reconstructs the type from its component map.
    #[staticmethod]
    fn from_mapping(mapping: BTreeMap<String, String>) -> PyResult<Self> {
        CoreBinary::from_mapping(&mapping)
            .map(|inner| Binary { inner })
            .map_err(value_err)
    }

    /// The canonical byte form.
    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.to_bytes())
    }

    /// Reconstructs the type from its byte form.
    #[staticmethod]
    fn from_bytes(data: &[u8]) -> PyResult<Self> {
        CoreBinary::from_bytes(data)
            .map(|inner| Binary { inner })
            .map_err(value_err)
    }

    /// The JSON form.
    fn to_json(&self) -> String {
        self.inner.to_json()
    }

    /// Reconstructs the type from its JSON form.
    #[staticmethod]
    fn from_json(value: &str) -> PyResult<Self> {
        CoreBinary::from_json(value)
            .map(|inner| Binary { inner })
            .map_err(value_err)
    }

    fn __str__(&self) -> String {
        self.inner.to_str().into_owned()
    }

    fn __repr__(&self) -> String {
        format!("Binary(large={})", py_bool(self.inner.is_large()))
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .extract::<Binary>()
            .is_ok_and(|o| self.inner == o.inner)
    }

    fn __hash__(&self) -> u64 {
        hash_of(&self.inner)
    }

    fn __getnewargs__(&self) -> (bool,) {
        (self.inner.is_large(),)
    }
}
