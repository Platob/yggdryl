//! Native Python view of Yggdryl fields and metadata.

use std::collections::BTreeMap;
use std::sync::Arc;

use arrow_array::RecordBatch as ArrowRecordBatch;
use arrow_pyarrow::{FromPyArrow, PyArrowType, ToPyArrow};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema as ArrowSchema};
use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyDict, PyString};
use yggdryl::ArrowCast;
use yggdryl::{DataType as CoreDataType, Field as CoreField, Scheme as CoreScheme};

use crate::datatype::{
    PyDataType, PyDataTypeIterator, arrow_array_from_pyarrow, arrow_array_to_pyarrow,
    arrow_scalar_to_pyarrow_type, core_data_type_from_value, core_field_to_pyarrow,
    default_arrow_scalar_to_pyarrow,
};
use crate::media::{
    PyMediaType, PyMimeType, core_media_type_from_value, core_mime_type_from_value,
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

    let imported: PyResult<CoreField> = (|| {
        let arrow_field = ArrowField::from_pyarrow_bound(value)?;
        let mut field = CoreField::try_from(arrow_field).map_err(value_error)?;

        // PyArrow's Field C Schema bridge can omit datatype-only flags such as
        // Map.keys_sorted. Its standalone datatype bridge is lossless, so use
        // that authoritative type when it differs from the field projection.
        let py_data_type = value.getattr("type")?;
        let arrow_data_type = ArrowDataType::from_pyarrow_bound(&py_data_type)?;
        let data_type = CoreDataType::try_from(arrow_data_type).map_err(value_error)?;
        if field.data_type() != &data_type {
            field = field.try_with_data_type(data_type).map_err(value_error)?;
        }
        Ok(field)
    })();
    imported.map_err(|error| {
        if error.is_instance_of::<PyTypeError>(value.py()) {
            PyTypeError::new_err("expected a yggdryl.Field, field string, or PyArrow Field")
        } else {
            error
        }
    })
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
        Self {
            inner,
            read_only: false,
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
                let table = crate::record::polars_to_arrow(&frame)?;
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
            Err(PyTypeError::new_err(
                "frozen record schema fields are read-only",
            ))
        } else {
            Ok(())
        }
    }
}

#[pymethods]
impl PyField {
    #[new]
    #[pyo3(signature = (name, data_type, nullable=true, metadata=None))]
    fn new(
        name: String,
        data_type: &Bound<'_, PyAny>,
        nullable: bool,
        metadata: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let data_type = core_data_type_from_value(data_type)?;
        if let Some(metadata) = metadata {
            let mut pairs = BTreeMap::new();
            extend_metadata_pairs(metadata, &mut pairs)?;
            return CoreField::from_parts(name, data_type, nullable, pairs)
                .map(Self::from_inner)
                .map_err(value_error);
        }
        let field = CoreField::new(name, data_type, nullable);
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
        let module = hint.py().import("yggdryl.records._hints")?;
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

    #[staticmethod]
    fn from_arrow(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        core_field_from_value(value).map(Self::from_inner)
    }

    /// Imports one complete Arrow Schema through Yggdryl's Arrow IPC metadata
    /// rules. This is private glue for the Python record class factory.
    #[staticmethod]
    fn _record_root_from_arrow_schema(
        value: PyArrowType<ArrowSchema>,
        name: &str,
    ) -> PyResult<Self> {
        let PyArrowType(schema) = value;
        yggdryl::arrow::record_schema_from_arrow(name, &schema)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Projects a complete record root as an Arrow transport Schema while
    /// keeping the reserved dictionary-ID sidecar out of Field metadata.
    fn _record_root_to_arrow_transport_schema(&self) -> PyResult<PyArrowType<ArrowSchema>> {
        yggdryl::arrow::record_schema_to_arrow(&self.inner)
            .map(PyArrowType)
            .map_err(value_error)
    }

    #[staticmethod]
    fn from_json(value: &str) -> PyResult<Self> {
        CoreField::from_json(value)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    fn to_arrow<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        core_field_to_pyarrow(py, &self.inner)
    }

    /// Returns the cached Python annotation corresponding to this Field.
    fn default_pyhint<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let field = Py::new(py, self.clone())?;
        py.import("yggdryl.records._defaults")?
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
        py.import("yggdryl.records._defaults")?
            .getattr("_default_pyvalue_from_field")?
            .call1((field, scalar))
    }

    /// Returns a recursively normalized field for a named compatibility target.
    fn to_scheme_compat(&self, target: &str) -> PyResult<Self> {
        let target = CoreScheme::from_str(target).map_err(value_error)?;
        self.inner
            .to_scheme_compat(&target)
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
        // Project the complete Field so registered extension metadata can be
        // rehydrated by PyArrow before selecting its scalar target type.
        let arrow_field = core_field_to_pyarrow(py, &self.inner)?;
        let target = arrow_field.getattr("type")?;
        let scalar = arrow_scalar_to_pyarrow_type(py, value, target, safe)?;
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

    /// The full name of [`cast_arrow_batch`](Self::cast_arrow_batch).
    #[pyo3(signature = (value, *, safe=true))]
    fn cast_arrow_record_batch<'py>(
        &self,
        py: Python<'py>,
        value: &Bound<'py, PyAny>,
        safe: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.cast_arrow_batch(py, value, safe)
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
        use crate::record::declared_by;

        // polars: the frame comes back a frame, the lazy frame stays lazy.
        if declared_by(value, "polars", "DataFrame") {
            let table = crate::record::polars_to_arrow(value)?;
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
        let reader = crate::record::batch_reader_from_any(
            value,
            &yggdryl::generic::RecordOptions::for_mime_type(&yggdryl::MimeType::ARROW_STREAM)
                .map_err(value_error)?,
        )?;
        let cast = yggdryl::arrow::cast_reader(reader, &self.inner, safe).map_err(value_error)?;
        crate::record::batch_reader_to_pyarrow(py, cast)
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
        use crate::record::declared_by;

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
    #[pyo3(signature = (*, indent = None))]
    fn to_json(&self, indent: Option<u8>) -> PyResult<String> {
        self.inner
            .to_json_with_formatting(crate::formatting_of(indent))
            .map_err(value_error)
    }

    /// Consume and serialize as structural JSON.
    #[allow(clippy::wrong_self_convention)]
    #[pyo3(signature = (*, indent = None))]
    fn into_json(&self, indent: Option<u8>) -> PyResult<String> {
        self.to_json(indent)
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
    #[pyo3(signature = (*, indent = Some(2)))]
    fn to_yaml(&self, indent: Option<u8>) -> PyResult<String> {
        self.inner
            .to_yaml_with_formatting(crate::formatting_of(indent))
            .map_err(value_error)
    }

    /// Consume and serialize as YAML.
    #[allow(clippy::wrong_self_convention)]
    #[pyo3(signature = (*, indent = Some(2)))]
    fn into_yaml(&self, indent: Option<u8>) -> PyResult<String> {
        self.to_yaml(indent)
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
    #[pyo3(signature = (*, indent = None))]
    fn to_toml(&self, indent: Option<u8>) -> PyResult<String> {
        self.inner
            .to_toml_with_formatting(crate::formatting_of(indent))
            .map_err(value_error)
    }

    /// Consume and serialize as TOML.
    #[allow(clippy::wrong_self_convention)]
    #[pyo3(signature = (*, indent = None))]
    fn into_toml(&self, indent: Option<u8>) -> PyResult<String> {
        self.to_toml(indent)
    }

    /// Project this value onto a plain structural mapping.
    ///
    /// The core's one structural model - the model JSON, YAML, and TOML are
    /// all expressed over - handed back as a `dict`, so a schema drops into any
    /// document a caller already builds. Spelled `to_dict` rather than
    /// `to_value` because `from_value` is already this module's
    /// boundary-inference entry point and a Python caller reads a `dict` here.
    fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        crate::value::as_py(py, &self.inner.to_value())
    }

    /// Read this value back from a plain structural mapping.
    ///
    /// The inverse of `to_dict`, through the core's one conversion.
    #[staticmethod]
    fn from_dict(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        CoreField::from_value(crate::value::from_py(value)?)
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

    /// The readable rendering, for IPython and notebook cells.
    fn _repr_pretty_(&self, printer: &Bound<'_, PyAny>, _cycle: bool) -> PyResult<()> {
        printer.call_method1("text", (self.pretty(),))?;
        Ok(())
    }

    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    #[getter]
    fn data_type(&self) -> PyDataType {
        PyDataType {
            inner: self.inner.data_type().clone(),
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
    fn catalog_name(&self) -> Option<&str> {
        self.inner.catalog_name()
    }

    #[getter]
    fn schema_name(&self) -> Option<&str> {
        self.inner.schema_name()
    }

    #[getter]
    fn table_name(&self) -> Option<&str> {
        self.inner.table_name()
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
        self.inner.accept()
    }

    #[getter]
    fn accept_encoding(&self) -> Option<&str> {
        self.inner.accept_encoding()
    }

    #[getter]
    fn accept_language(&self) -> Option<&str> {
        self.inner.accept_language()
    }

    #[getter]
    fn accept_ranges(&self) -> Option<&str> {
        self.inner.accept_ranges()
    }

    #[getter]
    fn cache_control(&self) -> Option<&str> {
        self.inner.cache_control()
    }

    #[getter]
    fn content_disposition(&self) -> Option<&str> {
        self.inner.content_disposition()
    }

    #[getter]
    fn content_encoding(&self) -> Option<&str> {
        self.inner.content_encoding()
    }

    #[getter]
    fn content_language(&self) -> Option<&str> {
        self.inner.content_language()
    }

    #[getter]
    fn content_length(&self) -> PyResult<Option<u64>> {
        self.inner.content_length().map_err(value_error)
    }

    #[getter]
    fn content_location(&self) -> Option<&str> {
        self.inner.content_location()
    }

    #[getter]
    fn content_range(&self) -> Option<&str> {
        self.inner.content_range()
    }

    #[getter]
    fn content_type(&self) -> Option<&str> {
        self.inner.content_type()
    }

    #[getter]
    fn mime_type(&self) -> PyResult<PyMimeType> {
        self.inner
            .mime_type()
            .map(PyMimeType::from_core)
            .map_err(value_error)
    }

    #[getter]
    fn media_type(&self) -> PyResult<PyMediaType> {
        self.inner
            .media_type()
            .map(PyMediaType::from_core)
            .map_err(value_error)
    }

    #[getter]
    fn etag(&self) -> Option<&str> {
        self.inner.etag()
    }

    #[getter]
    fn expires(&self) -> Option<&str> {
        self.inner.expires()
    }

    #[getter]
    fn last_modified(&self) -> Option<&str> {
        self.inner.last_modified()
    }

    #[getter]
    fn http_location(&self) -> PyResult<Option<PyUrl>> {
        self.inner
            .http_location()
            .map(|value| value.map(PyUrl::from_core))
            .map_err(value_error)
    }

    #[getter]
    fn range(&self) -> Option<&str> {
        self.inner.range()
    }

    #[getter]
    fn vary(&self) -> Option<&str> {
        self.inner.vary()
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

    fn set_alias(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_alias(value).map_err(value_error)
    }

    fn remove_alias(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_alias())
    }

    fn set_catalog_name(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_catalog_name(value).map_err(value_error)
    }

    fn remove_catalog_name(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_catalog_name())
    }

    fn set_schema_name(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_schema_name(value).map_err(value_error)
    }

    fn remove_schema_name(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_schema_name())
    }

    fn set_table_name(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_table_name(value).map_err(value_error)
    }

    fn remove_table_name(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_table_name())
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
        self.inner.set_accept(value).map_err(value_error)
    }

    fn remove_accept(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_accept())
    }

    fn set_accept_encoding(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_accept_encoding(value).map_err(value_error)
    }

    fn remove_accept_encoding(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_accept_encoding())
    }

    fn set_accept_language(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_accept_language(value).map_err(value_error)
    }

    fn remove_accept_language(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_accept_language())
    }

    fn set_accept_ranges(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_accept_ranges(value).map_err(value_error)
    }

    fn remove_accept_ranges(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_accept_ranges())
    }

    fn set_cache_control(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_cache_control(value).map_err(value_error)
    }

    fn remove_cache_control(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_cache_control())
    }

    fn set_content_disposition(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_content_disposition(value)
            .map_err(value_error)
    }

    fn remove_content_disposition(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_content_disposition())
    }

    fn set_content_encoding(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_content_encoding(value).map_err(value_error)
    }

    fn remove_content_encoding(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_content_encoding())
    }

    fn set_content_language(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_content_language(value).map_err(value_error)
    }

    fn remove_content_language(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_content_language())
    }

    fn set_content_length(&mut self, value: ContentLength) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_content_length(value.0);
        Ok(())
    }

    fn remove_content_length(&mut self) -> PyResult<Option<u64>> {
        self.require_mutable()?;
        self.inner.remove_content_length().map_err(value_error)
    }

    fn set_content_location(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_content_location(value).map_err(value_error)
    }

    fn remove_content_location(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_content_location())
    }

    fn set_content_range(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_content_range(value).map_err(value_error)
    }

    fn remove_content_range(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_content_range())
    }

    fn set_content_type(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_content_type(value).map_err(value_error)
    }

    fn remove_content_type(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_content_type())
    }

    fn set_mime_type(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_mime_type(core_mime_type_from_value(value)?);
        Ok(())
    }

    fn remove_mime_type(&mut self) -> PyResult<Option<PyMimeType>> {
        self.require_mutable()?;
        self.inner
            .remove_mime_type()
            .map(|value| value.map(PyMimeType::from_core))
            .map_err(value_error)
    }

    fn set_media_type(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_media_type(core_media_type_from_value(value)?)
            .map_err(value_error)
    }

    fn remove_media_type(&mut self) -> PyResult<Option<PyMediaType>> {
        self.require_mutable()?;
        self.inner
            .remove_media_type()
            .map(|value| value.map(PyMediaType::from_core))
            .map_err(value_error)
    }

    fn set_etag(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_etag(value).map_err(value_error)
    }

    fn remove_etag(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_etag())
    }

    fn set_expires(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_expires(value).map_err(value_error)
    }

    fn remove_expires(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_expires())
    }

    fn set_last_modified(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_last_modified(value).map_err(value_error)
    }

    fn remove_last_modified(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_last_modified())
    }

    fn set_http_location(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_http_location(core_url_from_value(value)?);
        Ok(())
    }

    fn remove_http_location(&mut self) -> PyResult<Option<PyUrl>> {
        self.require_mutable()?;
        self.inner
            .remove_http_location()
            .map(|value| value.map(PyUrl::from_core))
            .map_err(value_error)
    }

    fn set_range(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_range(value).map_err(value_error)
    }

    fn remove_range(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_range())
    }

    fn set_vary(&mut self, value: String) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_vary(value).map_err(value_error)
    }

    fn remove_vary(&mut self) -> PyResult<Option<String>> {
        self.require_mutable()?;
        Ok(self.inner.remove_vary())
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
    fn protocol(slf: Py<Self>, scheme: &str) -> PyResult<PyProtocolMetadata> {
        let scheme = CoreScheme::from_str(scheme).map_err(value_error)?;
        Ok(PyProtocolMetadata::new(slf, scheme))
    }

    /// Returns the live HTTP and HTTPS property view.
    #[getter]
    fn http(slf: Py<Self>) -> PyProtocolMetadata {
        PyProtocolMetadata::new(slf, CoreScheme::HTTP)
    }

    /// Returns the live file protocol property view.
    #[getter]
    fn file(slf: Py<Self>) -> PyProtocolMetadata {
        PyProtocolMetadata::new(slf, CoreScheme::FILE)
    }

    /// Returns the live uniform resource name property view.
    #[getter]
    fn urn(slf: Py<Self>) -> PyProtocolMetadata {
        PyProtocolMetadata::new(slf, CoreScheme::URN)
    }

    /// Returns the live short-spelling `PostgreSQL` property view.
    #[getter]
    fn postgres(slf: Py<Self>) -> PyProtocolMetadata {
        PyProtocolMetadata::new(slf, CoreScheme::POSTGRES)
    }

    /// Returns the live long-spelling `PostgreSQL` property view.
    #[getter]
    fn postgresql(slf: Py<Self>) -> PyProtocolMetadata {
        PyProtocolMetadata::new(slf, CoreScheme::POSTGRESQL)
    }

    /// Returns the live `MySQL` property view.
    #[getter]
    fn mysql(slf: Py<Self>) -> PyProtocolMetadata {
        PyProtocolMetadata::new(slf, CoreScheme::MYSQL)
    }

    /// Returns the live Arrow property view.
    #[getter]
    fn arrow(slf: Py<Self>) -> PyProtocolMetadata {
        PyProtocolMetadata::new(slf, CoreScheme::ARROW)
    }

    /// Returns the live generic SQL property view.
    #[getter]
    fn sql(slf: Py<Self>) -> PyProtocolMetadata {
        PyProtocolMetadata::new(slf, CoreScheme::SQL)
    }

    /// Returns the live AWS Glue property view.
    #[getter]
    fn glue(slf: Py<Self>) -> PyProtocolMetadata {
        PyProtocolMetadata::new(slf, CoreScheme::GLUE)
    }

    /// Returns the live Apache Iceberg property view.
    #[getter]
    fn iceberg(slf: Py<Self>) -> PyProtocolMetadata {
        PyProtocolMetadata::new(slf, CoreScheme::ICEBERG)
    }

    /// Returns the live Financial Information eXchange property view.
    #[getter]
    fn fix(slf: Py<Self>) -> PyProtocolMetadata {
        PyProtocolMetadata::new(slf, CoreScheme::FIX)
    }

    /// Returns the live Yggdryl field property view.
    #[getter]
    fn field(slf: Py<Self>) -> PyProtocolMetadata {
        PyProtocolMetadata::new(slf, CoreScheme::FIELD)
    }

    /// Returns the live Yggdryl datatype property view.
    #[getter]
    fn dtype(slf: Py<Self>) -> PyProtocolMetadata {
        PyProtocolMetadata::new(slf, CoreScheme::DTYPE)
    }

    /// Returns the live Amazon S3 property view.
    #[getter]
    fn s3(slf: Py<Self>) -> PyProtocolMetadata {
        PyProtocolMetadata::new(slf, CoreScheme::S3)
    }

    /// Returns the live Google Cloud Storage property view.
    #[getter]
    fn gs(slf: Py<Self>) -> PyProtocolMetadata {
        PyProtocolMetadata::new(slf, CoreScheme::GS)
    }

    /// Returns the live Azure Blob Storage property view.
    #[getter]
    fn az(slf: Py<Self>) -> PyProtocolMetadata {
        PyProtocolMetadata::new(slf, CoreScheme::AZ)
    }

    /// Returns the live Apache Spark property view.
    #[getter]
    fn spark(slf: Py<Self>) -> PyProtocolMetadata {
        PyProtocolMetadata::new(slf, CoreScheme::SPARK)
    }

    /// Returns the live Polars property view.
    #[getter]
    fn polars(slf: Py<Self>) -> PyProtocolMetadata {
        PyProtocolMetadata::new(slf, CoreScheme::POLARS)
    }

    /// Returns the live pandas property view.
    #[getter]
    fn pandas(slf: Py<Self>) -> PyProtocolMetadata {
        PyProtocolMetadata::new(slf, CoreScheme::PANDAS)
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

    /// Makes a cached record schema value immutable at the Python boundary.
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
    fn __getitem__(&self, key: &Bound<'_, PyAny>) -> PyResult<Self> {
        crate::child_of(self.inner.data_type(), key).map(Self::from_inner)
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
        crate::set_child(&mut self.inner, key, child)
    }

    /// Remove a nested child by name or by position, closing the gap.
    fn __delitem__(&mut self, key: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        crate::remove_child(&mut self.inner, key)
    }

    /// The number of nested children, as `DataType` reports it.
    fn __len__(&self) -> usize {
        self.inner.field_len()
    }

    /// Iterate the nested children, as `DataType` does.
    fn __iter__(&self) -> PyDataTypeIterator {
        PyDataTypeIterator::over(self.inner.data_type().clone())
    }

    /// Whether a child name, position, or field is among the children.
    fn __contains__(&self, value: &Bound<'_, PyAny>) -> bool {
        crate::child_of(self.inner.data_type(), value).is_ok()
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

    fn __hash__(&self) -> u64 {
        self.inner.stable_layout_hash()
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (String,))> {
        let callable = py.get_type::<Self>().getattr("from_str")?.unbind();
        Ok((callable, (self.inner.to_string(),)))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// One protocol's field properties, addressed by their bare names.
///
/// The value is a live view rather than a snapshot: it holds the Python
/// `Field` object and reaches the core through it on every call. A write
/// through the view is therefore visible on the field, a write on the field is
/// visible through the view, and two views of one field see each other's
/// writes. Every mutation goes through the field's own frozen-schema gate.
#[pyclass(name = "ProtocolMetadata", module = "yggdryl._native")]
pub(crate) struct PyProtocolMetadata {
    field: Py<PyField>,
    scheme: CoreScheme,
}

impl PyProtocolMetadata {
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
}

#[pymethods]
impl PyProtocolMetadata {
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
}
