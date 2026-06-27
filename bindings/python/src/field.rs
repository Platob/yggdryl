//! The `Field` pyclass — a named, nullable :class:`DataType` graph node.

use pyo3::prelude::*;
use yggdryl_core::Mapping;
use yggdryl_schema::{Field as CoreField, MergeStrategy};

use crate::datatype::DataType;
use crate::{hash_str, schema_err};

/// A named, nullable :class:`DataType` with metadata, an optional parent (for graph
/// traversal) and child accessors. A struct-typed field is a schema.
#[pyclass(name = "Field", module = "yggdryl")]
#[derive(Clone)]
pub struct Field {
    pub(crate) inner: CoreField,
}

fn wrap(inner: CoreField) -> Field {
    Field { inner }
}

#[pymethods]
impl Field {
    /// Build from a name, :class:`DataType` and nullability.
    #[new]
    #[pyo3(signature = (name, data_type, nullable = true))]
    fn new(name: &str, data_type: &DataType, nullable: bool) -> Self {
        wrap(CoreField::new(name, data_type.inner.clone(), nullable))
    }

    /// Parse a ``"name: type"`` field string (``not null`` suffix = non-nullable).
    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        CoreField::from_str(value).map(wrap).map_err(schema_err)
    }

    /// Build from a dict (``name`` / ``type`` / ``nullable`` / ``comment``).
    #[staticmethod]
    fn from_mapping(fields: Mapping) -> PyResult<Self> {
        CoreField::from_mapping(&fields)
            .map(wrap)
            .map_err(schema_err)
    }

    /// Parse from the structural JSON of :meth:`to_json`.
    #[staticmethod]
    fn from_json(value: &str) -> PyResult<Self> {
        CoreField::from_json(value).map(wrap).map_err(schema_err)
    }

    // ---- accessors ----

    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    #[getter]
    fn data_type(&self) -> DataType {
        DataType {
            inner: self.inner.data_type().clone(),
        }
    }

    #[getter]
    fn nullable(&self) -> bool {
        self.inner.is_nullable()
    }

    /// The metadata dict.
    fn metadata(&self) -> Mapping {
        self.inner.metadata().clone()
    }

    /// The ``comment`` metadata, if any.
    #[getter]
    fn comment(&self) -> Option<&str> {
        self.inner.comment()
    }

    /// Read one metadata value.
    fn get_metadata(&self, key: &str) -> Option<String> {
        self.inner.get_metadata(key).map(str::to_string)
    }

    /// Set one metadata entry in place.
    fn set_metadata(&mut self, key: &str, value: &str) {
        self.inner.set_metadata(key, value);
    }

    /// Remove one metadata entry in place, returning the old value.
    fn remove_metadata(&mut self, key: &str) -> Option<String> {
        self.inner.remove_metadata(key)
    }

    /// Set the ``comment`` metadata in place.
    fn set_comment(&mut self, comment: &str) {
        self.inner.set_comment(comment);
    }

    // ---- builders (non-mutating) ----

    fn with_name(&self, name: &str) -> Self {
        wrap(self.inner.clone().with_name(name))
    }

    fn with_data_type(&self, data_type: &DataType) -> Self {
        wrap(self.inner.clone().with_data_type(data_type.inner.clone()))
    }

    fn with_nullable(&self, nullable: bool) -> Self {
        wrap(self.inner.clone().with_nullable(nullable))
    }

    fn with_metadata(&self, metadata: Mapping) -> Self {
        wrap(self.inner.clone().with_metadata(metadata))
    }

    fn with_metadata_entry(&self, key: &str, value: &str) -> Self {
        wrap(self.inner.clone().with_metadata_entry(key, value))
    }

    fn with_comment(&self, comment: &str) -> Self {
        wrap(self.inner.clone().with_comment(comment))
    }

    fn without_metadata(&self) -> Self {
        wrap(self.inner.clone().without_metadata())
    }

    /// A copy overriding any component passed and keeping the rest.
    #[pyo3(signature = (name = None, data_type = None, nullable = None, metadata = None))]
    fn copy(
        &self,
        name: Option<String>,
        data_type: Option<DataType>,
        nullable: Option<bool>,
        metadata: Option<Mapping>,
    ) -> Self {
        wrap(
            self.inner
                .copy(name, data_type.map(|d| d.inner), nullable, metadata),
        )
    }

    // ---- graph ----

    /// The navigational parent, if linked.
    #[getter]
    fn parent(&self) -> Option<Field> {
        self.inner.parent().cloned().map(wrap)
    }

    fn with_parent(&self, parent: &Field) -> Self {
        wrap(self.inner.clone().with_parent(parent.inner.clone()))
    }

    fn set_parent(&mut self, parent: &Field) {
        self.inner.set_parent(parent.inner.clone());
    }

    fn without_parent(&self) -> Self {
        wrap(self.inner.clone().without_parent())
    }

    /// The topmost ancestor reachable via :attr:`parent` (or ``self``).
    fn root(&self) -> Field {
        wrap(self.inner.root().clone())
    }

    /// A copy with parent links wired throughout the struct tree.
    fn with_linked_children(&self) -> Self {
        wrap(self.inner.clone().with_linked_children())
    }

    /// The child fields (empty unless this is a struct).
    fn children(&self) -> Vec<Field> {
        self.inner.children().iter().cloned().map(wrap).collect()
    }

    /// The number of child fields.
    #[getter]
    fn child_count(&self) -> usize {
        self.inner.child_count()
    }

    /// The child at `index`, if any.
    fn child_at(&self, index: usize) -> Option<Field> {
        self.inner.child_at(index).cloned().map(wrap)
    }

    /// The first child matching `name` (case-insensitive).
    fn child(&self, name: &str) -> Option<Field> {
        self.inner.child(name).cloned().map(wrap)
    }

    /// The first child matching `name` exactly (case-sensitive).
    fn child_exact(&self, name: &str) -> Option<Field> {
        self.inner.child_exact(name).cloned().map(wrap)
    }

    /// The index of the first child matching `name` (case-insensitive).
    fn child_index(&self, name: &str) -> Option<usize> {
        self.inner.child_index(name)
    }

    // ---- merge ----

    /// Merge with `other` (names must match) under a strategy.
    #[pyo3(signature = (other, strategy = "promote"))]
    fn merge(&self, other: &Field, strategy: &str) -> PyResult<Field> {
        let strategy = MergeStrategy::from_str(strategy).map_err(schema_err)?;
        self.inner
            .merge(&other.inner, strategy)
            .map(wrap)
            .map_err(schema_err)
    }

    // ---- serialisation ----

    /// Render to a dict (``name`` / ``type`` / ``nullable`` / ``comment``).
    fn to_mapping(&self) -> Mapping {
        self.inner.to_mapping()
    }

    /// Serialise to a lossless structural JSON string.
    fn to_json(&self) -> String {
        self.inner.to_json()
    }

    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, pyo3::types::PyBytes> {
        pyo3::types::PyBytes::new_bound(py, &self.inner.to_bytes())
    }

    fn __str__(&self) -> String {
        self.inner.to_str()
    }

    fn __repr__(&self) -> String {
        format!("Field('{}')", self.inner.to_str())
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        hash_str(&self.inner.to_json())
    }

    /// Reconstruct losslessly through structural JSON.
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<(PyObject, (String,))> {
        let from_json = py.get_type_bound::<Self>().getattr("from_json")?;
        Ok((from_json.into(), (self.inner.to_json(),)))
    }
}
