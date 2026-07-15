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
use yggdryl_core::io::nested::{StructField as CoreStructField, StructSerie as CoreStructSerie};
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
    );
    Err(PyValueError::new_err(
        "expected a yggdryl column (a Serie: U8Serie … Utf8Serie, D32Serie …, NullSerie, or \
         StructSerie)",
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
        other => {
            return Err(PyValueError::new_err(format!(
                "a struct child of type {} has no Python column wrapper",
                other.name()
            )))
        }
    })
}

/// A `Field` (leaf) or `StructField` (nested) Python object → an erased [`AnyField`].
fn extract_any_field(obj: &Bound<'_, PyAny>) -> PyResult<AnyField> {
    if let Ok(field) = obj.extract::<PyRef<Field>>() {
        return Ok(AnyField::leaf(field.inner.clone()));
    }
    if let Ok(field) = obj.extract::<PyRef<StructField>>() {
        return Ok(field.inner.as_any_field().clone());
    }
    Err(PyValueError::new_err(
        "expected a Field or a StructField as a struct child field",
    ))
}

/// An erased [`AnyField`] → its concrete Python `Field` / `StructField` object.
fn rewrap_field(any: &AnyField, py: Python<'_>) -> PyResult<PyObject> {
    if any.is_struct() {
        let inner = CoreStructField::from_any_field(any.clone())
            .expect("a struct AnyField rebuilds a StructField");
        Ok(StructField { inner }.into_py(py))
    } else {
        let inner = any
            .as_leaf()
            .expect("a non-struct AnyField is a leaf")
            .clone();
        Ok(Field { inner }.into_py(py))
    }
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
            Some(field) => rewrap_field(field, py),
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
        let schema = FFI_ArrowSchema::try_from(array.data_type()).map_err(arrow_err)?;
        let capsule = PyCapsule::new_bound(py, schema, Some(capsule_name("arrow_schema")))?;
        Ok(capsule.into_any().unbind())
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
        let data = self.inner.to_arrow_array().map_err(io_err)?.into_data();
        let (ffi_array, ffi_schema) = to_ffi(&data).map_err(arrow_err)?;
        let schema_capsule =
            PyCapsule::new_bound(py, ffi_schema, Some(capsule_name("arrow_schema")))?;
        let array_capsule = PyCapsule::new_bound(py, ffi_array, Some(capsule_name("arrow_array")))?;
        Ok((
            schema_capsule.into_any().unbind(),
            array_capsule.into_any().unbind(),
        ))
    }

    /// Imports any object exposing the Arrow C Data Interface (a pyarrow `StructArray` /
    /// `RecordBatch`) into a struct column, zero-copy — the inverse of `__arrow_c_array__`.
    #[staticmethod]
    fn from_arrow(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
        let pair = obj.call_method0("__arrow_c_array__")?;
        let (schema_cap, array_cap): (Bound<'_, PyAny>, Bound<'_, PyAny>) = pair.extract()?;
        let schema_cap = schema_cap.downcast::<PyCapsule>()?;
        let array_cap = array_cap.downcast::<PyCapsule>()?;
        // Move the FFI structs out of the producer's capsules, then blank the sources so the
        // producer's capsule destructors see a released struct and do not double-free.
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
        let array = arrow_array::make_array(data);
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

/// Adds `StructField` / `StructSerie` to the `yggdryl.types` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<StructField>()?;
    module.add_class::<StructSerie>()?;
    Ok(())
}
