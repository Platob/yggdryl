//! The `Field` pyclass — a named :class:`DataType` with optional byte metadata and
//! the reserved comment / index_name / index_level accessors.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use yggdryl_schema::Field as CoreField;

use crate::datatype::DataType;

/// A named, typed schema node with optional byte-keyed metadata.
#[pyclass(name = "Field", module = "yggdryl")]
#[derive(Clone)]
pub struct Field {
    pub(crate) inner: CoreField,
}

#[pymethods]
impl Field {
    /// A field with the given `name` and `dtype`.
    #[new]
    fn new(name: String, dtype: &DataType) -> Self {
        Field {
            inner: CoreField::new(name, dtype.inner.clone()),
        }
    }

    /// The field name.
    #[getter]
    fn name(&self) -> &str {
        &self.inner.name
    }
    #[setter]
    fn set_name(&mut self, value: String) {
        self.inner.name = value;
    }

    /// The field's :class:`DataType`.
    #[getter]
    fn dtype(&self) -> DataType {
        DataType {
            inner: self.inner.dtype.clone(),
        }
    }
    #[setter]
    fn set_dtype(&mut self, value: &DataType) {
        self.inner.dtype = value.inner.clone();
    }

    // ---- metadata (bytes -> bytes) ----

    /// The byte metadata as a ``dict[bytes, bytes]``, else ``None``.
    #[getter]
    fn metadata<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyDict>> {
        let map = self.inner.metadata.as_ref()?;
        let dict = PyDict::new_bound(py);
        for (key, value) in map {
            let _ = dict.set_item(PyBytes::new_bound(py, key), PyBytes::new_bound(py, value));
        }
        Some(dict)
    }
    #[setter(metadata)]
    fn replace_metadata(&mut self, value: Option<BTreeMap<Vec<u8>, Vec<u8>>>) {
        self.inner.metadata = value.filter(|map| !map.is_empty());
    }

    // ---- reserved typed metadata (mutating setters) ----

    /// The field's comment, if any.
    #[getter]
    fn comment(&self) -> Option<String> {
        self.inner.comment()
    }
    #[setter]
    fn set_comment(&mut self, value: Option<&str>) {
        self.inner.set_comment(value);
    }

    /// The field's index name, if any.
    #[getter]
    fn index_name(&self) -> Option<String> {
        self.inner.index_name()
    }
    #[setter]
    fn set_index_name(&mut self, value: Option<&str>) {
        self.inner.set_index_name(value);
    }

    /// The field's index level (a ``u16``), if any.
    #[getter]
    fn index_level(&self) -> Option<u16> {
        self.inner.index_level()
    }
    #[setter]
    fn set_index_level(&mut self, value: Option<u16>) {
        self.inner.set_index_level(value);
    }

    // ---- dunders ----

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    fn __repr__(&self) -> String {
        format!("Field({:?}, {})", self.inner.name, self.inner.dtype.name())
    }
}
