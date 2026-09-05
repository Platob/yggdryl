//! Native Python view of Yggdryl fields and metadata.

use std::collections::BTreeMap;
use std::sync::Arc;

use arrow_array::RecordBatch as ArrowRecordBatch;
use arrow_pyarrow::{FromPyArrow, ToPyArrow};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema as ArrowSchema};
use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyDict, PyList, PyString};
use yggdryl::ArrowCast;
use yggdryl::{DataType as CoreDataType, Field as CoreField, Scheme as CoreScheme};

use crate::enums::{
    PyMediaType, PyMimeType, core_media_type_from_value, core_mime_type_from_value,
};
use crate::fix::{FixTag, branch_from_py, id_from_py};
use crate::types::datatype::{
    PyAsciiEnum, PyDataType, PyDataTypeIterator, arrow_array_from_pyarrow, arrow_array_to_pyarrow,
    arrow_scalar_to_pyarrow_type, ascii_arrow_scalar, core_dtype_from_value, core_field_to_pyarrow,
    default_arrow_scalar_to_pyarrow,
};
use crate::uri::{PyUrl, core_url_from_value};
use crate::{PyDifferenceIterator, compare, value_error};

pub(crate) fn core_field_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreField> {
    if let Ok(value) = value.extract::<PyRef<'_, PyField>>() {
        return Ok(value.inner.clone());
    }
    if let Ok(value) = value.extract::<&str>() {
        return CoreField::from_str(value).map_err(value_error);
    }

    if value
        .py()
        .import("dataclasses")?
        .getattr("is_dataclass")?
        .call1((value,))?
        .extract::<bool>()?
    {
        let field = python_field(value)?;
        let field = field.extract::<PyRef<'_, PyField>>()?;
        return Ok(field.inner.clone());
    }

    let imported: PyResult<CoreField> = (|| {
        let arrow_field = ArrowField::from_pyarrow_bound(value)?;
        let mut field = CoreField::try_from(arrow_field).map_err(value_error)?;

        // PyArrow's Field C Schema bridge can omit datatype-only flags such as
        // Map.keys_sorted. Its standalone datatype bridge is lossless, so use
        // that authoritative type when it differs from the field's own Arrow
        // projection; a recognized extension projects to its storage type and
        // keeps its identity.
        let py_dtype = value.getattr("type")?;
        let arrow_dtype = ArrowDataType::from_pyarrow_bound(&py_dtype)?;
        let projected = field.dtype().clone().into_arrow().map_err(value_error)?;
        if projected != arrow_dtype {
            let dtype = CoreDataType::try_from(arrow_dtype).map_err(value_error)?;
            field = field.try_with_dtype(dtype).map_err(value_error)?;
        }
        Ok(field)
    })();
    imported.map_err(|error| {
        if error.is_instance_of::<PyTypeError>(value.py()) {
            PyTypeError::new_err(
                "expected a yggdryl.Field, dataclass, field string, or PyArrow Field",
            )
        } else {
            error
        }
    })
}

/// Return the exact native `Field` object cached for a dataclass class or instance.
fn python_field<'py>(value: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let field = value
        .py()
        .import("yggdryl.types._classes")?
        .getattr("field")?
        .call1((value,))?;
    {
        let _field = field.extract::<PyRef<'_, PyField>>()?;
    }
    Ok(field)
}

/// Import a complete `PyArrow` Schema without trusting its lossy aggregate C
/// Schema children.  `PyArrow`'s standalone Field/DataType bridge carries
/// nested flags such as `Map.keys_sorted`, while its Schema bridge does not.
/// Root transport metadata still comes from the aggregate schema so native
/// record-schema rules can consume reserved sidecars exactly once.
pub(crate) fn core_schema_from_pyarrow(value: &Bound<'_, PyAny>) -> PyResult<ArrowSchema> {
    let imported = ArrowSchema::from_pyarrow_bound(value)?;
    let mut fields = Vec::with_capacity(imported.fields().len());
    for value in value.try_iter()? {
        let field = core_field_from_value(&value?)?;
        fields.push(field.into_arrow_ref().map_err(value_error)?);
    }
    Ok(ArrowSchema::new_with_metadata(
        fields,
        imported.metadata().clone(),
    ))
}

/// Export a complete `PyArrow` Schema from exact standalone native Fields.
/// The aggregate Arrow C Schema path drops nested datatype flags, so it is
/// used only to calculate transport metadata (including reserved sidecars).
fn core_schema_to_pyarrow<'py>(py: Python<'py>, root: &CoreField) -> PyResult<Bound<'py, PyAny>> {
    let transported = root
        .clone()
        .into_arrow_exchange_schema()
        .map_err(value_error)?;
    let fields = PyList::empty(py);
    for field in root.fields() {
        fields.append(core_field_to_pyarrow(py, field)?)?;
    }
    let metadata = PyDict::new(py);
    for (key, value) in transported.metadata() {
        metadata.set_item(key, value)?;
    }
    let kwargs = PyDict::new(py);
    kwargs.set_item("metadata", metadata)?;
    py.import("pyarrow")?
        .getattr("schema")?
        .call((fields,), Some(&kwargs))
}

fn extend_metadata_pairs(
    value: &Bound<'_, PyAny>,
    pairs: &mut BTreeMap<String, String>,
) -> PyResult<()> {
    let iterable = if value.hasattr("items")? {
        value.call_method0("items")?
    } else {
        value.clone()
    };
    for item in iterable.try_iter()? {
        let (key, value) = item?.extract::<(String, String)>()?;
        pairs.insert(key, value);
    }
    Ok(())
}

/// A Yggdryl field whose mapping protocol directly manages core metadata.
#[pyclass(name = "Field", module = "yggdryl._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyField {
    pub(crate) inner: CoreField,
    read_only: bool,
    hash_locked: bool,
}

#[derive(Clone, Copy)]
struct FieldId(i32);

#[derive(Clone, Copy)]
struct ContentLength(u64);

impl FromPyObject<'_, '_> for FieldId {
    type Error = PyErr;

    fn extract(value: Borrowed<'_, '_, PyAny>) -> PyResult<Self> {
        if value.is_instance_of::<PyBool>() {
            return Err(PyTypeError::new_err(
                "field id must be an integer, not bool",
            ));
        }
        value.extract::<i32>().map(Self)
    }
}

impl FromPyObject<'_, '_> for ContentLength {
    type Error = PyErr;

    fn extract(value: Borrowed<'_, '_, PyAny>) -> PyResult<Self> {
        if value.is_instance_of::<PyBool>() {
            return Err(PyTypeError::new_err(
                "content length must be an unsigned integer, not bool",
            ));
        }
        value.extract::<u64>().map(Self)
    }
}

impl PyField {
    pub(crate) fn from_inner(inner: CoreField) -> Self {
        Self::from_inner_with_read_only(inner, false)
    }

    pub(crate) fn from_inner_with_read_only(inner: CoreField, read_only: bool) -> Self {
        Self {
            inner,
            read_only,
            hash_locked: false,
        }
    }

    /// Cast a polars `LazyFrame` while keeping it lazy.
    ///
    /// The plan's schema comes from `collect_schema`, which computes no rows;
    /// the cast itself is mapped over the engine's batches, so the frame
    /// stays streamable and nothing is collected until the caller collects.
    fn cast_polars_lazy<'py>(
        &self,
        py: Python<'py>,
        frame: &Bound<'py, PyAny>,
        safe: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        // Plan-only: proves the frame answers schema questions without rows.
        frame.call_method0("collect_schema")?;

        // The output schema is the target's, spelled as polars spells it -
        // derived from a zero-row crossing, so no data moves here either.
        let polars = py.import("polars")?;
        let empty = {
            let arrow_field = core_field_to_pyarrow(py, &self.inner)?;
            let schema = py
                .import("pyarrow")?
                .call_method1("schema", (arrow_field.getattr("type")?,))?;
            let kwargs = pyo3::types::PyDict::new(py);
            kwargs.set_item("schema", schema)?;
            py.import("pyarrow")?.getattr("Table")?.call_method(
                "from_batches",
                (Vec::<Bound<'py, PyAny>>::new(),),
                Some(&kwargs),
            )?
        };
        let target_schema = polars
            .call_method1("from_arrow", (empty,))?
            .getattr("schema")?;

        let field = self.inner.clone();
        let cast_one = pyo3::types::PyCFunction::new_closure(
            py,
            None,
            None,
            move |args: &Bound<'_, pyo3::types::PyTuple>,
                  _kwargs: Option<&Bound<'_, PyDict>>|
                  -> PyResult<Py<PyAny>> {
                let frame = args.get_item(0)?;
                let py = frame.py();
                let this = Self::from_inner(field.clone());
                let table = crate::iomedia::polars_to_arrow(&frame)?;
                let cast = this.cast_arrow(py, &table, safe)?;
                Ok(py
                    .import("polars")?
                    .call_method1("from_arrow", (cast,))?
                    .unbind())
            },
        )?;
        let kwargs = pyo3::types::PyDict::new(py);
        kwargs.set_item("schema", target_schema)?;
        frame.call_method("map_batches", (cast_one,), Some(&kwargs))
    }

    fn require_mutable(&self) -> PyResult<()> {
        if self.read_only {
            Err(PyTypeError::new_err("frozen fields are read-only"))
        } else if self.hash_locked {
            Err(PyTypeError::new_err(
                "a hashed Field is frozen; copy it before mutation",
            ))
        } else {
            Ok(())
        }
    }
}

#[pymethods]
impl PyField {
    #[new]
    #[pyo3(signature = (name, dtype, nullable=true, metadata=None))]
    fn new(
        name: String,
        dtype: &Bound<'_, PyAny>,
        nullable: bool,
        metadata: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let dtype = core_dtype_from_value(dtype)?;
        if let Some(metadata) = metadata {
            let mut pairs = BTreeMap::new();
            extend_metadata_pairs(metadata, &mut pairs)?;
            return CoreField::from_parts(name, dtype, nullable, pairs)
                .map(Self::from_inner)
                .map_err(value_error);
        }
        let field = CoreField::new(name, dtype, nullable);
        field.validate().map_err(value_error)?;
        Ok(Self::from_inner(field))
    }

    #[staticmethod]
    fn from_value(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        core_field_from_value(value).map(Self::from_inner)
    }

    /// Infers a native field from a name and Python type annotation.
    #[staticmethod]
    #[pyo3(signature = (name, hint, metadata=None))]
    fn from_pyhint(
        name: &str,
        hint: &Bound<'_, PyAny>,
        metadata: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let module = hint.py().import("yggdryl.types._hints")?;
        let inferred = if let Some(metadata) = metadata {
            module
                .getattr("field_from_pyhint")?
                .call1((name, hint, metadata))?
        } else {
            module.getattr("field_from_pyhint")?.call1((name, hint))?
        };
        let inferred = inferred.extract::<PyRef<'_, Self>>()?;
        Ok(inferred.clone())
    }

    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        CoreField::from_str(value)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Rebuild pickle state without carrying a transient hash lock.
    #[staticmethod]
    fn _from_pickle(value: &str, read_only: bool) -> PyResult<Self> {
        CoreField::from_str(value)
            .map(|inner| Self::from_inner_with_read_only(inner, read_only))
            .map_err(value_error)
    }

    #[staticmethod]
    fn from_arrow(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        core_field_from_value(value).map(Self::from_inner)
    }

    /// Imports one complete Arrow Schema through Yggdryl's Arrow IPC metadata
    /// rules as a non-nullable Struct Field.
    #[staticmethod]
    #[pyo3(signature = (schema, name = "row"))]
    fn from_arrow_schema(schema: &Bound<'_, PyAny>, name: &str) -> PyResult<Self> {
        let schema = core_schema_from_pyarrow(schema)?;
        CoreField::from_arrow_schema(name, &schema)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Projects a complete Struct root as an Arrow transport Schema while
    /// keeping the reserved dictionary-ID sidecar out of Field metadata.
    #[allow(clippy::wrong_self_convention)]
    fn into_arrow_schema<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        core_schema_to_pyarrow(py, &self.inner)
    }

    /// Builds a plain dataclass whose fields follow this native Struct field.
    #[pyo3(signature = (name = None, *, module = None))]
    fn into_dataclass<'py>(
        slf: &Bound<'py, Self>,
        name: Option<&str>,
        module: Option<&str>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let invalid_root = {
            let field = slf.borrow();
            !field.inner.is_struct() || field.inner.is_nullable()
        };
        if invalid_root {
            return Err(PyTypeError::new_err(
                "into_dataclass requires a non-nullable Struct Field",
            ));
        }
        let py = slf.py();
        let kwargs = PyDict::new(py);
        if let Some(name) = name {
            kwargs.set_item("name", name)?;
        }
        if let Some(module) = module {
            kwargs.set_item("module", module)?;
        }
        py.import("yggdryl.types._classes")?
            .getattr("_dataclass_from_field")?
            .call((slf,), Some(&kwargs))
    }

    #[staticmethod]
    fn from_json(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        // One entry point for the three shapes a caller already has: the
        // document as text, the same document as the bytes it was read from,
        // or the structure `json.loads` already turned it into. Dispatching
        // here rather than making the caller pick keeps `dict` and `str`
        // equally first-class, which is what `into_dict` and `into_json`
        // already imply.
        if let Ok(text) = value.extract::<&str>() {
            return CoreField::from_json(text)
                .map(Self::from_inner)
                .map_err(value_error);
        }
        // Bytes-like by downcast rather than by extracting `Vec<u8>`, which a
        // list of small integers would also satisfy - and a JSON document that
        // really is a sequence must still reach the native path below.
        if let Ok(bytes) = value.cast::<pyo3::types::PyBytes>() {
            return CoreField::from_json_bytes(bytes.as_bytes())
                .map(Self::from_inner)
                .map_err(value_error);
        }
        if let Ok(bytes) = value.cast::<pyo3::types::PyByteArray>() {
            return CoreField::from_json_bytes(&bytes.to_vec())
                .map(Self::from_inner)
                .map_err(value_error);
        }
        CoreField::from_value(crate::types::scalar::from_py(value)?)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Returns the cached Python annotation corresponding to this Field.
    fn default_pyhint<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let field = Py::new(py, self.clone())?;
        py.import("yggdryl.types._defaults")?
            .getattr("_default_pyhint_from_field")?
            .call1((field,))
    }

    /// Returns the core-selected Field default as an exact `PyArrow` Scalar.
    fn default_arrow_scalar<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let array = self.inner.default_arrow_array().map_err(value_error)?;
        default_arrow_scalar_to_pyarrow(py, &self.inner, &array)
    }

    /// Returns the core-selected Field default in its cached Python type plan.
    fn default_pyvalue<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let array = self.inner.default_arrow_array().map_err(value_error)?;
        let scalar = default_arrow_scalar_to_pyarrow(py, &self.inner, &array)?;
        let field = Py::new(py, self.clone())?;
        py.import("yggdryl.types._defaults")?
            .getattr("_default_pyvalue_from_field")?
            .call1((field, scalar))
    }

    /// Returns a recursively normalized field for a named compatibility target.
    #[allow(clippy::wrong_self_convention)]
    fn into_scheme_compat(&self, target: &str) -> PyResult<Self> {
        let target = CoreScheme::from_str(target).map_err(value_error)?;
        self.inner
            .clone()
            .into_scheme_compat(&target)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Constructs a `PyArrow` scalar with this field's exact datatype.
    #[pyo3(signature = (value, *, safe=true))]
    fn arrow_scalar<'py>(
        &self,
        py: Python<'py>,
        value: &Bound<'py, PyAny>,
        safe: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let scalar =
            if self.inner.dtype().is_ascii() || matches!(self.inner.dtype(), CoreDataType::Uuid) {
                ascii_arrow_scalar(py, value, self.inner.dtype(), safe)?
            } else {
                // Project the complete Field so registered extension metadata can
                // be rehydrated by PyArrow before selecting its scalar target type.
                let arrow_field = core_field_to_pyarrow(py, &self.inner)?;
                let target = arrow_field.getattr("type")?;
                arrow_scalar_to_pyarrow_type(py, value, target, safe)?
            };
        if !self.inner.is_nullable() && !scalar.getattr("is_valid")?.extract::<bool>()? {
            return Err(PyValueError::new_err(format!(
                "field {:?} is not nullable",
                self.inner.name()
            )));
        }
        Ok(scalar)
    }

    /// Casts and null/default-fills one `PyArrow` Array through the exact Field.
    #[pyo3(signature = (value, *, safe=true))]
    fn cast_arrow_array<'py>(
        &self,
        py: Python<'py>,
        value: &Bound<'py, PyAny>,
        safe: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let input = arrow_array_from_pyarrow(value)?;
        let array = self
            .inner
            .cast_arrow_array(Arc::clone(&input), safe)
            .map_err(value_error)?;
        if Arc::ptr_eq(&input, &array) {
            let has_extension_metadata = self.inner.has_metadata("ARROW:extension:name")
                || self.inner.has_metadata("ARROW:extension:metadata");
            if !has_extension_metadata {
                return Ok(value.clone());
            }
            // An Arrow Array carries only its storage datatype on the Rust
            // side.  Extension identity is Field metadata, so a storage array
            // can be a native no-op while still requiring an exact Field C
            // Schema export to rehydrate its PyArrow ExtensionType.
            let target = core_field_to_pyarrow(py, &self.inner)?.getattr("type")?;
            if value.getattr("type")?.eq(&target)? {
                return Ok(value.clone());
            }
        }
        arrow_array_to_pyarrow(py, &array, Some(&self.inner))
    }

    /// Reconciles one `PyArrow` `RecordBatch` to this exact Struct Field.
    #[pyo3(signature = (value, *, safe=true))]
    fn cast_arrow_batch<'py>(
        &self,
        py: Python<'py>,
        value: &Bound<'py, PyAny>,
        safe: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let batch = ArrowRecordBatch::from_pyarrow_bound(value)?;
        let source_schema = batch.schema();
        let source_columns = batch.columns().to_vec();
        let cast = self
            .inner
            .cast_arrow_batch(batch, safe)
            .map_err(value_error)?;
        if Arc::ptr_eq(&source_schema, &cast.schema())
            && source_columns
                .iter()
                .zip(cast.columns())
                .all(|(left, right)| Arc::ptr_eq(left, right))
        {
            return Ok(value.clone());
        }
        cast.to_pyarrow(py)
    }

    /// Casts one `PyArrow` scalar - or a one-row Array - to this exact Field.
    #[pyo3(signature = (value, *, safe=true))]
    fn cast_arrow_scalar<'py>(
        &self,
        py: Python<'py>,
        value: &Bound<'py, PyAny>,
        safe: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        if value.is_instance(&py.import("pyarrow")?.getattr("Array")?)? {
            if value.len()? != 1 {
                return Err(PyValueError::new_err(format!(
                    "a scalar cast takes exactly one row, got {}",
                    value.len()?
                )));
            }
            return self.arrow_scalar(py, &value.get_item(0)?, safe);
        }
        self.arrow_scalar(py, value, safe)
    }

    /// Casts whatever Arrow-shaped thing `value` is to this exact Field.
    ///
    /// The kind is inferred and the result keeps it: a `pyarrow` `Scalar`,
    /// `Array`, `ChunkedArray`, `RecordBatch`, `Table`, `RecordBatchReader`,
    /// `Dataset`, or `Scanner` comes back as a scalar, array, batch, table,
    /// or streamed reader; a `polars` `DataFrame` crosses at the newest
    /// compat level - view arrays stay view arrays - and comes back a
    /// `DataFrame`; a `polars` `LazyFrame` stays lazy, its schema read with
    /// `collect_schema` and the cast mapped over its batches, so nothing is
    /// collected until the caller collects; a `pandas` `DataFrame` or
    /// `Series` crosses through Arrow and comes back as itself. Streams are
    /// cast batch by batch - nothing is collected that could flow.
    #[pyo3(signature = (value, *, safe=true))]
    fn cast_arrow<'py>(
        &self,
        py: Python<'py>,
        value: &Bound<'py, PyAny>,
        safe: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        use crate::iomedia::declared_by;

        // polars: the frame comes back a frame, the lazy frame stays lazy.
        if declared_by(value, "polars", "DataFrame") {
            let table = crate::iomedia::polars_to_arrow(value)?;
            let cast = self.cast_arrow(py, &table, safe)?;
            return py.import("polars")?.call_method1("from_arrow", (cast,));
        }
        if declared_by(value, "polars", "LazyFrame") {
            return self.cast_polars_lazy(py, value, safe);
        }
        if declared_by(value, "polars", "Series") {
            let array = value.call_method0("to_arrow")?;
            let cast = self.cast_arrow_array(py, &array, safe)?;
            return py.import("polars")?.call_method1("from_arrow", (cast,));
        }
        // pandas: through Arrow and back, with pandas' own compat checks.
        if declared_by(value, "pandas", "DataFrame") {
            let table = py
                .import("pyarrow")?
                .getattr("Table")?
                .call_method1("from_pandas", (value,))?;
            let cast = self.cast_arrow(py, &table, safe)?;
            return cast.call_method0("to_pandas");
        }
        if declared_by(value, "pandas", "Series") {
            let array = py
                .import("pyarrow")?
                .getattr("Array")?
                .call_method1("from_pandas", (value,))?;
            let cast = self.cast_arrow_array(py, &array, safe)?;
            return cast.call_method0("to_pandas");
        }

        let pyarrow = py.import("pyarrow")?;
        let is = |name: &str| -> PyResult<bool> { value.is_instance(&pyarrow.getattr(name)?) };
        if is("Scalar")? {
            return self.arrow_scalar(py, value, safe);
        }
        if is("ChunkedArray")? {
            let combined = value.call_method0("combine_chunks")?;
            let cast = self.cast_arrow_array(py, &combined, safe)?;
            return pyarrow.call_method1("chunked_array", (vec![cast],));
        }
        if is("Array")? {
            return self.cast_arrow_array(py, value, safe);
        }
        if is("RecordBatch")? {
            return self.cast_arrow_batch(py, value, safe);
        }
        if is("Table")? {
            let reader = self.cast_arrow(py, &value.call_method0("to_reader")?, safe)?;
            return reader.call_method0("read_all");
        }
        // Everything that streams - a RecordBatchReader, a Dataset, a
        // Scanner, anything exporting the C stream - casts batch by batch.
        let reader = crate::iomedia::batch_reader_from_any(
            value,
            &yggdryl::media::RecordOptions::for_mime_type(&yggdryl::MimeType::ARROW_STREAM)
                .map_err(value_error)?,
        )?;
        let cast = yggdryl::arrow::cast_reader(reader, &self.inner, safe).map_err(value_error)?;
        crate::iomedia::batch_reader_to_pyarrow(py, cast)
    }

    /// The generic cast: [`cast_arrow`](Self::cast_arrow) for anything
    /// Arrow-shaped, and a typed scalar for a plain Python value.
    #[pyo3(signature = (value, *, safe=true))]
    fn cast<'py>(
        &self,
        py: Python<'py>,
        value: &Bound<'py, PyAny>,
        safe: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        use crate::iomedia::declared_by;

        let module_root = value
            .get_type()
            .module()
            .and_then(|module| module.extract::<String>())
            .unwrap_or_default();
        let arrow_shaped = module_root.starts_with("pyarrow")
            || module_root.starts_with("polars")
            || declared_by(value, "pandas", "DataFrame")
            || declared_by(value, "pandas", "Series")
            || value.hasattr("__arrow_c_stream__")?
            || value.hasattr("__arrow_c_array__")?;
        if arrow_shaped {
            return self.cast_arrow(py, value, safe);
        }
        // A plain Python value becomes the typed scalar this Field declares.
        self.arrow_scalar(py, value, safe)
    }

    #[allow(clippy::wrong_self_convention)]
    fn into_arrow<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        core_field_to_pyarrow(py, &self.inner)
    }

    /// Serialize as deterministic structural JSON.
    ///
    /// `indent=None` is compact - today's output and the default; an integer
    /// pretty-prints with that many spaces per nesting level, exactly as
    /// `json.dumps(indent=n)` reads.
    #[allow(clippy::wrong_self_convention)]
    #[pyo3(signature = (*, indent = None))]
    fn into_json(&self, indent: Option<u8>) -> PyResult<String> {
        self.inner
            .clone()
            .into_json_with_formatting(crate::formatting_of(indent))
            .map_err(value_error)
    }

    /// Serialize to structural JSON bytes.
    ///
    /// The same document `into_json` renders, encoded rather than decoded, for
    /// a caller writing it straight to a file or a socket. `from_json` reads
    /// these bytes back without being told which of the three shapes it got.
    #[allow(clippy::wrong_self_convention)]
    #[pyo3(signature = (*, indent = None))]
    fn into_json_bytes<'py>(
        &self,
        py: Python<'py>,
        indent: Option<u8>,
    ) -> PyResult<Bound<'py, pyo3::types::PyBytes>> {
        let text = self
            .inner
            .clone()
            .into_json_with_formatting(crate::formatting_of(indent))
            .map_err(value_error)?;
        Ok(pyo3::types::PyBytes::new(py, text.as_bytes()))
    }

    /// Deserialize and validate from structural YAML.
    ///
    /// The same structure `from_json` reads, in YAML's syntax - so a config
    /// document can carry a declared schema inline beside its other settings.
    #[staticmethod]
    fn from_yaml(value: &str) -> PyResult<Self> {
        CoreField::from_yaml(value)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Serialize as YAML: block style, one key per line.
    ///
    /// `indent=2` is the default. `indent=None` asks for flow style -
    /// `{a: 1, b: 2}` on one line - which is valid YAML and round-trips, and
    /// is never what a caller gets by accident.
    #[allow(clippy::wrong_self_convention)]
    #[pyo3(signature = (*, indent = Some(2)))]
    fn into_yaml(&self, indent: Option<u8>) -> PyResult<String> {
        self.inner
            .clone()
            .into_yaml_with_formatting(crate::formatting_of(indent))
            .map_err(value_error)
    }

    /// Deserialize and validate from structural TOML.
    #[staticmethod]
    fn from_toml(value: &str) -> PyResult<Self> {
        CoreField::from_toml(value)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Serialize as TOML.
    ///
    /// TOML has no null, and this model never needs one: an unset optional
    /// attribute is omitted rather than faked, so nothing is lost.
    #[allow(clippy::wrong_self_convention)]
    #[pyo3(signature = (*, indent = None))]
    fn into_toml(&self, indent: Option<u8>) -> PyResult<String> {
        self.inner
            .clone()
            .into_toml_with_formatting(crate::formatting_of(indent))
            .map_err(value_error)
    }

    /// Project this value onto a plain structural mapping.
    ///
    /// The core's one structural model - the model JSON, YAML, and TOML are
    /// all expressed over - handed back as a `dict`, so a schema drops into any
    /// document a caller already builds.
    #[allow(clippy::wrong_self_convention)]
    fn into_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        crate::types::scalar::as_py(py, &self.inner.clone().into_value())
    }

    /// Read this value back from a plain structural mapping.
    ///
    /// The inverse of `into_dict`, through the core's one conversion.
    #[staticmethod]
    fn from_dict(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        CoreField::from_value(crate::types::scalar::from_py(value)?)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// A readable, indented rendering of this value and everything under it.
    ///
    /// `str` and `repr` are unchanged - the compact constructor form, which
    /// round-trips through `from_str` - because that is what a Python reader
    /// expects of `repr` and what the parsers depend on. This is the form for
    /// a human looking at a schema three levels deep.
    fn pretty(&self) -> String {
        format!("{:#}", self.inner)
    }

    /// The readable rendering, for `IPython` and notebook cells.
    fn _repr_pretty_(&self, printer: &Bound<'_, PyAny>, _cycle: bool) -> PyResult<()> {
        printer.call_method1("text", (self.pretty(),))?;
        Ok(())
    }

    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    #[getter]
    fn dtype(&self) -> PyDataType {
        PyDataType {
            inner: self.inner.dtype().clone(),
            hash_locked: false,
            borrowed_from_field: true,
            children_read_only: self.read_only,
        }
    }

    #[getter]
    fn nullable(&self) -> bool {
        self.inner.is_nullable()
    }

    #[getter]
    fn parquet_field_id(&self) -> PyResult<Option<i32>> {
        self.inner.parquet_field_id().map_err(value_error)
    }

    /// The enum this field's ASCII values name, ``None`` when it declares none.
    ///
    /// The declaration is one ``field:enum`` document, so it reaches Arrow, a
    /// file, and another runtime as ordinary field metadata and comes back the
    /// enum that was written.
    #[getter]
    fn ascii_enum(&self) -> PyResult<Option<PyAsciiEnum>> {
        self.inner
            .ascii_enum()
            .map(|value| value.map(PyAsciiEnum::from_inner))
            .map_err(value_error)
    }

    #[getter]
    fn dictionary_id(&self) -> Option<i64> {
        self.inner.dictionary_id()
    }

    #[getter]
    fn dictionary_is_ordered(&self) -> Option<bool> {
        self.inner.dictionary_is_ordered()
    }

    #[getter]
    fn alias(&self) -> Option<&str> {
        self.inner.alias()
    }

    #[getter]
    fn comment(&self) -> Option<&str> {
        self.inner.comment()
    }

    #[getter]
    fn display(&self) -> Option<&str> {
        self.inner.display()
    }

    #[getter]
    fn location(&self) -> PyResult<Option<PyUrl>> {
        self.inner
            .location()
            .map(|value| value.map(PyUrl::from_core))
            .map_err(value_error)
    }

    #[getter]
    fn accept(&self) -> Option<&str> {
        self.inner.as_http().accept()
    }

    #[getter]
    fn accept_encoding(&self) -> Option<&str> {
        self.inner.as_http().accept_encoding()
    }

    #[getter]
    fn accept_language(&self) -> Option<&str> {
        self.inner.as_http().accept_language()
    }

    #[getter]
    fn accept_ranges(&self) -> Option<&str> {
        self.inner.as_http().accept_ranges()
    }

    #[getter]
    fn cache_control(&self) -> Option<&str> {
        self.inner.as_http().cache_control()
    }

    #[getter]
    fn content_disposition(&self) -> Option<&str> {
        self.inner.as_http().content_disposition()
    }

    #[getter]
    fn content_encoding(&self) -> Option<&str> {
        self.inner.as_http().content_encoding()
    }

    #[getter]
    fn content_language(&self) -> Option<&str> {
        self.inner.as_http().content_language()
    }

    #[getter]
    fn content_length(&self) -> PyResult<Option<u64>> {
        self.inner.as_http().content_length().map_err(value_error)
    }

    #[getter]
    fn content_location(&self) -> Option<&str> {
        self.inner.as_http().content_location()
    }

    #[getter]
    fn content_range(&self) -> Option<&str> {
        self.inner.as_http().content_range()
    }

    #[getter]
    fn content_type(&self) -> Option<&str> {
        self.inner.as_http().content_type()
    }

    #[getter]
    fn mime_type(&self) -> PyResult<PyMimeType> {
        self.inner
            .as_http()
            .mime_type()
            .map(PyMimeType::from_core)
            .map_err(value_error)
    }

    #[getter]
    fn media_type(&self) -> PyResult<PyMediaType> {
        self.inner
            .as_http()
            .media_type()
            .map(PyMediaType::from_core)
            .map_err(value_error)
    }

    #[getter]
    fn etag(&self) -> Option<&str> {
        self.inner.as_http().etag()
    }

    #[getter]
    fn expires(&self) -> Option<&str> {
        self.inner.as_http().expires()
    }

    #[getter]
    fn last_modified(&self) -> Option<&str> {
        self.inner.as_http().last_modified()
    }

    #[getter]
    fn http_location(&self) -> PyResult<Option<PyUrl>> {
        self.inner
            .as_http()
            .location()
            .map(|value| value.map(PyUrl::from_core))
            .map_err(value_error)
    }

    #[getter]
    fn range(&self) -> Option<&str> {
        self.inner.as_http().range()
    }

    #[getter]
    fn vary(&self) -> Option<&str> {
        self.inner.as_http().vary()
    }

    fn set_dictionary_options(&mut self, id: i64, is_ordered: bool) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_dictionary_options(id, is_ordered)
            .map_err(value_error)
    }

    fn set_parquet_field_id(&mut self, id: FieldId) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_parquet_field_id(id.0);
        Ok(())
    }

    fn remove_parquet_field_id(&mut self) -> PyResult<Option<i32>> {
        self.require_mutable()?;
        self.inner.remove_parquet_field_id().map_err(value_error)
    }

    /// Declares the enum this field's ASCII values name.
    fn set_ascii_enum(&mut self, value: &PyAsciiEnum) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_ascii_enum(value.as_inner())
            .map_err(value_error)
    }

    /// Removes the declaration and returns the enum it held.
    fn remove_ascii_enum(&mut self) -> PyResult<Option<PyAsciiEnum>> {
        self.require_mutable()?;
        self.inner
            .remove_ascii_enum()
            .map(|value| value.map(PyAsciiEnum::from_inner))
            .map_err(value_error)
    }

    fn set_alias(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_alias(value).map_err(value_error)
    }

    fn remove_alias(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_alias())
    }

    fn set_comment(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_comment(value).map_err(value_error)
    }

    fn remove_comment(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_comment())
    }

    fn set_display(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_display(value).map_err(value_error)
    }

    fn remove_display(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_display())
    }

    fn set_location(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_location(core_url_from_value(value)?);
        Ok(())
    }

    fn remove_location(&mut self) -> PyResult<Option<PyUrl>> {
        self.require_mutable()?;
        self.inner
            .remove_location()
            .map(|value| value.map(PyUrl::from_core))
            .map_err(value_error)
    }

    fn set_accept(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .set_accept(value)
            .map_err(value_error)
    }

    fn remove_accept(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.as_http_mut().remove_accept())
    }

    fn set_accept_encoding(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .set_accept_encoding(value)
            .map_err(value_error)
    }

    fn remove_accept_encoding(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.as_http_mut().remove_accept_encoding())
    }

    fn set_accept_language(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .set_accept_language(value)
            .map_err(value_error)
    }

    fn remove_accept_language(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.as_http_mut().remove_accept_language())
    }

    fn set_accept_ranges(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .set_accept_ranges(value)
            .map_err(value_error)
    }

    fn remove_accept_ranges(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.as_http_mut().remove_accept_ranges())
    }

    fn set_cache_control(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .set_cache_control(value)
            .map_err(value_error)
    }

    fn remove_cache_control(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.as_http_mut().remove_cache_control())
    }

    fn set_content_disposition(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .set_content_disposition(value)
            .map_err(value_error)
    }

    fn remove_content_disposition(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.as_http_mut().remove_content_disposition())
    }

    fn set_content_encoding(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .set_content_encoding(value)
            .map_err(value_error)
    }

    fn remove_content_encoding(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.as_http_mut().remove_content_encoding())
    }

    fn set_content_language(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .set_content_language(value)
            .map_err(value_error)
    }

    fn remove_content_language(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.as_http_mut().remove_content_language())
    }

    fn set_content_length(&mut self, value: ContentLength) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.as_http_mut().set_content_length(value.0);
        Ok(())
    }

    fn remove_content_length(&mut self) -> PyResult<Option<u64>> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .remove_content_length()
            .map_err(value_error)
    }

    fn set_content_location(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .set_content_location(value)
            .map_err(value_error)
    }

    fn remove_content_location(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.as_http_mut().remove_content_location())
    }

    fn set_content_range(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .set_content_range(value)
            .map_err(value_error)
    }

    fn remove_content_range(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.as_http_mut().remove_content_range())
    }

    fn set_content_type(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .set_content_type(value)
            .map_err(value_error)
    }

    fn remove_content_type(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.as_http_mut().remove_content_type())
    }

    fn set_mime_type(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .set_mime_type(core_mime_type_from_value(value)?);
        Ok(())
    }

    fn remove_mime_type(&mut self) -> PyResult<Option<PyMimeType>> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .remove_mime_type()
            .map(|value| value.map(PyMimeType::from_core))
            .map_err(value_error)
    }

    fn set_media_type(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .set_media_type(core_media_type_from_value(value)?)
            .map_err(value_error)
    }

    fn remove_media_type(&mut self) -> PyResult<Option<PyMediaType>> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .remove_media_type()
            .map(|value| value.map(PyMediaType::from_core))
            .map_err(value_error)
    }

    fn set_etag(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .set_etag(value)
            .map_err(value_error)
    }

    fn remove_etag(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.as_http_mut().remove_etag())
    }

    fn set_expires(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .set_expires(value)
            .map_err(value_error)
    }

    fn remove_expires(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.as_http_mut().remove_expires())
    }

    fn set_last_modified(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .set_last_modified(value)
            .map_err(value_error)
    }

    fn remove_last_modified(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.as_http_mut().remove_last_modified())
    }

    fn set_http_location(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .set_location(core_url_from_value(value)?);
        Ok(())
    }

    fn remove_http_location(&mut self) -> PyResult<Option<PyUrl>> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .remove_location()
            .map(|value| value.map(PyUrl::from_core))
            .map_err(value_error)
    }

    fn set_range(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .set_range(value)
            .map_err(value_error)
    }

    fn remove_range(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.as_http_mut().remove_range())
    }

    fn set_vary(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .as_http_mut()
            .set_vary(value)
            .map_err(value_error)
    }

    fn remove_vary(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.as_http_mut().remove_vary())
    }

    fn get_property(&self, scheme: &str, name: &str) -> PyResult<Option<&str>> {
        let scheme = CoreScheme::from_str(scheme).map_err(value_error)?;
        Ok(self.inner.get_property(&scheme, name))
    }

    fn has_property(&self, scheme: &str, name: &str) -> PyResult<bool> {
        let scheme = CoreScheme::from_str(scheme).map_err(value_error)?;
        Ok(self.inner.has_property(&scheme, name))
    }

    fn set_property(
        &mut self,
        scheme: &str,
        name: &str,
        value: String,
    ) -> PyResult<Option<String>> {
        self.require_mutable()?;
        let scheme = CoreScheme::from_str(scheme).map_err(value_error)?;
        self.inner
            .set_property(&scheme, name, value)
            .map_err(value_error)
    }

    fn remove_property(&mut self, scheme: &str, name: &str) -> PyResult<Option<String>> {
        self.require_mutable()?;
        let scheme = CoreScheme::from_str(scheme).map_err(value_error)?;
        Ok(self.inner.remove_property(&scheme, name))
    }

    fn property_iter(&self, scheme: &str) -> PyResult<PyFieldPropertyIterator> {
        let scheme = CoreScheme::from_str(scheme).map_err(value_error)?;
        Ok(PyFieldPropertyIterator::new(
            &self.inner,
            scheme,
            PropertyIteratorKind::Items,
        ))
    }

    fn clear_properties(&mut self, scheme: &str) -> PyResult<()> {
        self.require_mutable()?;
        let scheme = CoreScheme::from_str(scheme).map_err(value_error)?;
        self.inner.clear_properties(&scheme);
        Ok(())
    }

    /// Returns one protocol's properties as a live view of this field.
    ///
    /// The scheme accepts every spelling `get_property` accepts, so `HTTPS`
    /// selects the one canonical `http:` namespace.
    fn protocol(slf: Py<Self>, scheme: &str) -> PyResult<PyProtocolField> {
        let scheme = CoreScheme::from_str(scheme).map_err(value_error)?;
        Ok(PyProtocolField::new(slf, scheme))
    }

    /// Returns the live HTTP and HTTPS property view.
    #[getter]
    fn http(slf: Py<Self>) -> PyProtocolField {
        PyProtocolField::new(slf, CoreScheme::HTTP)
    }

    /// Returns the live file protocol property view.
    #[getter]
    fn file(slf: Py<Self>) -> PyProtocolField {
        PyProtocolField::new(slf, CoreScheme::FILE)
    }

    /// Returns the live uniform resource name property view.
    #[getter]
    fn urn(slf: Py<Self>) -> PyProtocolField {
        PyProtocolField::new(slf, CoreScheme::URN)
    }

    /// Returns the live short-spelling `PostgreSQL` property view.
    #[getter]
    fn postgres(slf: Py<Self>) -> PyProtocolField {
        PyProtocolField::new(slf, CoreScheme::POSTGRES)
    }

    /// Returns the live long-spelling `PostgreSQL` property view.
    #[getter]
    fn postgresql(slf: Py<Self>) -> PyProtocolField {
        PyProtocolField::new(slf, CoreScheme::POSTGRESQL)
    }

    /// Returns the live `MySQL` property view.
    #[getter]
    fn mysql(slf: Py<Self>) -> PyProtocolField {
        PyProtocolField::new(slf, CoreScheme::MYSQL)
    }

    /// Returns the live Arrow property view.
    #[getter]
    fn arrow(slf: Py<Self>) -> PyProtocolField {
        PyProtocolField::new(slf, CoreScheme::ARROW)
    }

    /// Returns the live generic SQL property view.
    #[getter]
    fn sql(slf: Py<Self>) -> PyProtocolField {
        PyProtocolField::new(slf, CoreScheme::SQL)
    }

    /// Returns the live AWS Glue property view.
    #[getter]
    fn glue(slf: Py<Self>) -> PyProtocolField {
        PyProtocolField::new(slf, CoreScheme::GLUE)
    }

    /// Returns the live Apache Iceberg property view.
    #[getter]
    fn iceberg(slf: Py<Self>) -> PyProtocolField {
        PyProtocolField::new(slf, CoreScheme::ICEBERG)
    }

    /// Returns the live Financial Information eXchange property view.
    #[getter]
    fn fix(slf: Py<Self>) -> PyProtocolField {
        PyProtocolField::new(slf, CoreScheme::FIX)
    }

    /// Returns the live Yggdryl field property view.
    ///
    /// Named for the namespace it exposes rather than plain `field`, which on
    /// a schema node reaches a nested child.
    #[getter]
    fn field_properties(slf: Py<Self>) -> PyProtocolField {
        PyProtocolField::new(slf, CoreScheme::FIELD)
    }

    /// Returns the live Amazon S3 property view.
    #[getter]
    fn s3(slf: Py<Self>) -> PyProtocolField {
        PyProtocolField::new(slf, CoreScheme::S3)
    }

    /// Returns the live Google Cloud Storage property view.
    #[getter]
    fn gs(slf: Py<Self>) -> PyProtocolField {
        PyProtocolField::new(slf, CoreScheme::GS)
    }

    /// Returns the live Azure Blob Storage property view.
    #[getter]
    fn az(slf: Py<Self>) -> PyProtocolField {
        PyProtocolField::new(slf, CoreScheme::AZ)
    }

    /// Returns the live Apache Spark property view.
    #[getter]
    fn spark(slf: Py<Self>) -> PyProtocolField {
        PyProtocolField::new(slf, CoreScheme::SPARK)
    }

    /// Returns the live Polars property view.
    #[getter]
    fn polars(slf: Py<Self>) -> PyProtocolField {
        PyProtocolField::new(slf, CoreScheme::POLARS)
    }

    /// Returns the live pandas property view.
    #[getter]
    fn pandas(slf: Py<Self>) -> PyProtocolField {
        PyProtocolField::new(slf, CoreScheme::PANDAS)
    }

    /// Returns whether this field carries the values a path spells out.
    #[getter]
    fn is_partition(&self) -> bool {
        self.inner.is_partition()
    }

    /// Marks or unmarks this field as one a path spells out.
    fn set_partition(&mut self, partition: bool) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_partition(partition);
        Ok(())
    }

    /// Returns the struct children that partition the rows.
    #[getter]
    fn partition_fields(&self) -> Vec<Self> {
        self.inner
            .partition_fields()
            .cloned()
            .map(Self::from_inner)
            .collect()
    }

    /// Returns the names of the struct children that partition the rows.
    #[getter]
    fn partition_field_names(&self) -> Vec<&str> {
        self.inner.partition_field_names().collect()
    }

    /// Returns how many struct children partition the rows.
    #[getter]
    fn partition_field_len(&self) -> usize {
        self.inner.partition_field_len()
    }

    /// Returns whether any struct child partitions the rows.
    #[getter]
    fn has_partition_fields(&self) -> bool {
        self.inner.has_partition_fields()
    }

    /// Returns this struct root holding only the columns a path spells out.
    fn only_partition_fields(&self) -> PyResult<Self> {
        self.inner
            .only_partition_fields()
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Returns this struct root without the columns a path spells out.
    fn without_partition_fields(&self) -> PyResult<Self> {
        self.inner
            .without_partition_fields()
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Returns this struct root with the named children marked as partitions.
    fn with_partition_fields(&self, names: &Bound<'_, PyAny>) -> PyResult<Self> {
        let names = names.extract::<Vec<String>>()?;
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        self.inner
            .with_partition_fields(&names)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Compare recursively, optionally ignoring metadata on every Field.
    #[pyo3(signature = (other, with_metadata=true))]
    fn equals(&self, other: &Self, with_metadata: bool) -> bool {
        self.inner.equals(&other.inner, with_metadata)
    }

    /// Iterate stable, terminal-readable recursive difference lines.
    #[pyo3(signature = (other, with_metadata=true, return_equal=false))]
    fn show_diffs(
        &self,
        other: &Self,
        with_metadata: bool,
        return_equal: bool,
    ) -> PyDifferenceIterator {
        PyDifferenceIterator::from_fields(&self.inner, &other.inner, with_metadata, return_equal)
    }

    /// Join every recursive difference line.
    ///
    /// ``return_equal`` reports the equal marker instead of an empty
    /// string when the two values match.
    #[pyo3(signature = (other, with_metadata=true, return_equal=true))]
    fn show_diff(&self, other: &Self, with_metadata: bool, return_equal: bool) -> String {
        self.inner
            .show_diff(&other.inner, with_metadata, return_equal)
    }

    /// Makes a cached class field immutable at the Python boundary.
    fn _freeze(&mut self) {
        self.read_only = true;
    }

    /// A live mapping view of this field's metadata.
    ///
    /// Item access on a `Field` reaches a nested **child**, so this view is
    /// where metadata is reached - `field.metadata["owner"]` - and it carries
    /// the whole mapping protocol (`get`, `keys`, `values`, `items`, `update`,
    /// `clear`). The view is live rather than a snapshot: it holds
    /// this field and reads through it on every call, so a write on the field
    /// is visible in the view and a write through the view is visible on the
    /// field.
    #[getter]
    fn metadata(slf: Py<Self>) -> PyFieldMetadata {
        PyFieldMetadata { field: slf }
    }

    /// Reach a nested child by name or by position.
    ///
    /// Item access on a schema node means a **child**, never metadata: a `str`
    /// is a child name and raises `KeyError` when absent, an `int` is a
    /// position counting from the end when negative and raising `IndexError`
    /// when out of range, and anything else raises `TypeError`. `DataType`
    /// carries exactly the same behavior, so a caller walking one object graph
    /// gets a child from every node in it. Metadata is reached through
    /// `field.metadata[...]` or `field.get_metadata(...)`.
    ///
    /// Chained subscripts descend: `row["order"]["price"]`. There is no dotted
    /// path form.
    /// Returns the field that describes both this one and `other`.
    ///
    /// The datatype is `DataType.merge_with`'s answer; this adds the name
    /// (kept from the receiver), nullability (either side being nullable
    /// carries over), and metadata (the union, this field winning a clash).
    #[pyo3(signature = (other, upscale=true))]
    fn merge_with(&self, other: &Bound<'_, PyAny>, upscale: bool) -> PyResult<Self> {
        let other = core_field_from_value(other)?;
        self.inner
            .merge_with(&other, upscale)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Every leaf under this node, named by its dotted path.
    ///
    /// Struct nesting flattens all the way down, and a leaf under a nullable
    /// ancestor is nullable. Collections are leaves: a list or a map is one
    /// column, and `explode_fields` is what reaches inside one. Every name
    /// this answers is one `field_by_path` resolves.
    fn unnest_fields(&self) -> Vec<Self> {
        self.inner
            .unnest_fields()
            .into_iter()
            .map(Self::from_inner)
            .collect()
    }

    /// This node's children with every collection replaced by what it holds.
    ///
    /// A list answers its item, a map its entries, a dictionary or run-end
    /// node the values it encodes, and anything else itself - so the result
    /// names the same columns in the same order. One level only, so the depth
    /// is the caller's decision.
    fn explode_fields(&self) -> Vec<Self> {
        self.inner
            .explode_fields()
            .into_iter()
            .map(Self::from_inner)
            .collect()
    }

    /// Returns the nested child at `index`, or `None`.
    ///
    /// Negative positions count from the end, as everywhere else.
    fn get_field_at(&self, index: isize) -> Option<Self> {
        crate::field_at_of(self.inner.dtype(), index)
            .ok()
            .map(|field| Self::from_inner_with_read_only(field, self.read_only))
    }

    /// Returns the nested child `path` resolves to, or `None`.
    ///
    /// A child carrying the whole string wins before the string is decomposed
    /// on `.`, so a name containing a dot stays reachable.
    fn get_field_by_path(&self, path: &str) -> Option<Self> {
        self.inner
            .get_field_by_path(path)
            .cloned()
            .map(|field| Self::from_inner_with_read_only(field, self.read_only))
    }

    /// Returns the nested child a position or a path names, or `None`.
    #[pyo3(signature = (key=None, *, idx=None, path=None))]
    fn get_field(
        &self,
        key: Option<&Bound<'_, PyAny>>,
        idx: Option<isize>,
        path: Option<&str>,
    ) -> PyResult<Option<Self>> {
        let found = match crate::one_field_key(key, idx, path)? {
            crate::FieldKey::Path(path) => self.inner.get_field_by_path(&path).cloned(),
            crate::FieldKey::Position(index) => {
                crate::normalize_index(index, self.inner.field_len())
                    .and_then(|at| self.inner.get_field_at(at).cloned())
            }
        };
        Ok(found.map(|field| Self::from_inner_with_read_only(field, self.read_only)))
    }

    /// Returns the nested child at `index`.
    ///
    /// Raises `IndexError` when there is no child at that position.
    fn field_at(&self, index: isize) -> PyResult<Self> {
        crate::field_at_of(self.inner.dtype(), index)
            .map(|field| Self::from_inner_with_read_only(field, self.read_only))
    }

    /// Returns the nested child `path` resolves to.
    ///
    /// Raises `KeyError` when no child carries that name and no decomposition
    /// of it resolves.
    fn field_by_path(&self, path: &str) -> PyResult<Self> {
        crate::field_by_path_of(self.inner.dtype(), path)
            .map(|field| Self::from_inner_with_read_only(field, self.read_only))
    }

    /// Returns the nested child a position or a path names.
    ///
    /// The key may be positional, or named `idx=` or `path=`; naming more than
    /// one is a `TypeError` rather than a silent precedence rule.
    #[pyo3(signature = (key=None, *, idx=None, path=None))]
    fn field(
        &self,
        key: Option<&Bound<'_, PyAny>>,
        idx: Option<isize>,
        path: Option<&str>,
    ) -> PyResult<Self> {
        let resolved = match crate::one_field_key(key, idx, path)? {
            crate::FieldKey::Path(path) => crate::field_by_path_of(self.inner.dtype(), &path),
            crate::FieldKey::Position(index) => crate::field_at_of(self.inner.dtype(), index),
        }?;
        Ok(Self::from_inner_with_read_only(resolved, self.read_only))
    }

    /// Replaces the nested child at `index`.
    fn set_field_at(&mut self, index: isize, child: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        let child = core_field_from_value(child)?;
        let position = crate::normalize_index(index, self.inner.field_len())
            .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err(index))?;
        self.inner
            .set_field_at(position, child)
            .map_err(value_error)
    }

    /// Replaces the nested child `path` resolves to, appending an unresolved
    /// name under it.
    fn set_field_by_path(&mut self, path: &str, child: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        let child = core_field_from_value(child)?;
        self.inner
            .set_field_by_path(path, child)
            .map_err(value_error)
    }

    /// Replaces the nested child a position or a path names.
    #[pyo3(signature = (key=None, child=None, *, idx=None, path=None))]
    fn set_field(
        &mut self,
        key: Option<&Bound<'_, PyAny>>,
        child: Option<&Bound<'_, PyAny>>,
        idx: Option<isize>,
        path: Option<&str>,
    ) -> PyResult<()> {
        let child = child.ok_or_else(|| PyTypeError::new_err("set_field() needs a child"))?;
        match crate::one_field_key(key, idx, path)? {
            crate::FieldKey::Path(path) => self.set_field_by_path(&path, child),
            crate::FieldKey::Position(index) => self.set_field_at(index, child),
        }
    }

    /// Removes and returns the nested child at `index`.
    fn remove_field_at(&mut self, index: isize) -> PyResult<Self> {
        self.require_mutable()?;
        let position = crate::normalize_index(index, self.inner.field_len())
            .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err(index))?;
        self.inner
            .remove_field_at(position)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Removes and returns the nested child `path` resolves to.
    fn remove_field_by_path(&mut self, path: &str) -> PyResult<Self> {
        self.require_mutable()?;
        self.inner
            .remove_field_by_path(path)
            .map(Self::from_inner)
            .map_err(|_| pyo3::exceptions::PyKeyError::new_err(path.to_owned()))
    }

    /// Removes and returns the nested child a position or a path names.
    #[pyo3(signature = (key=None, *, idx=None, path=None))]
    fn remove_field(
        &mut self,
        key: Option<&Bound<'_, PyAny>>,
        idx: Option<isize>,
        path: Option<&str>,
    ) -> PyResult<Self> {
        match crate::one_field_key(key, idx, path)? {
            crate::FieldKey::Path(path) => self.remove_field_by_path(&path),
            crate::FieldKey::Position(index) => self.remove_field_at(index),
        }
    }

    fn __getitem__(&self, key: &Bound<'_, PyAny>) -> PyResult<Self> {
        crate::field_of(self.inner.dtype(), key)
            .map(|field| Self::from_inner_with_read_only(field, self.read_only))
    }

    /// Replace a nested child, or append one under an unknown name.
    ///
    /// By name: an existing child is replaced in place, keeping its position;
    /// an **unknown name appends** a new child, which is the natural way to
    /// build a schema up. By position: replaces only - a position past the end
    /// raises `IndexError` rather than growing the node silently.
    ///
    /// The assignment routes through the core's cache-aware child mutation, so
    /// the child set is revalidated and any Arrow projection cache is
    /// invalidated exactly once. No `&mut` child ever escapes to bypass it.
    fn __setitem__(&mut self, key: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        let child = core_field_from_value(value)?;
        match crate::FieldKey::from_py(key)? {
            crate::FieldKey::Path(path) => self
                .inner
                .set_field_by_path(&path, child)
                .map_err(value_error),
            crate::FieldKey::Position(index) => {
                let position = crate::normalize_index(index, self.inner.field_len())
                    .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err(index))?;
                self.inner
                    .set_field_at(position, child)
                    .map_err(value_error)
            }
        }
    }

    /// Remove a nested child by name or by position, closing the gap.
    fn __delitem__(&mut self, key: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        match crate::FieldKey::from_py(key)? {
            crate::FieldKey::Path(path) => self
                .inner
                .remove_field_by_path(&path)
                .map(|_| ())
                .map_err(|_| pyo3::exceptions::PyKeyError::new_err(path)),
            crate::FieldKey::Position(index) => {
                let position = crate::normalize_index(index, self.inner.field_len())
                    .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err(index))?;
                self.inner
                    .remove_field_at(position)
                    .map(|_| ())
                    .map_err(value_error)
            }
        }
    }

    /// The number of nested children, as `DataType` reports it.
    fn __len__(&self) -> usize {
        self.inner.field_len()
    }

    /// Iterate the nested children, as `DataType` does.
    fn __iter__(&self) -> PyDataTypeIterator {
        PyDataTypeIterator::over(self.inner.dtype().clone(), self.read_only)
    }

    /// Whether a child name, position, or field is among the children.
    fn __contains__(&self, value: &Bound<'_, PyAny>) -> bool {
        crate::field_of(self.inner.dtype(), value).is_ok()
            || value.extract::<PyRef<'_, Self>>().is_ok_and(|field| {
                (0..self.inner.field_len())
                    .filter_map(|index| self.inner.get_field(index))
                    .any(|candidate| candidate == &field.inner)
            })
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Field.from_str({:?})", self.inner.to_string())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&mut self) -> isize {
        self.hash_locked = true;
        crate::python_hash(self.inner.stable_hash())
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (String, bool))> {
        let callable = py.get_type::<Self>().getattr("_from_pickle")?.unbind();
        Ok((callable, (self.inner.to_string(), self.read_only)))
    }

    fn __copy__(&self) -> Self {
        Self::from_inner_with_read_only(self.inner.clone(), self.read_only)
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        Self::from_inner_with_read_only(self.inner.clone(), self.read_only)
    }
}

/// One protocol's field properties, addressed by their bare names.
///
/// The value is a live view rather than a snapshot: it holds the Python
/// `Field` object and reaches the core through it on every call. A write
/// through the view is therefore visible on the field, a write on the field is
/// visible through the view, and two views of one field see each other's
/// writes. Every mutation goes through the field's own frozen-schema gate.
#[pyclass(name = "ProtocolField", module = "yggdryl._native")]
pub(crate) struct PyProtocolField {
    field: Py<PyField>,
    scheme: CoreScheme,
}

impl PyProtocolField {
    fn new(field: Py<PyField>, scheme: CoreScheme) -> Self {
        Self { field, scheme }
    }

    /// Borrows the viewed field for reading.
    fn borrow_field<'py>(&self, py: Python<'py>) -> PyResult<PyRef<'py, PyField>> {
        Ok(self.field.bind(py).try_borrow()?)
    }

    /// Borrows the viewed field for writing, refusing a frozen one.
    fn borrow_field_mut<'py>(&self, py: Python<'py>) -> PyResult<PyRefMut<'py, PyField>> {
        let field = self.field.bind(py).try_borrow_mut()?;
        field.require_mutable()?;
        Ok(field)
    }

    /// Refuse a typed FIX property on a view of another protocol.
    ///
    /// The typed vocabulary belongs to one protocol, so it is answered only
    /// by the view `field.fix` returns; every other scheme reads and writes
    /// its own properties through the mapping protocol above.
    fn require_fix(&self, property: &str) -> PyResult<()> {
        if self.scheme == CoreScheme::FIX {
            return Ok(());
        }
        Err(PyTypeError::new_err(format!(
            "{property} is a fix property, and this is a {} view",
            self.scheme.as_str()
        )))
    }
}

#[pymethods]
impl PyProtocolField {
    // A view of mutable metadata cannot promise a stable hash, so it is
    // unhashable for the same reason an explicitly mutated MediaType is.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// Returns the protocol this view remembers.
    #[getter]
    fn scheme(&self) -> &str {
        self.scheme.as_str()
    }

    /// Returns the canonical key prefix this view applies.
    #[getter]
    fn prefix(&self, py: Python<'_>) -> PyResult<String> {
        let field = self.borrow_field(py)?;
        Ok(field.inner.protocol(&self.scheme).prefix().to_owned())
    }

    /// Returns the full metadata key one property name is stored under.
    fn key(&self, py: Python<'_>, name: &str) -> PyResult<String> {
        let field = self.borrow_field(py)?;
        Ok(field.inner.protocol(&self.scheme).key(name))
    }

    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        let field = self.borrow_field(py)?;
        Ok(field.inner.protocol(&self.scheme).len())
    }

    /// Answers emptiness without counting the properties `len` would count.
    fn __bool__(&self, py: Python<'_>) -> PyResult<bool> {
        let field = self.borrow_field(py)?;
        Ok(!field.inner.protocol(&self.scheme).is_empty())
    }

    fn __iter__(&self, py: Python<'_>) -> PyResult<PyFieldPropertyIterator> {
        self.keys(py)
    }

    fn __contains__(&self, py: Python<'_>, name: &Bound<'_, PyAny>) -> PyResult<bool> {
        let Ok(name) = name.extract::<&str>() else {
            return Ok(false);
        };
        let field = self.borrow_field(py)?;
        Ok(field.inner.protocol(&self.scheme).contains_key(name))
    }

    fn __getitem__(&self, py: Python<'_>, name: &str) -> PyResult<String> {
        let field = self.borrow_field(py)?;
        let protocol = field.inner.protocol(&self.scheme);
        protocol
            .get(name)
            .map(str::to_owned)
            // The core names a missing property by the key it is stored under,
            // which is also the key a caller reads on the field itself.
            .ok_or_else(|| PyKeyError::new_err(protocol.key(name)))
    }

    fn __setitem__(&self, py: Python<'_>, name: &str, value: String) -> PyResult<()> {
        let mut field = self.borrow_field_mut(py)?;
        field
            .inner
            .protocol_mut(&self.scheme)
            .insert(name, value)
            .map(|_| ())
            .map_err(value_error)
    }

    fn __delitem__(&self, py: Python<'_>, name: &str) -> PyResult<()> {
        let mut field = self.borrow_field_mut(py)?;
        let mut protocol = field.inner.protocol_mut(&self.scheme);
        let key = protocol.key(name);
        protocol
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| PyKeyError::new_err(key))
    }

    #[pyo3(signature = (name, default=None, /))]
    fn get(&self, py: Python<'_>, name: &str, default: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
        let field = self.borrow_field(py)?;
        Ok(field.inner.protocol(&self.scheme).get(name).map_or_else(
            || default.unwrap_or_else(|| py.None()),
            |value| PyString::new(py, value).into_any().unbind(),
        ))
    }

    fn keys(&self, py: Python<'_>) -> PyResult<PyFieldPropertyIterator> {
        let field = self.borrow_field(py)?;
        Ok(PyFieldPropertyIterator::new(
            &field.inner,
            self.scheme.clone(),
            PropertyIteratorKind::Names,
        ))
    }

    fn values(&self, py: Python<'_>) -> PyResult<PyFieldPropertyIterator> {
        let field = self.borrow_field(py)?;
        Ok(PyFieldPropertyIterator::new(
            &field.inner,
            self.scheme.clone(),
            PropertyIteratorKind::Values,
        ))
    }

    fn items(&self, py: Python<'_>) -> PyResult<PyFieldPropertyIterator> {
        let field = self.borrow_field(py)?;
        Ok(PyFieldPropertyIterator::new(
            &field.inner,
            self.scheme.clone(),
            PropertyIteratorKind::Items,
        ))
    }

    /// This protocol's comment, falling back to the field's straight one.
    ///
    /// `get`, iteration and `len` stay literal about what this protocol
    /// carries; the fallback lives here so a view never reports a property
    /// that iterating it would not yield.
    #[getter]
    fn comment(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let field = self.borrow_field(py)?;
        Ok(field
            .inner
            .protocol(&self.scheme)
            .comment()
            .map(str::to_owned))
    }

    /// This protocol's display name, falling back to the field's straight one.
    #[getter]
    fn display(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let field = self.borrow_field(py)?;
        Ok(field
            .inner
            .protocol(&self.scheme)
            .display()
            .map(str::to_owned))
    }

    /// The dictionary this field belongs to, on the `fix` view.
    ///
    /// A branch crosses as text: `"standard"` is the FIX specification's own
    /// dictionary and what an absent `fix:branch` means, and assigning it
    /// removes the key rather than storing it. A spelling that is not a branch
    /// is a `ValueError` carrying the native parse failure, and a refusal -
    /// a tag the specification assigns cannot move to another dictionary -
    /// leaves the field unchanged.
    #[getter]
    fn branch(&self, py: Python<'_>) -> PyResult<String> {
        self.require_fix("branch")?;
        let field = self.borrow_field(py)?;
        field
            .inner
            .as_fix()
            .branch()
            .map(|branch| branch.as_str().to_owned())
            .map_err(value_error)
    }

    #[setter]
    fn set_branch(&self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_fix("branch")?;
        let branch = branch_from_py(&value.extract::<String>()?)?;
        let mut field = self.borrow_field_mut(value.py())?;
        field
            .inner
            .as_fix_mut()
            .set_branch(&branch)
            .map_err(value_error)
    }

    /// This field's identity, `branch:tag`, on the `fix` view.
    ///
    /// Derived from the branch and the canonical tag on every read and never
    /// stored, so it is `None` exactly when `fix:tag` is absent. Assigning one
    /// moves both halves at once, which is the only ordering-safe way to move
    /// a field between dictionaries.
    #[getter]
    fn id(&self, py: Python<'_>) -> PyResult<Option<String>> {
        self.require_fix("id")?;
        let field = self.borrow_field(py)?;
        Ok(field
            .inner
            .as_fix()
            .id()
            .map_err(value_error)?
            .map(|id| id.to_string()))
    }

    #[setter]
    fn set_id(&self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_fix("id")?;
        let id = id_from_py(&value.extract::<String>()?)?;
        let mut field = self.borrow_field_mut(value.py())?;
        field.inner.as_fix_mut().set_id(&id).map_err(value_error)
    }

    /// The canonical FIX tag, on the `fix` view.
    ///
    /// Reads and writes `fix:tag` through the core's own typed accessors, so
    /// the property name is never spelled at a call site. `del view["tag"]`
    /// removes it, the way every other property is removed. A tag below
    /// `STANDARD_TAG_LIMIT` is the FIX specification's own, so a field in
    /// another branch cannot claim it.
    #[getter]
    fn tag(&self, py: Python<'_>) -> PyResult<Option<i32>> {
        self.require_fix("tag")?;
        let field = self.borrow_field(py)?;
        field.inner.as_fix().tag().map_err(value_error)
    }

    #[setter]
    fn set_tag(&self, tag: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_fix("tag")?;
        let value = tag.extract::<FixTag>()?;
        let mut field = self.borrow_field_mut(tag.py())?;
        field
            .inner
            .as_fix_mut()
            .set_tag(value.0)
            .map_err(value_error)
    }

    /// The alternate tags, highest priority first.
    ///
    /// An absent property is an empty list, and assigning an empty iterable
    /// removes it: a field states alternate tags only when it has them.
    #[getter]
    fn tags(&self, py: Python<'_>) -> PyResult<Vec<i32>> {
        self.require_fix("tags")?;
        let field = self.borrow_field(py)?;
        field.inner.as_fix().tags().map_err(value_error)
    }

    #[setter]
    fn set_tags(&self, tags: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_fix("tags")?;
        let mut parsed = Vec::new();
        for value in tags.try_iter()? {
            parsed.push(value?.extract::<FixTag>()?.0);
        }
        let mut field = self.borrow_field_mut(tags.py())?;
        field
            .inner
            .as_fix_mut()
            .set_tags(&parsed)
            .map_err(value_error)
    }

    /// The alternate names, highest priority first.
    ///
    /// Assigning an empty iterable removes the property.
    #[getter]
    fn aliases(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        self.require_fix("aliases")?;
        let field = self.borrow_field(py)?;
        Ok(field.inner.as_fix().aliases().map(str::to_owned).collect())
    }

    #[setter]
    fn set_aliases(&self, aliases: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_fix("aliases")?;
        let mut parsed = Vec::new();
        for value in aliases.try_iter()? {
            parsed.push(value?.extract::<String>()?);
        }
        let mut field = self.borrow_field_mut(aliases.py())?;
        field
            .inner
            .as_fix_mut()
            .set_aliases(parsed)
            .map_err(value_error)
    }

    /// The specification's own wording for this field.
    #[getter]
    fn description(&self, py: Python<'_>) -> PyResult<Option<String>> {
        self.require_fix("description")?;
        let field = self.borrow_field(py)?;
        Ok(field.inner.as_fix().description().map(str::to_owned))
    }

    #[setter]
    fn set_description(&self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_fix("description")?;
        let text = value.extract::<String>()?;
        let mut field = self.borrow_field_mut(value.py())?;
        field
            .inner
            .as_fix_mut()
            .set_description(text)
            .map_err(value_error)
    }

    /// Merges another protocol view's properties into this one, in place.
    ///
    /// A name this view already carries keeps its value, so the merge only
    /// ever adds. Properties of other protocols are untouched.
    fn merge_with(&self, py: Python<'_>, other: &Self) -> PyResult<()> {
        // Collect before borrowing mutably: `other` may be a view of this same
        // field, and a live borrow of it would collide with the write below.
        let additions: Vec<(String, String)> = {
            let source = other.borrow_field(py)?;
            let held = self.borrow_field(py)?;
            let view = held.inner.protocol(&self.scheme);
            source
                .inner
                .protocol(&other.scheme)
                .iter()
                .filter(|(name, _)| view.get(name).is_none())
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect()
        };
        let mut field = self.borrow_field_mut(py)?;
        field
            .inner
            .protocol_mut(&self.scheme)
            .update(additions)
            .map_err(value_error)
    }

    #[pyo3(signature = (values=None, /, **kwargs))]
    fn update(
        &self,
        py: Python<'_>,
        values: Option<&Bound<'_, PyAny>>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        // Collect before borrowing: the overlay may itself be a view of this
        // same field, and the core validates the whole overlay atomically.
        let mut pairs = BTreeMap::new();
        if let Some(values) = values {
            extend_metadata_pairs(values, &mut pairs)?;
        }
        if let Some(kwargs) = kwargs {
            for (key, value) in kwargs.iter() {
                pairs.insert(key.extract()?, value.extract()?);
            }
        }
        let mut field = self.borrow_field_mut(py)?;
        field
            .inner
            .protocol_mut(&self.scheme)
            .update(pairs)
            .map_err(value_error)
    }

    fn clear(&self, py: Python<'_>) -> PyResult<()> {
        let mut field = self.borrow_field_mut(py)?;
        field.inner.protocol_mut(&self.scheme).clear();
        Ok(())
    }

    fn __str__(&self, py: Python<'_>) -> PyResult<String> {
        let field = self.borrow_field(py)?;
        Ok(field.inner.protocol(&self.scheme).to_string())
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let field = self.borrow_field(py)?;
        Ok(format!("{:?}", field.inner.protocol(&self.scheme)))
    }

    /// Compares the properties two views hold, not the fields behind them.
    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(py.NotImplemented());
        };
        let left = self.borrow_field(py)?;
        let right = other.borrow_field(py)?;
        let equal = left.inner.protocol(&self.scheme) == right.inner.protocol(&other.scheme);
        Ok(PyBool::new(py, equal).to_owned().into_any().unbind())
    }
}

#[derive(Clone, Copy)]
enum MetadataIteratorKind {
    Keys,
    Values,
    Items,
}

#[derive(Clone, Copy)]
enum PropertyIteratorKind {
    Names,
    Values,
    Items,
}

/// Snapshot iterator over a field's sorted metadata.
#[pyclass(module = "yggdryl._native")]
pub(crate) struct PyFieldMetadataIterator {
    inner: CoreField,
    after_key: Option<String>,
    remaining: usize,
    kind: MetadataIteratorKind,
}

/// Snapshot iterator over one protocol's field properties.
#[pyclass(module = "yggdryl._native")]
pub(crate) struct PyFieldPropertyIterator {
    inner: CoreField,
    scheme: CoreScheme,
    after_name: Option<String>,
    remaining: usize,
    kind: PropertyIteratorKind,
}

impl PyFieldPropertyIterator {
    fn new(field: &CoreField, scheme: CoreScheme, kind: PropertyIteratorKind) -> Self {
        let remaining = field.property_iter(&scheme).count();
        Self {
            inner: field.clone(),
            scheme,
            after_name: None,
            remaining,
            kind,
        }
    }
}

#[pymethods]
impl PyFieldPropertyIterator {
    // Consumption changes iterator state.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        let Some((name, value)) = self
            .inner
            .next_property_entry(&self.scheme, self.after_name.as_deref())
        else {
            self.remaining = 0;
            return Ok(None);
        };
        let name = name.to_owned();
        let output = match self.kind {
            PropertyIteratorKind::Names => PyString::new(py, &name).into_any().unbind(),
            PropertyIteratorKind::Values => PyString::new(py, value).into_any().unbind(),
            PropertyIteratorKind::Items => (name.as_str(), value)
                .into_pyobject(py)?
                .into_any()
                .unbind(),
        };
        self.after_name = Some(name);
        self.remaining = self.remaining.saturating_sub(1);
        Ok(Some(output))
    }

    fn __length_hint__(&self) -> usize {
        self.remaining
    }
}

impl PyFieldMetadataIterator {
    fn new(field: &CoreField, kind: MetadataIteratorKind) -> Self {
        Self {
            inner: field.clone(),
            after_key: None,
            remaining: field.metadata_len(),
            kind,
        }
    }
}

#[pymethods]
impl PyFieldMetadataIterator {
    // Consumption changes iterator state.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        let Some((key, value)) = self.inner.next_metadata_entry(self.after_key.as_deref()) else {
            self.remaining = 0;
            return Ok(None);
        };
        let key = key.to_owned();
        let output = match self.kind {
            MetadataIteratorKind::Keys => PyString::new(py, &key).into_any().unbind(),
            MetadataIteratorKind::Values => PyString::new(py, value).into_any().unbind(),
            MetadataIteratorKind::Items => {
                (key.as_str(), value).into_pyobject(py)?.into_any().unbind()
            }
        };
        self.after_key = Some(key);
        self.remaining = self.remaining.saturating_sub(1);
        Ok(Some(output))
    }

    fn __length_hint__(&self) -> usize {
        self.remaining
    }
}

/// A field's metadata, addressed by its keys.
///
/// This is where item syntax legitimately means "a key": every subscript on the
/// view is a metadata key, while every subscript on the `Field` itself is a
/// nested child. Reached as `field.metadata`.
///
/// The value is a live view rather than a snapshot: it holds the Python `Field`
/// object and reaches the core through it on every call, so writes are visible
/// in both directions. Mutation routes through the field's own cache-aware
/// metadata methods.
#[pyclass(name = "FieldMetadata", module = "yggdryl._native")]
pub(crate) struct PyFieldMetadata {
    field: Py<PyField>,
}

impl PyFieldMetadata {
    /// Borrows the viewed field for reading.
    fn borrow_field<'py>(&self, py: Python<'py>) -> PyResult<PyRef<'py, PyField>> {
        Ok(self.field.bind(py).try_borrow()?)
    }

    /// Borrows the viewed field for writing, refusing a frozen one.
    fn borrow_field_mut<'py>(&self, py: Python<'py>) -> PyResult<PyRefMut<'py, PyField>> {
        let field = self.field.bind(py).try_borrow_mut()?;
        field.require_mutable()?;
        Ok(field)
    }
}

#[pymethods]
impl PyFieldMetadata {
    // A live view of mutable metadata cannot promise a stable hash.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        Ok(self.borrow_field(py)?.inner.metadata_len())
    }

    fn __bool__(&self, py: Python<'_>) -> PyResult<bool> {
        Ok(!self.borrow_field(py)?.inner.is_metadata_empty())
    }

    fn __iter__(&self, py: Python<'_>) -> PyResult<PyFieldMetadataIterator> {
        self.keys(py)
    }

    fn __contains__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<bool> {
        let Ok(key) = key.extract::<&str>() else {
            return Ok(false);
        };
        Ok(self.borrow_field(py)?.inner.has_metadata(key))
    }

    fn __getitem__(&self, py: Python<'_>, key: &str) -> PyResult<String> {
        self.borrow_field(py)?
            .inner
            .get_metadata(key)
            .map(str::to_owned)
            .ok_or_else(|| PyKeyError::new_err(key.to_owned()))
    }

    fn __setitem__(&self, py: Python<'_>, key: String, value: String) -> PyResult<()> {
        self.borrow_field_mut(py)?
            .inner
            .insert_metadata(key, value)
            .map(|_| ())
            .map_err(value_error)
    }

    fn __delitem__(&self, py: Python<'_>, key: &str) -> PyResult<()> {
        self.borrow_field_mut(py)?
            .inner
            .remove_metadata(key)
            .map(|_| ())
            .ok_or_else(|| PyKeyError::new_err(key.to_owned()))
    }

    #[pyo3(signature = (key, default=None, /))]
    fn get(&self, py: Python<'_>, key: &str, default: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
        let field = self.borrow_field(py)?;
        Ok(field.inner.get_metadata(key).map_or_else(
            || default.unwrap_or_else(|| py.None()),
            |value| PyString::new(py, value).into_any().unbind(),
        ))
    }

    fn keys(&self, py: Python<'_>) -> PyResult<PyFieldMetadataIterator> {
        let field = self.borrow_field(py)?;
        Ok(PyFieldMetadataIterator::new(
            &field.inner,
            MetadataIteratorKind::Keys,
        ))
    }

    fn values(&self, py: Python<'_>) -> PyResult<PyFieldMetadataIterator> {
        let field = self.borrow_field(py)?;
        Ok(PyFieldMetadataIterator::new(
            &field.inner,
            MetadataIteratorKind::Values,
        ))
    }

    fn items(&self, py: Python<'_>) -> PyResult<PyFieldMetadataIterator> {
        let field = self.borrow_field(py)?;
        Ok(PyFieldMetadataIterator::new(
            &field.inner,
            MetadataIteratorKind::Items,
        ))
    }

    #[pyo3(signature = (values=None, /, **kwargs))]
    fn update(
        &self,
        py: Python<'_>,
        values: Option<&Bound<'_, PyAny>>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let mut pairs = BTreeMap::new();
        if let Some(values) = values {
            extend_metadata_pairs(values, &mut pairs)?;
        }
        if let Some(kwargs) = kwargs {
            for (key, value) in kwargs.iter() {
                pairs.insert(key.extract()?, value.extract()?);
            }
        }
        self.borrow_field_mut(py)?
            .inner
            .update_metadata(pairs)
            .map_err(value_error)
    }

    fn clear(&self, py: Python<'_>) -> PyResult<()> {
        self.borrow_field_mut(py)?.inner.clear_metadata();
        Ok(())
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let field = self.borrow_field(py)?;
        let entries: Vec<String> = field
            .inner
            .metadata_iter()
            .map(|(key, value)| format!("{key:?}: {value:?}"))
            .collect();
        Ok(format!("FieldMetadata({{{}}})", entries.join(", ")))
    }

    /// Compare the metadata entries, independently of the fields behind them.
    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(py.NotImplemented());
        };
        let left = self.borrow_field(py)?;
        let right = other.borrow_field(py)?;
        let equal = left.inner.metadata_iter().eq(right.inner.metadata_iter());
        Ok(PyBool::new(py, equal).to_owned().into_any().unbind())
    }
}
