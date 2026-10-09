//! An Apache Iceberg table, over the same `IOBase` handle Python already has.
//!
//! A table is a folder and nothing else, so this binding takes the handle a
//! caller already built with [`crate::iobase::PyIOBase`] and hands it to the core
//! [`Table`] - or takes the table's location as it was named and hands that
//! to the core's own location doors, which is how a table an Amazon S3 Tables
//! table bucket keeps is reached with nothing built first. Rows cross the
//! boundary the way they do everywhere else here -
//! as a `pyarrow.RecordBatchReader` over the Arrow C Stream interface - so a
//! scan is lazy on both sides and a commit copies nothing.
//!
//! The metadata values below (a snapshot, a manifest, a data file, a partition
//! spec) are read-only views of the core structs. They exist so a caller can
//! ask what a commit produced without opening the Avro files by hand; none of
//! them can be constructed from Python, because only a commit writes one.

use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyDict, PyString, PyTuple, PyType};

use yggdryl::holder::Holder;
use yggdryl::iceberg::{
    Compaction, DataFile, FieldSummary, FormatVersion, IcebergCatalog, IcebergNamespace,
    IcebergOptions, IcebergTable, ManifestContent, ManifestFile, PartitionField, PartitionSpec,
    ScanPlan, SchemaUpdate, Snapshot, WriteStaging, assign_field_ids, can_promote, last_column_id,
    schema_from_json, schema_into_json,
};
use yggdryl::media::{DEFAULT_ROOT_NAME, IORecordOptions as _};
use yggdryl::{Catalog, Handle, IOBase as _, Namespace, Table};
use yggdryl::{Field as CoreField, Scalar};

use crate::datatype::core_dtype_from_value;
use crate::enums::{PyMimeType, core_mime_type_from_value};
use crate::field::{PyField, core_field_from_value};
use crate::graph::ellipsis;
use crate::iobase::PyIOBase;
use crate::iomedia::{
    batch_reader_from_any, batch_reader_from_records, batch_reader_to_pyarrow,
    core_root_field_from_value, string_pairs_from_value,
};
use crate::uri::{core_uri_from_value, core_url_from_value};
use crate::value_error;

/// Read one required key from a private pickle state mapping.
fn required_pickle_item<'py>(
    state: &Bound<'py, PyDict>,
    name: &str,
) -> PyResult<Bound<'py, PyAny>> {
    state
        .get_item(name)?
        .ok_or_else(|| PyValueError::new_err(format!("native pickle state is missing {name:?}")))
}

/// Number every field of a schema, depth first, and return the numbered copy.
///
/// Iceberg resolves a column by identifier rather than by position, so a schema
/// reaches [`PyTable::create`] already numbered. This is the core's numbering,
/// exposed because a caller building a schema from Python annotations or from a
/// `PyArrow` schema has no identifiers to start from.
///
/// # Errors
///
/// Raises `ValueError` when the value is not a non-null struct root.
#[pyfunction(name = "assign_field_ids")]
#[pyo3(signature = (schema, start = 1))]
pub(crate) fn iceberg_assign_field_ids(schema: &Bound<'_, PyAny>, start: i32) -> PyResult<PyField> {
    let mut root = core_root_field_from_value(schema, DEFAULT_ROOT_NAME)?;
    assign_field_ids(&mut root, start).map_err(value_error)?;
    Ok(PyField::from_inner(root))
}

/// Read one Iceberg schema document as a native root Field.
///
/// The document is an ordinary mapping - what `json.loads` produces - because an
/// Iceberg schema is ordinary JSON. `name` is what the struct root is called,
/// since the document names the columns and never the record.
///
/// # Errors
///
/// Raises `ValueError` when the document is not an Iceberg struct schema.
#[pyfunction(name = "schema_from_json")]
pub(crate) fn iceberg_schema_from_json(
    name: &str,
    document: &Bound<'_, PyAny>,
) -> PyResult<PyField> {
    let document = crate::scalar::from_py(document)?;
    schema_from_json(name, &document)
        .map(PyField::from_inner)
        .map_err(value_error)
}

/// Write a native root Field as an Iceberg schema document.
///
/// # Errors
///
/// Raises `ValueError` when the root is not a non-null struct whose columns
/// carry field identifiers.
#[pyfunction(name = "schema_into_json")]
pub(crate) fn iceberg_schema_into_json(
    py: Python<'_>,
    schema: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let root = core_root_field_from_value(schema, DEFAULT_ROOT_NAME)?;
    let document = schema_into_json(&root).map_err(value_error)?;
    crate::scalar::as_py(py, &document)
}

/// Check one type change against the promotions Iceberg allows.
///
/// Both sides accept anything a datatype crosses the boundary as: the native
/// wrapper, a datatype expression, or a `PyArrow` type. A legal promotion
/// returns `None`, because the question is "may I", not "how".
///
/// # Errors
///
/// Raises `ValueError` naming both types for every other change.
#[pyfunction(name = "can_promote")]
pub(crate) fn iceberg_can_promote(
    from_type: &Bound<'_, PyAny>,
    to_type: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let from_type = core_dtype_from_value(from_type)?;
    let to_type = core_dtype_from_value(to_type)?;
    can_promote(&from_type, &to_type).map_err(value_error)
}

/// Read a core format version out of the number or name Python spells it with.
fn format_version_from_value(value: &Bound<'_, PyAny>) -> PyResult<FormatVersion> {
    if let Ok(number) = value.extract::<i64>() {
        return FormatVersion::from_number(number).map_err(value_error);
    }
    Err(PyValueError::new_err(
        "expected an Iceberg format version of 1, 2, or 3",
    ))
}

/// The spec a `partition_by` argument states, none when it was omitted
/// (`...`): `None` unpartitioned, anything else [`spec_from_value`]'s
/// reading.
///
/// An omitted argument is skipped rather than defaulted here: a location
/// root hands the absence to the core, whose create derives the spec from
/// the schema as it stores it, and a handle root - whose core door takes a
/// spec - reads the schema's own declaration through [`declared_spec`].
fn stated_spec(value: &Bound<'_, PyAny>, schema: &CoreField) -> PyResult<Option<PartitionSpec>> {
    if value.is(value.py().Ellipsis()) {
        return Ok(None);
    }
    if value.is_none() {
        return Ok(Some(PartitionSpec::unpartitioned()));
    }
    spec_from_value(value, schema).map(Some)
}

/// The key a merge names: `...` (left out) and `None` name none, which the
/// core reads as the table's own key; a boolean is read by the options' one
/// merge-key door - `True` the table's own key as `None` is, `False`
/// refused at `$.merge_by` - and anything else is a selector.
fn merge_key_from_value(value: &Bound<'_, PyAny>) -> PyResult<yggdryl::Selector> {
    if value.is(value.py().Ellipsis()) {
        return Ok(yggdryl::Selector::all());
    }
    if value.is_instance_of::<PyBool>() {
        let mut options = yggdryl::ipc::IpcOptions::new();
        options
            .set_merge_by_scalar(&crate::scalar::from_py(value)?)
            .map_err(value_error)?;
        return Ok(options.merge_by().clone());
    }
    crate::expression::selector_from_value(value)
}

/// The spec a handle root is created under: the one stated, else the
/// schema's own `PARTITION:by` declaration.
fn declared_spec(stated: Option<PartitionSpec>, schema: &CoreField) -> PyResult<PartitionSpec> {
    match stated {
        Some(spec) => Ok(spec),
        None => PartitionSpec::from_schema(0, schema).map_err(value_error),
    }
}

/// Read a core partition spec out of what Python names one with.
///
/// A sequence is a `PARTITION:by` declaration, each entry a projection: a
/// bare column (`symbol`) an identity field, an epoch function over a column
/// (`days(ts)`, `minutes(ts, 15)`, `weeks(ts)`, `quarters(ts)`) or
/// `truncate(name, 4)` a derived one, an `as alias` naming it. The entries
/// are declared on a copy of the schema root and read by the core's one rule,
/// [`PartitionSpec::from_schema`], so a refusal names the entry it could not
/// honour.
fn spec_from_value(value: &Bound<'_, PyAny>, schema: &CoreField) -> PyResult<PartitionSpec> {
    if let Ok(spec) = value.extract::<PyRef<'_, PyPartitionSpec>>() {
        return Ok(spec.inner.clone());
    }
    if value.is_instance_of::<PyString>() {
        return Err(PyTypeError::new_err(
            "partition_by must be a PartitionSpec or an iterable of entries, not one string",
        ));
    }
    // Text crosses as it is, so a malformed entry is refused naming it; a
    // `Term` or a `(term, alias)` pair crosses as the text it spells.
    let mut entries = Vec::new();
    for entry in value.try_iter()? {
        let entry = entry?;
        entries.push(match entry.extract::<String>() {
            Ok(text) => text,
            Err(_) => crate::expression::projection_from_value(&entry)?.to_string(),
        });
    }
    let mut declared = schema.clone();
    declared
        .as_partition_mut()
        .set_by_texts(entries)
        .map_err(value_error)?;
    PartitionSpec::from_schema(0, &declared).map_err(value_error)
}

/// Project one Iceberg partition value as the Python value it stands for.
fn partition_values<'py>(py: Python<'py>, values: &[Scalar]) -> PyResult<Bound<'py, PyTuple>> {
    let projected: Vec<Py<PyAny>> = values
        .iter()
        .map(|value| crate::scalar::as_py(py, value))
        .collect::<PyResult<_>>()?;
    PyTuple::new(py, projected)
}

/// Project a field-id-keyed statistic as a mapping.
fn counts_by_id<'py>(py: Python<'py>, counts: &[(i32, i64)]) -> PyResult<Bound<'py, PyDict>> {
    let mapping = PyDict::new(py);
    for (id, count) in counts {
        mapping.set_item(id, count)?;
    }
    Ok(mapping)
}

/// Project a field-id-keyed bound as a mapping of encoded values.
fn bounds_by_id<'py>(py: Python<'py>, bounds: &[(i32, Vec<u8>)]) -> PyResult<Bound<'py, PyDict>> {
    let mapping = PyDict::new(py);
    for (id, value) in bounds {
        mapping.set_item(id, pyo3::types::PyBytes::new(py, value))?;
    }
    Ok(mapping)
}

/// Read a schema root the way [`PyTable::create`] needs it: numbered.
///
/// Numbering continues above the highest identifier already assigned - the
/// same rule [`catalog_schema_from_value`]'s callers apply through the core -
/// so a numbered schema keeps every id it came with, and a plain `PyArrow`
/// schema arrives here with none and leaves with all of them. The spec
/// builders resolve `partition_by` names to identifiers, which is why the
/// numbering happens at this boundary rather than inside [`Table::create`]'s
/// metadata alone.
fn numbered_schema_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreField> {
    let mut schema = core_root_field_from_value(value, DEFAULT_ROOT_NAME)?;
    let start = last_column_id(&schema)
        .map_err(value_error)?
        .saturating_add(1);
    assign_field_ids(&mut schema, start).map_err(value_error)?;
    Ok(schema)
}

/// Read the `(column, value)` filter pairs a scan takes; `None` means none.
fn filter_pairs_from_value(value: Option<&Bound<'_, PyAny>>) -> PyResult<Vec<(String, String)>> {
    match value {
        Some(value) => string_pairs_from_value(value),
        None => Ok(Vec::new()),
    }
}

/// Borrow owned filter pairs as the slice of string pairs the core takes.
///
/// The owned pairs outlive the call because a filter is read at the boundary
/// and the core is entered afterwards, so the borrow is taken here rather than
/// where the pairs are built.
fn borrowed_pairs(pairs: &[(String, String)]) -> Vec<(&str, &str)> {
    pairs
        .iter()
        .map(|(column, value)| (column.as_str(), value.as_str()))
        .collect()
}

/// Read a warehouse folder out of what Python names one with.
///
/// A handle is taken as the folder it addresses - the same inference
/// [`PyTable::create`]'s `root` runs through - and a string, path-like, or URL
/// describes one. Per the laziness contract, naming a folder that does not
/// exist yet touches nothing and is not an error.
pub(crate) fn folder_holder_from_value(value: &Bound<'_, PyAny>) -> PyResult<Holder> {
    if let Ok(handle) = value.extract::<PyRef<'_, PyIOBase>>() {
        return handle.folder_holder();
    }
    let url = core_url_from_value(value)?;
    crate::iobase::folder_holder_for(&url)
}

/// What a table door's `root` names: a handle in hand, taken as the container
/// it addresses, or the location of the table as the caller named it - text,
/// a path-like, a `Url`, or any identifier that locates one - which the core
/// opens by itself under the properties stated beside it.
enum TableRoot {
    /// Boxed: a holder is several times the size of a location.
    Handle(Box<Holder>),
    Location(yggdryl::Uri, yggdryl::Properties),
}

/// Read a table door's `root` and the `properties` stated beside it.
///
/// A handle root is reopened as the folder at its location, under the
/// environment - nothing its builder stated is carried - so properties
/// beside one have nothing to open and are refused by name rather than
/// dropped. A location
/// crosses as the identifier it is - never lowered here: a table bucket's
/// ARN and a table's ARN state what the location they lower to does not.
fn table_root_from_value(
    root: &Bound<'_, PyAny>,
    properties: Option<&Bound<'_, PyDict>>,
) -> PyResult<TableRoot> {
    let properties = crate::warehouse::properties_from_args(None, properties)?;
    if let Ok(handle) = root.extract::<PyRef<'_, PyIOBase>>() {
        if let Some((name, _)) = properties.iter().next() {
            return Err(PyValueError::new_err(format!(
                "expected no properties beside a handle, which is reopened as the folder at its \
                 location, got {name:?}; name the table by its location to open it under properties"
            )));
        }
        return handle.folder_holder().map(Box::new).map(TableRoot::Handle);
    }
    Ok(TableRoot::Location(core_uri_from_value(root)?, properties))
}

/// The keyword fields accepted by the `IcebergOptions` constructor.
const ICEBERG_OPTION_FIELDS: [&str; 12] = [
    "commit_retries",
    "commit_min_backoff_ms",
    "commit_max_backoff_ms",
    "commit_total_timeout_ms",
    "target_file_size",
    "read_parallelism",
    "read_parallel_min_files",
    "read_parallel_min_file_size",
    "write_parallelism",
    "max_open_partitions",
    "write_staging",
    "data_mime_type",
];

/// Read `write.staging` from the text or path-like value a caller hands over.
///
/// `off`, a folder URL, or a local path, each as `str`; a `pathlib.Path` or
/// any other path-like crosses through its own `__fspath__`.
fn write_staging_from_value(value: &Bound<'_, PyAny>) -> PyResult<WriteStaging> {
    let text: String = if let Ok(text) = value.extract::<String>() {
        text
    } else if let Ok(path) = value.call_method0("__fspath__") {
        path.extract::<String>()?
    } else {
        return Err(PyTypeError::new_err(format!(
            "expected 'off', a folder URL, or a path for write_staging, got {}",
            value.get_type().name()?
        )));
    };
    WriteStaging::from_str(&text).map_err(value_error)
}

/// Set one Iceberg option field from the Python value a keyword carries:
/// `false` when `key` names no field.
fn set_iceberg_option(
    options: &mut IcebergOptions,
    key: &str,
    value: &Bound<'_, PyAny>,
) -> PyResult<bool> {
    match key {
        "commit_retries" => options.set_commit_retries(value.extract::<u32>()?),
        "commit_min_backoff_ms" => options.set_commit_min_backoff_ms(value.extract::<u64>()?),
        "commit_max_backoff_ms" => options.set_commit_max_backoff_ms(value.extract::<u64>()?),
        "commit_total_timeout_ms" => {
            options.set_commit_total_timeout_ms(value.extract::<u64>()?);
        }
        "target_file_size" => options
            .set_target_file_size_bytes(value.extract::<u64>()?)
            .map_err(value_error)?,
        "read_parallelism" => options
            .set_read_parallelism(value.extract::<usize>()?)
            .map_err(value_error)?,
        "read_parallel_min_files" => {
            options.set_read_parallel_min_files(value.extract::<usize>()?);
        }
        "read_parallel_min_file_size" => {
            options.set_read_parallel_min_file_size_bytes(value.extract::<u64>()?);
        }
        "write_parallelism" => options
            .set_write_parallelism(value.extract::<usize>()?)
            .map_err(value_error)?,
        "max_open_partitions" => options
            .set_max_open_partitions(value.extract::<usize>()?)
            .map_err(value_error)?,
        "write_staging" => options
            .set_write_staging(write_staging_from_value(value)?)
            .map_err(value_error)?,
        "data_mime_type" => options
            .set_data_mime_type(core_mime_type_from_value(value)?)
            .map_err(value_error)?,
        _ => return Ok(false),
    }
    Ok(true)
}

/// Set each of `properties` on `options` by its own field, in canonical
/// order so the answer never depends on keyword order.
///
/// A `...` value is skipped as not given, and a name that is no field is
/// skipped with an `UnknownPropertyWarning` naming the closest one.
fn fold_iceberg_properties(
    options: &mut IcebergOptions,
    properties: &Bound<'_, PyDict>,
) -> PyResult<()> {
    let py = properties.py();
    for (key, _) in properties.iter() {
        let key = key.cast::<pyo3::types::PyString>()?.to_str()?;
        if !ICEBERG_OPTION_FIELDS.contains(&key) {
            crate::properties::warn_unknown(py, "IcebergOptions", key, ICEBERG_OPTION_FIELDS)?;
        }
    }
    let ellipsis = py.Ellipsis();
    for key in ICEBERG_OPTION_FIELDS {
        if let Some(value) = properties.get_item(key)?
            && !value.is(&ellipsis)
        {
            set_iceberg_option(options, key, &value)?;
        }
    }
    Ok(())
}

/// Read the Iceberg options one `options` argument names, strictly.
///
/// Iceberg configuration is [`IcebergOptions`] and never the generic record
/// options, so a `RecordOptions` here is refused by name rather than bridged.
fn core_iceberg_options_from_value(value: &Bound<'_, PyAny>) -> PyResult<IcebergOptions> {
    if let Ok(options) = value.extract::<PyRef<'_, PyIcebergOptions>>() {
        return Ok(options.inner.clone());
    }
    if value
        .extract::<PyRef<'_, crate::iomedia::PyRecordOptions>>()
        .is_ok()
    {
        return Err(PyTypeError::new_err(
            "expected IcebergOptions, got RecordOptions; Iceberg is configured by IcebergOptions \
             alone - the record options belong to the plain record surface",
        ));
    }
    Err(PyTypeError::new_err(format!(
        "expected IcebergOptions, got {}",
        value.get_type().fully_qualified_name().map_or_else(
            |_| "an unnameable value".to_owned(),
            |name| name.to_string()
        ),
    )))
}

/// Import an optional per-call options value, with `properties` set on a
/// copy of it - or, when none is given, of `base`, the handle's own override,
/// or of nothing set - by their own fields.
fn iceberg_call_options(
    options: Option<&Bound<'_, PyAny>>,
    properties: Option<&Bound<'_, PyDict>>,
    base: Option<&IcebergOptions>,
) -> PyResult<Option<IcebergOptions>> {
    let options = options.map(core_iceberg_options_from_value).transpose()?;
    let Some(properties) = properties.filter(|properties| !properties.is_empty()) else {
        return Ok(options);
    };
    let mut options = options
        .or_else(|| base.cloned())
        .unwrap_or_else(IcebergOptions::new);
    fold_iceberg_properties(&mut options, properties)?;
    Ok(Some(options))
}

/// Read a batch reader out of anything Python holds Iceberg rows in.
///
/// The same inference point the record surface uses, given the table's stored
/// schema as the declared field so plain mappings and dataclass rows type
/// against the table rather than guessing a shape from their first value. A
/// table that does not exist yet names no schema, and the incoming value is
/// then what declares one. The options value carries only that field; the
/// data-file format stays the table's own `data_mime_type`.
fn iceberg_batch_reader(
    table: Option<&IcebergTable<Handle>>,
    value: &Bound<'_, PyAny>,
) -> PyResult<yggdryl::arrow::BatchReader> {
    let mut options = yggdryl::media::RecordOptions::for_mime_type(&yggdryl::MimeType::PARQUET)
        .map_err(value_error)?;
    let declared = table.and_then(|table| table.schema().ok());
    if let Some(schema) = declared {
        options.set_field(schema.clone());
    }
    // An empty sequence names no shape, which is why the widest reader refuses
    // one - but a table that already declared its schema has the shape, and an
    // empty overwrite is how a caller deletes every row.
    if declared.is_some()
        && let Ok(length) = value.len()
        && length == 0
    {
        return batch_reader_from_records(value, &mut options);
    }
    batch_reader_from_any(value, &options)
}

/// Run one table operation under per-call options, restoring the handle after.
///
/// The override is shadowed for exactly the length of the call, so per-call
/// options never leak into the handle's own configuration.
fn with_call_options<R>(
    table: &mut IcebergTable<Handle>,
    options: Option<IcebergOptions>,
    operation: impl FnOnce(&mut IcebergTable<Handle>) -> PyResult<R>,
) -> PyResult<R> {
    let Some(options) = options else {
        return operation(table);
    };
    let saved = table.clear_options();
    table.set_options(options);
    let result = operation(table);
    match saved {
        Some(saved) => table.set_options(saved),
        None => {
            table.clear_options();
        }
    }
    result
}

/// Configuration for one table's commits, writes, and reads.
///
/// A Python view of the core [`IcebergOptions`]: the value records only what
/// was set on it, every getter answers the field's documented default when
/// nothing was, and a table resolves each field as explicit option, then
/// table property, then that default.
#[pyclass(
    name = "IcebergOptions",
    module = "yggdryl._native",
    skip_from_py_object
)]
#[derive(Default)]
pub(crate) struct PyIcebergOptions {
    pub(crate) inner: IcebergOptions,
    hash_locked: bool,
}

impl Clone for PyIcebergOptions {
    fn clone(&self) -> Self {
        Self::from_core(self.inner.clone())
    }
}

impl PyIcebergOptions {
    fn from_core(inner: IcebergOptions) -> Self {
        Self {
            inner,
            hash_locked: false,
        }
    }

    fn require_mutable(&self) -> PyResult<()> {
        if self.hash_locked {
            Err(PyTypeError::new_err(
                "hashed IcebergOptions are frozen; copy them before mutation",
            ))
        } else {
            Ok(())
        }
    }

    fn pickle_state<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let state = PyDict::new(py);
        if let Some(value) = self.inner.commit_retries_option() {
            state.set_item("commit_retries", value)?;
        }
        if let Some(value) = self.inner.commit_min_backoff_ms_option() {
            state.set_item("commit_min_backoff_ms", value)?;
        }
        if let Some(value) = self.inner.commit_max_backoff_ms_option() {
            state.set_item("commit_max_backoff_ms", value)?;
        }
        if let Some(value) = self.inner.commit_total_timeout_ms_option() {
            state.set_item("commit_total_timeout_ms", value)?;
        }
        if let Some(value) = self.inner.target_file_size_bytes_option() {
            state.set_item("target_file_size", value)?;
        }
        if let Some(value) = self.inner.read_parallelism_option() {
            state.set_item("read_parallelism", value)?;
        }
        if let Some(value) = self.inner.read_parallel_min_files_option() {
            state.set_item("read_parallel_min_files", value)?;
        }
        if let Some(value) = self.inner.read_parallel_min_file_size_bytes_option() {
            state.set_item("read_parallel_min_file_size", value)?;
        }
        if let Some(value) = self.inner.write_parallelism_option() {
            state.set_item("write_parallelism", value)?;
        }
        if let Some(value) = self.inner.max_open_partitions_option() {
            state.set_item("max_open_partitions", value)?;
        }
        if let Some(value) = self.inner.write_staging() {
            state.set_item("write_staging", value.to_string())?;
        }
        if let Some(value) = self.inner.data_mime_type_option() {
            state.set_item("data_mime_type", value.as_str())?;
        }
        Ok(state)
    }
}

#[pymethods]
impl PyIcebergOptions {
    /// Build an options value with nothing set, each keyword setting a field.
    ///
    /// A keyword naming no field is skipped with an `UnknownPropertyWarning`.
    #[new]
    #[pyo3(signature = (**properties))]
    fn new(properties: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        let mut inner = IcebergOptions::new();
        if let Some(properties) = properties {
            fold_iceberg_properties(&mut inner, properties)?;
        }
        Ok(Self::from_core(inner))
    }

    /// Rebuild exactly the options that were explicitly configured.
    ///
    /// A pickle state is this class's own writing, so a key it does not know
    /// is a corrupt state and refused rather than warned about.
    #[staticmethod]
    fn _from_pickle(state: &Bound<'_, PyDict>) -> PyResult<Self> {
        let mut inner = IcebergOptions::new();
        for (key, value) in state.iter() {
            let key = key.extract::<String>()?;
            if !set_iceberg_option(&mut inner, &key, &value)? {
                return Err(PyTypeError::new_err(format!(
                    "IcebergOptions._from_pickle() got an unexpected keyword argument {key:?}"
                )));
            }
        }
        Ok(Self::from_core(inner))
    }

    /// How many beaten commit attempts are retried. Default: 4.
    #[getter]
    fn commit_retries(&self) -> u32 {
        self.inner.commit_retries()
    }

    #[setter]
    fn set_commit_retries(&mut self, retries: u32) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_commit_retries(retries);
        Ok(())
    }

    /// The first commit retry wait in milliseconds. Default: 100.
    #[getter]
    fn commit_min_backoff_ms(&self) -> u64 {
        self.inner.commit_min_backoff_ms()
    }

    #[setter]
    fn set_commit_min_backoff_ms(&mut self, wait_ms: u64) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_commit_min_backoff_ms(wait_ms);
        Ok(())
    }

    /// The largest commit retry wait in milliseconds. Default: 60000.
    #[getter]
    fn commit_max_backoff_ms(&self) -> u64 {
        self.inner.commit_max_backoff_ms()
    }

    #[setter]
    fn set_commit_max_backoff_ms(&mut self, wait_ms: u64) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_commit_max_backoff_ms(wait_ms);
        Ok(())
    }

    /// The total commit retry-delay budget in milliseconds. Default: 1800000.
    #[getter]
    fn commit_total_timeout_ms(&self) -> u64 {
        self.inner.commit_total_timeout_ms()
    }

    #[setter]
    fn set_commit_total_timeout_ms(&mut self, timeout_ms: u64) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_commit_total_timeout_ms(timeout_ms);
        Ok(())
    }

    /// The size a data file aims for, in bytes. Default: 512 MiB.
    #[getter]
    fn target_file_size(&self) -> u64 {
        self.inner.target_file_size_bytes()
    }

    #[setter]
    fn set_target_file_size(&mut self, bytes: u64) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_target_file_size_bytes(bytes)
            .map_err(value_error)
    }

    /// How many data files a scan decodes at once. Default: the host's own
    /// parallelism, the whole host (`std::thread::available_parallelism`).
    #[getter]
    fn read_parallelism(&self) -> usize {
        self.inner.read_parallelism()
    }

    #[setter]
    fn set_read_parallelism(&mut self, threads: usize) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_read_parallelism(threads)
            .map_err(value_error)
    }

    /// How many large-enough files justify a parallel scan. Default: 2.
    #[getter]
    fn read_parallel_min_files(&self) -> usize {
        self.inner.read_parallel_min_files()
    }

    #[setter]
    fn set_read_parallel_min_files(&mut self, files: usize) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_read_parallel_min_files(files);
        Ok(())
    }

    /// The recorded size below which a file does not count toward justifying
    /// a parallel scan, in bytes. Default: 64 KiB.
    #[getter]
    fn read_parallel_min_file_size(&self) -> u64 {
        self.inner.read_parallel_min_file_size_bytes()
    }

    #[setter]
    fn set_read_parallel_min_file_size(&mut self, bytes: u64) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_read_parallel_min_file_size_bytes(bytes);
        Ok(())
    }

    /// How many partition groups a commit writes at once. Default: the
    /// resolved `read_parallelism`; 1 writes them one after another. The
    /// manifest lists a commit's files in partition-group order whatever the
    /// value.
    #[getter]
    fn write_parallelism(&self) -> usize {
        self.inner.write_parallelism()
    }

    #[setter]
    fn set_write_parallelism(&mut self, threads: usize) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_write_parallelism(threads)
            .map_err(value_error)
    }

    /// How many partitions an append, an overwrite or a compaction holds
    /// open at once. Default: 128. Past it, the open partition of the lowest
    /// tuple closes and its files are written while the source is still
    /// read, so a source in partition order is written as it arrives and
    /// never held whole; a partition arriving again after it closed is
    /// written again, as further files of the same commit. A keyed merge
    /// holds every partition open whatever this says.
    #[getter]
    fn max_open_partitions(&self) -> usize {
        self.inner.max_open_partitions()
    }

    #[setter]
    fn set_max_open_partitions(&mut self, partitions: usize) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_max_open_partitions(partitions)
            .map_err(value_error)
    }

    /// Where a commit stages its files before uploading them: `"off"`, or a
    /// local folder URL. `None` - the default - is the table's own: the
    /// temporary folder for a remote root, off for a local one.
    #[getter]
    fn write_staging(&self) -> Option<String> {
        self.inner.write_staging().map(ToString::to_string)
    }

    #[setter]
    fn set_write_staging(&mut self, staging: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_write_staging(write_staging_from_value(staging)?)
            .map_err(value_error)
    }

    /// The MIME type used for new data files. Default: `MimeType.PARQUET`.
    ///
    /// Only what a write produces is decided here: a scan decodes each data
    /// file as the format its manifest entry records, so one table can mix
    /// formats and still read as one shape. The table property is the spec's
    /// own `write.format.default`.
    #[getter]
    fn data_mime_type(&self) -> PyMimeType {
        PyMimeType::from_core(self.inner.data_mime_type())
    }

    #[setter]
    fn set_data_mime_type(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_data_mime_type(core_mime_type_from_value(value)?)
            .map_err(value_error)
    }

    /// Return a deterministic hash of every explicitly configured option.
    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&mut self) -> isize {
        self.hash_locked = true;
        crate::python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let state = self.pickle_state(py)?;
        Ok(format!(
            "IcebergOptions._from_pickle({})",
            state.repr()?.to_str()?
        ))
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        Ok((
            py.get_type::<Self>().getattr("_from_pickle")?.unbind(),
            (self.pickle_state(py)?.into_any().unbind(),),
        ))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// A warehouse folder of namespaces of Iceberg tables: the `Catalog`
/// subclass the Iceberg implementation answers.
///
/// Namespaces nest to any depth, each a folder; `metadata/catalog.json` and
/// `metadata/namespace.json` keep the stored properties; a table is a folder
/// laid out as one. The catalog is a description: constructing one touches
/// nothing, and every question - the views, the children, a dotted path - is
/// asked of the store when it is asked, through the members every `Catalog`
/// has.
#[pyclass(
    name = "IcebergCatalog",
    module = "yggdryl._native",
    frozen,
    extends = crate::warehouse::PyCatalog,
    skip_from_py_object
)]
pub(crate) struct PyIcebergCatalog;

/// The catalog `name` over what `warehouse` names, touching nothing.
fn iceberg_catalog(name: &str, warehouse: &Bound<'_, PyAny>) -> PyResult<IcebergCatalog> {
    match crate::warehouse::located_from_value(warehouse)? {
        crate::warehouse::Located::Handle(holder) => Ok(IcebergCatalog::bound(name, *holder)),
        crate::warehouse::Located::Url(url) => IcebergCatalog::new(name, url).map_err(value_error),
    }
}

#[pymethods]
impl PyIcebergCatalog {
    /// The catalog `name` over the warehouse folder `warehouse` names - an
    /// `IOBase` handle binds it, a `Url`, a string or a path-like names it -
    /// touching nothing. `properties` and the keywords are what the catalog
    /// states, which its folder and every object under it open with.
    #[new]
    #[pyo3(signature = (name, warehouse, *, description = None, properties = None, **keywords))]
    fn new(
        name: &str,
        warehouse: &Bound<'_, PyAny>,
        description: Option<&str>,
        properties: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyClassInitializer<Self>> {
        let mut catalog = iceberg_catalog(name, warehouse)?.with_properties(
            crate::warehouse::properties_from_args(properties, keywords)?,
        );
        if let Some(description) = description {
            catalog = catalog.with_description(description);
        }
        Ok(crate::warehouse::catalog_base(Catalog::from(catalog)).add_subclass(Self))
    }

    /// Create the catalog `name` in `warehouse`, writing its
    /// `metadata/catalog.json`; the write is what creates the folder, and a
    /// folder already holding anything is a conflict.
    #[classmethod]
    fn create(
        cls: &Bound<'_, PyType>,
        name: &str,
        warehouse: &Bound<'_, PyAny>,
    ) -> PyResult<Py<Self>> {
        let catalog = IcebergCatalog::create(name, folder_holder_from_value(warehouse)?)
            .map_err(value_error)?;
        Py::new(
            cls.py(),
            crate::warehouse::catalog_base(Catalog::from(catalog)).add_subclass(Self),
        )
    }

    /// The catalog `name` over `warehouse`, created when the folder is not
    /// there yet; a table or a file in its place is refused by name.
    #[classmethod]
    fn open_or_create(
        cls: &Bound<'_, PyType>,
        name: &str,
        warehouse: &Bound<'_, PyAny>,
    ) -> PyResult<Py<Self>> {
        let catalog = IcebergCatalog::open_or_create(name, folder_holder_from_value(warehouse)?)
            .map_err(value_error)?;
        Py::new(
            cls.py(),
            crate::warehouse::catalog_base(Catalog::from(catalog)).add_subclass(Self),
        )
    }
}

/// One namespace of an Iceberg catalog - a folder under the warehouse, its
/// `metadata/namespace.json` the stored properties - as the `Namespace`
/// subclass the Iceberg implementation answers.
#[pyclass(
    name = "IcebergNamespace",
    module = "yggdryl._native",
    frozen,
    extends = crate::warehouse::PyNamespace,
    skip_from_py_object
)]
pub(crate) struct PyIcebergNamespace;

#[pymethods]
impl PyIcebergNamespace {
    /// The namespace at `path` - dotted text or parts, its catalog's name
    /// first - over the folder `location` names, touching nothing: an
    /// `IOBase` handle binds the folder, a `Url`, a string or a path-like
    /// names it. `properties` and the keywords are what the namespace states.
    #[new]
    #[pyo3(signature = (path, location, *, properties = None, **keywords))]
    fn new(
        path: &Bound<'_, PyAny>,
        location: &Bound<'_, PyAny>,
        properties: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyClassInitializer<Self>> {
        let path = crate::warehouse::object_path_from_value(path)?;
        let namespace = match crate::warehouse::located_from_value(location)? {
            crate::warehouse::Located::Handle(holder) => IcebergNamespace::bound(path, *holder),
            crate::warehouse::Located::Url(url) => IcebergNamespace::new(path, url),
        }
        .map_err(value_error)?
        .with_properties(crate::warehouse::properties_from_args(
            properties, keywords,
        )?);
        Ok(crate::warehouse::namespace_base(Namespace::from(namespace)).add_subclass(Self))
    }
}

/// An Iceberg table reached entirely through one container handle: the
/// `Table` subclass an Iceberg implementation answers, so every member of a
/// warehouse table - the path, the properties, the record surface of
/// `IOBase` - is here beside the table's own.
#[pyclass(
    name = "IcebergTable",
    module = "yggdryl._native",
    frozen,
    extends = crate::warehouse::PyTable,
    skip_from_py_object
)]
pub(crate) struct PyIcebergTable;

/// The Iceberg table a holder is, when it holds one.
fn iceberg_of(holder: &Holder) -> PyResult<&IcebergTable<Handle>> {
    match holder {
        Holder::Table(table) => table
            .downcast_ref::<IcebergTable<Handle>>()
            .ok_or_else(|| not_iceberg(table)),
        _ => Err(PyValueError::new_err(
            "expected a handle holding an Iceberg table, got another handle",
        )),
    }
}

/// The Iceberg table a holder is, mutably.
fn iceberg_of_mut(holder: &mut Holder) -> PyResult<&mut IcebergTable<Handle>> {
    match holder {
        Holder::Table(table) => {
            if table.downcast_ref::<IcebergTable<Handle>>().is_none() {
                return Err(not_iceberg(table));
            }
            table
                .downcast_mut::<IcebergTable<Handle>>()
                .ok_or_else(|| PyValueError::new_err("expected an Iceberg table"))
        }
        _ => Err(PyValueError::new_err(
            "expected a handle holding an Iceberg table, got another handle",
        )),
    }
}

fn not_iceberg(table: &Table) -> PyErr {
    PyValueError::new_err(format!(
        "expected an Iceberg table, got {table} held by another implementation"
    ))
}

/// Borrow the Iceberg table below this object.
fn held<'a>(slf: &'a PyRef<'_, PyIcebergTable>) -> PyResult<&'a IcebergTable<Handle>> {
    iceberg_of(slf.as_super().as_super().inner()?)
}

/// Borrow the `IOBase` layer of this object mutably.
///
/// `Table` is frozen, so no `PyRefMut` reaches through it: the base is borrowed
/// as itself, and that borrow - the one flag every layer of the object shares -
/// fails as `RuntimeError` while another borrow holds it.
fn base_mut<'py>(slf: &Bound<'py, PyIcebergTable>) -> PyResult<PyRefMut<'py, PyIOBase>> {
    Ok(slf.as_super().as_super().try_borrow_mut()?)
}

/// Borrow the Iceberg table below a mutably borrowed base.
fn held_mut(base: &mut PyIOBase) -> PyResult<&mut IcebergTable<Handle>> {
    iceberg_of_mut(base.inner_mut()?)
}

/// The object an Iceberg table crosses as: the `Table` base over the handle,
/// then this class.
fn described_table(py: Python<'_>, table: IcebergTable<Handle>) -> PyResult<Py<PyIcebergTable>> {
    Py::new(
        py,
        crate::warehouse::table_base(Table::from(table)).add_subclass(PyIcebergTable),
    )
}

#[pymethods]
impl PyIcebergTable {
    /// Create a table, writing its first metadata document.
    ///
    /// `root` is the container handle the table lives in, or its location -
    /// a string, a path-like, a `Url`, a `Uri` or an `Arn` - which the core
    /// opens by itself under `properties`: a folder any backend holds, or
    /// `s3tables://<bucket>/<namespace>/<table>` for a table an Amazon S3
    /// Tables table bucket keeps, registered there and committed through
    /// its control plane, its namespace made on the way where the bucket
    /// does not hold it.
    ///
    /// `partition_by` accepts a [`PartitionSpec`] or the `PARTITION:by`
    /// entries to partition on - `symbol`, `days(ts)`, `minutes(ts, 15)`,
    /// `truncate(name, 4) as prefix` - read by the core's one rule; omitted,
    /// the schema's own declaration is read the same way, and `None` - like a
    /// schema declaring nothing - is unpartitioned. Unnumbered schema columns
    /// are numbered automatically, so a plain `PyArrow` schema works as it is;
    /// a schema that already carries field identifiers keeps every one of them.
    ///
    /// `format_version` omitted is 2 over a handle; over a location it is
    /// the `format-version` property, else the lowest version that states
    /// the schema - 3 for a nanosecond timestamp, a variant or an unknown
    /// column, else 2.
    #[classmethod]
    #[pyo3(signature = (root, schema, partition_by = ellipsis(), *, format_version = None, **properties))]
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands the `...` default over as `Py`.
    fn create(
        cls: &Bound<'_, PyType>,
        root: &Bound<'_, PyAny>,
        schema: &Bound<'_, PyAny>,
        partition_by: Py<PyAny>,
        format_version: Option<&Bound<'_, PyAny>>,
        properties: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<Self>> {
        let partition_by = partition_by.bind(schema.py());
        let schema = numbered_schema_from_value(schema)?;
        let spec = stated_spec(partition_by, &schema)?;
        let version = format_version.map(format_version_from_value).transpose()?;
        // Detached: the core writes to storage and, by a location, asks a
        // service, and a core thread that logs takes the GIL.
        let py = cls.py();
        let table = match table_root_from_value(root, properties)? {
            TableRoot::Handle(holder) => {
                let spec = declared_spec(spec, &schema)?;
                let version = version.unwrap_or(FormatVersion::V2);
                py.detach(move || {
                    IcebergTable::create(Handle::from(*holder), version, schema, spec)
                })
            }
            TableRoot::Location(location, properties) => py.detach(move || {
                IcebergTable::create_from_url(&location, &properties, version, schema, spec)
            }),
        }
        .map_err(value_error)?;
        described_table(py, table)
    }

    /// Open the table `root` names: `IcebergTable(root)`, since `open()` is
    /// the scope every handle has.
    ///
    /// A container handle is the folder the table lives in, and its current
    /// metadata document is read. A location - a string, a path-like, a
    /// `Url`, a `Uri` or an `Arn` - is opened by the core under
    /// `properties`: a folder any backend holds, or a table an Amazon S3
    /// Tables table bucket keeps, named `s3tables://<bucket>/<namespace>/<table>`
    /// or by its own ARN.
    #[new]
    #[pyo3(signature = (root, **properties))]
    fn new(
        root: &Bound<'_, PyAny>,
        properties: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyClassInitializer<Self>> {
        // Detached: the core reads storage and, by a location, asks a
        // service, and a core thread that logs takes the GIL.
        let py = root.py();
        let table = match table_root_from_value(root, properties)? {
            TableRoot::Handle(holder) => {
                py.detach(move || IcebergTable::open(Handle::from(*holder)))
            }
            TableRoot::Location(location, properties) => {
                py.detach(move || IcebergTable::from_url(&location, &properties))
            }
        }
        .map_err(value_error)?;
        Ok(crate::warehouse::table_base(Table::from(table)).add_subclass(Self))
    }

    /// Open the table if it exists, creating it otherwise.
    ///
    /// `root`, `format_version` and `properties` are read as
    /// [`Self::create`] reads them. Like it, unnumbered schema columns are
    /// numbered automatically; an existing table is opened as it is and
    /// `schema` describes only the table this call would create.
    #[classmethod]
    #[pyo3(signature = (root, schema, partition_by = ellipsis(), *, format_version = None, **properties))]
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands the `...` default over as `Py`.
    fn open_or_create(
        cls: &Bound<'_, PyType>,
        root: &Bound<'_, PyAny>,
        schema: &Bound<'_, PyAny>,
        partition_by: Py<PyAny>,
        format_version: Option<&Bound<'_, PyAny>>,
        properties: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<Self>> {
        let partition_by = partition_by.bind(schema.py());
        let schema = numbered_schema_from_value(schema)?;
        let spec = stated_spec(partition_by, &schema)?;
        let version = format_version.map(format_version_from_value).transpose()?;
        // Detached, as `create` is.
        let py = cls.py();
        let table = match table_root_from_value(root, properties)? {
            TableRoot::Handle(holder) => {
                let spec = declared_spec(spec, &schema)?;
                let version = version.unwrap_or(FormatVersion::V2);
                py.detach(move || {
                    IcebergTable::open_or_create(Handle::from(*holder), version, schema, spec)
                })
            }
            TableRoot::Location(location, properties) => py.detach(move || {
                IcebergTable::open_or_create_from_url(&location, &properties, version, schema, spec)
            }),
        }
        .map_err(value_error)?;
        described_table(py, table)
    }

    /// The folder the table lives in.
    ///
    /// The table's own root holder as a container - a bridged filesystem's
    /// folder role, an object store's client under its options, a local
    /// directory - rather than one rebuilt from its recorded location, because
    /// a location does not say which store holds it.
    #[getter]
    fn root(slf: &Bound<'_, Self>, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        let root = table.root().get().map_err(value_error)?;
        if let Some(holder) = crate::iobase::container_holder(root)? {
            return crate::iobase::describe(py, holder);
        }
        let url = root
            .url()
            .ok_or_else(|| PyValueError::new_err("this table has no location"))?;
        crate::iobase::describe(py, crate::iobase::folder_holder_for(url)?)
    }

    /// The table's base location, as a URI.
    #[getter]
    fn location(slf: &Bound<'_, Self>) -> PyResult<String> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        Ok(table.metadata().map_err(value_error)?.location().to_owned())
    }

    /// The revision of the specification the metadata is written to.
    #[getter]
    fn format_version(slf: &Bound<'_, Self>) -> PyResult<i32> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        Ok(table
            .metadata()
            .map_err(value_error)?
            .format_version()
            .number())
    }

    /// The stable identifier of the table itself.
    #[getter]
    fn table_uuid(slf: &Bound<'_, Self>) -> PyResult<String> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        Ok(table
            .metadata()
            .map_err(value_error)?
            .table_uuid()
            .to_owned())
    }

    /// The version number of the current metadata document.
    #[getter]
    fn version(slf: &Bound<'_, Self>) -> PyResult<u32> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        table.metadata_version().map_err(value_error)
    }

    /// The name of the current metadata document.
    #[getter]
    fn metadata_file_name(slf: &Bound<'_, Self>) -> PyResult<String> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        table.metadata_file_name().map_err(value_error)
    }

    /// The location of the current metadata document, as a URI.
    #[getter]
    fn metadata_location(slf: &Bound<'_, Self>) -> PyResult<String> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        table.metadata_location().map_err(value_error)
    }

    /// The schema new data is written against.
    #[getter]
    fn schema(slf: &Bound<'_, Self>) -> PyResult<PyField> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        table
            .schema()
            .cloned()
            .map(PyField::from_inner)
            .map_err(value_error)
    }

    /// The partition spec new data is written against.
    #[getter]
    fn spec(slf: &Bound<'_, Self>) -> PyResult<PyPartitionSpec> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        table
            .metadata()
            .map_err(value_error)?
            .default_spec()
            .cloned()
            .map(PyPartitionSpec::from_core)
            .map_err(value_error)
    }

    /// The snapshot a reader sees, when the table has one.
    ///
    /// A table that has been created but never written has none, which is not a
    /// failure: it simply reads as no rows.
    #[getter]
    fn current_snapshot(slf: &Bound<'_, Self>) -> PyResult<Option<PySnapshot>> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        Ok(table
            .current_snapshot()
            .map_err(value_error)?
            .cloned()
            .map(PySnapshot::from_core))
    }

    /// Every retained snapshot, oldest first.
    #[getter]
    fn snapshots(slf: &Bound<'_, Self>) -> PyResult<Vec<PySnapshot>> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        Ok(table
            .metadata()
            .map_err(value_error)?
            .snapshots()
            .iter()
            .cloned()
            .map(PySnapshot::from_core)
            .collect())
    }

    /// Every schema the table has had, by identifier.
    #[getter]
    fn schemas(slf: &Bound<'_, Self>) -> PyResult<Vec<PyField>> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        Ok(table
            .metadata()
            .map_err(value_error)?
            .schemas()
            .iter()
            .cloned()
            .map(PyField::from_inner)
            .collect())
    }

    /// Every manifest the current snapshot points at.
    fn manifests(slf: &Bound<'_, Self>) -> PyResult<Vec<PyManifestFile>> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        Ok(table
            .manifests()
            .map_err(value_error)?
            .into_iter()
            .map(PyManifestFile::from_core)
            .collect())
    }

    /// Every manifest one retained snapshot points at.
    ///
    /// The snapshot is named by identifier rather than passed as a value,
    /// because the table is the authority on which snapshots it still retains
    /// - a `Snapshot` a caller kept from before an expiry describes a
    /// manifest list that may be gone. An identifier the table no longer
    /// retains is a `ValueError` naming it and the ones it does, the same
    /// failure [`scan_at`](Self::scan_at) reports for the same reason.
    fn manifests_at(slf: &Bound<'_, Self>, snapshot_id: i64) -> PyResult<Vec<PyManifestFile>> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        let metadata = table.metadata().map_err(value_error)?;
        let snapshot = metadata.snapshot_by_id(snapshot_id).ok_or_else(|| {
            let retained: Vec<String> = metadata
                .snapshots()
                .iter()
                .map(|snapshot| snapshot.snapshot_id.to_string())
                .collect();
            PyValueError::new_err(format!(
                "expected a retained snapshot id, got {snapshot_id}; the table retains [{}]",
                retained.join(", ")
            ))
        })?;
        Ok(table
            .manifests_at(snapshot)
            .map_err(value_error)?
            .into_iter()
            .map(PyManifestFile::from_core)
            .collect())
    }

    /// Every live data file of the current snapshot, with the spec it was
    /// written under.
    fn data_files(slf: &Bound<'_, Self>) -> PyResult<Vec<(PyDataFile, PyPartitionSpec)>> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        Ok(table
            .data_files()
            .map_err(value_error)?
            .into_iter()
            .map(|(file, spec)| {
                (
                    PyDataFile::from_core(file),
                    PyPartitionSpec::from_core(spec),
                )
            })
            .collect())
    }

    /// Read the current snapshot as a `pyarrow.RecordBatchReader`.
    ///
    /// `field` is pushed down to each data file as its column projection and is
    /// then cast to the scan root, so files written under different schemas read
    /// as one shape.
    /// That cast is what makes a table whose schema evolved readable as one
    /// shape: a file written before a column existed contributes null for it.
    /// `options` configures this scan.
    #[pyo3(signature = (field = None, *, options = None, **properties))]
    fn scan<'py>(
        slf: &Bound<'_, Self>,
        py: Python<'py>,
        field: Option<&Bound<'_, PyAny>>,
        options: Option<&Bound<'_, PyAny>>,
        properties: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mut base = base_mut(slf)?;
        let table = held_mut(&mut base)?;
        let resolved = iceberg_call_options(options, properties, table.explicit_options())?;
        let field = field
            .map(|field| core_root_field_from_value(field, DEFAULT_ROOT_NAME))
            .transpose()?;
        let reader = with_call_options(table, resolved, |table| {
            table.scan(field.as_ref()).map_err(value_error)
        })?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// Read the rows matching `filters` as a `pyarrow.RecordBatchReader`.
    ///
    /// `filters` is a mapping or a sequence of `(column, value)` pairs - the
    /// vocabulary `IOBase.children_where` uses. A filter on a partition column
    /// is answered by the plan alone, because every row of a file whose
    /// partition tuple matches holds that value; a filter on any other column
    /// is applied to the rows the surviving files hold, because statistics
    /// bound a file rather than select a row. Either way the rows that come
    /// back are the rows that match. `field` and the options mean exactly what
    /// they mean on [`scan`](Self::scan).
    #[pyo3(signature = (filters = None, field = None, *, options = None, **properties))]
    fn scan_where<'py>(
        slf: &Bound<'_, Self>,
        py: Python<'py>,
        filters: Option<&Bound<'_, PyAny>>,
        field: Option<&Bound<'_, PyAny>>,
        options: Option<&Bound<'_, PyAny>>,
        properties: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mut base = base_mut(slf)?;
        let table = held_mut(&mut base)?;
        let resolved = iceberg_call_options(options, properties, table.explicit_options())?;
        let pairs = filter_pairs_from_value(filters)?;
        let field = field
            .map(|field| core_root_field_from_value(field, DEFAULT_ROOT_NAME))
            .transpose()?;
        let reader = with_call_options(table, resolved, |table| {
            table
                .scan_where(&borrowed_pairs(&pairs), field.as_ref())
                .map_err(value_error)
        })?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// Read the rows a branch or tag names, as a `pyarrow.RecordBatchReader`.
    ///
    /// This is [`snapshot_by_ref`](Self::snapshot_by_ref) followed by
    /// [`scan_at`](Self::scan_at), so a ref is read as the schema its snapshot
    /// was written under and `filters` and `field` mean what they mean there.
    /// A name the table does not carry is an error naming the refs it does.
    #[pyo3(signature = (name, filters = None, field = None, *, options = None, **properties))]
    fn scan_ref<'py>(
        slf: &Bound<'_, Self>,
        py: Python<'py>,
        name: &str,
        filters: Option<&Bound<'_, PyAny>>,
        field: Option<&Bound<'_, PyAny>>,
        options: Option<&Bound<'_, PyAny>>,
        properties: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mut base = base_mut(slf)?;
        let table = held_mut(&mut base)?;
        let resolved = iceberg_call_options(options, properties, table.explicit_options())?;
        let pairs = filter_pairs_from_value(filters)?;
        let field = field
            .map(|field| core_root_field_from_value(field, DEFAULT_ROOT_NAME))
            .transpose()?;
        let reader = with_call_options(table, resolved, |table| {
            table
                .scan_ref(name, &borrowed_pairs(&pairs), field.as_ref())
                .map_err(value_error)
        })?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// Read the rows matching one predicate as a `pyarrow.RecordBatchReader`.
    ///
    /// `filter` is a `Filter`, a `Term`, or the text of a predicate, which
    /// parses. It is the whole expression language rather than equality
    /// pairs: ranges, null tests, `in` lists, and nested paths. Planning
    /// prunes with the metadata chain, and only the conjuncts it could not
    /// settle are tested against the rows.
    #[pyo3(signature = (filter, schema = None))]
    fn scan_matching<'py>(
        slf: &Bound<'_, Self>,
        py: Python<'py>,
        filter: &Bound<'_, PyAny>,
        schema: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        let filter = crate::expression::filter_from_value(filter)?;
        let field = schema
            .map(|schema| core_root_field_from_value(schema, DEFAULT_ROOT_NAME))
            .transpose()?;
        let reader = table
            .scan_matching(filter, field.as_ref())
            .map_err(value_error)?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// Report what one predicate lets the scan leave alone.
    ///
    /// The mapping carries `tasks`, `files_skipped`, `manifests_read`,
    /// `manifests_skipped`, and `record_count`, so "a filtered read touches
    /// only the files the metadata says it must" is a number a caller checks.
    fn plan_matching<'py>(
        slf: &Bound<'_, Self>,
        py: Python<'py>,
        filter: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        let filter = crate::expression::filter_from_value(filter)?;
        let plan = table.plan_matching(filter).map_err(value_error)?;
        let answer = pyo3::types::PyDict::new(py);
        answer.set_item("tasks", plan.tasks.len())?;
        answer.set_item("files_skipped", plan.files_skipped())?;
        answer.set_item("manifests_read", plan.manifests_read)?;
        answer.set_item("manifests_skipped", plan.manifests_skipped())?;
        answer.set_item("record_count", plan.record_count().map_err(value_error)?)?;
        Ok(answer)
    }

    /// Append `batches` as a new snapshot, keeping everything already stored.
    ///
    /// A table whose schema states `identifier-field-ids` takes only the rows
    /// whose key - the identity partition columns, then the identifier
    /// columns, the key a merge naming none matches on - is neither stored in
    /// their partition nor met earlier in the same write: the first arrival
    /// is kept, no stored file is rewritten, and an append that keeps no row
    /// commits no snapshot. The rows it leaves out are counted only where a
    /// door answers an `IOResult` - `append_serie`, `append_arrow_reader` -
    /// since this one answers nothing. Such an append decides what is absent
    /// against one snapshot, so a concurrent commit that beats it raises the
    /// commit conflict, as a merge does, rather than rebasing. A table
    /// stating no identifier appends every row; the declaration is the only
    /// switch.
    ///
    /// `options` configures this write without changing the handle's own
    /// configuration.
    #[pyo3(signature = (batches, *, options = None, **properties))]
    fn append(
        slf: &Bound<'_, Self>,
        batches: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
        properties: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let mut base = base_mut(slf)?;
        let table = held_mut(&mut base)?;
        let resolved = iceberg_call_options(options, properties, table.explicit_options())?;
        let batches = iceberg_batch_reader(Some(&*table), batches)?;
        slf.py().detach(|| {
            with_call_options(table, resolved, |table| {
                table.commit_append(batches).map_err(value_error)
            })
        })
    }

    /// Replace the partitions `batches` fall in as a new snapshot: every
    /// row of an unpartitioned table, and of a partitioned one the
    /// partitions the rows touch - no row replaces nothing there, and
    /// `overwrite_where(None, [])` empties it.
    ///
    /// `options` configures this write as it does
    /// [`append`](Self::append).
    #[pyo3(signature = (batches, *, options = None, **properties))]
    fn overwrite(
        slf: &Bound<'_, Self>,
        batches: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
        properties: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let mut base = base_mut(slf)?;
        let table = held_mut(&mut base)?;
        let resolved = iceberg_call_options(options, properties, table.explicit_options())?;
        let batches = iceberg_batch_reader(Some(&*table), batches)?;
        slf.py().detach(|| {
            with_call_options(table, resolved, |table| {
                table.commit_overwrite(batches).map_err(value_error)
            })
        })
    }

    /// Replace only the rows `filters` selects with `batches`, keeping every
    /// other file.
    ///
    /// A file the filters exclude is carried into the new snapshot exactly as
    /// it is - same location, same statistics, same commit order - so
    /// overwriting one partition of a thousand rewrites one partition. Unlike
    /// [`append`](Self::append), an overwrite beaten by a concurrent commit
    /// cannot rebase: what it keeps was planned against a snapshot the winner
    /// may have replaced, and the incoming rows are already consumed, so it
    /// raises rather than risk losing the winner's rows. The caller re-reads
    /// and retries with fresh input.
    #[pyo3(signature = (filters, batches, *, options = None, **properties))]
    fn overwrite_where(
        slf: &Bound<'_, Self>,
        filters: Option<&Bound<'_, PyAny>>,
        batches: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
        properties: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let mut base = base_mut(slf)?;
        let table = held_mut(&mut base)?;
        let resolved = iceberg_call_options(options, properties, table.explicit_options())?;
        let pairs = filter_pairs_from_value(filters)?;
        let batches = iceberg_batch_reader(Some(&*table), batches)?;
        slf.py().detach(|| {
            with_call_options(table, resolved, |table| {
                table
                    .commit_overwrite_where(&borrowed_pairs(&pairs), batches)
                    .map_err(value_error)
            })
        })
    }

    /// Merge `batches` into the stored rows, matching on `merge_by`: a
    /// `Selector`, the text of one, or the key column names.
    ///
    /// An incoming row replaces the stored row whose match-key columns equal
    /// its own and is inserted when there is none, so this is the upsert. Only
    /// the files whose recorded bounds can hold an incoming key are read and
    /// rewritten - a file that is not read keeps every row it had, however
    /// coarse the statistics are - so the write costs the files it can
    /// actually change rather than the whole table.
    ///
    /// `merge_by` left out, `None` or `True` matches on the table's own key:
    /// its identity partition columns, then the columns its schema's
    /// `identifier-field-ids` name. A partitioned table stating no
    /// identifier replaces the partitions the rows fall in, and an
    /// unpartitioned one stating none is refused naming `$.merge_by`, as
    /// `False` always is.
    ///
    /// A stored row is replaced only where the last incoming row of its key
    /// differs from it: a partition whose rows the merge leaves as they were
    /// keeps its files under their exact paths, and a merge that changes no
    /// row and adds no key commits no snapshot. A merge keyed by the
    /// partition alone replaces the partitions its rows fall in whether or
    /// not a row changed.
    ///
    /// `safe` is the cast strictness the incoming batches are held to: the
    /// default refuses a value the table's column cannot hold rather than
    /// storing a silently wrapped one.
    #[pyo3(signature = (batches, merge_by = ellipsis(), *, safe = true, options = None, **properties))]
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands the `...` default over as `Py`.
    fn merge(
        slf: &Bound<'_, Self>,
        batches: &Bound<'_, PyAny>,
        merge_by: Py<PyAny>,
        safe: bool,
        options: Option<&Bound<'_, PyAny>>,
        properties: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let keys = merge_key_from_value(merge_by.bind(slf.py()))?;
        let mut base = base_mut(slf)?;
        let table = held_mut(&mut base)?;
        let resolved = iceberg_call_options(options, properties, table.explicit_options())?;
        let batches = iceberg_batch_reader(Some(&*table), batches)?;
        slf.py().detach(|| {
            with_call_options(table, resolved, |table| {
                table
                    .commit_merge(batches, &keys, safe)
                    .map_err(value_error)
            })
        })
    }

    /// Merge `batches` into the rows `filters` selects, matching on
    /// `merge_by`.
    ///
    /// The filters narrow which stored files the merge may touch at all, and
    /// the key bounds narrow that further, so an upsert into one partition
    /// reads one partition. Everything else - the match rule, the table's
    /// own key where `merge_by` is left out, `None` or `True`, the snapshot
    /// a merge changing nothing does not commit, `safe`, the refusal to
    /// rebase after a lost commit - is exactly [`merge`](Self::merge).
    #[pyo3(signature = (filters, batches, merge_by = ellipsis(), *, safe = true, options = None, **properties))]
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands the `...` default over as `Py`.
    fn merge_where(
        slf: &Bound<'_, Self>,
        filters: Option<&Bound<'_, PyAny>>,
        batches: &Bound<'_, PyAny>,
        merge_by: Py<PyAny>,
        safe: bool,
        options: Option<&Bound<'_, PyAny>>,
        properties: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let keys = merge_key_from_value(merge_by.bind(slf.py()))?;
        let mut base = base_mut(slf)?;
        let table = held_mut(&mut base)?;
        let resolved = iceberg_call_options(options, properties, table.explicit_options())?;
        let pairs = filter_pairs_from_value(filters)?;
        let batches = iceberg_batch_reader(Some(&*table), batches)?;
        slf.py().detach(|| {
            with_call_options(table, resolved, |table| {
                table
                    .commit_merge_where(&borrowed_pairs(&pairs), batches, &keys, safe)
                    .map_err(value_error)
            })
        })
    }

    /// Store an explicit options override every later call resolves first.
    ///
    /// A field the override sets shadows the table property of the same name,
    /// and a field it leaves unset still resolves property-then-default. The
    /// override lives on this handle alone - it is never written to the
    /// table; [`update_properties`](Self::update_properties) is what stores a
    /// setting on the table itself.
    fn set_options(slf: &Bound<'_, Self>, options: &Bound<'_, PyAny>) -> PyResult<()> {
        let mut base = base_mut(slf)?;
        let table = held_mut(&mut base)?;
        table.set_options(core_iceberg_options_from_value(options)?);
        Ok(())
    }

    /// Resolve this table's effective options, field by field: the explicit
    /// override, then the table property of the same name, then the default.
    fn options(slf: &Bound<'_, Self>) -> PyResult<PyIcebergOptions> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        table
            .options()
            .map(PyIcebergOptions::from_core)
            .map_err(value_error)
    }

    /// Add a schema, make it current, and write a new metadata document.
    fn evolve_schema(slf: &Bound<'_, Self>, schema: &Bound<'_, PyAny>) -> PyResult<i32> {
        let mut base = base_mut(slf)?;
        let table = held_mut(&mut base)?;
        let schema = core_root_field_from_value(schema, DEFAULT_ROOT_NAME)?;
        table.evolve_schema(schema).map_err(value_error)
    }

    /// Read one retained snapshot's rows: time travel as an ordinary scan.
    ///
    /// The rows are read as the schema that was current when the snapshot was
    /// written. `filters` is a mapping or a sequence of `(column, value)`
    /// pairs - the vocabulary `IOBase.children_where` uses - answered by the
    /// plan for a partition column and row by row for every other; `schema`
    /// keeps the columns it names, exactly as `scan` does.
    #[pyo3(signature = (snapshot_id, filters = None, schema = None, *, options = None, **properties))]
    fn scan_at<'py>(
        slf: &Bound<'_, Self>,
        py: Python<'py>,
        snapshot_id: i64,
        filters: Option<&Bound<'_, PyAny>>,
        schema: Option<&Bound<'_, PyAny>>,
        options: Option<&Bound<'_, PyAny>>,
        properties: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mut base = base_mut(slf)?;
        let table = held_mut(&mut base)?;
        let resolved = iceberg_call_options(options, properties, table.explicit_options())?;
        let pairs = filter_pairs_from_value(filters)?;
        let field = schema
            .map(|schema| core_root_field_from_value(schema, DEFAULT_ROOT_NAME))
            .transpose()?;
        let reader = with_call_options(table, resolved, |table| {
            table
                .scan_at(snapshot_id, &borrowed_pairs(&pairs), field.as_ref())
                .map_err(value_error)
        })?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// Plan the current snapshot's scan without reading a single row.
    ///
    /// The plan is what the metadata alone decided: which data files a scan
    /// would open, and how many files and manifests the partition tuples and
    /// column statistics let it leave closed. `filters` is the mapping or
    /// sequence of `(column, value)` pairs [`scan_where`](Self::scan_where)
    /// takes, so a caller can assert on the pruning before paying for the
    /// read.
    #[pyo3(signature = (filters = None))]
    fn plan(slf: &Bound<'_, Self>, filters: Option<&Bound<'_, PyAny>>) -> PyResult<PyScanPlan> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        let pairs = filter_pairs_from_value(filters)?;
        let plan = table.plan(&borrowed_pairs(&pairs)).map_err(value_error)?;
        PyScanPlan::from_core(&plan).map_err(value_error)
    }

    /// Plan one retained snapshot's scan: the planning half of time travel.
    ///
    /// The filters are resolved against the schema that was current when the
    /// snapshot was written, and the same three-level pruning a
    /// [`plan`](Self::plan) of the present runs applies, so history reports
    /// the numbers the present reports.
    #[pyo3(signature = (snapshot_id, filters = None))]
    fn plan_at(
        slf: &Bound<'_, Self>,
        snapshot_id: i64,
        filters: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyScanPlan> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        let pairs = filter_pairs_from_value(filters)?;
        let plan = table
            .plan_at(snapshot_id, &borrowed_pairs(&pairs))
            .map_err(value_error)?;
        PyScanPlan::from_core(&plan).map_err(value_error)
    }

    /// Create a branch at one retained snapshot, as one metadata commit.
    ///
    /// Writing *to* a branch other than `main` remains future work - a
    /// commit's parent is always the current snapshot - so a branch is read
    /// with [`scan_ref`](Self::scan_ref) and moved with
    /// [`fast_forward`](Self::fast_forward).
    fn create_branch(slf: &Bound<'_, Self>, name: &str, snapshot_id: i64) -> PyResult<()> {
        let mut base = base_mut(slf)?;
        let table = held_mut(&mut base)?;
        table.create_branch(name, snapshot_id).map_err(value_error)
    }

    /// Create a tag at one retained snapshot, as one metadata commit.
    fn create_tag(slf: &Bound<'_, Self>, name: &str, snapshot_id: i64) -> PyResult<()> {
        let mut base = base_mut(slf)?;
        let table = held_mut(&mut base)?;
        table.create_tag(name, snapshot_id).map_err(value_error)
    }

    /// Remove one branch or tag, as one metadata commit.
    ///
    /// A name the table does not have is an error rather than an empty
    /// commit.
    fn remove_ref(slf: &Bound<'_, Self>, name: &str) -> PyResult<()> {
        let mut base = base_mut(slf)?;
        let table = held_mut(&mut base)?;
        table
            .remove_snapshot_ref(name)
            .map(|_| ())
            .map_err(value_error)
    }

    /// Move a branch forward to a descendant snapshot, as one metadata commit.
    ///
    /// The target must be retained and must reach the branch's head by walking
    /// parent identifiers, so a fast-forward can never lose history: it is the
    /// one way a branch other than `main` moves, since a commit's parent is
    /// always the current snapshot.
    fn fast_forward(slf: &Bound<'_, Self>, name: &str, snapshot_id: i64) -> PyResult<()> {
        let mut base = base_mut(slf)?;
        let table = held_mut(&mut base)?;
        table
            .fast_forward_branch(name, snapshot_id)
            .map_err(value_error)
    }

    /// Expire the snapshots retention no longer keeps, returning their ids.
    ///
    /// Omitted cutoff and retain count use table properties. Explicit snapshot
    /// ids join age-based selection; retained heads cannot be removed.
    /// Statistics metadata is removed, while physical files remain.
    #[pyo3(signature = (older_than_ms = None, retain_last = None, snapshot_ids = None))]
    fn expire_snapshots(
        slf: &Bound<'_, Self>,
        older_than_ms: Option<i64>,
        retain_last: Option<usize>,
        snapshot_ids: Option<Vec<i64>>,
    ) -> PyResult<Vec<i64>> {
        let mut base = base_mut(slf)?;
        let table = held_mut(&mut base)?;
        let snapshot_ids = snapshot_ids.unwrap_or_default();
        table
            .expire_snapshots(older_than_ms, retain_last, &snapshot_ids)
            .map_err(value_error)
    }

    /// Return the retained snapshot a branch or tag names.
    ///
    /// The `main` branch follows the current snapshot, so a table that has
    /// been written to always answers for it.
    fn snapshot_by_ref(slf: &Bound<'_, Self>, name: &str) -> PyResult<PySnapshot> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        table
            .snapshot_by_ref(name)
            .cloned()
            .map(PySnapshot::from_core)
            .map_err(value_error)
    }

    /// The size a data file aims for, in bytes.
    ///
    /// The table property `write.target-file-size-bytes` decides, falling back
    /// to the schema root's `ICEBERG:` protocol property of the same name and
    /// then to Iceberg's own 512 MiB default.
    #[getter]
    fn target_file_size(slf: &Bound<'_, Self>) -> PyResult<u64> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        table.target_file_size_bytes().map_err(value_error)
    }

    /// Merge the current snapshot's undersized data files, partition by
    /// partition, as one `replace` snapshot.
    ///
    /// A table with nothing to compact is left exactly as it is: no snapshot
    /// is committed and the returned [`PyCompaction`] is all zeros.
    fn compact(slf: &Bound<'_, Self>) -> PyResult<PyCompaction> {
        let mut base = base_mut(slf)?;
        let table = held_mut(&mut base)?;
        slf.py()
            .detach(|| table.compact())
            .map(PyCompaction::from_core)
            .map_err(value_error)
    }

    /// Render when each snapshot became current, oldest first.
    ///
    /// The columns are `made_current_at`, `snapshot_id`, `parent_id`, and
    /// `is_current_ancestor`, the names `PyIceberg`'s `history` table uses.
    fn inspect_history<'py>(slf: &Bound<'_, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        let reader = table.inspect_history().map_err(value_error)?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// Render every retained snapshot with its operation and summary.
    ///
    /// The columns are `committed_at`, `snapshot_id`, `parent_id`,
    /// `operation`, `manifest_list`, and the free-form `summary` map.
    fn inspect_snapshots<'py>(
        slf: &Bound<'_, Self>,
        py: Python<'py>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        let reader = table.inspect_snapshots().map_err(value_error)?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// Render the live data files of the current snapshot.
    ///
    /// The columns are `file_path`, `file_format`, `spec_id`, the rendered
    /// `partition` chain, `record_count`, and `file_size_in_bytes`.
    fn inspect_files<'py>(slf: &Bound<'_, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        let reader = table.inspect_files().map_err(value_error)?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// Set and remove free-form table properties as one metadata commit.
    ///
    /// `updates` is a mapping or a sequence of `(key, value)` pairs and
    /// `removes` an iterable of keys; the updates land first, so a key named
    /// by both ends up removed. A call given neither commits nothing at all.
    #[pyo3(signature = (updates = None, removes = None))]
    fn update_properties(
        slf: &Bound<'_, Self>,
        updates: Option<&Bound<'_, PyAny>>,
        removes: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let mut base = base_mut(slf)?;
        let table = held_mut(&mut base)?;
        let updates = match updates {
            Some(value) => string_pairs_from_value(value)?,
            None => Vec::new(),
        };
        let removes = match removes {
            Some(value) => crate::enums::strings_from_iterable(value, "removes")?,
            None => Vec::new(),
        };
        if updates.is_empty() && removes.is_empty() {
            return Ok(());
        }
        table
            .commit_metadata_changes(|metadata| {
                for (key, value) in &updates {
                    metadata.set_property(key.as_str(), value.as_str())?;
                }
                for key in &removes {
                    metadata.remove_property(key)?;
                }
                Ok(())
            })
            .map_err(value_error)
    }

    /// Start recording a column-level schema evolution against this table.
    ///
    /// The recording methods touch nothing; [`PySchemaUpdate::commit`] hands
    /// the recording to the core `Table::update_schema`, which writes one new
    /// metadata document. `with table.update_schema() as update:` commits on a
    /// clean exit and discards on an exception.
    fn update_schema(slf: &Bound<'_, Self>) -> PyResult<PySchemaUpdate> {
        let update = {
            let borrowed = slf.try_borrow()?;
            let metadata = held(&borrowed)?.metadata().map_err(value_error)?;
            SchemaUpdate::from_metadata(metadata).map_err(value_error)?
        };
        Ok(PySchemaUpdate {
            table: slf.clone().unbind(),
            update: Some(update),
        })
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let slf = slf.try_borrow()?;
        let table = held(&slf)?;
        let metadata = table.metadata().map_err(value_error)?;
        Ok(format!(
            "IcebergTable({:?}, format_version={}, version={})",
            metadata.location(),
            metadata.format_version().number(),
            table.metadata_version().map_err(value_error)?,
        ))
    }
}

/// A recorded set of column operations against one table's current schema.
///
/// Built by `Table.update_schema`, it holds the core `SchemaUpdate`. Each
/// recording method returns the update itself so calls chain, and `commit`
/// hands it to the core `Table::update_schema` - added columns numbered above
/// `last-column-id`, renames keeping their identifier, promotions gated by
/// `can_promote`, a beaten commit replayed onto the winner's schema - as one
/// new metadata document. Used as a context manager, a clean exit commits and
/// an exception discards.
#[pyclass(name = "SchemaUpdate", module = "yggdryl._native", skip_from_py_object)]
pub(crate) struct PySchemaUpdate {
    /// The table the update was started from and commits back to.
    table: Py<PyIcebergTable>,
    /// The core recording; `None` once the update committed or was discarded.
    update: Option<SchemaUpdate>,
}

impl PySchemaUpdate {
    /// Borrow the recording, refusing an update already committed or discarded.
    fn open(&mut self) -> PyResult<&mut SchemaUpdate> {
        self.update.as_mut().ok_or_else(spent_schema_update)
    }
}

/// The refusal an update answers once it has committed or been discarded.
fn spent_schema_update() -> PyErr {
    PyValueError::new_err("expected an open schema update, got one already committed or discarded")
}

#[pymethods]
impl PySchemaUpdate {
    // A transactional update changes until commit or discard.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// Record a new column under `parent` - `""` for the root, a dotted path
    /// for a nested struct.
    ///
    /// `field` accepts anything a Field crosses the boundary as: the native
    /// wrapper, a field expression, a `PyArrow` field. On commit the column is
    /// numbered fresh above the table's `last-column-id`, so a retired
    /// identifier is never reused.
    fn add_column<'py>(
        mut slf: PyRefMut<'py, Self>,
        parent: &str,
        field: &Bound<'_, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let update = slf.open()?;
        update.add_column(parent, core_field_from_value(field)?);
        Ok(slf)
    }

    /// Record the removal of the column at `path`, retiring its identifier.
    fn drop_column<'py>(mut slf: PyRefMut<'py, Self>, path: &str) -> PyResult<PyRefMut<'py, Self>> {
        slf.open()?.drop_column(path);
        Ok(slf)
    }

    /// Record a rename of the column at `path`; its identifier keeps rows
    /// written under the pre-rename name readable.
    fn rename_column<'py>(
        mut slf: PyRefMut<'py, Self>,
        path: &str,
        name: &str,
    ) -> PyResult<PyRefMut<'py, Self>> {
        slf.open()?.rename_column(path, name);
        Ok(slf)
    }

    /// Record a new documentation string on the column at `path`: the
    /// column's own description, which the schema states as its `doc`. An
    /// empty one clears it.
    fn update_doc<'py>(
        mut slf: PyRefMut<'py, Self>,
        path: &str,
        doc: &str,
    ) -> PyResult<PyRefMut<'py, Self>> {
        slf.open()?.update_doc(path, doc);
        Ok(slf)
    }

    /// Record that the column at `path` becomes optional.
    ///
    /// Required to optional is the only direction nullability can evolve, so
    /// there is no reverse method.
    fn make_nullable<'py>(
        mut slf: PyRefMut<'py, Self>,
        path: &str,
    ) -> PyResult<PyRefMut<'py, Self>> {
        slf.open()?.make_nullable(path);
        Ok(slf)
    }

    /// Record a type promotion on the column at `path`, checked against the
    /// legal Iceberg promotions when the update commits.
    ///
    /// `dtype` accepts anything a datatype crosses the boundary as: the
    /// native wrapper, a datatype expression, a `PyArrow` type.
    fn update_type<'py>(
        mut slf: PyRefMut<'py, Self>,
        path: &str,
        dtype: &Bound<'_, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let update = slf.open()?;
        update.update_type(path, core_dtype_from_value(dtype)?);
        Ok(slf)
    }

    /// Commit the recorded operations as one new metadata document and
    /// return the schema identifier they made current.
    ///
    /// The core `Table::update_schema` replays the recording onto the
    /// metadata each attempt reads, so a commit beaten by another writer
    /// rebases onto the winner's schema; the table describes the new shape
    /// when this returns and the old one on any failure. An update that
    /// recorded nothing commits nothing and answers the current schema's
    /// identifier. The update is spent either way.
    fn commit(&mut self, py: Python<'_>) -> PyResult<i32> {
        let update = self.update.take().ok_or_else(spent_schema_update)?;
        let mut base = base_mut(self.table.bind(py))?;
        held_mut(&mut base)?
            .update_schema(&update)
            .map_err(value_error)
    }

    /// Enter the update's scope; the recording happens inside it.
    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Leave the scope: a clean exit commits, an exception discards.
    ///
    /// The exception is never swallowed, and a discarded update commits
    /// nothing, so the table still describes exactly what is stored.
    #[pyo3(signature = (exception_type = None, exception = None, traceback = None))]
    fn __exit__(
        &mut self,
        py: Python<'_>,
        exception_type: Option<&Bound<'_, PyAny>>,
        exception: Option<&Bound<'_, PyAny>>,
        traceback: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        let _ = (exception, traceback);
        if exception_type.is_some() {
            self.update = None;
            return Ok(false);
        }
        if self.update.is_some() {
            self.commit(py)?;
        }
        Ok(false)
    }

    fn __repr__(&self) -> String {
        format!(
            "SchemaUpdate(empty={}, open={})",
            if self.update.as_ref().is_none_or(SchemaUpdate::is_empty) {
                "True"
            } else {
                "False"
            },
            if self.update.is_some() {
                "True"
            } else {
                "False"
            },
        )
    }
}

/// A bounded five-count report of what a scan decided before reading rows.
///
/// The core plan holds the data files themselves, because a write needs them;
/// this view keeps only the counts, because a caller asking what the metadata
/// pruned is asking a question about numbers - "did partitioning work" - and
/// the file list is the scan's own business. The counts are the whole answer:
/// `files_planned` plus `files_skipped` is every live file a read manifest
/// listed, and `manifests_read` plus `manifests_skipped` is every manifest the
/// snapshot points at.
#[pyclass(
    name = "ScanPlan",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyScanPlan {
    /// The rows the planned files hold, as the manifests counted them.
    record_count: i64,
    /// The data files the scan will open.
    files_planned: usize,
    /// Live files a read manifest listed that the filters excluded.
    files_skipped: usize,
    /// Manifests that had to be opened because their summaries allowed a match.
    manifests_read: usize,
    /// Manifests excluded on their summary alone, never opened.
    manifests_skipped: usize,
}

impl PyScanPlan {
    fn from_core(plan: &ScanPlan) -> yggdryl::Result<Self> {
        Ok(Self {
            record_count: plan.record_count()?,
            files_planned: plan.tasks.len(),
            files_skipped: plan.files_skipped(),
            manifests_read: plan.manifests_read,
            manifests_skipped: plan.manifests_skipped(),
        })
    }

    const fn identity(&self) -> (i64, usize, usize, usize, usize) {
        (
            self.record_count,
            self.files_planned,
            self.files_skipped,
            self.manifests_read,
            self.manifests_skipped,
        )
    }

    fn identity_value(&self) -> Scalar {
        Scalar::from_sequence([
            Scalar::from(self.record_count),
            Scalar::from(u64::try_from(self.files_planned).unwrap_or(u64::MAX)),
            Scalar::from(u64::try_from(self.files_skipped).unwrap_or(u64::MAX)),
            Scalar::from(u64::try_from(self.manifests_read).unwrap_or(u64::MAX)),
            Scalar::from(u64::try_from(self.manifests_skipped).unwrap_or(u64::MAX)),
        ])
    }
}

#[pymethods]
impl PyScanPlan {
    /// Rebuild the complete count report for pickle without planning a scan.
    #[staticmethod]
    fn _from_pickle(
        record_count: i64,
        files_planned: usize,
        files_skipped: usize,
        manifests_read: usize,
        manifests_skipped: usize,
    ) -> Self {
        Self {
            record_count,
            files_planned,
            files_skipped,
            manifests_read,
            manifests_skipped,
        }
    }

    /// The rows the planned files hold, as the manifests counted them.
    ///
    /// This is the count a scan would yield only when every filter is on a
    /// partition column: a file survives on its statistics, and a filter on
    /// any other column then selects rows within it.
    #[getter]
    fn record_count(&self) -> i64 {
        self.record_count
    }

    /// How many data files the scan will open.
    #[getter]
    fn files_planned(&self) -> usize {
        self.files_planned
    }

    /// How many live data files the metadata let the scan leave closed.
    #[getter]
    fn files_skipped(&self) -> usize {
        self.files_skipped
    }

    /// How many manifests had to be opened to plan the scan.
    #[getter]
    fn manifests_read(&self) -> usize {
        self.manifests_read
    }

    /// How many manifests the manifest-list summaries alone ruled out.
    #[getter]
    fn manifests_skipped(&self) -> usize {
        self.manifests_skipped
    }

    fn __repr__(&self) -> String {
        format!(
            "ScanPlan._from_pickle({}, {}, {}, {}, {})",
            self.record_count,
            self.files_planned,
            self.files_skipped,
            self.manifests_read,
            self.manifests_skipped,
        )
    }

    /// Return a deterministic hash of the complete count report.
    fn stable_hash(&self) -> u64 {
        self.identity_value().stable_hash()
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(
            crate::compare(self.identity().cmp(&other.identity()), operation)
                .into_pyobject(other.py())?
                .to_owned()
                .into_any()
                .unbind(),
        )
    }

    #[allow(clippy::type_complexity)]
    fn __reduce__(
        &self,
        py: Python<'_>,
    ) -> PyResult<(Py<PyAny>, (i64, usize, usize, usize, usize))> {
        Ok((
            py.get_type::<Self>().getattr("_from_pickle")?.unbind(),
            self.identity(),
        ))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// What one `Table.compact` call did, in numbers a caller can assert on.
///
/// A compaction with nothing to do reports zeros, because it commits nothing.
#[pyclass(
    name = "Compaction",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyCompaction {
    inner: Compaction,
}

impl PyCompaction {
    fn from_core(inner: Compaction) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyCompaction {
    #[staticmethod]
    fn _from_pickle(files_before: usize, files_after: usize, bytes_rewritten: i64) -> Self {
        Self::from_core(Compaction {
            files_before,
            files_after,
            bytes_rewritten,
        })
    }

    /// How many live data files were read and replaced.
    #[getter]
    fn files_before(&self) -> usize {
        self.inner.files_before
    }

    /// How many data files the rewrite produced in their place.
    #[getter]
    fn files_after(&self) -> usize {
        self.inner.files_after
    }

    /// The recorded size of the replaced files, in bytes.
    #[getter]
    fn bytes_rewritten(&self) -> i64 {
        self.inner.bytes_rewritten
    }

    fn __repr__(&self) -> String {
        format!(
            "Compaction._from_pickle({}, {}, {})",
            self.inner.files_before, self.inner.files_after, self.inner.bytes_rewritten,
        )
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (usize, usize, i64))> {
        Ok((
            py.get_type::<Self>().getattr("_from_pickle")?.unbind(),
            (
                self.inner.files_before,
                self.inner.files_after,
                self.inner.bytes_rewritten,
            ),
        ))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// How a source column becomes a partition column.
#[pyclass(
    name = "PartitionField",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyPartitionField {
    inner: PartitionField,
}

#[pymethods]
#[allow(clippy::wrong_self_convention)] // Python projections preserve immutable wrappers.
impl PyPartitionField {
    /// Read one native partition-field JSON value.
    #[classmethod]
    fn from_json(_cls: &Bound<'_, PyType>, document: &Bound<'_, PyAny>) -> PyResult<Self> {
        let document = crate::scalar::from_py(document)?;
        PartitionField::from_json(&document)
            .map(|inner| Self { inner })
            .map_err(value_error)
    }

    /// The partition column's name, which is also its directory prefix.
    #[getter]
    fn name(&self) -> &str {
        self.inner.name.as_str()
    }

    /// The transform's Iceberg name, such as `identity` or `bucket[16]`.
    #[getter]
    fn transform(&self) -> String {
        self.inner.transform.to_string()
    }

    /// The identifier of the schema field this partitions on.
    #[getter]
    fn source_id(&self) -> i32 {
        self.inner.source_id
    }

    /// The identifier of the partition field itself.
    #[getter]
    fn field_id(&self) -> i32 {
        self.inner.field_id
    }

    /// Return the native partition-field JSON value as natural Python data.
    fn into_json(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let document = self.inner.clone().into_json().map_err(value_error)?;
        crate::scalar::as_py(py, &document)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let document = self.into_json(py)?;
        Ok(format!(
            "PartitionField.from_json({})",
            document.bind(py).repr()?.to_str()?
        ))
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        Ok((
            py.get_type::<Self>().getattr("from_json")?.unbind(),
            (self.into_json(py)?,),
        ))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// The columns a table partitions on, in directory order.
#[pyclass(
    name = "PartitionSpec",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyPartitionSpec {
    inner: PartitionSpec,
}

impl PyPartitionSpec {
    fn from_core(inner: PartitionSpec) -> Self {
        Self { inner }
    }
}

#[pymethods]
#[allow(clippy::wrong_self_convention)] // Python projections preserve immutable wrappers.
impl PyPartitionSpec {
    /// Read one native partition-spec JSON value.
    #[classmethod]
    fn from_json(_cls: &Bound<'_, PyType>, document: &Bound<'_, PyAny>) -> PyResult<Self> {
        let document = crate::scalar::from_py(document)?;
        PartitionSpec::from_json(&document)
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// The unpartitioned spec, which every table has as spec zero.
    #[classmethod]
    fn unpartitioned(_cls: &Bound<'_, PyType>) -> Self {
        Self::from_core(PartitionSpec::unpartitioned())
    }

    /// Partition on the named columns' values, unchanged.
    ///
    /// Identity is one of the two transforms that can place a row, so it is the
    /// one a table written from here uses.
    #[classmethod]
    #[pyo3(signature = (schema, columns, *, spec_id = 0))]
    fn identity(
        _cls: &Bound<'_, PyType>,
        schema: &Bound<'_, PyAny>,
        columns: &Bound<'_, PyAny>,
        spec_id: i32,
    ) -> PyResult<Self> {
        let schema = core_root_field_from_value(schema, DEFAULT_ROOT_NAME)?;
        let names = crate::enums::strings_from_iterable(columns, "columns")?;
        let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();
        PartitionSpec::identity(spec_id, &schema, &borrowed)
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// The identifier of this spec within the table.
    #[getter]
    fn spec_id(&self) -> i32 {
        self.inner.spec_id
    }

    /// Whether the spec partitions on nothing.
    fn is_unpartitioned(&self) -> bool {
        self.inner.is_unpartitioned()
    }

    /// The partition columns, in the order they nest as directories.
    #[getter]
    fn fields(&self) -> Vec<PyPartitionField> {
        self.inner
            .fields
            .iter()
            .cloned()
            .map(|inner| PyPartitionField { inner })
            .collect()
    }

    fn __len__(&self) -> usize {
        self.inner.fields.len()
    }

    /// Return the native partition-spec JSON value as natural Python data.
    fn into_json(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let document = self.inner.clone().into_json().map_err(value_error)?;
        crate::scalar::as_py(py, &document)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let document = self.into_json(py)?;
        Ok(format!(
            "PartitionSpec.from_json({})",
            document.bind(py).repr()?.to_str()?
        ))
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        Ok((
            py.get_type::<Self>().getattr("from_json")?.unbind(),
            (self.into_json(py)?,),
        ))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// One commit: what a table looked like at a point in time.
#[pyclass(
    name = "Snapshot",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PySnapshot {
    inner: Snapshot,
}

impl PySnapshot {
    fn from_core(inner: Snapshot) -> Self {
        Self { inner }
    }
}

#[pymethods]
#[allow(clippy::wrong_self_convention)] // Python projections preserve immutable wrappers.
impl PySnapshot {
    /// Read one native snapshot JSON value.
    #[classmethod]
    fn from_json(_cls: &Bound<'_, PyType>, document: &Bound<'_, PyAny>) -> PyResult<Self> {
        let document = crate::scalar::from_py(document)?;
        Snapshot::from_json(&document)
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// The identifier of this snapshot, unique within the table.
    #[getter]
    fn snapshot_id(&self) -> i64 {
        self.inner.snapshot_id
    }

    /// The snapshot this one was produced from, when there was one.
    #[getter]
    fn parent_snapshot_id(&self) -> Option<i64> {
        self.inner.parent_snapshot_id
    }

    /// The commit order, absent in v1 tables.
    #[getter]
    fn sequence_number(&self) -> Option<i64> {
        self.inner.sequence_number
    }

    /// When the commit happened, in milliseconds since the Unix epoch.
    #[getter]
    fn timestamp_ms(&self) -> i64 {
        self.inner.timestamp_ms
    }

    /// The location of the manifest list this snapshot's manifests are in.
    #[getter]
    fn manifest_list(&self) -> &str {
        self.inner.manifest_list.as_str()
    }

    /// Direct manifest locations carried by a v1 snapshot.
    #[getter]
    fn manifests<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyTuple>>> {
        self.inner
            .manifests
            .as_ref()
            .map(|paths| PyTuple::new(py, paths.iter().map(<_ as AsRef<str>>::as_ref)))
            .transpose()
    }

    /// What the commit did, defaulting to `append`.
    #[getter]
    fn operation(&self) -> &str {
        self.inner.operation()
    }

    /// The schema in effect when the snapshot was written.
    #[getter]
    fn schema_id(&self) -> Option<i32> {
        self.inner.schema_id
    }

    /// V3 encryption key used by this snapshot, when encrypted.
    #[getter]
    fn encryption_key_id(&self) -> Option<&str> {
        self.inner.encryption_key_id.as_deref()
    }

    /// First row identifier assigned by a v3 snapshot.
    #[getter]
    fn first_row_id(&self) -> Option<i64> {
        self.inner.first_row_id
    }

    /// Rows added by a v3 snapshot.
    #[getter]
    fn added_rows(&self) -> Option<i64> {
        self.inner.added_rows
    }

    /// Everything the commit recorded about itself.
    #[getter]
    fn summary<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let summary = PyDict::new(py);
        for (key, value) in &self.inner.summary {
            summary.set_item(key.as_str(), value.as_str())?;
        }
        Ok(summary)
    }

    /// Return the native snapshot JSON value for one Iceberg format version.
    #[pyo3(signature = (version=3))]
    fn into_json(&self, py: Python<'_>, version: i64) -> PyResult<Py<PyAny>> {
        let version = FormatVersion::from_number(version).map_err(value_error)?;
        let document = self.inner.clone().into_json(version).map_err(value_error)?;
        crate::scalar::as_py(py, &document)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let version = if self.inner.sequence_number.is_none() {
            1
        } else {
            3
        };
        let document = self.into_json(py, version)?;
        Ok(format!(
            "Snapshot.from_json({})",
            document.bind(py).repr()?.to_str()?
        ))
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let version = if self.inner.sequence_number.is_none() {
            1
        } else {
            3
        };
        Ok((
            py.get_type::<Self>().getattr("from_json")?.unbind(),
            (self.into_json(py, version)?,),
        ))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// One manifest of a snapshot: which files it covers and what they hold.
#[pyclass(
    name = "ManifestFile",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyManifestFile {
    inner: ManifestFile,
}

impl PyManifestFile {
    fn from_core(inner: ManifestFile) -> Self {
        Self { inner }
    }

    fn partitions_view<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let mut partitions = Vec::with_capacity(self.inner.partitions.len());
        for summary in &self.inner.partitions {
            let contains_null = summary
                .contains_null
                .into_pyobject(py)?
                .to_owned()
                .into_any()
                .unbind();
            let contains_nan = match summary.contains_nan {
                Some(value) => value.into_pyobject(py)?.to_owned().into_any().unbind(),
                None => py.None(),
            };
            let lower_bound = summary.lower_bound.as_deref().map_or_else(
                || py.None(),
                |value| PyBytes::new(py, value).into_any().unbind(),
            );
            let upper_bound = summary.upper_bound.as_deref().map_or_else(
                || py.None(),
                |value| PyBytes::new(py, value).into_any().unbind(),
            );
            partitions.push(
                PyTuple::new(py, [contains_null, contains_nan, lower_bound, upper_bound])?
                    .into_any()
                    .unbind(),
            );
        }
        PyTuple::new(py, partitions)
    }

    fn pickle_state<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let state = PyDict::new(py);
        state.set_item("manifest_path", self.inner.manifest_path.as_str())?;
        state.set_item("manifest_length", self.inner.manifest_length)?;
        state.set_item("partition_spec_id", self.inner.partition_spec_id)?;
        state.set_item("content", self.inner.content.code())?;
        state.set_item("sequence_number", self.inner.sequence_number)?;
        state.set_item("min_sequence_number", self.inner.min_sequence_number)?;
        state.set_item("added_snapshot_id", self.inner.added_snapshot_id)?;
        state.set_item("added_files_count", self.inner.added_files_count)?;
        state.set_item("existing_files_count", self.inner.existing_files_count)?;
        state.set_item("deleted_files_count", self.inner.deleted_files_count)?;
        state.set_item("added_rows_count", self.inner.added_rows_count)?;
        state.set_item("existing_rows_count", self.inner.existing_rows_count)?;
        state.set_item("deleted_rows_count", self.inner.deleted_rows_count)?;
        state.set_item("partitions", self.partitions_view(py)?)?;
        match self.inner.key_metadata.as_deref() {
            Some(bytes) => state.set_item("key_metadata", PyBytes::new(py, bytes))?,
            None => state.set_item("key_metadata", py.None())?,
        }
        state.set_item("first_row_id", self.inner.first_row_id)?;
        Ok(state)
    }
}

#[pymethods]
impl PyManifestFile {
    /// Rebuild the complete immutable manifest-list row for pickle.
    #[staticmethod]
    fn _from_pickle(state: &Bound<'_, PyDict>) -> PyResult<Self> {
        let partitions = required_pickle_item(state, "partitions")?
            .extract::<Vec<(bool, Option<bool>, Option<Vec<u8>>, Option<Vec<u8>>)>>()?
            .into_iter()
            .map(
                |(contains_null, contains_nan, lower_bound, upper_bound)| FieldSummary {
                    contains_null,
                    contains_nan,
                    lower_bound,
                    upper_bound,
                },
            )
            .collect();
        Ok(Self::from_core(ManifestFile {
            manifest_path: required_pickle_item(state, "manifest_path")?
                .extract::<String>()?
                .into(),
            manifest_length: required_pickle_item(state, "manifest_length")?.extract()?,
            partition_spec_id: required_pickle_item(state, "partition_spec_id")?.extract()?,
            content: ManifestContent::from_code(required_pickle_item(state, "content")?.extract()?)
                .map_err(value_error)?,
            sequence_number: required_pickle_item(state, "sequence_number")?.extract()?,
            min_sequence_number: required_pickle_item(state, "min_sequence_number")?.extract()?,
            added_snapshot_id: required_pickle_item(state, "added_snapshot_id")?.extract()?,
            added_files_count: required_pickle_item(state, "added_files_count")?.extract()?,
            existing_files_count: required_pickle_item(state, "existing_files_count")?.extract()?,
            deleted_files_count: required_pickle_item(state, "deleted_files_count")?.extract()?,
            added_rows_count: required_pickle_item(state, "added_rows_count")?.extract()?,
            existing_rows_count: required_pickle_item(state, "existing_rows_count")?.extract()?,
            deleted_rows_count: required_pickle_item(state, "deleted_rows_count")?.extract()?,
            partitions,
            key_metadata: required_pickle_item(state, "key_metadata")?.extract()?,
            first_row_id: required_pickle_item(state, "first_row_id")?.extract()?,
        }))
    }

    /// The manifest's location, as a URI.
    #[getter]
    fn path(&self) -> &str {
        self.inner.manifest_path.as_str()
    }

    /// The size of the manifest in bytes.
    #[getter]
    fn length(&self) -> i64 {
        self.inner.manifest_length
    }

    /// The identifier of the spec the manifest's entries were written under.
    #[getter]
    fn partition_spec_id(&self) -> i32 {
        self.inner.partition_spec_id
    }

    /// Whether the manifest lists `data` files or `deletes`.
    #[getter]
    fn content(&self) -> String {
        match self.inner.content {
            ManifestContent::Data => "data".to_owned(),
            ManifestContent::Deletes => "deletes".to_owned(),
            other => other.code().to_string(),
        }
    }

    /// Whether the manifest lists data files rather than delete files.
    fn is_data(&self) -> bool {
        self.inner.content == ManifestContent::Data
    }

    /// The commit order assigned when the manifest was added.
    #[getter]
    fn sequence_number(&self) -> i64 {
        self.inner.sequence_number
    }

    /// The lowest commit order of any entry in the manifest.
    #[getter]
    fn min_sequence_number(&self) -> i64 {
        self.inner.min_sequence_number
    }

    /// The snapshot that added the manifest.
    #[getter]
    fn added_snapshot_id(&self) -> i64 {
        self.inner.added_snapshot_id
    }

    /// The files the manifest marks added.
    #[getter]
    fn added_files_count(&self) -> Option<i32> {
        self.inner.added_files_count
    }

    /// The files the manifest marks existing.
    #[getter]
    fn existing_files_count(&self) -> Option<i32> {
        self.inner.existing_files_count
    }

    /// The files the manifest marks deleted.
    #[getter]
    fn deleted_files_count(&self) -> Option<i32> {
        self.inner.deleted_files_count
    }

    /// The rows in the added files.
    #[getter]
    fn added_rows_count(&self) -> Option<i64> {
        self.inner.added_rows_count
    }

    /// The rows in the existing files.
    #[getter]
    fn existing_rows_count(&self) -> Option<i64> {
        self.inner.existing_rows_count
    }

    /// The rows in the deleted files, when reported.
    #[getter]
    fn deleted_rows_count(&self) -> Option<i64> {
        self.inner.deleted_rows_count
    }

    /// Partition summaries in the partition spec's field order.
    #[getter]
    fn partitions<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        self.partitions_view(py)
    }

    /// Implementation-specific encryption metadata for the manifest file.
    #[getter]
    fn key_metadata<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.inner
            .key_metadata
            .as_deref()
            .map(|bytes| PyBytes::new(py, bytes))
    }

    /// First row identifier assigned by a v3 manifest.
    #[getter]
    fn first_row_id(&self) -> Option<i64> {
        self.inner.first_row_id
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let state = self.pickle_state(py)?;
        Ok(format!(
            "ManifestFile._from_pickle({})",
            state.repr()?.to_str()?
        ))
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        Ok((
            py.get_type::<Self>().getattr("_from_pickle")?.unbind(),
            (self.pickle_state(py)?.into_any().unbind(),),
        ))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// One data file a manifest lists.
#[pyclass(
    name = "DataFile",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyDataFile {
    inner: DataFile,
}

impl PyDataFile {
    fn from_core(inner: DataFile) -> Self {
        Self { inner }
    }

    fn pickle_state<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let state = PyDict::new(py);
        state.set_item("content", self.inner.content)?;
        state.set_item("file_path", self.inner.file_path.as_str())?;
        state.set_item("mime_type", self.inner.mime_type.as_str())?;
        let partition = self
            .inner
            .partition
            .iter()
            .map(|value| crate::scalar::scalar_pickle_state(py, value))
            .collect::<PyResult<Vec<_>>>()?;
        state.set_item("partition", PyTuple::new(py, partition)?)?;
        state.set_item("record_count", self.inner.record_count)?;
        state.set_item("file_size_in_bytes", self.inner.file_size_in_bytes)?;
        state.set_item("column_sizes", self.inner.column_sizes.clone())?;
        state.set_item("value_counts", self.inner.value_counts.clone())?;
        state.set_item("null_value_counts", self.inner.null_value_counts.clone())?;
        state.set_item("nan_value_counts", self.inner.nan_value_counts.clone())?;
        state.set_item("lower_bounds", self.inner.lower_bounds.clone())?;
        state.set_item("upper_bounds", self.inner.upper_bounds.clone())?;
        match self.inner.key_metadata.as_deref() {
            Some(bytes) => state.set_item("key_metadata", PyBytes::new(py, bytes))?,
            None => state.set_item("key_metadata", py.None())?,
        }
        state.set_item("split_offsets", self.inner.split_offsets.clone())?;
        state.set_item("equality_ids", self.inner.equality_ids.clone())?;
        state.set_item("sort_order_id", self.inner.sort_order_id)?;
        state.set_item("first_row_id", self.inner.first_row_id)?;
        state.set_item(
            "referenced_data_file",
            self.inner.referenced_data_file.as_deref(),
        )?;
        state.set_item("content_offset", self.inner.content_offset)?;
        state.set_item("content_size_in_bytes", self.inner.content_size_in_bytes)?;
        Ok(state)
    }
}

#[pymethods]
impl PyDataFile {
    /// Rebuild the complete immutable data-file description for pickle.
    #[staticmethod]
    fn _from_pickle(state: &Bound<'_, PyDict>) -> PyResult<Self> {
        let partition = required_pickle_item(state, "partition")?
            .try_iter()?
            .map(|value| crate::scalar::scalar_from_pickle_state(&value?, 0))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(Self::from_core(DataFile {
            content: required_pickle_item(state, "content")?.extract()?,
            file_path: required_pickle_item(state, "file_path")?
                .extract::<String>()?
                .into(),
            mime_type: core_mime_type_from_value(&required_pickle_item(state, "mime_type")?)?,
            partition,
            record_count: required_pickle_item(state, "record_count")?.extract()?,
            file_size_in_bytes: required_pickle_item(state, "file_size_in_bytes")?.extract()?,
            column_sizes: required_pickle_item(state, "column_sizes")?.extract()?,
            value_counts: required_pickle_item(state, "value_counts")?.extract()?,
            null_value_counts: required_pickle_item(state, "null_value_counts")?.extract()?,
            nan_value_counts: required_pickle_item(state, "nan_value_counts")?.extract()?,
            lower_bounds: required_pickle_item(state, "lower_bounds")?.extract()?,
            upper_bounds: required_pickle_item(state, "upper_bounds")?.extract()?,
            key_metadata: required_pickle_item(state, "key_metadata")?.extract()?,
            split_offsets: required_pickle_item(state, "split_offsets")?.extract()?,
            equality_ids: required_pickle_item(state, "equality_ids")?.extract()?,
            sort_order_id: required_pickle_item(state, "sort_order_id")?.extract()?,
            first_row_id: required_pickle_item(state, "first_row_id")?.extract()?,
            referenced_data_file: required_pickle_item(state, "referenced_data_file")?
                .extract::<Option<String>>()?
                .map(Into::into),
            content_offset: required_pickle_item(state, "content_offset")?.extract()?,
            content_size_in_bytes: required_pickle_item(state, "content_size_in_bytes")?
                .extract()?,
        }))
    }

    /// The file's location, as a URI.
    #[getter]
    fn path(&self) -> &str {
        self.inner.file_path.as_str()
    }

    /// The encoding the file uses.
    #[getter]
    fn mime_type(&self) -> PyMimeType {
        PyMimeType::from_core(self.inner.mime_type.clone())
    }

    /// The partition tuple, one value per partition field of the spec.
    ///
    /// The manifest is the authority on a partition value, not the directory
    /// name: a null is spelled `null` in a path, and a path cannot say whether
    /// that is the string or the absence.
    #[getter]
    fn partition<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        partition_values(py, &self.inner.partition)
    }

    /// The rows in the file.
    #[getter]
    fn record_count(&self) -> i64 {
        self.inner.record_count
    }

    /// The size of the file in bytes.
    #[getter]
    fn file_size_in_bytes(&self) -> i64 {
        self.inner.file_size_in_bytes
    }

    /// The values per column, keyed by field identifier.
    #[getter]
    fn value_counts<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        counts_by_id(py, &self.inner.value_counts)
    }

    /// The nulls per column, keyed by field identifier.
    #[getter]
    fn null_value_counts<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        counts_by_id(py, &self.inner.null_value_counts)
    }

    /// The NaN values per column, keyed by field identifier.
    #[getter]
    fn nan_value_counts<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        counts_by_id(py, &self.inner.nan_value_counts)
    }

    /// The stored bytes per column, keyed by field identifier.
    #[getter]
    fn column_sizes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        counts_by_id(py, &self.inner.column_sizes)
    }

    /// The minimum per column, keyed by field identifier.
    ///
    /// A bound travels as the encoded value Iceberg stores, not as a decoded
    /// scalar, and it is present only for the types whose encoding the two
    /// formats agree on - which is what makes it safe to compare.
    #[getter]
    fn lower_bounds<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        bounds_by_id(py, &self.inner.lower_bounds)
    }

    /// The maximum per column, keyed by field identifier.
    #[getter]
    fn upper_bounds<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        bounds_by_id(py, &self.inner.upper_bounds)
    }

    /// The byte offsets a reader may split the file at.
    #[getter]
    fn split_offsets(&self) -> Vec<i64> {
        self.inner.split_offsets.clone()
    }

    /// Implementation-specific encryption key metadata.
    #[getter]
    fn key_metadata<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.inner
            .key_metadata
            .as_deref()
            .map(|bytes| PyBytes::new(py, bytes))
    }

    /// Field identifiers used by an equality-delete file.
    #[getter]
    fn equality_ids(&self) -> Option<Vec<i32>> {
        self.inner.equality_ids.clone()
    }

    /// The sort order the file was written in, when one applies.
    #[getter]
    fn sort_order_id(&self) -> Option<i32> {
        self.inner.sort_order_id
    }

    /// First row identifier assigned to this v3 data file.
    #[getter]
    fn first_row_id(&self) -> Option<i64> {
        self.inner.first_row_id
    }

    /// Data file referenced by position-delete metadata.
    #[getter]
    fn referenced_data_file(&self) -> Option<&str> {
        self.inner.referenced_data_file.as_deref()
    }

    /// Byte offset of referenced v3 content.
    #[getter]
    fn content_offset(&self) -> Option<i64> {
        self.inner.content_offset
    }

    /// Byte length of referenced v3 content.
    #[getter]
    fn content_size_in_bytes(&self) -> Option<i64> {
        self.inner.content_size_in_bytes
    }

    /// Zero for rows, one for position deletes, two for equality deletes.
    #[getter]
    fn content(&self) -> i32 {
        self.inner.content
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let state = self.pickle_state(py)?;
        Ok(format!(
            "DataFile._from_pickle({})",
            state.repr()?.to_str()?
        ))
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        Ok((
            py.get_type::<Self>().getattr("_from_pickle")?.unbind(),
            (self.pickle_state(py)?.into_any().unbind(),),
        ))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}
