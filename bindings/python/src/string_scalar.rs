//! Python wrapper for [`yggdryl_core::StringScalar`].

use pyo3::prelude::*;
use yggdryl_core::{Scalar, StringScalar as CoreStringScalar};

use crate::{anytype_to_py, hash_of, value_err};

/// A single UTF-8 string value, or null.
#[pyclass(module = "yggdryl", name = "StringScalar", frozen)]
#[derive(Clone)]
pub struct StringScalar {
    pub(crate) inner: CoreStringScalar,
}

#[pymethods]
impl StringScalar {
    #[new]
    #[pyo3(signature = (value = None))]
    fn new(value: Option<String>) -> Self {
        StringScalar {
            inner: match value {
                Some(text) => CoreStringScalar::new(text),
                None => CoreStringScalar::null(),
            },
        }
    }

    /// The null `string` scalar.
    #[staticmethod]
    fn null() -> Self {
        StringScalar {
            inner: CoreStringScalar::null(),
        }
    }

    /// Whether the scalar holds the null value.
    #[getter]
    fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The scalar's data type (a `Utf8` object).
    #[getter]
    fn data_type(&self, py: Python<'_>) -> PyResult<PyObject> {
        anytype_to_py(py, &self.inner.data_type())
    }

    /// The scalar's text, or `None` if null.
    #[getter]
    fn value(&self) -> Option<String> {
        self.inner.as_str().map(str::to_owned)
    }

    /// The number of UTF-8 bytes (`0` if null).
    fn __len__(&self) -> usize {
        self.inner.as_bytes().map_or(0, <[u8]>::len)
    }

    /// The JSON form.
    fn to_json(&self) -> String {
        self.inner.to_json()
    }

    /// Reconstructs a scalar from its JSON form.
    #[staticmethod]
    fn from_json(value: &str) -> PyResult<Self> {
        CoreStringScalar::from_json(value)
            .map(|inner| StringScalar { inner })
            .map_err(value_err)
    }

    fn __str__(&self) -> String {
        self.inner.as_str().unwrap_or("").to_string()
    }

    fn __repr__(&self) -> String {
        match self.inner.as_str() {
            Some(text) => format!("StringScalar({text:?})"),
            None => "StringScalar(None)".to_string(),
        }
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .extract::<StringScalar>()
            .is_ok_and(|o| self.inner == o.inner)
    }

    fn __hash__(&self) -> u64 {
        hash_of(&self.inner)
    }

    fn __getnewargs__(&self) -> (Option<String>,) {
        (self.inner.as_str().map(str::to_owned),)
    }
}
