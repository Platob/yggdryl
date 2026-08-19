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

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple, PyType};

use yggdryl::generic::Holder;
use yggdryl::iceberg::{
    Catalog, Compaction, DataFile, FormatVersion, ManifestContent, ManifestFile, PartitionField,
    PartitionSpec, SchemaUpdate, Snapshot, Table, assign_field_ids, can_promote, last_field_id,
    schema_from_json, schema_to_json,
};
use yggdryl::io::IOBase as _;
use yggdryl::{DataType as CoreDataType, Field as CoreField, Value};

use crate::datatype::core_data_type_from_value;
use crate::field::{PyField, core_field_from_value};
use crate::io::PyIOBase;
use crate::record::{batch_reader_from_value, batch_reader_to_pyarrow, core_root_field_from_value};
use crate::uri::core_url_from_value;
use crate::value_error;

/// The root name given to a schema that arrives as a bare Arrow schema.
const SCHEMA_ROOT_NAME: &str = "row";

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
/// The document is an ordinary mapping - what `json.load` produces - because an
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
    let document = crate::value::from_py(document)?;
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
#[pyfunction(name = "schema_to_json")]
pub(crate) fn iceberg_schema_to_json(
    py: Python<'_>,
    schema: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let root = core_root_field_from_value(schema, SCHEMA_ROOT_NAME)?;
    let document = schema_to_json(&root).map_err(value_error)?;
    crate::value::as_py(py, &document)
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
fn partition_values<'py>(py: Python<'py>, values: &[Value]) -> PyResult<Bound<'py, PyTuple>> {
    let projected: Vec<Py<PyAny>> = values
        .iter()
        .map(|value| crate::value::as_py(py, value))
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
    let start = last_field_id(&schema)
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
    Holder::folder(url.to_path().map_err(value_error)?).map_err(value_error)
}

/// A warehouse folder of namespaces of Iceberg tables.
///
/// The catalog is a description of where tables live, not proof that any do:
/// constructing one touches nothing, and every operation resolves its dotted
/// name - `"nyc.taxis"` is the folder `nyc/taxis` - against the warehouse
/// handle at the moment it runs.
#[pyclass(name = "Catalog", module = "yggdryl._native", skip_from_py_object)]
pub(crate) struct PyCatalog {
    inner: Catalog<Holder>,
}

#[pymethods]
impl PyCatalog {
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
                    .to_path()
                    .map_err(value_error)?,
            )
            .map_err(value_error)?,
        ))
    }

    /// Create the named table, writing its first metadata document.
    ///
    /// Unnumbered schema fields are numbered, and the partition spec is derived
    /// from the columns the schema itself marks - a schema that marks none
    /// produces an unpartitioned table.
    fn create_table(&self, name: &str, schema: &Bound<'_, PyAny>) -> PyResult<PyTable> {
        let schema = catalog_schema_from_value(schema)?;
        self.inner
            .create_table(name, schema)
            .map(PyTable::from_core)
            .map_err(value_error)
    }

    /// Open the named table.
    fn table(&self, name: &str) -> PyResult<PyTable> {
        self.inner
            .table(name)
            .map(PyTable::from_core)
            .map_err(value_error)
    }

    /// Return whether the named table exists.
    fn has_table(&self, name: &str) -> PyResult<bool> {
        self.inner.has_table(name).map_err(value_error)
    }

    /// Open the named table if it exists, creating it otherwise.
    ///
    /// An existing table is opened as it is - `schema` describes only the
    /// table this call would create.
    fn open_or_create_table(&self, name: &str, schema: &Bound<'_, PyAny>) -> PyResult<PyTable> {
        let schema = catalog_schema_from_value(schema)?;
        self.inner
            .open_or_create_table(name, schema)
            .map(PyTable::from_core)
            .map_err(value_error)
    }

    /// Append `data` to the named table, creating it on first write.
    ///
    /// A table that is not there yet takes its schema from the rows: partition
    /// marks riding the Arrow fields' metadata become the spec, so a marked
    /// schema lays its files out partitioned from the very first append.
    /// Returns the table so the caller can keep going.
    fn append(&self, name: &str, data: &Bound<'_, PyAny>) -> PyResult<PyTable> {
        let data = batch_reader_from_value(data)?;
        self.inner
            .append(name, data)
            .map(PyTable::from_core)
            .map_err(value_error)
    }

    /// Replace the named table's rows with `data`, creating it on first write.
    ///
    /// An existing table keeps its previous snapshot readable, which is what
    /// makes the overwrite reversible. Returns the table so the caller can
    /// keep going.
    fn overwrite(&self, name: &str, data: &Bound<'_, PyAny>) -> PyResult<PyTable> {
        let data = batch_reader_from_value(data)?;
        self.inner
            .overwrite(name, data)
            .map(PyTable::from_core)
            .map_err(value_error)
    }

    /// List the namespaces one level below `parent`, as dotted names.
    ///
    /// `None` lists the warehouse's own child folders. A parent that does not
    /// exist lists nothing rather than failing.
    #[pyo3(signature = (parent = None))]
    fn list_namespaces(&self, parent: Option<&str>) -> PyResult<Vec<String>> {
        self.inner.list_namespaces(parent).map_err(value_error)
    }

    /// List the tables in a namespace, as sorted dotted names.
    fn list_tables(&self, namespace: &str) -> PyResult<Vec<String>> {
        self.inner.list_tables(namespace).map_err(value_error)
    }

    /// One namespace as a view: `catalog["analytics"]`.
    ///
    /// The view exists whether or not the folder does, exactly as a handle
    /// describes a location without proof, so indexing never fails.
    fn namespace(slf: &Bound<'_, Self>, name: &str) -> PyNamespace {
        PyNamespace {
            catalog: slf.clone().unbind(),
            name: name.to_owned(),
        }
    }

    /// `catalog["analytics"]` is [`namespace`](Self::namespace).
    fn __getitem__(slf: &Bound<'_, Self>, name: &str) -> PyNamespace {
        Self::namespace(slf, name)
    }

    /// `"analytics" in catalog` asks whether the namespace is listed.
    fn __contains__(&self, name: &str) -> PyResult<bool> {
        Ok(self
            .inner
            .list_namespaces(None)
            .map_err(value_error)?
            .iter()
            .any(|namespace| namespace == name))
    }

    /// Iterating a catalog yields its top-level namespace views.
    fn __iter__(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let names = slf
            .borrow()
            .inner
            .list_namespaces(None)
            .map_err(value_error)?;
        let namespaces: Vec<PyNamespace> = names
            .into_iter()
            .map(|name| PyNamespace {
                catalog: slf.clone().unbind(),
                name,
            })
            .collect();
        let list = pyo3::types::PyList::new(slf.py(), namespaces)?;
        Ok(list.as_any().try_iter()?.unbind().into_any())
    }

    fn __len__(&self) -> PyResult<usize> {
        Ok(self.inner.list_namespaces(None).map_err(value_error)?.len())
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
    #[getter]
    fn root(&self) -> PyResult<PyIOBase> {
        Ok(PyIOBase::from_core(
            Holder::folder(
                self.inner
                    .root()
                    .url()
                    .ok_or_else(|| PyValueError::new_err("this table has no location"))?
                    .to_path()
                    .map_err(value_error)?,
            )
            .map_err(value_error)?,
        ))
    }

    /// The table's base location, as a URI.
    #[getter]
    fn location(&self) -> &str {
        self.inner.metadata().location.as_str()
    }

    /// The revision of the specification the metadata is written to.
    #[getter]
    fn format_version(&self) -> i32 {
        self.inner.metadata().format_version.number()
    }

    /// The stable identifier of the table itself.
    #[getter]
    fn table_uuid(&self) -> &str {
        self.inner.metadata().table_uuid.as_str()
    }

    /// The version number of the current metadata document.
    #[getter]
    fn version(&self) -> u32 {
        self.inner.version()
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
            .snapshots
            .iter()
            .cloned()
            .map(PySnapshot::from_core)
            .collect()
    }

    /// The free-form table properties the metadata document carries.
    #[getter]
    fn properties<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let properties = PyDict::new(py);
        for (key, value) in &self.inner.metadata().properties {
            properties.set_item(key.as_str(), value.as_str())?;
        }
        Ok(properties)
    }

    /// Every schema the table has had, by identifier.
    #[getter]
    fn schemas(&self) -> Vec<PyField> {
        self.inner
            .metadata()
            .schemas
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
    #[pyo3(signature = (field = None))]
    fn scan<'py>(
        &self,
        py: Python<'py>,
        field: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let field = field
            .map(|field| core_root_field_from_value(field, SCHEMA_ROOT_NAME))
            .transpose()?;
        let reader = self.inner.scan(field.as_ref()).map_err(value_error)?;
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
        answer.set_item("record_count", plan.record_count())?;
        Ok(answer)
    }

    /// Append `batches` as a new snapshot, keeping everything already stored.
    fn append(&mut self, batches: &Bound<'_, PyAny>) -> PyResult<()> {
        let batches = batch_reader_from_value(batches)?;
        self.inner.append(batches).map_err(value_error)
    }

    /// Replace every row with `batches` as a new snapshot.
    fn overwrite(&mut self, batches: &Bound<'_, PyAny>) -> PyResult<()> {
        let batches = batch_reader_from_value(batches)?;
        self.inner.overwrite(batches).map_err(value_error)
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
    #[pyo3(signature = (snapshot_id, filters = None, schema = None))]
    fn scan_at<'py>(
        &self,
        py: Python<'py>,
        snapshot_id: i64,
        filters: Option<&Bound<'_, PyAny>>,
        schema: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let pairs = filter_pairs_from_value(filters)?;
        let borrowed: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect();
        let field = schema
            .map(|schema| core_root_field_from_value(schema, SCHEMA_ROOT_NAME))
            .transpose()?;
        let reader = self
            .inner
            .scan_at(snapshot_id, &borrowed, field.as_ref())
            .map_err(value_error)?;
        batch_reader_to_pyarrow(py, reader)
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
        self.inner.target_file_size().map_err(value_error)
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
            .commit_changes(|metadata| {
                for (key, value) in &updates {
                    metadata.set_property(key.as_str(), value.as_str())?;
                }
                for key in &removes {
                    metadata.remove_property(key);
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
            self.inner.metadata().location.as_str(),
            self.inner.metadata().format_version.number(),
            self.inner.version(),
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

    /// Record a rename of the column at `path`; its identifier is kept, which
    /// is what keeps the rows written under the old name readable.
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
            .commit_changes(move |metadata| {
                // Replayed by reference: a beaten commit rebases and runs this
                // closure again on the winner's metadata, so the recording
                // must survive every attempt.
                let mut update = SchemaUpdate::for_metadata(metadata)?;
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
                let evolved = update.apply()?;
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
            "Compaction(files_before={}, files_after={}, bytes_rewritten={})",
            self.inner.files_before, self.inner.files_after, self.inner.bytes_rewritten,
        )
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
impl PyPartitionField {
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

    fn __repr__(&self) -> String {
        format!(
            "PartitionField({:?}, transform={:?})",
            self.inner.name.as_str(),
            self.inner.transform.to_string(),
        )
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
impl PyPartitionSpec {
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

    fn __repr__(&self) -> String {
        format!(
            "PartitionSpec(spec_id={}, fields={})",
            self.inner.spec_id,
            self.inner.fields.len(),
        )
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
impl PySnapshot {
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

    /// Everything the commit recorded about itself.
    #[getter]
    fn summary<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let summary = PyDict::new(py);
        for (key, value) in &self.inner.summary {
            summary.set_item(key.as_str(), value.as_str())?;
        }
        Ok(summary)
    }

    fn __repr__(&self) -> String {
        format!(
            "Snapshot({}, operation={:?})",
            self.inner.snapshot_id,
            self.inner.operation(),
        )
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
}

#[pymethods]
impl PyManifestFile {
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

    /// Whether the manifest lists data files rather than delete files.
    fn is_data(&self) -> bool {
        self.inner.content == ManifestContent::Data
    }

    /// The commit order assigned when the manifest was added.
    #[getter]
    fn sequence_number(&self) -> i64 {
        self.inner.sequence_number
    }

    /// The snapshot that added the manifest.
    #[getter]
    fn added_snapshot_id(&self) -> i64 {
        self.inner.added_snapshot_id
    }

    /// The files the manifest marks added.
    #[getter]
    fn added_files_count(&self) -> i32 {
        self.inner.added_files_count
    }

    /// The files the manifest marks existing.
    #[getter]
    fn existing_files_count(&self) -> i32 {
        self.inner.existing_files_count
    }

    /// The files the manifest marks deleted.
    #[getter]
    fn deleted_files_count(&self) -> i32 {
        self.inner.deleted_files_count
    }

    /// The rows in the added files.
    #[getter]
    fn added_rows_count(&self) -> i64 {
        self.inner.added_rows_count
    }

    /// The rows in the existing files.
    #[getter]
    fn existing_rows_count(&self) -> i64 {
        self.inner.existing_rows_count
    }

    fn __repr__(&self) -> String {
        format!(
            "ManifestFile({:?}, added_files_count={})",
            self.inner.manifest_path.as_str(),
            self.inner.added_files_count,
        )
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
}

#[pymethods]
impl PyDataFile {
    /// The file's location, as a URI.
    #[getter]
    fn path(&self) -> &str {
        self.inner.file_path.as_str()
    }

    /// The encoding the file uses, such as `PARQUET`.
    #[getter]
    fn file_format(&self) -> String {
        self.inner.file_format.to_string()
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

    /// The sort order the file was written in, when one applies.
    #[getter]
    fn sort_order_id(&self) -> Option<i32> {
        self.inner.sort_order_id
    }

    /// Zero for rows, one for position deletes, two for equality deletes.
    #[getter]
    fn content(&self) -> i32 {
        self.inner.content
    }

    fn __repr__(&self) -> String {
        format!(
            "DataFile({:?}, record_count={})",
            self.inner.file_path.as_str(),
            self.inner.record_count,
        )
    }
}

/// One namespace of a catalog: the first half of `catalog[ns][table]`.
///
/// Indexing reads - `namespace["trades"]` opens the table or raises
/// `KeyError` - and assigning gets-or-creates: a schema-like value opens the
/// table, creating it with that schema when absent, and a rows-like value
/// replaces the table's rows, creating it from the rows' own schema on first
/// write.
#[pyclass(name = "Namespace", module = "yggdryl._native", skip_from_py_object)]
pub(crate) struct PyNamespace {
    catalog: Py<PyCatalog>,
    name: String,
}

#[pymethods]
impl PyNamespace {
    /// The namespace's dotted name.
    #[getter]
    fn name(&self) -> &str {
        &self.name
    }

    /// Open the named table.
    fn table(&self, py: Python<'_>, name: &str) -> PyResult<PyTable> {
        let catalog = self.catalog.borrow(py);
        catalog
            .inner
            .namespace(&self.name)
            .table(name)
            .map(PyTable::from_core)
            .map_err(value_error)
    }

    /// Return whether the named table exists here.
    fn has_table(&self, py: Python<'_>, name: &str) -> PyResult<bool> {
        let catalog = self.catalog.borrow(py);
        catalog
            .inner
            .namespace(&self.name)
            .has_table(name)
            .map_err(value_error)
    }

    /// Open the named table, creating it with `schema` when absent.
    fn open_or_create_table(
        &self,
        py: Python<'_>,
        name: &str,
        schema: &Bound<'_, PyAny>,
    ) -> PyResult<PyTable> {
        let schema = catalog_schema_from_value(schema)?;
        let catalog = self.catalog.borrow(py);
        catalog
            .inner
            .namespace(&self.name)
            .open_or_create_table(name, schema)
            .map(PyTable::from_core)
            .map_err(value_error)
    }

    /// This namespace's tables, as bare names.
    fn list_tables(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        let catalog = self.catalog.borrow(py);
        catalog
            .inner
            .namespace(&self.name)
            .list_tables()
            .map_err(value_error)
    }

    /// The namespaces one level below this one, as bare names.
    fn list_namespaces(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        let catalog = self.catalog.borrow(py);
        catalog
            .inner
            .namespace(&self.name)
            .list_namespaces()
            .map_err(value_error)
    }

    /// `namespace["trades"]` opens the table; a missing one is a `KeyError`.
    fn __getitem__(&self, py: Python<'_>, name: &str) -> PyResult<PyTable> {
        if !self.has_table(py, name)? {
            return Err(pyo3::exceptions::PyKeyError::new_err(format!(
                "no table named {name:?} in namespace {:?}",
                self.name
            )));
        }
        self.table(py, name)
    }

    /// Assigning gets or creates.
    ///
    /// A schema-like value - a `Field`, `DataType`, `pyarrow` schema, or
    /// datatype string - opens the table, creating it with that schema when
    /// absent. Anything rows-like replaces the table's rows, creating the
    /// table from the rows' own schema on first write.
    fn __setitem__(&self, py: Python<'_>, name: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
        if let Ok(schema) = catalog_schema_from_value(value) {
            let catalog = self.catalog.borrow(py);
            catalog
                .inner
                .namespace(&self.name)
                .open_or_create_table(name, schema)
                .map_err(value_error)?;
            return Ok(());
        }
        let data = batch_reader_from_value(value)?;
        let catalog = self.catalog.borrow(py);
        catalog
            .inner
            .namespace(&self.name)
            .overwrite(name, data)
            .map_err(value_error)?;
        Ok(())
    }

    /// `"trades" in namespace` asks whether the table exists.
    fn __contains__(&self, py: Python<'_>, name: &str) -> PyResult<bool> {
        self.has_table(py, name)
    }

    /// Iterating a namespace yields its bare table names.
    fn __iter__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let list = pyo3::types::PyList::new(py, self.list_tables(py)?)?;
        Ok(list.as_any().try_iter()?.unbind().into_any())
    }

    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        Ok(self.list_tables(py)?.len())
    }

    fn __repr__(&self) -> String {
        format!("Namespace({:?})", self.name)
    }
}
