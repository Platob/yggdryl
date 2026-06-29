//! Python wrapper for [`yggdryl_core::AnyField`].

use std::collections::BTreeMap;

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use yggdryl_core::{AnyField, DataType};

use crate::{anytype_to_py, hash_of, py_bool, py_to_anytype, value_err};

/// A named, nullable, typed field with string→string metadata.
#[pyclass(module = "yggdryl", name = "Field", frozen)]
#[derive(Clone)]
pub struct Field {
    pub(crate) inner: AnyField,
}

#[pymethods]
impl Field {
    #[new]
    #[pyo3(signature = (name, data_type, nullable = true, metadata = None))]
    fn new(
        name: String,
        data_type: &Bound<'_, PyAny>,
        nullable: bool,
        metadata: Option<BTreeMap<String, String>>,
    ) -> PyResult<Self> {
        let data_type = py_to_anytype(data_type)?;
        let mut field = AnyField::new(name, data_type, nullable);
        if let Some(metadata) = metadata {
            field = field.with_metadata(metadata);
        }
        Ok(Field { inner: field })
    }

    /// The field's name.
    #[getter]
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type (a `Binary` or `Utf8` object).
    #[getter]
    fn data_type(&self, py: Python<'_>) -> PyResult<PyObject> {
        anytype_to_py(py, self.inner.data_type())
    }

    /// Whether the field admits null values.
    #[getter]
    fn nullable(&self) -> bool {
        self.inner.is_nullable()
    }

    /// The field's metadata.
    #[getter]
    fn metadata(&self) -> BTreeMap<String, String> {
        self.inner.metadata().clone()
    }

    /// A copy with a different name.
    fn with_name(&self, name: String) -> Self {
        Field {
            inner: self.inner.with_name(name),
        }
    }

    /// A copy with a different nullability.
    fn with_nullable(&self, nullable: bool) -> Self {
        Field {
            inner: self.inner.with_nullable(nullable),
        }
    }

    /// A copy with the given metadata.
    fn with_metadata(&self, metadata: BTreeMap<String, String>) -> Self {
        Field {
            inner: self.inner.with_metadata(metadata),
        }
    }

    /// A copy with the metadata cleared.
    fn without_metadata(&self) -> Self {
        Field {
            inner: self.inner.without_metadata(),
        }
    }

    /// The component map (`name`, `type`, `nullable`, `metadata.<key>`).
    fn to_mapping(&self) -> BTreeMap<String, String> {
        self.inner.to_mapping()
    }

    /// Reconstructs a field from its component map.
    #[staticmethod]
    fn from_mapping(mapping: BTreeMap<String, String>) -> PyResult<Self> {
        AnyField::from_mapping(&mapping)
            .map(|inner| Field { inner })
            .map_err(value_err)
    }

    /// The canonical, length-prefixed byte form.
    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.to_bytes())
    }

    /// Reconstructs a field from its byte form.
    #[staticmethod]
    fn from_bytes(data: &[u8]) -> PyResult<Self> {
        AnyField::from_bytes(data)
            .map(|inner| Field { inner })
            .map_err(value_err)
    }

    /// The JSON form.
    fn to_json(&self) -> String {
        self.inner.to_json()
    }

    /// Reconstructs a field from its JSON form.
    #[staticmethod]
    fn from_json(value: &str) -> PyResult<Self> {
        AnyField::from_json(value)
            .map(|inner| Field { inner })
            .map_err(value_err)
    }

    fn __repr__(&self) -> String {
        format!(
            "Field(name={:?}, type={:?}, nullable={})",
            self.inner.name(),
            self.inner.data_type().to_str(),
            py_bool(self.inner.is_nullable()),
        )
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .extract::<Field>()
            .is_ok_and(|o| self.inner == o.inner)
    }

    fn __hash__(&self) -> u64 {
        hash_of(&self.inner)
    }

    fn __getnewargs__(
        &self,
        py: Python<'_>,
    ) -> PyResult<(String, PyObject, bool, BTreeMap<String, String>)> {
        Ok((
            self.inner.name().to_string(),
            anytype_to_py(py, self.inner.data_type())?,
            self.inner.is_nullable(),
            self.inner.metadata().clone(),
        ))
    }
}
