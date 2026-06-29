//! Python wrapper for [`yggdryl_core::BinaryScalar`].

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use yggdryl_core::{BinaryScalar as CoreBinaryScalar, Scalar};

use crate::{anytype_to_py, hash_of, value_err};

/// A single binary value, or null.
#[pyclass(module = "yggdryl", name = "BinaryScalar", frozen)]
#[derive(Clone)]
pub struct BinaryScalar {
    pub(crate) inner: CoreBinaryScalar,
}

#[pymethods]
impl BinaryScalar {
    #[new]
    #[pyo3(signature = (value = None))]
    fn new(value: Option<&[u8]>) -> Self {
        BinaryScalar {
            inner: match value {
                Some(bytes) => CoreBinaryScalar::new(bytes),
                None => CoreBinaryScalar::null(),
            },
        }
    }

    /// The null `binary` scalar.
    #[staticmethod]
    fn null() -> Self {
        BinaryScalar {
            inner: CoreBinaryScalar::null(),
        }
    }

    /// Whether the scalar holds the null value.
    #[getter]
    fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The scalar's data type (a `Binary` object).
    #[getter]
    fn data_type(&self, py: Python<'_>) -> PyResult<PyObject> {
        anytype_to_py(py, &self.inner.data_type())
    }

    /// The scalar's bytes, or `None` if null.
    #[getter]
    fn value<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.inner
            .as_bytes()
            .map(|bytes| PyBytes::new_bound(py, bytes))
    }

    /// The number of bytes (`0` if null).
    fn __len__(&self) -> usize {
        self.inner.len().unwrap_or(0)
    }

    /// The JSON form.
    fn to_json(&self) -> String {
        self.inner.to_json()
    }

    /// Reconstructs a scalar from its JSON form.
    #[staticmethod]
    fn from_json(value: &str) -> PyResult<Self> {
        CoreBinaryScalar::from_json(value)
            .map(|inner| BinaryScalar { inner })
            .map_err(value_err)
    }

    fn __repr__(&self) -> String {
        match self.inner.as_bytes() {
            Some(bytes) => format!("BinaryScalar({bytes:?})"),
            None => "BinaryScalar(None)".to_string(),
        }
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .extract::<BinaryScalar>()
            .is_ok_and(|o| self.inner == o.inner)
    }

    fn __hash__(&self) -> u64 {
        hash_of(&self.inner)
    }

    fn __getnewargs__(&self, py: Python<'_>) -> (PyObject,) {
        match self.inner.as_bytes() {
            Some(bytes) => (PyBytes::new_bound(py, bytes).into_any().unbind(),),
            None => (py.None(),),
        }
    }
}
