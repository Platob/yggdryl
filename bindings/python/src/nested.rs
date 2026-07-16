//! The `yggdryl.types` submodule's **nested (composite) layer** — `StructField` (the centralized
//! struct schema) and `StructSerie` (a nullable struct column of heterogeneous child columns),
//! mirroring `yggdryl_core::io::nested`.
//!
//! A `StructField` is a value type (hashable, pickles through its byte codec) describing an ordered,
//! named set of child fields (each a `Field` or a nested `StructField`). A `StructSerie` is a
//! struct column: its children are the crate's existing `Serie` columns (`U8Serie` … `Utf8Serie`,
//! `D32Serie` …, `NullSerie`, or a nested `StructSerie`), erased through the core's `AnySerie`. It
//! serializes to the **same canonical bytes** in every language, so a struct built here round-trips
//! byte-for-byte with the Rust core and the Node extension.

// pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type `From`.
#![allow(clippy::useless_conversion)]

use std::collections::hash_map::DefaultHasher;
use std::ffi::CString;
use std::hash::{Hash, Hasher};

use arrow_array::ffi::{from_ffi, to_ffi, FFI_ArrowArray};
use arrow_array::Array;
use arrow_schema::ffi::FFI_ArrowSchema;
use pyo3::exceptions::{PyIndexError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyCapsule};

use yggdryl_core::io::fixed::{Dec128, Dec256, Dec32, Dec64, NullSerie as CoreNullSerie};
use yggdryl_core::io::nested::{
    ListField as CoreListField, ListSerie as CoreListSerie, MapField as CoreMapField,
    MapSerie as CoreMapSerie, StructField as CoreStructField, StructSerie as CoreStructSerie,
};
use yggdryl_core::io::var::{Binary, Utf8};
use yggdryl_core::io::{boxed, AnyField, AnySerie, DataTypeId, IoError};

use crate::deccolumn::{D128Serie, D256Serie, D32Serie, D64Serie};
use crate::nullvalues::NullSerie;
use crate::types::{DataType, Field};
use crate::values::{
    F16Serie, F32Serie, F64Serie, I128Serie, I16Serie, I256Serie, I32Serie, I64Serie, I8Serie,
    I96Serie, U128Serie, U16Serie, U256Serie, U32Serie, U64Serie, U8Serie, U96Serie,
};
use crate::varvalues::{BinarySerie, Utf8Serie};

/// Maps an [`IoError`] to a Python `ValueError` carrying its guided text.
fn io_err(error: IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Names a (self-describing) erased column in place — the one-line replacement for the removed
/// `NamedSerie` carrier (the name goes straight into the column's own header).
fn named_column(mut column: Box<dyn AnySerie>, name: &str) -> Box<dyn AnySerie> {
    column.set_name(name);
    column
}

/// Maps an Arrow error to a Python `ValueError`.
fn arrow_err(error: arrow_schema::ArrowError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// A capsule name (`"arrow_schema"` / `"arrow_array"`), as the Arrow PyCapsule protocol requires.
fn capsule_name(name: &str) -> CString {
    CString::new(name).expect("a static ASCII capsule name has no interior NUL")
}

/// Boxes any yggdryl column wrapper (fixed / decimal / var / null / nested) into an erased
/// [`AnySerie`], by cloning its `inner` core column. Every wrapper shares the `inner` field and its
/// core type implements `AnySerie`, so one list of types covers them all.
fn extract_column(obj: &Bound<'_, PyAny>) -> PyResult<Box<dyn AnySerie>> {
    macro_rules! try_wrappers {
        ($($W:ty),+ $(,)?) => {
            $( if let Ok(w) = obj.extract::<PyRef<$W>>() { return Ok(boxed(w.inner.clone())); } )+
        };
    }
    try_wrappers!(
        U8Serie,
        U16Serie,
        U32Serie,
        U64Serie,
        U96Serie,
        U128Serie,
        U256Serie,
        I8Serie,
        I16Serie,
        I32Serie,
        I64Serie,
        I96Serie,
        I128Serie,
        I256Serie,
        F16Serie,
        F32Serie,
        F64Serie,
        D32Serie,
        D64Serie,
        D128Serie,
        D256Serie,
        Utf8Serie,
        BinarySerie,
        NullSerie,
        StructSerie,
        ListSerie,
        MapSerie,
    );
    Err(PyValueError::new_err(
        "expected a yggdryl column (a Serie: U8Serie … Utf8Serie, D32Serie …, NullSerie, \
         StructSerie, ListSerie, or MapSerie)",
    ))
}

/// Re-wraps an erased child column back into its concrete Python `Serie` class, keyed on its type.
fn rewrap_column(any: &(dyn AnySerie + 'static), py: Python<'_>) -> PyResult<PyObject> {
    macro_rules! fixed {
        ($t:ty, $W:ident) => {
            $W {
                inner: any.as_serie::<$t>().expect("type_id matched").clone(),
            }
            .into_py(py)
        };
    }
    macro_rules! decimal {
        ($B:ty, $W:ident) => {
            $W {
                inner: any.as_decimal::<$B>().expect("type_id matched").clone(),
            }
            .into_py(py)
        };
    }
    Ok(match any.type_id() {
        DataTypeId::U8 => fixed!(u8, U8Serie),
        DataTypeId::U16 => fixed!(u16, U16Serie),
        DataTypeId::U32 => fixed!(u32, U32Serie),
        DataTypeId::U64 => fixed!(u64, U64Serie),
        DataTypeId::U96 => fixed!(yggdryl_core::io::fixed::U96, U96Serie),
        DataTypeId::U128 => fixed!(u128, U128Serie),
        DataTypeId::U256 => fixed!(yggdryl_core::io::fixed::U256, U256Serie),
        DataTypeId::I8 => fixed!(i8, I8Serie),
        DataTypeId::I16 => fixed!(i16, I16Serie),
        DataTypeId::I32 => fixed!(i32, I32Serie),
        DataTypeId::I64 => fixed!(i64, I64Serie),
        DataTypeId::I96 => fixed!(yggdryl_core::io::fixed::I96, I96Serie),
        DataTypeId::I128 => fixed!(i128, I128Serie),
        DataTypeId::I256 => fixed!(yggdryl_core::io::fixed::I256, I256Serie),
        DataTypeId::F16 => fixed!(yggdryl_core::io::fixed::f16, F16Serie),
        DataTypeId::F32 => fixed!(f32, F32Serie),
        DataTypeId::F64 => fixed!(f64, F64Serie),
        DataTypeId::D32 => decimal!(Dec32, D32Serie),
        DataTypeId::D64 => decimal!(Dec64, D64Serie),
        DataTypeId::D128 => decimal!(Dec128, D128Serie),
        DataTypeId::D256 => decimal!(Dec256, D256Serie),
        DataTypeId::Utf8 => Utf8Serie {
            inner: any
                .as_bytes_serie::<Utf8>()
                .expect("type_id matched")
                .clone(),
        }
        .into_py(py),
        DataTypeId::Binary => BinarySerie {
            inner: any
                .as_bytes_serie::<Binary>()
                .expect("type_id matched")
                .clone(),
        }
        .into_py(py),
        DataTypeId::Null => NullSerie {
            inner: any
                .downcast_ref::<CoreNullSerie>()
                .expect("type_id matched")
                .clone(),
        }
        .into_py(py),
        DataTypeId::Struct => StructSerie {
            inner: any
                .downcast_ref::<CoreStructSerie>()
                .expect("type_id matched")
                .clone(),
        }
        .into_py(py),
        DataTypeId::List => ListSerie {
            inner: any
                .downcast_ref::<CoreListSerie>()
                .expect("type_id matched")
                .clone(),
        }
        .into_py(py),
        DataTypeId::Map => MapSerie {
            inner: any
                .downcast_ref::<CoreMapSerie>()
                .expect("type_id matched")
                .clone(),
        }
        .into_py(py),
        other => {
            return Err(PyValueError::new_err(format!(
                "a nested child of type {} has no Python column wrapper",
                other.name()
            )))
        }
    })
}

/// A `Field` (leaf) or nested `StructField` / `ListField` / `MapField` Python object → an erased
/// [`AnyField`].
fn extract_any_field(obj: &Bound<'_, PyAny>) -> PyResult<AnyField> {
    if let Ok(field) = obj.extract::<PyRef<Field>>() {
        return Ok(AnyField::leaf(field.inner.clone()));
    }
    if let Ok(field) = obj.extract::<PyRef<StructField>>() {
        return Ok(field.inner.as_any_field().clone());
    }
    if let Ok(field) = obj.extract::<PyRef<ListField>>() {
        return Ok(field.inner.as_any_field().clone());
    }
    if let Ok(field) = obj.extract::<PyRef<MapField>>() {
        return Ok(field.inner.as_any_field().clone());
    }
    Err(PyValueError::new_err(
        "expected a Field, StructField, ListField, or MapField as a child field",
    ))
}

/// An erased [`AnyField`] → its concrete Python `Field` / `StructField` / `ListField` / `MapField`
/// object (the recursion mirror of [`rewrap_column`]).
fn rewrap_field(any: &AnyField, py: Python<'_>) -> PyResult<PyObject> {
    if any.is_struct() {
        let inner = CoreStructField::from_any_field(any.clone())
            .expect("a struct AnyField rebuilds a StructField");
        Ok(StructField { inner }.into_py(py))
    } else if any.is_list() {
        let inner = CoreListField::from_any_field(any.clone())
            .expect("a list AnyField rebuilds a ListField");
        Ok(ListField { inner }.into_py(py))
    } else if any.is_map() {
        let inner =
            CoreMapField::from_any_field(any.clone()).expect("a map AnyField rebuilds a MapField");
        Ok(MapField { inner }.into_py(py))
    } else {
        let inner = any
            .as_leaf()
            .expect("a non-nested AnyField is a leaf")
            .clone();
        Ok(Field { inner }.into_py(py))
    }
}

// ---- Arrow C Data Interface (PyCapsule) helpers, shared by every nested column ---------------

/// Exports an Arrow array as the Arrow C Data Interface **(schema, array)** capsule pair, zero-copy.
fn export_c_array(py: Python<'_>, array: arrow_array::ArrayRef) -> PyResult<(PyObject, PyObject)> {
    let data = array.to_data();
    let (ffi_array, ffi_schema) = to_ffi(&data).map_err(arrow_err)?;
    let schema_capsule = PyCapsule::new_bound(py, ffi_schema, Some(capsule_name("arrow_schema")))?;
    let array_capsule = PyCapsule::new_bound(py, ffi_array, Some(capsule_name("arrow_array")))?;
    Ok((
        schema_capsule.into_any().unbind(),
        array_capsule.into_any().unbind(),
    ))
}

/// Exports an Arrow data type as the Arrow C Data Interface **schema** capsule (`"arrow_schema"`).
fn export_c_schema(py: Python<'_>, data_type: &arrow_schema::DataType) -> PyResult<PyObject> {
    let schema = FFI_ArrowSchema::try_from(data_type).map_err(arrow_err)?;
    let capsule = PyCapsule::new_bound(py, schema, Some(capsule_name("arrow_schema")))?;
    Ok(capsule.into_any().unbind())
}

/// Imports any object exposing the Arrow C Data Interface into an owned Arrow array, zero-copy —
/// the shared inverse of [`export_c_array`] every nested `from_arrow` routes through.
fn import_c_array(obj: &Bound<'_, PyAny>) -> PyResult<arrow_array::ArrayRef> {
    let pair = obj.call_method0("__arrow_c_array__")?;
    let (schema_cap, array_cap): (Bound<'_, PyAny>, Bound<'_, PyAny>) = pair.extract()?;
    let schema_cap = schema_cap.downcast::<PyCapsule>()?;
    let array_cap = array_cap.downcast::<PyCapsule>()?;
    // Move the FFI structs out of the producer's capsules, then blank the sources so the producer's
    // capsule destructors see a released struct and do not double-free.
    let (ffi_array, ffi_schema) = unsafe {
        let array_ptr = array_cap.pointer() as *mut FFI_ArrowArray;
        let schema_ptr = schema_cap.pointer() as *mut FFI_ArrowSchema;
        let array = std::ptr::read(array_ptr);
        let schema = std::ptr::read(schema_ptr);
        std::ptr::write(array_ptr, FFI_ArrowArray::empty());
        std::ptr::write(schema_ptr, FFI_ArrowSchema::empty());
        (array, schema)
    };
    // SAFETY: the FFI structs were produced by a conforming Arrow C Data Interface exporter
    // (pyarrow), and we take ownership of them above (blanking the sources).
    let data = unsafe { from_ffi(ffi_array, &ffi_schema) }.map_err(arrow_err)?;
    Ok(arrow_array::make_array(data))
}

/// The **centralized struct schema** — a name, nullability, [`Headers`](crate::headers) metadata, and
/// an ordered list of child fields (each a `Field` or nested `StructField`).
#[pyclass(module = "yggdryl.types")]
#[derive(Clone)]
pub struct StructField {
    pub(crate) inner: CoreStructField,
}

#[pymethods]
impl StructField {
    /// A struct schema from a name, its ordered child fields, and its nullability (default `True`).
    #[new]
    #[pyo3(signature = (name, fields, nullable = true))]
    fn new(name: &str, fields: &Bound<'_, PyAny>, nullable: bool) -> PyResult<Self> {
        let mut children = Vec::new();
        for field in fields.iter()? {
            children.push(extract_any_field(&field?)?);
        }
        Ok(Self {
            inner: CoreStructField::new(name, children, nullable),
        })
    }

    /// The struct's name.
    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    /// Whether the struct column admits nulls.
    #[getter]
    fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// The element type's name (`"struct"`).
    #[getter]
    fn type_name(&self) -> &'static str {
        "struct"
    }

    /// This schema's [`DataType`].
    #[getter]
    fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::Struct)
    }

    /// The number of child fields.
    #[getter]
    fn num_fields(&self) -> usize {
        self.inner.num_fields()
    }

    /// The child field at `index` as a `Field` / `StructField`; raises `IndexError` out of range.
    fn field(&self, py: Python<'_>, index: usize) -> PyResult<PyObject> {
        match self.inner.field(index) {
            Some(field) => rewrap_field(field, py),
            None => Err(PyIndexError::new_err("StructField index out of range")),
        }
    }

    /// The child field named `name`, or `None`.
    fn field_named(&self, py: Python<'_>, name: &str) -> PyResult<Option<PyObject>> {
        match self.inner.field_named(name) {
            Some(field) => rewrap_field(field, py).map(Some),
            None => Ok(None),
        }
    }

    /// The 0-based index of the child field named `name`, or `None`.
    fn index_of(&self, name: &str) -> Option<usize> {
        self.inner.index_of(name)
    }

    /// The child fields, in order, as a list of `Field` / `StructField`.
    fn fields(&self, py: Python<'_>) -> PyResult<Vec<PyObject>> {
        self.inner
            .fields()
            .iter()
            .map(|field| rewrap_field(field, py))
            .collect()
    }

    /// A copy of the struct's metadata [`Headers`](crate::headers).
    #[getter]
    fn metadata(&self) -> crate::headers::Headers {
        crate::headers::Headers {
            inner: self.inner.metadata().clone(),
        }
    }

    // ---- ergonomic immutable updates ---------------------------------------------------

    /// A fresh schema renamed to `name`.
    fn with_name(&self, name: &str) -> Self {
        Self {
            inner: self.inner.with_name(name),
        }
    }

    /// A fresh schema with `nullable` set.
    fn with_nullable(&self, nullable: bool) -> Self {
        Self {
            inner: self.inner.with_nullable(nullable),
        }
    }

    /// A fresh schema with one more child field appended.
    fn with_field(&self, field: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.with_field(extract_any_field(field)?),
        })
    }

    /// A fresh schema with the given metadata (`Headers` or `dict[str, str]`) attached.
    fn with_metadata(&self, metadata: &Bound<'_, PyAny>) -> PyResult<Self> {
        let meta = crate::headers::Headers::from_py(Some(metadata))?;
        Ok(Self {
            inner: self.inner.with_metadata(meta),
        })
    }

    /// A fresh schema with one extra `key = value` metadata entry.
    fn with_metadata_entry(&self, key: &str, value: &str) -> Self {
        Self {
            inner: self.inner.with_metadata_entry(key, value),
        }
    }

    /// This schema's canonical bytes (schema tree codec, Arrow-independent).
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.as_any_field().serialize_bytes())
    }

    /// Reconstructs a schema from [`serialize_bytes`](Self::serialize_bytes).
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        let field = AnyField::deserialize_bytes(bytes).map_err(io_err)?;
        CoreStructField::from_any_field(field)
            .map(|inner| Self { inner })
            .ok_or_else(|| PyValueError::new_err("the bytes did not decode to a struct field"))
    }

    fn __len__(&self) -> usize {
        self.inner.num_fields()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    /// An explicit copy.
    fn copy(&self) -> Self {
        self.clone()
    }
    fn __copy__(&self) -> Self {
        self.clone()
    }
    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// Pickles through `deserialize_bytes`.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<StructField>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.as_any_field().serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    fn __repr__(&self) -> String {
        format!(
            "StructField(name={:?}, num_fields={}, nullable={})",
            self.inner.name(),
            self.inner.num_fields(),
            self.inner.nullable()
        )
    }
}

/// A **nullable struct column** — one child column per field (all the same length), an ordered
/// schema, and an optional top-level validity mask (a null struct row).
#[pyclass(module = "yggdryl.types")]
#[derive(Clone)]
pub struct StructSerie {
    pub(crate) inner: CoreStructSerie,
}

#[pymethods]
impl StructSerie {
    /// A struct column from `(name, column)` pairs — each `column` is a yggdryl `Serie`. The schema
    /// is inferred from each column's type. Raises `ValueError` if the columns differ in length.
    #[new]
    fn new(columns: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut names = Vec::new();
        let mut cols = Vec::new();
        for pair in columns.iter()? {
            let pair = pair?;
            let (name, column): (String, Bound<'_, PyAny>) = pair.extract()?;
            names.push(name);
            cols.push(extract_column(&column)?);
        }
        let named: Vec<(&str, Box<dyn AnySerie>)> =
            names.iter().map(String::as_str).zip(cols).collect();
        CoreStructSerie::from_named(named)
            .map(|inner| Self { inner })
            .map_err(io_err)
    }

    /// A struct column from `(name, column)` pairs — the self-describing builder mirroring the core's
    /// [`StructSerie::from_series`](yggdryl_core::io::nested::StructSerie::from_series): each `column`
    /// is named in its own header before storage. It is functionally identical to the constructor; it
    /// names the core factory so the three languages read alike. Raises `ValueError` if the columns
    /// differ in length.
    #[staticmethod]
    fn from_series(columns: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut named = Vec::new();
        for pair in columns.iter()? {
            let pair = pair?;
            let (name, column): (String, Bound<'_, PyAny>) = pair.extract()?;
            named.push(named_column(extract_column(&column)?, &name));
        }
        CoreStructSerie::from_series(named)
            .map(|inner| Self { inner })
            .map_err(io_err)
    }

    /// The number of child columns (fields).
    #[getter]
    fn num_columns(&self) -> usize {
        self.inner.num_columns()
    }

    /// The number of null struct rows.
    #[getter]
    fn null_count(&self) -> usize {
        CoreStructSerie::null_count(&self.inner)
    }

    /// Whether any struct row is null.
    #[getter]
    fn has_nulls(&self) -> bool {
        self.inner.has_nulls()
    }

    /// Whether the column has no rows.
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// This column's [`DataType`].
    #[getter]
    fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::Struct)
    }

    /// A [`StructField`] naming this struct column (nullability inferred from its null rows).
    fn to_field(&self, name: &str) -> StructField {
        StructField {
            inner: self.inner.to_field(name),
        }
    }

    /// The child field at `index` as a `Field` / `StructField`; raises `IndexError` out of range.
    fn field(&self, py: Python<'_>, index: usize) -> PyResult<PyObject> {
        match self.inner.field(index) {
            Some(field) => rewrap_field(&field, py),
            None => Err(PyIndexError::new_err(
                "StructSerie field index out of range",
            )),
        }
    }

    /// The child column at `index` as its concrete `Serie`; raises `IndexError` out of range.
    fn column(&self, py: Python<'_>, index: usize) -> PyResult<PyObject> {
        match self.inner.column(index) {
            Some(column) => rewrap_column(column, py),
            None => Err(PyIndexError::new_err(
                "StructSerie column index out of range",
            )),
        }
    }

    /// The child column named `name` as its concrete `Serie`, or `None`.
    fn column_named(&self, py: Python<'_>, name: &str) -> PyResult<Option<PyObject>> {
        match self.inner.column_named(name) {
            Some(column) => rewrap_column(column, py).map(Some),
            None => Ok(None),
        }
    }

    /// The column's canonical bytes — a self-contained `[schema][len][validity?][children]` frame,
    /// identical across Rust / Python / Node.
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.serialize_bytes())
    }

    /// Reconstructs a struct column from [`serialize_bytes`](Self::serialize_bytes).
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        CoreStructSerie::deserialize_bytes(bytes)
            .map(|inner| Self { inner })
            .map_err(io_err)
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    /// The child columns, in order, as a list of concrete `Serie`.
    fn columns(&self, py: Python<'_>) -> PyResult<Vec<PyObject>> {
        (0..self.inner.num_columns())
            .map(|index| rewrap_column(self.inner.column(index).expect("in range"), py))
            .collect()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    /// An explicit copy.
    fn copy(&self) -> Self {
        self.clone()
    }
    fn __copy__(&self) -> Self {
        self.clone()
    }
    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// Pickles through `deserialize_bytes`.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<StructSerie>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    fn __repr__(&self) -> String {
        format!(
            "StructSerie(len={}, num_columns={}, null_count={})",
            self.inner.len(),
            self.inner.num_columns(),
            CoreStructSerie::null_count(&self.inner)
        )
    }
}

/// The **zero-copy Arrow C Data Interface** (PyCapsule protocol) for a struct column, so pyarrow
/// imports it with no payload copy — `pyarrow.array(table)` / `pyarrow.record_batch(table)` and
/// back via [`from_arrow`](StructSerie::from_arrow).
#[pymethods]
impl StructSerie {
    /// The Arrow C Data Interface **schema** capsule (`"arrow_schema"`).
    fn __arrow_c_schema__(&self, py: Python<'_>) -> PyResult<PyObject> {
        let array = self.inner.to_arrow_array().map_err(io_err)?;
        export_c_schema(py, array.data_type())
    }

    /// The Arrow C Data Interface **(schema, array)** capsule pair, exported zero-copy. The
    /// `requested_schema` hint is accepted for protocol compatibility and ignored (this column
    /// always exports its native schema).
    #[pyo3(signature = (requested_schema = None))]
    fn __arrow_c_array__(
        &self,
        py: Python<'_>,
        requested_schema: Option<PyObject>,
    ) -> PyResult<(PyObject, PyObject)> {
        let _ = requested_schema;
        export_c_array(
            py,
            std::sync::Arc::new(self.inner.to_arrow_array().map_err(io_err)?),
        )
    }

    /// Imports any object exposing the Arrow C Data Interface (a pyarrow `StructArray` /
    /// `RecordBatch`) into a struct column, zero-copy — the inverse of `__arrow_c_array__`.
    #[staticmethod]
    fn from_arrow(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
        let array = import_c_array(obj)?;
        let struct_array = array
            .as_any()
            .downcast_ref::<arrow_array::StructArray>()
            .ok_or_else(|| PyValueError::new_err("the imported Arrow array is not a struct"))?;
        let nullable = struct_array.null_count() > 0;
        let field = arrow_schema::Field::new("", struct_array.data_type().clone(), nullable);
        CoreStructSerie::from_arrow_array(struct_array, &field)
            .map(|inner| Self { inner })
            .map_err(io_err)
    }
}

// =====================================================================================
// List family: ListField (the centralized list schema) + ListSerie (a nullable list column).
// =====================================================================================

/// The **centralized list schema** — a name, nullability, [`Headers`](crate::headers) metadata, and a
/// single element (item) field (itself a `Field` / `StructField` / `ListField` / `MapField`).
#[pyclass(module = "yggdryl.types")]
#[derive(Clone)]
pub struct ListField {
    pub(crate) inner: CoreListField,
}

#[pymethods]
impl ListField {
    /// A list schema from a name, its element (item) field, and its nullability (default `True`).
    #[new]
    #[pyo3(signature = (name, item, nullable = true))]
    fn new(name: &str, item: &Bound<'_, PyAny>, nullable: bool) -> PyResult<Self> {
        Ok(Self {
            inner: CoreListField::new(name, extract_any_field(item)?, nullable),
        })
    }

    /// The list's name.
    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    /// Whether the list column admits nulls.
    #[getter]
    fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// The element type's name (`"list"`).
    #[getter]
    fn type_name(&self) -> &'static str {
        "list"
    }

    /// This schema's [`DataType`].
    #[getter]
    fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::List)
    }

    /// The element (item) field as a `Field` / `StructField` / `ListField` / `MapField`.
    #[getter]
    fn item(&self, py: Python<'_>) -> PyResult<PyObject> {
        rewrap_field(self.inner.item(), py)
    }

    /// A copy of the list's metadata [`Headers`](crate::headers).
    #[getter]
    fn metadata(&self) -> crate::headers::Headers {
        crate::headers::Headers {
            inner: self.inner.metadata().clone(),
        }
    }

    // ---- ergonomic immutable updates ---------------------------------------------------

    /// A fresh schema renamed to `name`.
    fn with_name(&self, name: &str) -> Self {
        Self {
            inner: self.inner.with_name(name),
        }
    }

    /// A fresh schema with `nullable` set.
    fn with_nullable(&self, nullable: bool) -> Self {
        Self {
            inner: self.inner.with_nullable(nullable),
        }
    }

    /// A fresh schema with a new element (item) field.
    fn with_item(&self, item: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.with_item(extract_any_field(item)?),
        })
    }

    /// A fresh schema with the given metadata (`Headers` or `dict[str, str]`) attached.
    fn with_metadata(&self, metadata: &Bound<'_, PyAny>) -> PyResult<Self> {
        let meta = crate::headers::Headers::from_py(Some(metadata))?;
        Ok(Self {
            inner: self.inner.with_metadata(meta),
        })
    }

    /// A fresh schema with one extra `key = value` metadata entry.
    fn with_metadata_entry(&self, key: &str, value: &str) -> Self {
        Self {
            inner: self.inner.with_metadata_entry(key, value),
        }
    }

    /// This schema's canonical bytes (schema tree codec, Arrow-independent).
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.as_any_field().serialize_bytes())
    }

    /// Reconstructs a schema from [`serialize_bytes`](Self::serialize_bytes).
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        let field = AnyField::deserialize_bytes(bytes).map_err(io_err)?;
        CoreListField::from_any_field(field)
            .map(|inner| Self { inner })
            .ok_or_else(|| PyValueError::new_err("the bytes did not decode to a list field"))
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    /// An explicit copy.
    fn copy(&self) -> Self {
        self.clone()
    }
    fn __copy__(&self) -> Self {
        self.clone()
    }
    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// Pickles through `deserialize_bytes`.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<ListField>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.as_any_field().serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    fn __repr__(&self) -> String {
        format!(
            "ListField(name={:?}, nullable={})",
            self.inner.name(),
            self.inner.nullable()
        )
    }
}

/// A **nullable list column** — `i32` offsets over one flattened child column (itself any yggdryl
/// `Serie`), plus an optional top-level validity mask. Row `i` is the child sub-range
/// `child[offsets[i] .. offsets[i + 1]]`.
#[pyclass(module = "yggdryl.types")]
#[derive(Clone)]
pub struct ListSerie {
    pub(crate) inner: CoreListSerie,
}

#[pymethods]
impl ListSerie {
    /// A list column from a flattened child `item` column, `offsets` (`len + 1` entries into the
    /// child), an optional per-row `present` mask (`present[i] == False` marks row `i` a null list),
    /// and the item field name (default `"item"`). Raises `ValueError` on invalid offsets.
    #[new]
    #[pyo3(signature = (item, offsets, present = None, item_name = "item"))]
    fn new(
        item: &Bound<'_, PyAny>,
        offsets: Vec<i32>,
        present: Option<Vec<bool>>,
        item_name: &str,
    ) -> PyResult<Self> {
        let items = named_column(extract_column(item)?, item_name);
        CoreListSerie::from_values(items, &offsets, present.as_deref())
            .map(|inner| Self { inner })
            .map_err(io_err)
    }

    /// The number of null list rows.
    #[getter]
    fn null_count(&self) -> usize {
        self.inner.null_count()
    }

    /// Whether any list row is null.
    #[getter]
    fn has_nulls(&self) -> bool {
        self.inner.has_nulls()
    }

    /// Whether the column has no rows.
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// This column's [`DataType`].
    #[getter]
    fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::List)
    }

    /// The flattened child column, as its concrete `Serie`.
    #[getter]
    fn values(&self, py: Python<'_>) -> PyResult<PyObject> {
        rewrap_column(self.inner.values(), py)
    }

    /// The row offsets (`len + 1` entries into the flattened child).
    #[getter]
    fn offsets(&self) -> Vec<i32> {
        self.inner.offsets().to_vec()
    }

    /// The element (item) field as a `Field` / `StructField` / `ListField` / `MapField`.
    #[getter]
    fn item_field(&self, py: Python<'_>) -> PyResult<PyObject> {
        rewrap_field(&self.inner.item_field(), py)
    }

    /// The row at `index` as its element sub-`Serie`, or `None` if the row is null; raises
    /// `IndexError` out of range.
    fn row(&self, py: Python<'_>, index: usize) -> PyResult<Option<PyObject>> {
        if index >= self.inner.len() {
            return Err(PyIndexError::new_err("ListSerie row index out of range"));
        }
        let scalar = self.inner.row_scalar(index);
        if scalar.is_null() {
            return Ok(None);
        }
        rewrap_column(scalar.items(), py).map(Some)
    }

    /// A [`ListField`] naming this list column (nullability inferred from its null rows).
    fn to_field(&self, name: &str) -> ListField {
        ListField {
            inner: self.inner.to_field(name),
        }
    }

    /// The column's canonical bytes — a self-contained `[schema][len][validity?][offsets][child]`
    /// frame, identical across Rust / Python / Node.
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.serialize_bytes())
    }

    /// Reconstructs a list column from [`serialize_bytes`](Self::serialize_bytes).
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        CoreListSerie::deserialize_bytes(bytes)
            .map(|inner| Self { inner })
            .map_err(io_err)
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    /// An explicit copy.
    fn copy(&self) -> Self {
        self.clone()
    }
    fn __copy__(&self) -> Self {
        self.clone()
    }
    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// Pickles through `deserialize_bytes`.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<ListSerie>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    fn __repr__(&self) -> String {
        format!(
            "ListSerie(len={}, null_count={})",
            self.inner.len(),
            self.inner.null_count()
        )
    }
}

/// The **zero-copy Arrow C Data Interface** (PyCapsule protocol) for a list column, so pyarrow imports
/// it with no payload copy — `pyarrow.array(list_serie)` → a `ListArray` and back via
/// [`from_arrow`](ListSerie::from_arrow).
#[pymethods]
impl ListSerie {
    /// The Arrow C Data Interface **schema** capsule (`"arrow_schema"`).
    fn __arrow_c_schema__(&self, py: Python<'_>) -> PyResult<PyObject> {
        let array = self.inner.to_arrow_array().map_err(io_err)?;
        export_c_schema(py, array.data_type())
    }

    /// The Arrow C Data Interface **(schema, array)** capsule pair, exported zero-copy. The
    /// `requested_schema` hint is accepted for protocol compatibility and ignored.
    #[pyo3(signature = (requested_schema = None))]
    fn __arrow_c_array__(
        &self,
        py: Python<'_>,
        requested_schema: Option<PyObject>,
    ) -> PyResult<(PyObject, PyObject)> {
        let _ = requested_schema;
        export_c_array(
            py,
            std::sync::Arc::new(self.inner.to_arrow_array().map_err(io_err)?),
        )
    }

    /// Imports any object exposing the Arrow C Data Interface (a pyarrow `ListArray`) into a list
    /// column, zero-copy — the inverse of `__arrow_c_array__`.
    #[staticmethod]
    fn from_arrow(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
        let array = import_c_array(obj)?;
        let nullable = array.null_count() > 0;
        let field = arrow_schema::Field::new("", array.data_type().clone(), nullable);
        CoreListSerie::from_arrow_array(array.as_ref(), &field)
            .map(|inner| Self { inner })
            .map_err(io_err)
    }
}

// =====================================================================================
// Map family: MapField (the centralized map schema) + MapSerie (a nullable map column).
// =====================================================================================

/// The **centralized map schema** — a name, nullability, [`Headers`](crate::headers) metadata, a
/// `keys_sorted` flag, and the `key` / `value` fields (each a `Field` / `StructField` / `ListField` /
/// `MapField`).
#[pyclass(module = "yggdryl.types")]
#[derive(Clone)]
pub struct MapField {
    pub(crate) inner: CoreMapField,
}

#[pymethods]
impl MapField {
    /// A map schema from a name, its `key` and `value` fields, its nullability (default `True`), and
    /// whether the entries are sorted by key (default `False`).
    #[new]
    #[pyo3(signature = (name, key, value, nullable = true, keys_sorted = false))]
    fn new(
        name: &str,
        key: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
        nullable: bool,
        keys_sorted: bool,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: CoreMapField::new(
                name,
                extract_any_field(key)?,
                extract_any_field(value)?,
                nullable,
                keys_sorted,
            ),
        })
    }

    /// The map's name.
    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    /// Whether the map column admits nulls.
    #[getter]
    fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// The element type's name (`"map"`).
    #[getter]
    fn type_name(&self) -> &'static str {
        "map"
    }

    /// This schema's [`DataType`].
    #[getter]
    fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::Map)
    }

    /// The key field as a `Field` / `StructField` / `ListField` / `MapField`.
    #[getter]
    fn key(&self, py: Python<'_>) -> PyResult<PyObject> {
        rewrap_field(self.inner.key(), py)
    }

    /// The value field as a `Field` / `StructField` / `ListField` / `MapField`.
    #[getter]
    fn value(&self, py: Python<'_>) -> PyResult<PyObject> {
        rewrap_field(self.inner.value(), py)
    }

    /// Whether the entries are sorted by key.
    #[getter]
    fn keys_sorted(&self) -> bool {
        self.inner.keys_sorted()
    }

    /// A copy of the map's metadata [`Headers`](crate::headers).
    #[getter]
    fn metadata(&self) -> crate::headers::Headers {
        crate::headers::Headers {
            inner: self.inner.metadata().clone(),
        }
    }

    // ---- ergonomic immutable updates ---------------------------------------------------

    /// A fresh schema renamed to `name`.
    fn with_name(&self, name: &str) -> Self {
        Self {
            inner: self.inner.with_name(name),
        }
    }

    /// A fresh schema with `nullable` set.
    fn with_nullable(&self, nullable: bool) -> Self {
        Self {
            inner: self.inner.with_nullable(nullable),
        }
    }

    /// A fresh schema with the `keys_sorted` flag set.
    fn with_keys_sorted(&self, keys_sorted: bool) -> Self {
        Self {
            inner: self.inner.with_keys_sorted(keys_sorted),
        }
    }

    /// A fresh schema with the given metadata (`Headers` or `dict[str, str]`) attached.
    fn with_metadata(&self, metadata: &Bound<'_, PyAny>) -> PyResult<Self> {
        let meta = crate::headers::Headers::from_py(Some(metadata))?;
        Ok(Self {
            inner: self.inner.with_metadata(meta),
        })
    }

    /// A fresh schema with one extra `key = value` metadata entry.
    fn with_metadata_entry(&self, key: &str, value: &str) -> Self {
        Self {
            inner: self.inner.with_metadata_entry(key, value),
        }
    }

    /// This schema's canonical bytes (schema tree codec, Arrow-independent).
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.as_any_field().serialize_bytes())
    }

    /// Reconstructs a schema from [`serialize_bytes`](Self::serialize_bytes).
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        let field = AnyField::deserialize_bytes(bytes).map_err(io_err)?;
        CoreMapField::from_any_field(field)
            .map(|inner| Self { inner })
            .ok_or_else(|| PyValueError::new_err("the bytes did not decode to a map field"))
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    /// An explicit copy.
    fn copy(&self) -> Self {
        self.clone()
    }
    fn __copy__(&self) -> Self {
        self.clone()
    }
    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// Pickles through `deserialize_bytes`.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<MapField>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.as_any_field().serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    fn __repr__(&self) -> String {
        format!(
            "MapField(name={:?}, nullable={}, keys_sorted={})",
            self.inner.name(),
            self.inner.nullable(),
            self.inner.keys_sorted()
        )
    }
}

/// A **nullable map column** — the optimized alias of `List<Struct<{key, value}>>`: `i32` offsets over
/// a flattened `key` column and `value` column, plus an optional top-level validity mask and a
/// `keys_sorted` flag. Row `i` is the entries `key[j] -> value[j]` for `j` in `[offsets[i],
/// offsets[i + 1])`.
#[pyclass(module = "yggdryl.types")]
#[derive(Clone)]
pub struct MapSerie {
    pub(crate) inner: CoreMapSerie,
}

#[pymethods]
impl MapSerie {
    /// A map column from a flattened `keys` column, a `values` column, `offsets` (`len + 1` entries
    /// into the entries), an optional per-row `present` mask, whether the entries are sorted by key,
    /// and the key/value field names (default `"key"` / `"value"`). A map key is never null, so `keys`
    /// must not contain nulls. Raises `ValueError` on a null key, invalid offsets, or length mismatch.
    #[new]
    #[pyo3(signature = (keys, values, offsets, present = None, keys_sorted = false, key_name = "key", value_name = "value"))]
    fn new(
        keys: &Bound<'_, PyAny>,
        values: &Bound<'_, PyAny>,
        offsets: Vec<i32>,
        present: Option<Vec<bool>>,
        keys_sorted: bool,
        key_name: &str,
        value_name: &str,
    ) -> PyResult<Self> {
        let keys = named_column(extract_column(keys)?, key_name);
        let values = named_column(extract_column(values)?, value_name);
        CoreMapSerie::from_entries(keys, values, &offsets, present.as_deref(), keys_sorted)
            .map(|inner| Self { inner })
            .map_err(io_err)
    }

    /// The number of null map rows.
    #[getter]
    fn null_count(&self) -> usize {
        self.inner.null_count()
    }

    /// Whether any map row is null.
    #[getter]
    fn has_nulls(&self) -> bool {
        self.inner.has_nulls()
    }

    /// Whether the column has no rows.
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Whether the entries are sorted by key.
    #[getter]
    fn keys_sorted(&self) -> bool {
        self.inner.keys_sorted()
    }

    /// This column's [`DataType`].
    #[getter]
    fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::Map)
    }

    /// The flattened key column, as its concrete `Serie`.
    #[getter]
    fn keys(&self, py: Python<'_>) -> PyResult<PyObject> {
        rewrap_column(self.inner.keys(), py)
    }

    /// The flattened value column, as its concrete `Serie`.
    #[getter]
    fn values(&self, py: Python<'_>) -> PyResult<PyObject> {
        rewrap_column(self.inner.values(), py)
    }

    /// The key field as a `Field` / `StructField` / `ListField` / `MapField`.
    #[getter]
    fn key_field(&self, py: Python<'_>) -> PyResult<PyObject> {
        rewrap_field(&self.inner.key_field(), py)
    }

    /// The value field as a `Field` / `StructField` / `ListField` / `MapField`.
    #[getter]
    fn value_field(&self, py: Python<'_>) -> PyResult<PyObject> {
        rewrap_field(&self.inner.value_field(), py)
    }

    /// The row offsets (`len + 1` entries into the flattened entries).
    #[getter]
    fn offsets(&self) -> Vec<i32> {
        self.inner.offsets().to_vec()
    }

    /// The value mapped to `key` in row `row`, or `None` if the row is null / out of range or the key
    /// is absent. The `key` is a single-element yggdryl `Serie` of the key type (its first element is
    /// the probe); the result crosses as a one-element `Serie` of the value type. Delegates the lookup
    /// to the core [`MapSerie::get_value`](yggdryl_core::io::nested::MapSerie::get_value).
    fn get_value(
        &self,
        py: Python<'_>,
        row: usize,
        key: &Bound<'_, PyAny>,
    ) -> PyResult<Option<PyObject>> {
        let probe_col = extract_column(key)?;
        if probe_col.is_empty() {
            return Err(PyValueError::new_err(
                "get_value needs a non-empty single-element Serie for the key (its first element is \
                 the probe)",
            ));
        }
        let probe = probe_col.value(0);
        match self.inner.get_value(row, &probe) {
            None => Ok(None),
            // The core returns the matched value; locate it in the row's value range so the value
            // column can be sliced and rewrapped uniformly (any value type) into a one-element Serie.
            Some(found) => {
                let (start, end) = self.inner.value_range(row).unwrap_or((0, 0));
                let values = self.inner.values();
                match (start..end).find(|&index| values.value(index) == found) {
                    Some(index) => rewrap_column(values.slice(index, 1).as_ref(), py).map(Some),
                    None => Ok(None),
                }
            }
        }
    }

    /// The row at `index` as its `key -> value` entries `StructSerie` (columns `[keys, values]`), or
    /// `None` if the row is null; raises `IndexError` out of range.
    fn row(&self, py: Python<'_>, index: usize) -> PyResult<Option<PyObject>> {
        if index >= self.inner.len() {
            return Err(PyIndexError::new_err("MapSerie row index out of range"));
        }
        let scalar = self.inner.row_scalar(index);
        if scalar.is_null() {
            return Ok(None);
        }
        rewrap_column(scalar.entries(), py).map(Some)
    }

    /// A [`MapField`] naming this map column (nullability inferred from its null rows).
    fn to_field(&self, name: &str) -> MapField {
        MapField {
            inner: self.inner.to_field(name),
        }
    }

    /// The column's canonical bytes — a self-contained `[schema][len][validity?][offsets][entries]`
    /// frame, identical across Rust / Python / Node.
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.serialize_bytes())
    }

    /// Reconstructs a map column from [`serialize_bytes`](Self::serialize_bytes).
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        CoreMapSerie::deserialize_bytes(bytes)
            .map(|inner| Self { inner })
            .map_err(io_err)
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    /// An explicit copy.
    fn copy(&self) -> Self {
        self.clone()
    }
    fn __copy__(&self) -> Self {
        self.clone()
    }
    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// Pickles through `deserialize_bytes`.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<MapSerie>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    fn __repr__(&self) -> String {
        format!(
            "MapSerie(len={}, null_count={}, keys_sorted={})",
            self.inner.len(),
            self.inner.null_count(),
            self.inner.keys_sorted()
        )
    }
}

/// The **zero-copy Arrow C Data Interface** (PyCapsule protocol) for a map column, so pyarrow imports
/// it with no payload copy — `pyarrow.array(map_serie)` → a `MapArray` and back via
/// [`from_arrow`](MapSerie::from_arrow).
#[pymethods]
impl MapSerie {
    /// The Arrow C Data Interface **schema** capsule (`"arrow_schema"`).
    fn __arrow_c_schema__(&self, py: Python<'_>) -> PyResult<PyObject> {
        let array = self.inner.to_arrow_array().map_err(io_err)?;
        export_c_schema(py, array.data_type())
    }

    /// The Arrow C Data Interface **(schema, array)** capsule pair, exported zero-copy. The
    /// `requested_schema` hint is accepted for protocol compatibility and ignored.
    #[pyo3(signature = (requested_schema = None))]
    fn __arrow_c_array__(
        &self,
        py: Python<'_>,
        requested_schema: Option<PyObject>,
    ) -> PyResult<(PyObject, PyObject)> {
        let _ = requested_schema;
        export_c_array(
            py,
            std::sync::Arc::new(self.inner.to_arrow_array().map_err(io_err)?),
        )
    }

    /// Imports any object exposing the Arrow C Data Interface (a pyarrow `MapArray`) into a map
    /// column, zero-copy — the inverse of `__arrow_c_array__`.
    #[staticmethod]
    fn from_arrow(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
        let array = import_c_array(obj)?;
        let nullable = array.null_count() > 0;
        let field = arrow_schema::Field::new("", array.data_type().clone(), nullable);
        CoreMapSerie::from_arrow_array(array.as_ref(), &field)
            .map(|inner| Self { inner })
            .map_err(io_err)
    }
}

/// Adds the nested (`Struct` / `List` / `Map`) field + column classes to the `yggdryl.types`
/// submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<StructField>()?;
    module.add_class::<StructSerie>()?;
    module.add_class::<ListField>()?;
    module.add_class::<ListSerie>()?;
    module.add_class::<MapField>()?;
    module.add_class::<MapSerie>()?;
    Ok(())
}
