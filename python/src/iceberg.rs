//! An Apache Iceberg table, over the same `IOBase` handle Python already has.
//!
//! A table is a folder and nothing else, so this binding takes the handle a
//! caller already built with [`crate::io::PyIOBase`] and hands it to the core
//! [`Table`]. Rows cross the boundary the way they do everywhere else here -
//! as a `pyarrow.RecordBatchReader` over the Arrow C Stream interface - so a
//! scan is lazy on both sides and a commit copies nothing.
//!
//! The metadata values below (a snapshot, a manifest, a data file, a partition
//! spec) are read-only views of the core structs. They exist so a caller can
//! ask what a commit produced without opening the Avro files by hand; none of
//! them can be constructed from Python, because only a commit writes one.

use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyTuple, PyType};

use yggdryl::generic::Holder;
use yggdryl::generic::IORecordOptions as _;
use yggdryl::iceberg::{
    Catalog, Compaction, DataFile, FieldSummary, FormatVersion, IcebergOptions, ManifestContent,
    ManifestFile, PartitionField, PartitionSpec, ScanPlan, SchemaUpdate, Snapshot, Table,
    assign_field_ids, can_promote, last_column_id, schema_from_json, schema_into_json,
};
use yggdryl::io::IOBase as _;
use yggdryl::{DataType as CoreDataType, Field as CoreField, Scalar};

use crate::datatype::core_data_type_from_value;
use crate::field::{PyField, core_field_from_value};
use crate::io::PyIOBase;
use crate::media::{PyMimeType, core_mime_type_from_value};
use crate::record::{batch_reader_from_any, batch_reader_to_pyarrow, core_root_field_from_value};
use crate::uri::core_url_from_value;
use crate::value_error;

/// The root name given to a schema that arrives as a bare Arrow schema.
const SCHEMA_ROOT_NAME: &str = "row";

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
    let mut root = core_root_field_from_value(schema, SCHEMA_ROOT_NAME)?;
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
    let root = core_root_field_from_value(schema, SCHEMA_ROOT_NAME)?;
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
    let from_type = core_data_type_from_value(from_type)?;
    let to_type = core_data_type_from_value(to_type)?;
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

/// Read a core partition spec out of what Python names one with.
///
/// A sequence of column names is the spelling a caller reaches for, and it
/// means the identity transform over those columns - the only transform that
/// can place a row without inverting a hash.
fn spec_from_value(value: &Bound<'_, PyAny>, schema: &CoreField) -> PyResult<PartitionSpec> {
    if let Ok(spec) = value.extract::<PyRef<'_, PyPartitionSpec>>() {
        return Ok(spec.inner.clone());
    }
    let columns = crate::media::strings_from_iterable(value, "partition_by")?;
    let borrowed: Vec<&str> = columns.iter().map(String::as_str).collect();
    PartitionSpec::identity(0, schema, &borrowed).map_err(value_error)
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

/// Read a schema root out of what Python describes a table's columns with.
///
/// Everything [`PyTable::create`] accepts passes through the same boundary
/// helper unchanged; a plain iterable of Fields is assembled into a struct
/// root besides, because a caller building columns one by one holds exactly
/// that.
fn catalog_schema_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreField> {
    if value.extract::<PyRef<'_, PyField>>().is_ok()
        || value.extract::<&str>().is_ok()
        || value.hasattr("__arrow_c_schema__")?
    {
        return core_root_field_from_value(value, SCHEMA_ROOT_NAME);
    }
    if let Ok(items) = value.try_iter() {
        let mut fields = Vec::new();
        for item in items {
            fields.push(core_field_from_value(&item?)?);
        }
        let data_type = CoreDataType::from_fields(fields).map_err(value_error)?;
        return Ok(data_type.required_field(SCHEMA_ROOT_NAME));
    }
    core_root_field_from_value(value, SCHEMA_ROOT_NAME)
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
    let mut schema = core_root_field_from_value(value, SCHEMA_ROOT_NAME)?;
    let start = last_column_id(&schema)
        .map_err(value_error)?
        .saturating_add(1);
    assign_field_ids(&mut schema, start).map_err(value_error)?;
    Ok(schema)
}

/// Read `(key, value)` string pairs out of a mapping or an iterable of pairs.
///
/// These are the two shapes `IOBase.children_where` already accepts for the
/// same vocabulary, so a filter and a property update are spelled alike.
fn string_pairs_from_value(value: &Bound<'_, PyAny>) -> PyResult<Vec<(String, String)>> {
    let items = if value.hasattr("items")? {
        value.call_method0("items")?
    } else {
        value.clone()
    };
    let mut pairs = Vec::new();
    for item in items.try_iter()? {
        pairs.push(item?.extract::<(String, String)>()?);
    }
    Ok(pairs)
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
fn folder_holder_from_value(value: &Bound<'_, PyAny>) -> PyResult<Holder> {
    if let Ok(handle) = value.extract::<PyRef<'_, PyIOBase>>() {
        return handle.folder_holder();
    }
    let url = core_url_from_value(value)?;
    Holder::folder(url.into_path().map_err(value_error)?).map_err(value_error)
}

/// The keyword fields accepted by the `IcebergOptions` constructor.
const ICEBERG_OPTION_FIELDS: [&str; 10] = [
    "commit_retries",
    "commit_min_backoff_ms",
    "commit_max_backoff_ms",
    "commit_total_timeout_ms",
    "target_file_size",
    "read_parallelism",
    "read_parallel_min_files",
    "read_parallel_min_file_size",
    "compact_after_commits",
    "data_mime_type",
];

/// Set one Iceberg option field from the Python value a keyword carries.
fn set_iceberg_option(
    options: &mut IcebergOptions,
    key: &str,
    value: &Bound<'_, PyAny>,
) -> PyResult<()> {
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
        "compact_after_commits" => options.set_compact_after_commits(value.extract::<u32>()?),
        "data_mime_type" => options
            .set_data_mime_type(core_mime_type_from_value(value)?)
            .map_err(value_error)?,
        _ => unreachable!("apply_iceberg_option_fields checks the key first"),
    }
    Ok(())
}

/// Apply `IcebergOptions` constructor fields in canonical order.
fn apply_iceberg_option_fields(
    method: &str,
    options: &mut IcebergOptions,
    kwargs: Option<&Bound<'_, PyDict>>,
) -> PyResult<()> {
    let Some(kwargs) = kwargs else {
        return Ok(());
    };
    for (key, _) in kwargs.iter() {
        let key = key.extract::<String>()?;
        if !ICEBERG_OPTION_FIELDS.contains(&key.as_str()) {
            return Err(PyTypeError::new_err(format!(
                "{method}() got an unexpected keyword argument {key:?}"
            )));
        }
    }
    for key in ICEBERG_OPTION_FIELDS {
        if let Some(value) = kwargs.get_item(key)? {
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
        .extract::<PyRef<'_, crate::record::PyRecordOptions>>()
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

/// Import an optional per-call options value.
fn iceberg_call_options(options: Option<&Bound<'_, PyAny>>) -> PyResult<Option<IcebergOptions>> {
    options.map(core_iceberg_options_from_value).transpose()
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
    table: Option<&Table<Holder>>,
    value: &Bound<'_, PyAny>,
) -> PyResult<yggdryl::arrow::BatchReader> {
    let mut options = yggdryl::generic::RecordOptions::for_mime_type(&yggdryl::MimeType::PARQUET)
        .map_err(value_error)?;
    if let Some(schema) = table.and_then(|table| table.schema().ok()) {
        options.set_field(schema.clone());
    }
    batch_reader_from_any(value, &options)
}

/// Read a batch reader for a write that names its table, not a handle.
///
/// A table that already exists declares the schema its rows are typed
/// against; a create-on-write names none yet, and the rows declare it. The
/// lookup costs one metadata read on a call that is about to write several.
fn iceberg_named_batch_reader(
    tables: &yggdryl::iceberg::Tables<'_, Holder>,
    name: &str,
    value: &Bound<'_, PyAny>,
) -> PyResult<yggdryl::arrow::BatchReader> {
    let stored = tables.get(name).ok();
    iceberg_batch_reader(stored.as_ref(), value)
}

/// Run one table operation under per-call options, restoring the handle after.
///
/// The override is shadowed for exactly the length of the call, so per-call
/// options never leak into the handle's own configuration.
fn with_call_options<R>(
    table: &mut Table<Holder>,
    options: Option<IcebergOptions>,
    operation: impl FnOnce(&mut Table<Holder>) -> PyResult<R>,
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
        if let Some(value) = self.inner.compact_after_commits_option() {
            state.set_item("compact_after_commits", value)?;
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
    #[new]
    #[pyo3(signature = (**kwargs))]
    fn new(kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        let mut inner = IcebergOptions::new();
        apply_iceberg_option_fields("IcebergOptions", &mut inner, kwargs)?;
        Ok(Self::from_core(inner))
    }

    /// Rebuild exactly the options that were explicitly configured.
    #[staticmethod]
    fn _from_pickle(state: &Bound<'_, PyDict>) -> PyResult<Self> {
        let mut inner = IcebergOptions::new();
        apply_iceberg_option_fields("IcebergOptions._from_pickle", &mut inner, Some(state))?;
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
    /// parallelism, kept in 1..=8.
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

    /// How many large-enough files justify a parallel scan. Default: 16.
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
    /// a parallel scan, in bytes. Default: 4 MiB.
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

    /// After how many data commits an automatic compaction runs; `None` - the
    /// default - never compacts on its own, and 0 reads as off.
    #[getter]
    fn compact_after_commits(&self) -> Option<u32> {
        self.inner.compact_after_commits()
    }

    #[setter]
    fn set_compact_after_commits(&mut self, commits: u32) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_compact_after_commits(commits);
        Ok(())
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

/// A warehouse folder of namespaces of Iceberg tables.
///
/// The catalog is a description of where tables live, not proof that any do:
/// constructing one touches nothing, and every operation resolves its dotted
/// name - `"nyc.taxis"` is the folder `nyc/taxis` - against the warehouse
/// handle at the moment it runs. Its collections are
/// [`namespaces`][Self::namespaces] and [`tables`][Self::tables]; the two
/// dotted entry points - [`table`][Self::table] and
/// [`namespace`][Self::namespace] - are kept because a dotted identifier is a
/// real Iceberg spelling and deserves one call.
#[pyclass(name = "Catalog", module = "yggdryl._native", skip_from_py_object)]
pub(crate) struct PyCatalog {
    inner: Catalog<Holder>,
}

#[pymethods]
impl PyCatalog {
    // A catalog is a live external-resource handle, not a value snapshot.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// Describe a catalog over a warehouse folder, touching nothing.
    ///
    /// `warehouse` accepts an [`IOBase`][crate::io::PyIOBase] handle or
    /// anything that names a folder location: a string, a path-like, a
    /// [`Url`][crate::uri::PyUrl].
    #[new]
    fn new(warehouse: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: Catalog::new(folder_holder_from_value(warehouse)?),
        })
    }

    /// The warehouse folder the catalog resolves names against.
    #[getter]
    fn warehouse(&self) -> PyResult<PyIOBase> {
        Ok(PyIOBase::from_core(
            Holder::folder(
                self.inner
                    .warehouse()
                    .url()
                    .ok_or_else(|| PyValueError::new_err("this catalog has no location"))?
                    .clone()
                    .into_path()
                    .map_err(value_error)?,
            )
            .map_err(value_error)?,
        ))
    }

    /// Open the table a dotted name addresses - the one-call spelling of
    /// `catalog.tables[name]`.
    fn table(&self, name: &str) -> PyResult<PyTable> {
        self.inner
            .table(name)
            .map(PyTable::from_core)
            .map_err(value_error)
    }

    /// The namespace a dotted name addresses, as a view.
    ///
    /// The view exists whether or not the folder does, exactly as a handle
    /// describes a location without proof, so asking for one never fails.
    fn namespace(slf: &Bound<'_, Self>, name: String) -> PyNamespace {
        PyNamespace {
            catalog: slf.clone().unbind(),
            name,
        }
    }

    /// Append `data` to the named table, creating it on first write.
    ///
    /// A table that is not there yet takes its schema from the rows: partition
    /// marks riding the Arrow fields' metadata become the spec, so a marked
    /// schema lays its files out partitioned from the very first append.
    /// `options` configures this write. Returns the table so the caller can
    /// keep going.
    #[pyo3(signature = (name, data, *, options = None))]
    fn append(
        &self,
        name: &str,
        data: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyTable> {
        let resolved = iceberg_call_options(options)?;
        let tables = self.inner.tables();
        let data = iceberg_named_batch_reader(&tables, name, data)?;
        tables
            .append_arrow_reader_with_options(name, data, resolved)
            .map(PyTable::from_core)
            .map_err(value_error)
    }

    /// Replace the named table's rows with `data`, creating it on first write.
    ///
    /// An existing table keeps its previous snapshot readable, which is what
    /// makes the overwrite reversible. `options` configures this write.
    /// Returns the table so the caller can keep going.
    #[pyo3(signature = (name, data, *, options = None))]
    fn overwrite(
        &self,
        name: &str,
        data: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyTable> {
        let resolved = iceberg_call_options(options)?;
        let tables = self.inner.tables();
        let data = iceberg_named_batch_reader(&tables, name, data)?;
        tables
            .overwrite_arrow_reader_with_options(name, data, resolved)
            .map(PyTable::from_core)
            .map_err(value_error)
    }

    /// The catalog's namespaces, as a lazy map-oriented view.
    ///
    /// Constructing the view performs no I/O: membership, iteration, and
    /// length consult storage when asked, and indexing answers a
    /// [`Namespace`][PyNamespace]. This is the one collection spelling -
    /// `catalog.namespaces["sales"].tables["orders"]` chains to a table.
    #[getter]
    fn namespaces(slf: &Bound<'_, Self>) -> PyNamespaces {
        PyNamespaces {
            catalog: slf.clone().unbind(),
            parent: None,
        }
    }

    /// The catalog's tables, as the same lazy view over dotted names.
    ///
    /// `catalog.tables["sales.eu.orders"]` descends; an un-dotted name
    /// addresses a table directly under the warehouse root, and iterating
    /// lists exactly those.
    #[getter]
    fn tables(slf: &Bound<'_, Self>) -> PyTables {
        PyTables {
            catalog: slf.clone().unbind(),
            namespace: None,
        }
    }

    /// The catalog's own properties, from `metadata/catalog.json`.
    ///
    /// Absent means empty - never an error a caller has to catch.
    #[getter]
    fn properties<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let properties = PyDict::new(py);
        for (key, value) in &self.inner.properties().map_err(value_error)? {
            properties.set_item(key, value)?;
        }
        Ok(properties)
    }

    /// Set and remove catalog properties as one transactional write.
    ///
    /// `updates` is a mapping or a sequence of `(key, value)` pairs and
    /// `removes` an iterable of keys; the updates land first, so a key named
    /// by both ends up removed. A call given neither writes nothing at all.
    /// Keys under the reserved `iceberg:` prefix are refused by name.
    #[pyo3(signature = (updates = None, removes = None))]
    fn update_properties(
        &self,
        updates: Option<&Bound<'_, PyAny>>,
        removes: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let updates = match updates {
            Some(value) => string_pairs_from_value(value)?,
            None => Vec::new(),
        };
        let removes = match removes {
            Some(value) => crate::media::strings_from_iterable(value, "removes")?,
            None => Vec::new(),
        };
        if updates.is_empty() && removes.is_empty() {
            return Ok(());
        }
        self.inner
            .update_properties(updates, removes)
            .map_err(value_error)
    }

    fn __repr__(&self) -> String {
        format!(
            "Catalog({:?})",
            self.inner
                .warehouse()
                .url()
                .map_or_else(|| "<memory>".to_owned(), ToString::to_string),
        )
    }
}

/// An Iceberg table reached entirely through one container handle.
#[pyclass(name = "Table", module = "yggdryl._native", skip_from_py_object)]
pub(crate) struct PyTable {
    inner: Table<Holder>,
}

impl PyTable {
    fn from_core(inner: Table<Holder>) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyTable {
    // A table is a mutable external-resource handle with cached planning state.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// Create a table, writing its first metadata document.
    ///
    /// `partition_by` accepts a [`PartitionSpec`] or the column names to
    /// partition on; the default is unpartitioned. Unnumbered schema columns
    /// are numbered automatically, so a plain `PyArrow` schema works as it is;
    /// a schema that already carries field identifiers keeps every one of them.
    #[classmethod]
    #[pyo3(signature = (root, schema, partition_by = None, *, format_version = None))]
    fn create(
        _cls: &Bound<'_, PyType>,
        root: &PyIOBase,
        schema: &Bound<'_, PyAny>,
        partition_by: Option<&Bound<'_, PyAny>>,
        format_version: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let schema = numbered_schema_from_value(schema)?;
        let spec = match partition_by {
            Some(value) => spec_from_value(value, &schema)?,
            None => PartitionSpec::unpartitioned(),
        };
        let version = match format_version {
            Some(value) => format_version_from_value(value)?,
            None => FormatVersion::V2,
        };
        Table::create(root.folder_holder()?, version, schema, spec)
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// Open the table a container handle addresses.
    #[classmethod]
    fn open(_cls: &Bound<'_, PyType>, root: &PyIOBase) -> PyResult<Self> {
        Table::open(root.folder_holder()?)
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// Open the table if it exists, creating it otherwise.
    ///
    /// Like [`Self::create`], unnumbered schema columns are numbered
    /// automatically; an existing table is opened as it is and `schema`
    /// describes only the table this call would create.
    #[classmethod]
    #[pyo3(signature = (root, schema, partition_by = None, *, format_version = None))]
    fn open_or_create(
        _cls: &Bound<'_, PyType>,
        root: &PyIOBase,
        schema: &Bound<'_, PyAny>,
        partition_by: Option<&Bound<'_, PyAny>>,
        format_version: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let schema = numbered_schema_from_value(schema)?;
        let spec = match partition_by {
            Some(value) => spec_from_value(value, &schema)?,
            None => PartitionSpec::unpartitioned(),
        };
        let version = match format_version {
            Some(value) => format_version_from_value(value)?,
            None => FormatVersion::V2,
        };
        Table::open_or_create(root.folder_holder()?, version, schema, spec)
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// The folder the table lives in.
    ///
    /// Rebuilt from the table's own root handle rather than from its recorded
    /// location, because a location does not say which backend it belongs to:
    /// a table on a foreign Arrow filesystem must hand back a folder on that
    /// filesystem, not the local path its URL happens to spell.
    #[getter]
    fn root(&self) -> PyResult<PyIOBase> {
        let root = self.inner.root();
        if let Some(holder) = crate::io::arrow_folder_holder(root) {
            return Ok(PyIOBase::from_core(holder));
        }
        Ok(PyIOBase::from_core(
            Holder::folder(
                root.url()
                    .ok_or_else(|| PyValueError::new_err("this table has no location"))?
                    .clone()
                    .into_path()
                    .map_err(value_error)?,
            )
            .map_err(value_error)?,
        ))
    }

    /// The table's base location, as a URI.
    #[getter]
    fn location(&self) -> &str {
        self.inner.metadata().location()
    }

    /// The revision of the specification the metadata is written to.
    #[getter]
    fn format_version(&self) -> i32 {
        self.inner.metadata().format_version().number()
    }

    /// The stable identifier of the table itself.
    #[getter]
    fn table_uuid(&self) -> &str {
        self.inner.metadata().table_uuid()
    }

    /// The version number of the current metadata document.
    #[getter]
    fn version(&self) -> u32 {
        self.inner.metadata_version()
    }

    /// The name of the current metadata document.
    #[getter]
    fn metadata_file_name(&self) -> String {
        self.inner.metadata_file_name()
    }

    /// The location of the current metadata document, as a URI.
    #[getter]
    fn metadata_location(&self) -> PyResult<String> {
        self.inner.metadata_location().map_err(value_error)
    }

    /// The schema new data is written against.
    #[getter]
    fn schema(&self) -> PyResult<PyField> {
        self.inner
            .schema()
            .cloned()
            .map(PyField::from_inner)
            .map_err(value_error)
    }

    /// The partition spec new data is written against.
    #[getter]
    fn spec(&self) -> PyResult<PyPartitionSpec> {
        self.inner
            .metadata()
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
    fn current_snapshot(&self) -> Option<PySnapshot> {
        self.inner
            .current_snapshot()
            .cloned()
            .map(PySnapshot::from_core)
    }

    /// Every retained snapshot, oldest first.
    #[getter]
    fn snapshots(&self) -> Vec<PySnapshot> {
        self.inner
            .metadata()
            .snapshots()
            .iter()
            .cloned()
            .map(PySnapshot::from_core)
            .collect()
    }

    /// The free-form table properties the metadata document carries.
    #[getter]
    fn properties<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let properties = PyDict::new(py);
        for (key, value) in self.inner.metadata().properties() {
            properties.set_item(key.as_str(), value.as_str())?;
        }
        Ok(properties)
    }

    /// Every schema the table has had, by identifier.
    #[getter]
    fn schemas(&self) -> Vec<PyField> {
        self.inner
            .metadata()
            .schemas()
            .iter()
            .cloned()
            .map(PyField::from_inner)
            .collect()
    }

    /// Every manifest the current snapshot points at.
    fn manifests(&self) -> PyResult<Vec<PyManifestFile>> {
        Ok(self
            .inner
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
    fn manifests_at(&self, snapshot_id: i64) -> PyResult<Vec<PyManifestFile>> {
        let metadata = self.inner.metadata();
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
        Ok(self
            .inner
            .manifests_at(snapshot)
            .map_err(value_error)?
            .into_iter()
            .map(PyManifestFile::from_core)
            .collect())
    }

    /// Every live data file of the current snapshot, with the spec it was
    /// written under.
    fn data_files(&self) -> PyResult<Vec<(PyDataFile, PyPartitionSpec)>> {
        Ok(self
            .inner
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
    #[pyo3(signature = (field = None, *, options = None))]
    fn scan<'py>(
        &mut self,
        py: Python<'py>,
        field: Option<&Bound<'_, PyAny>>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let resolved = iceberg_call_options(options)?;
        let field = field
            .map(|field| core_root_field_from_value(field, SCHEMA_ROOT_NAME))
            .transpose()?;
        let reader = with_call_options(&mut self.inner, resolved, |table| {
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
    #[pyo3(signature = (filters = None, field = None, *, options = None))]
    fn scan_where<'py>(
        &mut self,
        py: Python<'py>,
        filters: Option<&Bound<'_, PyAny>>,
        field: Option<&Bound<'_, PyAny>>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let resolved = iceberg_call_options(options)?;
        let pairs = filter_pairs_from_value(filters)?;
        let field = field
            .map(|field| core_root_field_from_value(field, SCHEMA_ROOT_NAME))
            .transpose()?;
        let reader = with_call_options(&mut self.inner, resolved, |table| {
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
    #[pyo3(signature = (name, filters = None, field = None, *, options = None))]
    fn scan_ref<'py>(
        &mut self,
        py: Python<'py>,
        name: &str,
        filters: Option<&Bound<'_, PyAny>>,
        field: Option<&Bound<'_, PyAny>>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let resolved = iceberg_call_options(options)?;
        let pairs = filter_pairs_from_value(filters)?;
        let field = field
            .map(|field| core_root_field_from_value(field, SCHEMA_ROOT_NAME))
            .transpose()?;
        let reader = with_call_options(&mut self.inner, resolved, |table| {
            table
                .scan_ref(name, &borrowed_pairs(&pairs), field.as_ref())
                .map_err(value_error)
        })?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// Read the rows matching one predicate as a `pyarrow.RecordBatchReader`.
    ///
    /// `filter` is an `Expression` or the text of one, which parses. It is the
    /// whole expression language rather than equality pairs: ranges, null
    /// tests, `in` lists, nested paths, and `&holder.*` questions about the
    /// files themselves. Planning prunes with the metadata chain, and only the
    /// conjuncts it could not settle are tested against the rows.
    #[pyo3(signature = (filter, schema = None))]
    fn scan_matching<'py>(
        &self,
        py: Python<'py>,
        filter: &Bound<'_, PyAny>,
        schema: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let filter = crate::expression::expression_from_value(filter)?;
        let field = schema
            .map(|schema| core_root_field_from_value(schema, SCHEMA_ROOT_NAME))
            .transpose()?;
        let reader = self
            .inner
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
        &self,
        py: Python<'py>,
        filter: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        let filter = crate::expression::expression_from_value(filter)?;
        let plan = self.inner.plan_matching(filter).map_err(value_error)?;
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
    /// `options` configures this write without changing the handle's own
    /// configuration.
    #[pyo3(signature = (batches, *, options = None))]
    fn append(
        &mut self,
        batches: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let resolved = iceberg_call_options(options)?;
        let batches = iceberg_batch_reader(Some(&self.inner), batches)?;
        with_call_options(&mut self.inner, resolved, |table| {
            table.commit_append(batches).map_err(value_error)
        })
    }

    /// Replace every row with `batches` as a new snapshot.
    ///
    /// `options` configures this write as it does
    /// [`append`](Self::append).
    #[pyo3(signature = (batches, *, options = None))]
    fn overwrite(
        &mut self,
        batches: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let resolved = iceberg_call_options(options)?;
        let batches = iceberg_batch_reader(Some(&self.inner), batches)?;
        with_call_options(&mut self.inner, resolved, |table| {
            table.commit_overwrite(batches).map_err(value_error)
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
    #[pyo3(signature = (filters, batches, *, options = None))]
    fn overwrite_where(
        &mut self,
        filters: Option<&Bound<'_, PyAny>>,
        batches: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let resolved = iceberg_call_options(options)?;
        let pairs = filter_pairs_from_value(filters)?;
        let batches = iceberg_batch_reader(Some(&self.inner), batches)?;
        with_call_options(&mut self.inner, resolved, |table| {
            table
                .commit_overwrite_where(&borrowed_pairs(&pairs), batches)
                .map_err(value_error)
        })
    }

    /// Merge `batches` into the stored rows, matching on `merge_by_names`.
    ///
    /// An incoming row replaces the stored row whose match-key columns equal
    /// its own and is inserted when there is none, so this is the upsert. Only
    /// the files whose recorded bounds can hold an incoming key are read and
    /// rewritten - a file that is not read keeps every row it had, however
    /// coarse the statistics are - so the write costs the files it can
    /// actually change rather than the whole table. Matching on no column at
    /// all is a plain overwrite, because every row would then match every row.
    ///
    /// `safe` is the cast strictness the incoming batches are held to: the
    /// default refuses a value the table's column cannot hold rather than
    /// storing a silently wrapped one.
    #[pyo3(signature = (batches, merge_by_names, *, safe = true, options = None))]
    fn merge(
        &mut self,
        batches: &Bound<'_, PyAny>,
        merge_by_names: &Bound<'_, PyAny>,
        safe: bool,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let resolved = iceberg_call_options(options)?;
        let names = crate::media::strings_from_iterable(merge_by_names, "merge_by_names")?;
        let batches = iceberg_batch_reader(Some(&self.inner), batches)?;
        with_call_options(&mut self.inner, resolved, |table| {
            table
                .commit_merge(batches, &names, safe)
                .map_err(value_error)
        })
    }

    /// Merge `batches` into the rows `filters` selects, matching on
    /// `merge_by_names`.
    ///
    /// The filters narrow which stored files the merge may touch at all, and
    /// the key bounds narrow that further, so an upsert into one partition
    /// reads one partition. Everything else - the match rule, `safe`, the
    /// refusal to rebase after a lost commit - is exactly
    /// [`merge`](Self::merge).
    #[pyo3(signature = (filters, batches, merge_by_names, *, safe = true, options = None))]
    fn merge_where(
        &mut self,
        filters: Option<&Bound<'_, PyAny>>,
        batches: &Bound<'_, PyAny>,
        merge_by_names: &Bound<'_, PyAny>,
        safe: bool,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let resolved = iceberg_call_options(options)?;
        let pairs = filter_pairs_from_value(filters)?;
        let names = crate::media::strings_from_iterable(merge_by_names, "merge_by_names")?;
        let batches = iceberg_batch_reader(Some(&self.inner), batches)?;
        with_call_options(&mut self.inner, resolved, |table| {
            table
                .commit_merge_where(&borrowed_pairs(&pairs), batches, &names, safe)
                .map_err(value_error)
        })
    }

    /// Store an explicit options override every later call resolves first.
    ///
    /// A field the override sets shadows the table property of the same name,
    /// and a field it leaves unset still resolves property-then-default. The
    /// override lives on this handle alone - it is never written to the
    /// table; [`update_properties`](Self::update_properties) is what stores a
    /// setting on the table itself.
    fn set_options(&mut self, options: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner
            .set_options(core_iceberg_options_from_value(options)?);
        Ok(())
    }

    /// Resolve this table's effective options, field by field: the explicit
    /// override, then the table property of the same name, then the default.
    fn options(&self) -> PyResult<PyIcebergOptions> {
        self.inner
            .options()
            .map(PyIcebergOptions::from_core)
            .map_err(value_error)
    }

    /// Add a schema, make it current, and write a new metadata document.
    fn evolve_schema(&mut self, schema: &Bound<'_, PyAny>) -> PyResult<i32> {
        let schema = core_root_field_from_value(schema, SCHEMA_ROOT_NAME)?;
        self.inner.evolve_schema(schema).map_err(value_error)
    }

    /// Read one retained snapshot's rows: time travel as an ordinary scan.
    ///
    /// The rows are read as the schema that was current when the snapshot was
    /// written. `filters` is a mapping or a sequence of `(column, value)`
    /// pairs - the vocabulary `IOBase.children_where` uses - answered by the
    /// plan for a partition column and row by row for every other; `schema`
    /// keeps the columns it names, exactly as `scan` does.
    #[pyo3(signature = (snapshot_id, filters = None, schema = None, *, options = None))]
    fn scan_at<'py>(
        &mut self,
        py: Python<'py>,
        snapshot_id: i64,
        filters: Option<&Bound<'_, PyAny>>,
        schema: Option<&Bound<'_, PyAny>>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let resolved = iceberg_call_options(options)?;
        let pairs = filter_pairs_from_value(filters)?;
        let field = schema
            .map(|schema| core_root_field_from_value(schema, SCHEMA_ROOT_NAME))
            .transpose()?;
        let reader = with_call_options(&mut self.inner, resolved, |table| {
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
    fn plan(&self, filters: Option<&Bound<'_, PyAny>>) -> PyResult<PyScanPlan> {
        let pairs = filter_pairs_from_value(filters)?;
        let plan = self
            .inner
            .plan(&borrowed_pairs(&pairs))
            .map_err(value_error)?;
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
        &self,
        snapshot_id: i64,
        filters: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyScanPlan> {
        let pairs = filter_pairs_from_value(filters)?;
        let plan = self
            .inner
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
    fn create_branch(&mut self, name: &str, snapshot_id: i64) -> PyResult<()> {
        self.inner
            .create_branch(name, snapshot_id)
            .map_err(value_error)
    }

    /// Create a tag at one retained snapshot, as one metadata commit.
    fn create_tag(&mut self, name: &str, snapshot_id: i64) -> PyResult<()> {
        self.inner
            .create_tag(name, snapshot_id)
            .map_err(value_error)
    }

    /// Remove one branch or tag, as one metadata commit.
    ///
    /// A name the table does not have is an error rather than an empty
    /// commit.
    fn remove_ref(&mut self, name: &str) -> PyResult<()> {
        self.inner
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
    fn fast_forward(&mut self, name: &str, snapshot_id: i64) -> PyResult<()> {
        self.inner
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
        &mut self,
        older_than_ms: Option<i64>,
        retain_last: Option<usize>,
        snapshot_ids: Option<Vec<i64>>,
    ) -> PyResult<Vec<i64>> {
        let snapshot_ids = snapshot_ids.unwrap_or_default();
        self.inner
            .expire_snapshots(older_than_ms, retain_last, &snapshot_ids)
            .map_err(value_error)
    }

    /// Return the retained snapshot a branch or tag names.
    ///
    /// The `main` branch follows the current snapshot, so a table that has
    /// been written to always answers for it.
    fn snapshot_by_ref(&self, name: &str) -> PyResult<PySnapshot> {
        self.inner
            .snapshot_by_ref(name)
            .cloned()
            .map(PySnapshot::from_core)
            .map_err(value_error)
    }

    /// The size a data file aims for, in bytes.
    ///
    /// The table property `write.target-file-size-bytes` decides, falling back
    /// to the schema root's `iceberg:` protocol property of the same name and
    /// then to Iceberg's own 512 MiB default.
    #[getter]
    fn target_file_size(&self) -> PyResult<u64> {
        self.inner.target_file_size_bytes().map_err(value_error)
    }

    /// Merge the current snapshot's undersized data files, partition by
    /// partition, as one `replace` snapshot.
    ///
    /// A table with nothing to compact is left exactly as it is: no snapshot
    /// is committed and the returned [`PyCompaction`] is all zeros.
    fn compact(&mut self) -> PyResult<PyCompaction> {
        self.inner
            .compact()
            .map(PyCompaction::from_core)
            .map_err(value_error)
    }

    /// Render when each snapshot became current, oldest first.
    ///
    /// The columns are `made_current_at`, `snapshot_id`, `parent_id`, and
    /// `is_current_ancestor`, the names `PyIceberg`'s `history` table uses.
    fn inspect_history<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let reader = self.inner.inspect_history().map_err(value_error)?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// Render every retained snapshot with its operation and summary.
    ///
    /// The columns are `committed_at`, `snapshot_id`, `parent_id`,
    /// `operation`, `manifest_list`, and the free-form `summary` map.
    fn inspect_snapshots<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let reader = self.inner.inspect_snapshots().map_err(value_error)?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// Render the live data files of the current snapshot.
    ///
    /// The columns are `file_path`, `file_format`, `spec_id`, the rendered
    /// `partition` chain, `record_count`, and `file_size_in_bytes`.
    fn inspect_files<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let reader = self.inner.inspect_files().map_err(value_error)?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// Set and remove free-form table properties as one metadata commit.
    ///
    /// `updates` is a mapping or a sequence of `(key, value)` pairs and
    /// `removes` an iterable of keys; the updates land first, so a key named
    /// by both ends up removed. A call given neither commits nothing at all.
    #[pyo3(signature = (updates = None, removes = None))]
    fn update_properties(
        &mut self,
        updates: Option<&Bound<'_, PyAny>>,
        removes: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let updates = match updates {
            Some(value) => string_pairs_from_value(value)?,
            None => Vec::new(),
        };
        let removes = match removes {
            Some(value) => crate::media::strings_from_iterable(value, "removes")?,
            None => Vec::new(),
        };
        if updates.is_empty() && removes.is_empty() {
            return Ok(());
        }
        self.inner
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
    /// The recording methods touch nothing; [`PySchemaUpdate::commit`] plays
    /// them back through the core evolution rules and writes one new metadata
    /// document. `with table.update_schema() as update:` commits on a clean
    /// exit and discards on an exception.
    fn update_schema(slf: &Bound<'_, Self>) -> PySchemaUpdate {
        PySchemaUpdate {
            table: slf.clone().unbind(),
            ops: Vec::new(),
            consumed: false,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "Table({:?}, format_version={}, version={})",
            self.inner.metadata().location(),
            self.inner.metadata().format_version().number(),
            self.inner.metadata_version(),
        )
    }
}

/// One recorded column operation, held until its update commits.
///
/// This mirrors the core's own recording: inference happens when the caller
/// speaks - a field or datatype argument is read at the boundary - and the
/// operation itself runs in Rust when the update is played back.
enum RecordedOp {
    /// Append a column to the root (`""`) or to a nested struct.
    AddColumn {
        /// The dotted path of the struct the column lands under.
        parent: String,
        /// The column itself; stale identifiers are stripped on apply.
        field: CoreField,
    },
    /// Remove a column, retiring its identifier forever.
    DropColumn {
        /// The dotted path of the column.
        path: String,
    },
    /// Rename a column, keeping its identifier.
    RenameColumn {
        /// The dotted path of the column.
        path: String,
        /// The new name.
        name: String,
    },
    /// Set a column's `iceberg:doc` documentation string.
    UpdateDoc {
        /// The dotted path of the column.
        path: String,
        /// The documentation string.
        doc: String,
    },
    /// Relax a required column to optional.
    MakeNullable {
        /// The dotted path of the column.
        path: String,
    },
    /// Promote a column's type, gated by the legal Iceberg promotions.
    UpdateType {
        /// The dotted path of the column.
        path: String,
        /// The promoted type.
        data_type: CoreDataType,
    },
}

/// A recorded set of column operations against one table's current schema.
///
/// Built by `Table.update_schema`. Each recording method returns the update
/// itself so calls chain, and `commit` plays the operations back onto the
/// table's metadata - added columns numbered above `last-column-id`, renames
/// keeping their identifier, promotions gated by `can_promote` - as one new
/// metadata document. Used as a context manager, a clean exit commits and an
/// exception discards.
#[pyclass(name = "SchemaUpdate", module = "yggdryl._native", skip_from_py_object)]
pub(crate) struct PySchemaUpdate {
    /// The table the update was started from and commits back to.
    table: Py<PyTable>,
    /// The recorded operations, in call order.
    ops: Vec<RecordedOp>,
    /// Whether the update has already committed or been discarded.
    consumed: bool,
}

impl PySchemaUpdate {
    /// Refuse an update that has already committed or been discarded.
    fn check_open(&self) -> PyResult<()> {
        if self.consumed {
            return Err(PyValueError::new_err(
                "expected an open schema update, got one already committed or discarded",
            ));
        }
        Ok(())
    }
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
        slf.check_open()?;
        let field = core_field_from_value(field)?;
        slf.ops.push(RecordedOp::AddColumn {
            parent: parent.to_owned(),
            field,
        });
        Ok(slf)
    }

    /// Record the removal of the column at `path`, retiring its identifier.
    fn drop_column<'py>(mut slf: PyRefMut<'py, Self>, path: &str) -> PyResult<PyRefMut<'py, Self>> {
        slf.check_open()?;
        slf.ops.push(RecordedOp::DropColumn {
            path: path.to_owned(),
        });
        Ok(slf)
    }

    /// Record a rename of the column at `path`; its identifier keeps rows
    /// written under the pre-rename name readable.
    fn rename_column<'py>(
        mut slf: PyRefMut<'py, Self>,
        path: &str,
        name: &str,
    ) -> PyResult<PyRefMut<'py, Self>> {
        slf.check_open()?;
        slf.ops.push(RecordedOp::RenameColumn {
            path: path.to_owned(),
            name: name.to_owned(),
        });
        Ok(slf)
    }

    /// Record a new `iceberg:doc` documentation string on the column at
    /// `path`.
    fn update_doc<'py>(
        mut slf: PyRefMut<'py, Self>,
        path: &str,
        doc: &str,
    ) -> PyResult<PyRefMut<'py, Self>> {
        slf.check_open()?;
        slf.ops.push(RecordedOp::UpdateDoc {
            path: path.to_owned(),
            doc: doc.to_owned(),
        });
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
        slf.check_open()?;
        slf.ops.push(RecordedOp::MakeNullable {
            path: path.to_owned(),
        });
        Ok(slf)
    }

    /// Record a type promotion on the column at `path`, checked against the
    /// legal Iceberg promotions when the update commits.
    ///
    /// `data_type` accepts anything a datatype crosses the boundary as: the
    /// native wrapper, a datatype expression, a `PyArrow` type.
    fn update_type<'py>(
        mut slf: PyRefMut<'py, Self>,
        path: &str,
        data_type: &Bound<'_, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        slf.check_open()?;
        let data_type = core_data_type_from_value(data_type)?;
        slf.ops.push(RecordedOp::UpdateType {
            path: path.to_owned(),
            data_type,
        });
        Ok(slf)
    }

    /// Play the recorded operations back and write one new metadata document.
    ///
    /// The evolved schema is added to the table's metadata and made current in
    /// the same commit a property change uses, so the table describes the new
    /// shape when this returns and describes the old one on any failure. An
    /// update that recorded nothing commits nothing at all.
    fn commit(&mut self, py: Python<'_>) -> PyResult<()> {
        self.check_open()?;
        self.consumed = true;
        let ops = std::mem::take(&mut self.ops);
        if ops.is_empty() {
            return Ok(());
        }
        let mut table = self.table.bind(py).borrow_mut();
        table
            .inner
            .commit_metadata_changes(move |metadata| {
                // Replayed by reference: a beaten commit rebases and runs this
                // closure again on the winner's metadata, so the recording
                // must survive every attempt.
                let mut update = SchemaUpdate::from_metadata(metadata)?;
                for op in &ops {
                    match op {
                        RecordedOp::AddColumn { parent, field } => {
                            update.add_column(parent, field.clone());
                        }
                        RecordedOp::DropColumn { path } => update.drop_column(path),
                        RecordedOp::RenameColumn { path, name } => {
                            update.rename_column(path, name.clone());
                        }
                        RecordedOp::UpdateDoc { path, doc } => {
                            update.update_doc(path, doc.clone());
                        }
                        RecordedOp::MakeNullable { path } => update.make_nullable(path),
                        RecordedOp::UpdateType { path, data_type } => {
                            update.update_type(path, data_type.clone());
                        }
                    }
                }
                let evolved = update.into_field()?;
                let schema_id = metadata.add_schema(evolved)?;
                metadata.set_current_schema(schema_id)
            })
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
            self.ops.clear();
            self.consumed = true;
            return Ok(false);
        }
        if !self.consumed {
            self.commit(py)?;
        }
        Ok(false)
    }

    fn __repr__(&self) -> String {
        format!(
            "SchemaUpdate(ops={}, committed={})",
            self.ops.len(),
            if self.consumed { "True" } else { "False" },
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
        let schema = core_root_field_from_value(schema, SCHEMA_ROOT_NAME)?;
        let names = crate::media::strings_from_iterable(columns, "columns")?;
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

/// One namespace of a catalog: identity, plus its two collection views.
///
/// The namespace holds only its dotted name. Its tables are
/// [`tables`][Self::tables] and its child namespaces are
/// [`namespaces`][Self::namespaces], so access chains -
/// `catalog.namespaces["sales"].tables["orders"]` - and every collection
/// operation has exactly one home.
#[pyclass(name = "Namespace", module = "yggdryl._native", skip_from_py_object)]
pub(crate) struct PyNamespace {
    catalog: Py<PyCatalog>,
    name: String,
}

#[pymethods]
impl PyNamespace {
    // This is a live view through a catalog, not a detached namespace value.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// The namespace's dotted name.
    #[getter]
    fn name(&self) -> &str {
        &self.name
    }

    /// This namespace's tables, as a lazy map-oriented view.
    #[getter]
    fn tables(&self, py: Python<'_>) -> PyTables {
        PyTables {
            catalog: self.catalog.clone_ref(py),
            namespace: Some(self.name.clone()),
        }
    }

    /// The namespaces one level below this one, as the same view shape the
    /// catalog itself answers - the cascade that reaches a nested namespace.
    #[getter]
    fn namespaces(&self, py: Python<'_>) -> PyNamespaces {
        PyNamespaces {
            catalog: self.catalog.clone_ref(py),
            parent: Some(self.name.clone()),
        }
    }

    /// The namespace's properties, from `metadata/namespace.json`.
    ///
    /// Absent means empty - a namespace a table write brought into being
    /// carries no document and answers no properties, and that is not a
    /// failure.
    #[getter]
    fn properties<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let catalog = self.catalog.borrow(py);
        let namespace = catalog
            .inner
            .namespaces()
            .get(&self.name)
            .map_err(value_error)?;
        let properties = PyDict::new(py);
        for (key, value) in &namespace.properties().map_err(value_error)? {
            properties.set_item(key, value)?;
        }
        Ok(properties)
    }

    /// Set and remove namespace properties as one transactional write.
    ///
    /// `updates` is a mapping or a sequence of `(key, value)` pairs and
    /// `removes` an iterable of keys; the updates land first, so a key named
    /// by both ends up removed. A call given neither writes nothing at all.
    /// Keys under the reserved `iceberg:` prefix are refused by name.
    #[pyo3(signature = (updates = None, removes = None))]
    fn update_properties(
        &self,
        py: Python<'_>,
        updates: Option<&Bound<'_, PyAny>>,
        removes: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let updates = match updates {
            Some(value) => string_pairs_from_value(value)?,
            None => Vec::new(),
        };
        let removes = match removes {
            Some(value) => crate::media::strings_from_iterable(value, "removes")?,
            None => Vec::new(),
        };
        if updates.is_empty() && removes.is_empty() {
            return Ok(());
        }
        let catalog = self.catalog.borrow(py);
        catalog
            .inner
            .namespaces()
            .get(&self.name)
            .map_err(value_error)?
            .update_properties(updates, removes)
            .map_err(value_error)
    }

    fn __repr__(&self) -> String {
        format!("Namespace({:?})", self.name)
    }
}

/// The namespaces one level below a catalog or a namespace, as a lazy view.
///
/// The view materializes nothing up front: membership, iteration, and length
/// consult storage when asked, and indexing answers a
/// [`Namespace`][PyNamespace] - a missing name is a `KeyError` naming the
/// namespace. Two views over the same catalog observe each other's writes,
/// and a view stays valid across creation and deletion because every answer
/// comes from storage at call time.
#[pyclass(name = "Namespaces", module = "yggdryl._native", skip_from_py_object)]
pub(crate) struct PyNamespaces {
    catalog: Py<PyCatalog>,
    /// The parent namespace's dotted name; `None` is the warehouse root.
    parent: Option<String>,
}

impl PyNamespaces {
    /// Spell one child's full dotted name.
    fn dotted(&self, name: &str) -> String {
        match &self.parent {
            Some(parent) => format!("{parent}.{name}"),
            None => name.to_owned(),
        }
    }

    /// Wrap one namespace name as the view of it.
    fn namespace(&self, py: Python<'_>, name: &str) -> PyNamespace {
        PyNamespace {
            catalog: self.catalog.clone_ref(py),
            name: self.dotted(name),
        }
    }

    /// The names one level down, as the core's lazy iterator.
    ///
    /// A parent that does not exist lists nothing rather than failing, per
    /// the level's own listing contract.
    fn level(&self, py: Python<'_>, tables: bool) -> PyResult<yggdryl::iceberg::Names> {
        let catalog = self.catalog.borrow(py);
        match &self.parent {
            None => Ok(catalog.inner.namespaces().iter()),
            Some(parent) => match catalog.inner.namespaces().get(parent) {
                Ok(namespace) if tables => Ok(namespace.tables().iter()),
                Ok(namespace) => Ok(namespace.namespaces().iter()),
                Err(error) if error.is_absent() => Ok(yggdryl::iceberg::Names::empty()),
                Err(error) => Err(value_error(error)),
            },
        }
    }
}

#[pymethods]
impl PyNamespaces {
    // Collection answers depend on storage at the moment they are requested.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// `namespaces["sales"]` answers the namespace; a missing one is a
    /// `KeyError` carrying the native message.
    ///
    /// The lookup is the act, per the existence contract: the typed absence
    /// the core raises becomes the `KeyError` - the mapping protocol's type,
    /// the boundary's unchanged text - and nothing probes first.
    fn __getitem__(&self, py: Python<'_>, name: &str) -> PyResult<PyNamespace> {
        let dotted = self.dotted(name);
        let catalog = self.catalog.borrow(py);
        match catalog.inner.namespaces().get(&dotted) {
            Ok(_) => {
                drop(catalog);
                Ok(self.namespace(py, name))
            }
            Err(error) if error.is_absent() => Err(PyKeyError::new_err(error.to_string())),
            Err(error) => Err(value_error(error)),
        }
    }

    /// `"sales" in namespaces` asks storage whether the namespace exists.
    fn __contains__(&self, py: Python<'_>, name: &str) -> PyResult<bool> {
        let dotted = self.dotted(name);
        let catalog = self.catalog.borrow(py);
        catalog
            .inner
            .namespaces()
            .contains(&dotted)
            .map_err(value_error)
    }

    /// Iterating the view yields the bare namespace names, sorted, lazily.
    fn __iter__(&self, py: Python<'_>) -> PyResult<PyNames> {
        Ok(PyNames {
            names: self.level(py, false)?,
        })
    }

    /// How many namespaces are one level down, right now.
    ///
    /// This drains the level's listing, so it costs the full listing - never
    /// assume it is free on a wide warehouse.
    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        let mut count = 0;
        for name in self.level(py, false)? {
            name.map_err(value_error)?;
            count += 1;
        }
        Ok(count)
    }

    /// The names, as `dict.keys` answers them - the same lazy iterator.
    fn keys(&self, py: Python<'_>) -> PyResult<PyNames> {
        self.__iter__(py)
    }

    /// The namespaces themselves, in key order, lazily - each one wrapped as
    /// `__next__` reaches its name.
    fn values(&self, py: Python<'_>) -> PyResult<PyNamespaceIterator> {
        Ok(PyNamespaceIterator {
            catalog: self.catalog.clone_ref(py),
            parent: self.parent.clone(),
            names: self.level(py, false)?,
            kind: ViewIteratorKind::Values,
        })
    }

    /// `(name, namespace)` pairs, in key order, as lazily as `values`.
    fn items(&self, py: Python<'_>) -> PyResult<PyNamespaceIterator> {
        Ok(PyNamespaceIterator {
            catalog: self.catalog.clone_ref(py),
            parent: self.parent.clone(),
            names: self.level(py, false)?,
            kind: ViewIteratorKind::Items,
        })
    }

    /// Create the named namespace; one already there is an error.
    fn create(&self, py: Python<'_>, name: &str) -> PyResult<PyNamespace> {
        let dotted = self.dotted(name);
        {
            let catalog = self.catalog.borrow(py);
            catalog
                .inner
                .namespaces()
                .create(&dotted)
                .map(|_| ())
                .map_err(value_error)?;
        }
        Ok(self.namespace(py, name))
    }

    /// Open the named namespace, creating its folder when absent.
    fn open_or_create(&self, py: Python<'_>, name: &str) -> PyResult<PyNamespace> {
        let dotted = self.dotted(name);
        {
            let catalog = self.catalog.borrow(py);
            catalog
                .inner
                .namespaces()
                .open_or_create(&dotted)
                .map(|_| ())
                .map_err(value_error)?;
        }
        Ok(self.namespace(py, name))
    }

    fn __repr__(&self) -> String {
        match &self.parent {
            Some(parent) => format!("Namespaces({parent:?})"),
            None => "Namespaces()".to_owned(),
        }
    }
}

/// The lazy iterator every collection view walks.
///
/// It wraps the core names iterator directly, so nothing is collected on the
/// way across the boundary and a failure raises at the entry it happened on,
/// after which the iterator is exhausted. Removal is deliberately absent
/// everywhere in the hierarchy - the storage contract's `remove` deletes a
/// leaf or an empty container, and dropping a table is maintenance work, not
/// a `del` - so there is no `__delitem__` to pair this with.
#[pyclass(name = "IcebergNames", module = "yggdryl._native")]
pub(crate) struct PyNames {
    names: yggdryl::iceberg::Names,
}

#[pymethods]
impl PyNames {
    // Consumption changes iterator state.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> PyResult<Option<String>> {
        self.names.next().transpose().map_err(value_error)
    }
}

#[derive(Clone, Copy)]
enum ViewIteratorKind {
    Values,
    Items,
}

/// Lazy iterator behind `Namespaces.values()` and `Namespaces.items()`.
///
/// It walks the same core names iterator as `keys()` and wraps each name as
/// its [`Namespace`][PyNamespace] view only when `__next__` reaches it, so
/// taking one value from a level of many classifies one entry.
#[pyclass(name = "IcebergNamespaceIterator", module = "yggdryl._native")]
pub(crate) struct PyNamespaceIterator {
    catalog: Py<PyCatalog>,
    /// The parent namespace's dotted name; `None` is the warehouse root.
    parent: Option<String>,
    names: yggdryl::iceberg::Names,
    kind: ViewIteratorKind,
}

#[pymethods]
impl PyNamespaceIterator {
    // Consumption changes iterator state.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        let Some(name) = self.names.next().transpose().map_err(value_error)? else {
            return Ok(None);
        };
        let namespace = PyNamespace {
            catalog: self.catalog.clone_ref(py),
            name: match &self.parent {
                Some(parent) => format!("{parent}.{name}"),
                None => name.clone(),
            },
        };
        Ok(Some(match self.kind {
            ViewIteratorKind::Values => namespace.into_pyobject(py)?.into_any().unbind(),
            ViewIteratorKind::Items => (name, namespace).into_pyobject(py)?.into_any().unbind(),
        }))
    }
}

/// Lazy iterator behind `Tables.values()` and `Tables.items()`.
///
/// It walks the same core names iterator as `keys()` and opens each table
/// only when `__next__` reaches its name, so taking one value from a
/// namespace of many opens one table.
#[pyclass(name = "IcebergTableIterator", module = "yggdryl._native")]
pub(crate) struct PyTableIterator {
    catalog: Py<PyCatalog>,
    /// The owning namespace's dotted name; `None` is the warehouse root.
    namespace: Option<String>,
    names: yggdryl::iceberg::Names,
    kind: ViewIteratorKind,
}

#[pymethods]
impl PyTableIterator {
    // Consumption changes iterator state.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        let Some(name) = self.names.next().transpose().map_err(value_error)? else {
            return Ok(None);
        };
        let dotted = match &self.namespace {
            Some(namespace) => format!("{namespace}.{name}"),
            None => name.clone(),
        };
        let catalog = self.catalog.borrow(py);
        let table = match catalog.inner.tables().get(&dotted) {
            Ok(table) => PyTable::from_core(table),
            // A name the listing yielded that has vanished since is the same
            // absence indexing reports: the mapping protocol's KeyError, the
            // boundary's unchanged text.
            Err(error) if error.is_absent() => {
                return Err(PyKeyError::new_err(error.to_string()));
            }
            Err(error) => return Err(value_error(error)),
        };
        drop(catalog);
        Ok(Some(match self.kind {
            ViewIteratorKind::Values => table.into_pyobject(py)?.into_any().unbind(),
            ViewIteratorKind::Items => (name, table).into_pyobject(py)?.into_any().unbind(),
        }))
    }
}

/// The tables of one namespace - or of the warehouse root - as a lazy view.
///
/// The same shape as [`Namespaces`][PyNamespaces], one level down: indexing
/// opens a [`Table`][PyTable] - a missing name is a `KeyError` naming the
/// table - and the write conveniences that take a name create the table on
/// first write, from the incoming rows' own schema. At the root, names may be
/// fully dotted - `catalog.tables["sales.eu.orders"]` descends. Every answer
/// comes from storage at call time, so the view is never stale.
#[pyclass(name = "Tables", module = "yggdryl._native", skip_from_py_object)]
pub(crate) struct PyTables {
    catalog: Py<PyCatalog>,
    /// The owning namespace's dotted name; `None` is the warehouse root.
    namespace: Option<String>,
}

impl PyTables {
    /// Spell one table's full dotted name under this namespace.
    fn dotted(&self, name: &str) -> String {
        match &self.namespace {
            Some(namespace) => format!("{namespace}.{name}"),
            None => name.to_owned(),
        }
    }

    /// Run one operation over the root tables view, which accepts the dotted
    /// spelling this value builds - the resolution rule lives in the core
    /// collection, not here.
    fn with_core<R>(
        &self,
        py: Python<'_>,
        operation: impl FnOnce(yggdryl::iceberg::Tables<'_, Holder>) -> R,
    ) -> R {
        let catalog = self.catalog.borrow(py);
        operation(catalog.inner.tables())
    }

    /// The table names one level down, as the core's lazy iterator.
    fn level(&self, py: Python<'_>) -> PyResult<yggdryl::iceberg::Names> {
        let catalog = self.catalog.borrow(py);
        match &self.namespace {
            None => Ok(catalog.inner.tables().iter()),
            Some(parent) => match catalog.inner.namespaces().get(parent) {
                Ok(namespace) => Ok(namespace.tables().iter()),
                // A namespace that does not exist lists nothing rather than
                // failing.
                Err(error) if error.is_absent() => Ok(yggdryl::iceberg::Names::empty()),
                Err(error) => Err(value_error(error)),
            },
        }
    }
}

#[pymethods]
impl PyTables {
    // Collection answers depend on storage at the moment they are requested.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// `tables["orders"]` opens the table; a missing one is a `KeyError`
    /// carrying the native message.
    ///
    /// The lookup is the act: the typed absence the core raises becomes the
    /// `KeyError` - the mapping protocol's type, the boundary's unchanged
    /// text - and the locate that found the table already opened it.
    fn __getitem__(&self, py: Python<'_>, name: &str) -> PyResult<PyTable> {
        let dotted = self.dotted(name);
        match self.with_core(py, |view| view.get(&dotted)) {
            Ok(table) => Ok(PyTable::from_core(table)),
            Err(error) if error.is_absent() => Err(PyKeyError::new_err(error.to_string())),
            Err(error) => Err(value_error(error)),
        }
    }

    /// `"orders" in tables` asks storage whether the table exists.
    fn __contains__(&self, py: Python<'_>, name: &str) -> PyResult<bool> {
        let dotted = self.dotted(name);
        self.with_core(py, |view| view.contains(&dotted))
            .map_err(value_error)
    }

    /// Iterating the view yields the bare table names, sorted, lazily.
    fn __iter__(&self, py: Python<'_>) -> PyResult<PyNames> {
        Ok(PyNames {
            names: self.level(py)?,
        })
    }

    /// How many tables the namespace holds, right now.
    ///
    /// This drains the level's listing, so it costs the full listing - never
    /// assume it is free on a wide namespace.
    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        let mut count = 0;
        for name in self.level(py)? {
            name.map_err(value_error)?;
            count += 1;
        }
        Ok(count)
    }

    /// The names, as `dict.keys` answers them - the same lazy iterator.
    fn keys(&self, py: Python<'_>) -> PyResult<PyNames> {
        self.__iter__(py)
    }

    /// The tables themselves, in key order, lazily - each one opened only
    /// when `__next__` reaches its name.
    fn values(&self, py: Python<'_>) -> PyResult<PyTableIterator> {
        Ok(PyTableIterator {
            catalog: self.catalog.clone_ref(py),
            namespace: self.namespace.clone(),
            names: self.level(py)?,
            kind: ViewIteratorKind::Values,
        })
    }

    /// `(name, table)` pairs, in key order, as lazily as `values`.
    fn items(&self, py: Python<'_>) -> PyResult<PyTableIterator> {
        Ok(PyTableIterator {
            catalog: self.catalog.clone_ref(py),
            namespace: self.namespace.clone(),
            names: self.level(py)?,
            kind: ViewIteratorKind::Items,
        })
    }

    /// Create the named table, writing its first metadata document.
    ///
    /// Unnumbered schema fields are numbered, and the partition spec is
    /// derived from the columns the schema itself marks.
    fn create(&self, py: Python<'_>, name: &str, schema: &Bound<'_, PyAny>) -> PyResult<PyTable> {
        let schema = catalog_schema_from_value(schema)?;
        let dotted = self.dotted(name);
        self.with_core(py, |view| view.create(&dotted, schema))
            .map(PyTable::from_core)
            .map_err(value_error)
    }

    /// Open the named table, creating it with `schema` when absent.
    fn open_or_create(
        &self,
        py: Python<'_>,
        name: &str,
        schema: &Bound<'_, PyAny>,
    ) -> PyResult<PyTable> {
        let schema = catalog_schema_from_value(schema)?;
        let dotted = self.dotted(name);
        self.with_core(py, |view| view.open_or_create(&dotted, schema))
            .map(PyTable::from_core)
            .map_err(value_error)
    }

    /// Append `data` to the named table, creating it on first write.
    ///
    /// `options` configures this write. Returns the table so the caller can
    /// keep going.
    #[pyo3(signature = (name, data, *, options = None))]
    fn append(
        &self,
        py: Python<'_>,
        name: &str,
        data: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyTable> {
        let resolved = iceberg_call_options(options)?;
        let dotted = self.dotted(name);
        let data = self.with_core(py, |view| iceberg_named_batch_reader(&view, &dotted, data))?;
        self.with_core(py, |view| {
            view.append_arrow_reader_with_options(&dotted, data, resolved)
        })
        .map(PyTable::from_core)
        .map_err(value_error)
    }

    /// Replace the named table's rows with `data`, creating it on first write.
    ///
    /// `options` configures this write. Returns the table so the caller can
    /// keep going.
    #[pyo3(signature = (name, data, *, options = None))]
    fn overwrite(
        &self,
        py: Python<'_>,
        name: &str,
        data: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyTable> {
        let resolved = iceberg_call_options(options)?;
        let dotted = self.dotted(name);
        let data = self.with_core(py, |view| iceberg_named_batch_reader(&view, &dotted, data))?;
        self.with_core(py, |view| {
            view.overwrite_arrow_reader_with_options(&dotted, data, resolved)
        })
        .map(PyTable::from_core)
        .map_err(value_error)
    }

    fn __repr__(&self) -> String {
        match &self.namespace {
            Some(namespace) => format!("Tables({namespace:?})"),
            None => "Tables()".to_owned(),
        }
    }
}
