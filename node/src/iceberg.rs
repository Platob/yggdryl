//! Apache Iceberg tables, reached from JavaScript through one handle.
//!
//! A table is a folder: `metadata/` holds the JSON documents and the Avro
//! manifests, `data/` holds the Parquet files, and every one of them is a child
//! of the handle the table was built from. Nothing here opens a path, so the
//! same JavaScript works over a local directory today and over an object store
//! the moment a backend for one exists.

use std::collections::HashMap;

use napi::bindgen_prelude::{BigInt, Buffer, ClassInstance, Either, Either3, Reference, Result};
use napi_derive::napi;
use yggdryl::generic::Holder;
use yggdryl::iceberg::{
    Catalog as CoreCatalog, DataFile, FormatVersion, ManifestContent, ManifestFile,
    PartitionSpec as CorePartitionSpec, SchemaUpdate as CoreSchemaUpdate, Snapshot,
    Table as CoreTable, assign_field_ids, can_promote, last_field_id, schema_from_json,
    schema_to_json,
};
use yggdryl::{DataType as CoreDataType, Field as CoreField};

use crate::arrow::JsBatchReader;
use crate::codec::JsCodecValue;
use crate::datatype::{JsDataType, data_type_from_input};
use crate::field::{JsField, MetadataEntry};
use crate::io::{JsIOBase, LocationInput, folder_from_input};
use crate::napi_error;
use crate::uri::PartitionEntry;

/// A partition spec, or the column names one would be built from.
pub type PartitionInput<'a> = Either<ClassInstance<'a, JsPartitionSpec>, Vec<String>>;

/// A native root `Field`, a field expression, or the child fields of a row.
pub type TableSchemaInput<'a> =
    Either3<ClassInstance<'a, JsField>, String, Vec<ClassInstance<'a, JsField>>>;

/// A native `Field` or the field expression naming one.
pub type FieldInput<'a> = Either<ClassInstance<'a, JsField>, String>;

/// A native `DataType` or the type expression naming one.
pub type DataTypeInput<'a> = Either<ClassInstance<'a, JsDataType>, String>;

/// A retained snapshot id: the `bigint` a snapshot reports, or a safe number.
pub type SnapshotIdInput = Either<BigInt, f64>;

/// Scan filters: `(column, value)` text pairs as entries or as one mapping.
pub type ScanFilters = Either<Vec<PartitionEntry>, HashMap<String, String>>;

/// Table property updates: ordered entries or one plain mapping.
pub type PropertyUpdates = Either<Vec<MetadataEntry>, HashMap<String, String>>;

/// The root name a schema assembled from bare child fields is given.
///
/// It matches the name the core catalog gives a schema inferred from an
/// incoming reader, so both spellings of "just the columns" agree.
const ROOT_NAME: &str = "row";

/// Read the schema an input names: a root `Field` as it stands, an expression
/// through the core parser, or bare children assembled under a `row` root.
fn schema_from_input(value: TableSchemaInput<'_>) -> Result<CoreField> {
    match value {
        Either3::A(field) => Ok(field.inner.clone()),
        Either3::B(text) => CoreField::from_str(&text).map_err(napi_error),
        Either3::C(children) => {
            let fields = children.iter().map(|child| child.inner.clone());
            Ok(CoreDataType::from_fields(fields)
                .map_err(napi_error)?
                .required_field(ROOT_NAME))
        }
    }
}

/// Number a schema the way `Table.create` needs it.
///
/// Numbering continues above the highest identifier already assigned, so a
/// numbered schema keeps every id it came with and a plain schema arrives with
/// none and leaves with all of them. It happens at this boundary because the
/// spec builder resolves `partitionBy` names to identifiers.
fn numbered_schema(mut schema: CoreField) -> Result<CoreField> {
    let start = last_field_id(&schema)
        .map_err(napi_error)?
        .saturating_add(1);
    assign_field_ids(&mut schema, start).map_err(napi_error)?;
    Ok(schema)
}

/// Read the field an input names, exactly as `Field.from` infers one.
fn field_from_input(value: FieldInput<'_>) -> Result<CoreField> {
    match value {
        Either::A(field) => Ok(field.inner.clone()),
        Either::B(text) => CoreField::from_str(&text).map_err(napi_error),
    }
}

/// Read a snapshot id exactly: a `bigint` as is, a number below 2^53.
fn snapshot_id_from_input(value: SnapshotIdInput) -> Result<i64> {
    match value {
        Either::A(value) => {
            let (id, lossless) = value.get_i64();
            if !lossless {
                return Err(napi_error("snapshotId must fit in a signed 64-bit integer"));
            }
            Ok(id)
        }
        Either::B(value) => crate::exact_i64(value, "snapshotId"),
    }
}

/// Collect scan filters into owned `(column, value)` pairs.
fn filter_pairs(filters: Option<ScanFilters>) -> Vec<(String, String)> {
    match filters {
        None => Vec::new(),
        Some(Either::A(entries)) => entries
            .into_iter()
            .map(|entry| (entry.column, entry.value))
            .collect(),
        Some(Either::B(values)) => values.into_iter().collect(),
    }
}

/// Read the format version a number names, defaulting to v2.
fn format_version(value: Option<u32>) -> Result<FormatVersion> {
    match value {
        Some(number) => FormatVersion::from_number(i64::from(number)).map_err(napi_error),
        None => Ok(FormatVersion::V2),
    }
}

/// Resolve what a caller partitioned by, defaulting to unpartitioned.
///
/// Column names are the short spelling of the only spec a write can use, so
/// they build an identity spec against the schema they name columns of.
fn partition_spec(
    value: Option<PartitionInput<'_>>,
    schema: &CoreField,
) -> Result<CorePartitionSpec> {
    match value {
        None => Ok(CorePartitionSpec::unpartitioned()),
        Some(Either::A(spec)) => Ok(spec.inner.clone()),
        Some(Either::B(columns)) => {
            let names: Vec<&str> = columns.iter().map(String::as_str).collect();
            CorePartitionSpec::identity(0, schema, &names).map_err(napi_error)
        }
    }
}

/// One partition field of a spec.
#[napi(object)]
pub struct PartitionFieldView {
    /// Identifier of the schema column the value is derived from.
    pub source_id: i32,
    /// Identifier of the partition field itself.
    pub field_id: i32,
    /// The directory name this field writes.
    pub name: String,
    /// The transform applied to the source column.
    pub transform: String,
}

/// One per-column count a manifest records, keyed by field identifier.
#[napi(object)]
pub struct FieldCount {
    /// The schema field identifier the count belongs to.
    pub field_id: i32,
    /// The recorded count.
    pub count: i64,
}

/// One per-column bound a manifest records, as its encoded bytes.
#[napi(object)]
pub struct FieldBound {
    /// The schema field identifier the bound belongs to.
    pub field_id: i32,
    /// The single-value encoding of the bound.
    pub value: Buffer,
}

/// One committed version of a table's contents.
#[napi(object)]
pub struct SnapshotView {
    /// Identifier of this snapshot, unique within the table.
    pub snapshot_id: BigInt,
    /// The snapshot this one was produced from, when there was one.
    pub parent_snapshot_id: Option<BigInt>,
    /// Monotonic commit order, absent in v1 tables.
    pub sequence_number: Option<i64>,
    /// Wall-clock commit time in milliseconds since the Unix epoch.
    pub timestamp_ms: i64,
    /// Location of the Avro manifest list this snapshot's manifests are in.
    pub manifest_list: String,
    /// What the commit did, defaulting to `append`.
    pub operation: String,
    /// The commit summary, keyed by Iceberg's summary vocabulary.
    pub summary: HashMap<String, String>,
    /// The schema in effect when the snapshot was written.
    pub schema_id: Option<i32>,
}

/// One manifest of the current snapshot.
#[napi(object)]
pub struct ManifestFileView {
    /// The manifest's location, as a URI.
    pub manifest_path: String,
    /// Size of the manifest in bytes.
    pub manifest_length: i64,
    /// The partition spec the manifest's entries were written under.
    pub partition_spec_id: i32,
    /// Whether the manifest lists `data` files or `deletes`.
    pub content: String,
    /// Commit order assigned when the manifest was added.
    pub sequence_number: i64,
    /// Lowest commit order of any entry in the manifest.
    pub min_sequence_number: i64,
    /// The snapshot that added the manifest.
    pub added_snapshot_id: BigInt,
    /// Files the manifest marks added.
    pub added_files_count: i32,
    /// Files the manifest marks existing.
    pub existing_files_count: i32,
    /// Files the manifest marks deleted.
    pub deleted_files_count: i32,
    /// Rows in the added files.
    pub added_rows_count: i64,
    /// Rows in the existing files.
    pub existing_rows_count: i64,
    /// Rows in the deleted files.
    pub deleted_rows_count: i64,
}

/// One live data file of the current snapshot, with the spec that placed it.
///
/// This is a class rather than a plain object because a partition value crosses
/// as the native [`Value`](crate::codec::JsCodecValue) the manifest recorded.
/// Rendering it as text here would have to spell a null `null`, which is exactly
/// what makes a directory name unable to answer the question.
#[napi(js_name = "DataFile")]
pub struct JsDataFile {
    /// The manifest's record of the file.
    file: DataFile,
    /// The spec its partition tuple is ordered by.
    spec: CorePartitionSpec,
}

#[napi]
impl JsDataFile {
    /// Zero for rows, one for position deletes, two for equality deletes.
    #[napi(getter)]
    pub const fn content(&self) -> i32 {
        self.file.content
    }

    /// The file's location, as a URI.
    #[napi(getter)]
    pub fn file_path(&self) -> String {
        self.file.file_path.to_string()
    }

    /// The encoding the file uses.
    #[napi(getter)]
    pub fn file_format(&self) -> String {
        self.file.file_format.to_string()
    }

    /// The partition tuple the manifest records, in spec order.
    #[napi(getter)]
    pub fn partition(&self) -> Vec<JsCodecValue> {
        self.file
            .partition
            .iter()
            .map(|value| JsCodecValue::from_core(value.clone()))
            .collect()
    }

    /// The partition field names, in the same order as the tuple.
    #[napi(getter)]
    pub fn partition_names(&self) -> Vec<String> {
        self.spec
            .fields
            .iter()
            .map(|field| field.name.to_string())
            .collect()
    }

    /// Rows in the file.
    #[napi(getter)]
    pub const fn record_count(&self) -> i64 {
        self.file.record_count
    }

    /// Size of the file in bytes.
    #[napi(getter)]
    pub const fn file_size_in_bytes(&self) -> i64 {
        self.file.file_size_in_bytes
    }

    /// Stored bytes per column.
    #[napi(getter)]
    pub fn column_sizes(&self) -> Vec<FieldCount> {
        counts(&self.file.column_sizes)
    }

    /// Values per column.
    #[napi(getter)]
    pub fn value_counts(&self) -> Vec<FieldCount> {
        counts(&self.file.value_counts)
    }

    /// Nulls per column.
    #[napi(getter)]
    pub fn null_value_counts(&self) -> Vec<FieldCount> {
        counts(&self.file.null_value_counts)
    }

    /// Serialized minimum per column, where the two encodings agree on one.
    #[napi(getter)]
    pub fn lower_bounds(&self) -> Vec<FieldBound> {
        bounds(&self.file.lower_bounds)
    }

    /// Serialized maximum per column, where the two encodings agree on one.
    #[napi(getter)]
    pub fn upper_bounds(&self) -> Vec<FieldBound> {
        bounds(&self.file.upper_bounds)
    }

    /// The sort order the file was written in, when one applies.
    #[napi(getter)]
    pub const fn sort_order_id(&self) -> Option<i32> {
        self.file.sort_order_id
    }

    /// Return the file's location, so a data file prints as where it is.
    #[napi]
    pub fn to_string(&self) -> String {
        self.file_path()
    }
}

fn counts(values: &[(i32, i64)]) -> Vec<FieldCount> {
    values
        .iter()
        .map(|(field_id, count)| FieldCount {
            field_id: *field_id,
            count: *count,
        })
        .collect()
}

fn bounds(values: &[(i32, Vec<u8>)]) -> Vec<FieldBound> {
    values
        .iter()
        .map(|(field_id, value)| FieldBound {
            field_id: *field_id,
            value: value.clone().into(),
        })
        .collect()
}

fn snapshot_view(snapshot: &Snapshot) -> SnapshotView {
    SnapshotView {
        snapshot_id: BigInt::from(snapshot.snapshot_id),
        parent_snapshot_id: snapshot.parent_snapshot_id.map(BigInt::from),
        sequence_number: snapshot.sequence_number,
        timestamp_ms: snapshot.timestamp_ms,
        manifest_list: snapshot.manifest_list.to_string(),
        operation: snapshot.operation().to_owned(),
        summary: snapshot
            .summary
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect(),
        schema_id: snapshot.schema_id,
    }
}

/// Name what a manifest's entries describe.
///
/// The core enum is non-exhaustive, so a content this build does not have a
/// word for crosses as the integer Iceberg stores rather than as a panic.
fn manifest_content(content: ManifestContent) -> String {
    match content {
        ManifestContent::Data => "data".to_owned(),
        ManifestContent::Deletes => "deletes".to_owned(),
        other => other.code().to_string(),
    }
}

fn manifest_view(manifest: &ManifestFile) -> ManifestFileView {
    ManifestFileView {
        manifest_path: manifest.manifest_path.to_string(),
        manifest_length: manifest.manifest_length,
        partition_spec_id: manifest.partition_spec_id,
        content: manifest_content(manifest.content),
        sequence_number: manifest.sequence_number,
        min_sequence_number: manifest.min_sequence_number,
        added_snapshot_id: BigInt::from(manifest.added_snapshot_id),
        added_files_count: manifest.added_files_count,
        existing_files_count: manifest.existing_files_count,
        deleted_files_count: manifest.deleted_files_count,
        added_rows_count: manifest.added_rows_count,
        existing_rows_count: manifest.existing_rows_count,
        deleted_rows_count: manifest.deleted_rows_count,
    }
}

/// How a table turns column values into the directories it writes.
#[napi(js_name = "PartitionSpec")]
pub struct JsPartitionSpec {
    pub(crate) inner: CorePartitionSpec,
}

impl JsPartitionSpec {
    pub(crate) const fn from_core(inner: CorePartitionSpec) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsPartitionSpec {
    /// Describe a table that writes every row into one place.
    #[napi(factory)]
    pub fn unpartitioned() -> Self {
        Self::from_core(CorePartitionSpec::unpartitioned())
    }

    /// Partition by the named columns, storing each value as it stands.
    ///
    /// Identity is one of the two transforms that can place a row, so this is
    /// the spec a write can use; a `bucket`, `truncate`, or calendar spec reads
    /// here but is refused by name when it would have to place a row.
    #[napi(factory)]
    pub fn identity(schema: &JsField, columns: Vec<String>, spec_id: Option<i32>) -> Result<Self> {
        let names: Vec<&str> = columns.iter().map(String::as_str).collect();
        CorePartitionSpec::identity(spec_id.unwrap_or(0), &schema.inner, &names)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The identifier this spec is recorded under.
    #[napi(getter)]
    pub const fn spec_id(&self) -> i32 {
        self.inner.spec_id
    }

    /// The partition fields, in the order the directories nest.
    #[napi(getter)]
    pub fn fields(&self) -> Vec<PartitionFieldView> {
        self.inner
            .fields
            .iter()
            .map(|field| PartitionFieldView {
                source_id: field.source_id,
                field_id: field.field_id,
                name: field.name.to_string(),
                transform: field.transform.to_string(),
            })
            .collect()
    }

    /// Return whether this spec writes every row into one place.
    #[napi]
    pub fn is_unpartitioned(&self) -> bool {
        self.inner.is_unpartitioned()
    }
}

/// An Iceberg table reached entirely through one container handle.
#[napi(js_name = "Table")]
pub struct JsTable {
    inner: CoreTable<Holder>,
}

impl JsTable {
    const fn from_core(inner: CoreTable<Holder>) -> Self {
        Self { inner }
    }

    /// The root name a scan's batches are described by.
    fn root_name(&self) -> Result<String> {
        Ok(self.inner.schema().map_err(napi_error)?.name().to_owned())
    }
}

#[napi]
impl JsTable {
    /// Create a table, writing its first metadata document.
    ///
    /// `partitionBy` takes a [`PartitionSpec`](JsPartitionSpec) or the column
    /// names to partition on, and defaults to unpartitioned. Unnumbered schema
    /// columns are numbered automatically, so a plain schema works as it is; a
    /// schema that already carries field identifiers keeps every one of them.
    #[napi(factory)]
    pub fn create(
        root: LocationInput<'_>,
        schema: &JsField,
        partition_by: Option<PartitionInput<'_>>,
        version: Option<u32>,
    ) -> Result<Self> {
        let schema = numbered_schema(schema.inner.clone())?;
        let spec = partition_spec(partition_by, &schema)?;
        CoreTable::create(
            folder_from_input(root)?,
            format_version(version)?,
            schema,
            spec,
        )
        .map(Self::from_core)
        .map_err(napi_error)
    }

    /// Open the table a container handle addresses.
    #[napi(factory)]
    pub fn open(root: LocationInput<'_>) -> Result<Self> {
        CoreTable::open(folder_from_input(root)?)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Open the table if it exists, creating it otherwise.
    ///
    /// Like [`create`](Self::create), unnumbered schema columns are numbered
    /// automatically; an existing table is opened as it is and `schema`
    /// describes only the table this call would create.
    #[napi(factory)]
    pub fn open_or_create(
        root: LocationInput<'_>,
        schema: &JsField,
        partition_by: Option<PartitionInput<'_>>,
        version: Option<u32>,
    ) -> Result<Self> {
        let schema = numbered_schema(schema.inner.clone())?;
        let spec = partition_spec(partition_by, &schema)?;
        CoreTable::open_or_create(
            folder_from_input(root)?,
            format_version(version)?,
            schema,
            spec,
        )
        .map(Self::from_core)
        .map_err(napi_error)
    }

    /// The folder the table lives in.
    #[napi(getter)]
    pub fn root(&self) -> Result<JsIOBase> {
        JsIOBase::folder_at(&self.location())
    }

    /// The table's base location, as a URI.
    #[napi(getter)]
    pub fn location(&self) -> String {
        self.inner.metadata().location.to_string()
    }

    /// A stable identifier for the table itself, not for any one version.
    #[napi(getter)]
    pub fn table_uuid(&self) -> String {
        self.inner.metadata().table_uuid.to_string()
    }

    /// Which revision of the specification the metadata is written to.
    #[napi(getter)]
    pub const fn format_version(&self) -> i32 {
        self.inner.metadata().format_version.number()
    }

    /// The version number of the current metadata document.
    #[napi(getter)]
    pub const fn version(&self) -> u32 {
        self.inner.version()
    }

    /// Free-form table properties.
    #[napi(getter)]
    pub fn properties(&self) -> HashMap<String, String> {
        self.inner
            .metadata()
            .properties
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    /// The name of the current metadata document.
    #[napi(getter)]
    pub fn metadata_file_name(&self) -> String {
        self.inner.metadata_file_name()
    }

    /// The location of the current metadata document, as a URI.
    #[napi(getter)]
    pub fn metadata_location(&self) -> Result<String> {
        self.inner.metadata_location().map_err(napi_error)
    }

    /// The schema new data is written against.
    #[napi(getter)]
    pub fn schema(&self) -> Result<JsField> {
        self.inner
            .schema()
            .map(|schema| JsField::from_core(schema.clone()))
            .map_err(napi_error)
    }

    /// The partition spec new data is written against.
    #[napi(getter)]
    pub fn spec(&self) -> JsPartitionSpec {
        let metadata = self.inner.metadata();
        JsPartitionSpec::from_core(
            metadata
                .partition_specs
                .iter()
                .find(|spec| spec.spec_id == metadata.default_spec_id)
                .cloned()
                .unwrap_or_else(CorePartitionSpec::unpartitioned),
        )
    }

    /// The snapshot a reader sees, or `null` when the table has none.
    ///
    /// A freshly created or rolled-back table has snapshots but no current one,
    /// and reading it yields no rows rather than failing.
    #[napi(getter)]
    pub fn current_snapshot(&self) -> Option<SnapshotView> {
        self.inner.current_snapshot().map(snapshot_view)
    }

    /// Every schema the table has had, oldest first.
    #[napi(getter)]
    pub fn schemas(&self) -> Vec<JsField> {
        self.inner
            .metadata()
            .schemas
            .iter()
            .cloned()
            .map(JsField::from_core)
            .collect()
    }

    /// Every retained snapshot, oldest first.
    #[napi(getter)]
    pub fn snapshots(&self) -> Vec<SnapshotView> {
        self.inner
            .metadata()
            .snapshots
            .iter()
            .map(snapshot_view)
            .collect()
    }

    /// Every manifest the current snapshot points at.
    #[napi]
    pub fn manifests(&self) -> Result<Vec<ManifestFileView>> {
        Ok(self
            .inner
            .manifests()
            .map_err(napi_error)?
            .iter()
            .map(manifest_view)
            .collect())
    }

    /// Every live data file of the current snapshot.
    #[napi]
    pub fn data_files(&self) -> Result<Vec<JsDataFile>> {
        Ok(self
            .inner
            .data_files()
            .map_err(napi_error)?
            .into_iter()
            .map(|(file, spec)| JsDataFile { file, spec })
            .collect())
    }

    /// Read every row of the current snapshot, keeping the columns `field` names.
    ///
    /// Unlike a plain handle read, a scan *casts* each file to the root it is
    /// given after pushing the columns down, which is what makes a table whose
    /// schema evolved readable as one shape.
    #[napi]
    pub fn scan(&self, field: Option<&JsField>) -> Result<JsBatchReader> {
        let root_name = self.root_name()?;
        let reader = self
            .inner
            .scan(field.map(|field| &field.inner))
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, &root_name))
    }

    /// Read the rows matching one predicate as a `BatchReader`.
    ///
    /// `filter` is an `Expression` or the text of one, which parses. It is the
    /// whole expression language rather than equality pairs: ranges, null
    /// tests, `in` lists, nested paths, and `&holder.*` questions about the
    /// files themselves. Planning prunes with the metadata chain, and only the
    /// conjuncts it could not settle are tested against the rows.
    #[napi]
    pub fn scan_matching(
        &self,
        filter: napi::bindgen_prelude::Either<
            napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsExpression>,
            String,
        >,
        field: Option<&JsField>,
    ) -> Result<JsBatchReader> {
        let root_name = self.root_name()?;
        let filter = crate::expression::expression_from_input(filter)?;
        let reader = self
            .inner
            .scan_matching(filter, field.map(|field| &field.inner))
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, &root_name))
    }

    /// Report what one predicate lets the scan leave alone.
    #[napi]
    pub fn plan_matching(
        &self,
        filter: napi::bindgen_prelude::Either<
            napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsExpression>,
            String,
        >,
    ) -> Result<ScanPlanCounts> {
        let filter = crate::expression::expression_from_input(filter)?;
        let plan = self.inner.plan_matching(filter).map_err(napi_error)?;
        Ok(ScanPlanCounts {
            tasks: i64::try_from(plan.tasks.len()).unwrap_or(i64::MAX),
            files_skipped: i64::try_from(plan.files_skipped()).unwrap_or(i64::MAX),
            manifests_read: i64::try_from(plan.manifests_read).unwrap_or(i64::MAX),
            manifests_skipped: i64::try_from(plan.manifests_skipped()).unwrap_or(i64::MAX),
            record_count: plan.record_count(),
        })
    }

    /// Append `batches` as a new snapshot, keeping everything already stored.
    #[napi]
    pub fn append(&mut self, batches: &mut JsBatchReader) -> Result<()> {
        self.inner.append(batches.take()?).map_err(napi_error)
    }

    /// Replace every row with `batches` as a new snapshot.
    ///
    /// The previous snapshot stays readable; only the current pointer moves.
    #[napi]
    pub fn overwrite(&mut self, batches: &mut JsBatchReader) -> Result<()> {
        self.inner.overwrite(batches.take()?).map_err(napi_error)
    }

    /// Add a schema, make it current, and write a new metadata document.
    #[napi]
    pub fn evolve_schema(&mut self, schema: &JsField) -> Result<i32> {
        self.inner
            .evolve_schema(schema.inner.clone())
            .map_err(napi_error)
    }

    /// Read one retained snapshot's rows: time travel as an ordinary scan.
    ///
    /// `snapshotId` is the identifier a snapshot reports, as a `bigint` or as
    /// a number no larger than 2^53. `filters` is the same `(column, value)`
    /// pair vocabulary `childrenWhere` uses, and `schema` keeps the columns it
    /// names, exactly as on [`scan`](Self::scan). The rows are read as the
    /// schema the snapshot was written under.
    #[napi]
    pub fn scan_at(
        &self,
        snapshot_id: SnapshotIdInput,
        filters: Option<ScanFilters>,
        schema: Option<FieldInput<'_>>,
    ) -> Result<JsBatchReader> {
        let root_name = self.root_name()?;
        let snapshot_id = snapshot_id_from_input(snapshot_id)?;
        let pairs = filter_pairs(filters);
        let borrowed: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect();
        let schema = schema.map(field_from_input).transpose()?;
        let reader = self
            .inner
            .scan_at(snapshot_id, &borrowed, schema.as_ref())
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, &root_name))
    }

    /// Return the retained snapshot a branch or tag names.
    ///
    /// A name the table does not have is refused naming the refs it does.
    #[napi]
    pub fn snapshot_by_ref(&self, name: String) -> Result<SnapshotView> {
        self.inner
            .snapshot_by_ref(&name)
            .map(snapshot_view)
            .map_err(napi_error)
    }

    /// The size a data file aims for, in bytes.
    ///
    /// The table property `write.target-file-size-bytes` decides, falling back
    /// to the schema root's protocol property of the same name, then to
    /// Iceberg's own 512 MiB default. A present-but-unparseable value throws
    /// naming the key and the value rather than silently using the default.
    #[napi(getter)]
    pub fn target_file_size(&self) -> Result<i64> {
        let target = self.inner.target_file_size().map_err(napi_error)?;
        Ok(i64::try_from(target).unwrap_or(i64::MAX))
    }

    /// Merge the current snapshot's undersized data files, per partition.
    ///
    /// The commit is one `replace` snapshot, so the pre-compaction snapshot
    /// stays readable through [`scanAt`](Self::scan_at). A table with nothing
    /// to compact commits nothing and reports zeros.
    #[napi]
    pub fn compact(&mut self) -> Result<Compaction> {
        let outcome = self.inner.compact().map_err(napi_error)?;
        Ok(Compaction {
            files_before: i64::try_from(outcome.files_before).unwrap_or(i64::MAX),
            files_after: i64::try_from(outcome.files_after).unwrap_or(i64::MAX),
            bytes_rewritten: outcome.bytes_rewritten,
        })
    }

    /// Render when each snapshot became current, oldest first.
    ///
    /// The columns are `made_current_at`, `snapshot_id`, `parent_id`, and
    /// `is_current_ancestor`, the names `PyIceberg`'s `history` table uses.
    #[napi]
    pub fn inspect_history(&self) -> Result<JsBatchReader> {
        let reader = self.inner.inspect_history().map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, "history"))
    }

    /// Render every retained snapshot with its operation and summary.
    ///
    /// The columns are `committed_at`, `snapshot_id`, `parent_id`,
    /// `operation`, `manifest_list`, and the free-form `summary` map.
    #[napi]
    pub fn inspect_snapshots(&self) -> Result<JsBatchReader> {
        let reader = self.inner.inspect_snapshots().map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, "snapshots"))
    }

    /// Render the live data files of the current snapshot.
    ///
    /// The columns are `file_path`, `file_format`, `spec_id`, the rendered
    /// `partition` chain, `record_count`, and `file_size_in_bytes`.
    #[napi]
    pub fn inspect_files(&self) -> Result<JsBatchReader> {
        let reader = self.inner.inspect_files().map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, "files"))
    }

    /// Set and remove table properties as one metadata-only commit.
    ///
    /// `updates` is a mapping of properties to set and `removes` lists the
    /// keys to drop, in that order. Passing neither commits nothing at all: a
    /// commit that changes no property would still cost a metadata document.
    #[napi]
    pub fn update_properties(
        &mut self,
        updates: Option<PropertyUpdates>,
        removes: Option<Vec<String>>,
    ) -> Result<()> {
        let updates: Vec<(String, String)> = match updates {
            None => Vec::new(),
            Some(Either::A(entries)) => entries
                .into_iter()
                .map(|entry| (entry.key, entry.value))
                .collect(),
            Some(Either::B(values)) => values.into_iter().collect(),
        };
        let removes = removes.unwrap_or_default();
        if updates.is_empty() && removes.is_empty() {
            return Ok(());
        }
        self.inner
            .commit_changes(|metadata| {
                // Applied by reference: a beaten commit rebases and runs this
                // closure again on the winner's metadata.
                for (key, value) in &updates {
                    metadata.set_property(key.as_str(), value.as_str())?;
                }
                for key in &removes {
                    metadata.remove_property(key);
                }
                Ok(())
            })
            .map_err(napi_error)
    }

    /// The native half of `updateSchema().commit()`.
    ///
    /// The recorded operations are replayed onto a fresh core `SchemaUpdate`
    /// built from the metadata the table holds *now*, the evolved schema is
    /// added and made current, and one new metadata document is written. The
    /// wrapper needs no refresh: a failed commit leaves the table as it was.
    #[napi(js_name = "_commitSchemaUpdateNative", skip_typescript)]
    pub fn commit_schema_update(&mut self, update: &JsSchemaUpdate) -> Result<i32> {
        let mut evolution =
            CoreSchemaUpdate::for_metadata(self.inner.metadata()).map_err(napi_error)?;
        for op in &update.ops {
            match op {
                SchemaOp::AddColumn { parent, field } => {
                    evolution.add_column(parent, field.clone());
                }
                SchemaOp::DropColumn { path } => evolution.drop_column(path),
                SchemaOp::RenameColumn { path, name } => {
                    evolution.rename_column(path, name.clone());
                }
                SchemaOp::UpdateDoc { path, doc } => evolution.update_doc(path, doc.clone()),
                SchemaOp::MakeNullable { path } => evolution.make_nullable(path),
                SchemaOp::UpdateType { path, data_type } => {
                    evolution.update_type(path, data_type.clone());
                }
            }
        }
        let evolved = evolution.apply().map_err(napi_error)?;
        self.inner.evolve_schema(evolved).map_err(napi_error)
    }

    /// Return where the table lives, so a table prints as its location.
    #[napi]
    pub fn to_string(&self) -> String {
        self.location()
    }
}

/// What one `compact` call rewrote.
///
/// The sizes cross as numbers because a data file already reports
/// `fileSizeInBytes` as one, and the two must agree.
#[napi(object)]
pub struct Compaction {
    /// How many live data files were read and replaced.
    pub files_before: i64,
    /// How many data files the rewrite produced in their place.
    pub files_after: i64,
    /// The recorded size of the replaced files, in bytes.
    pub bytes_rewritten: i64,
}

/// One recorded column operation, held as native values until a commit.
enum SchemaOp {
    /// Append a column under a parent path, `""` naming the root itself.
    AddColumn { parent: String, field: CoreField },
    /// Remove a column, retiring its identifier forever.
    DropColumn { path: String },
    /// Rename a column, keeping its identifier.
    RenameColumn { path: String, name: String },
    /// Set a column's `iceberg:doc` documentation string.
    UpdateDoc { path: String, doc: String },
    /// Relax a required column to optional.
    MakeNullable { path: String },
    /// Promote a column's type, checked when the update is applied.
    UpdateType {
        path: String,
        data_type: CoreDataType,
    },
}

/// A recording of column operations against a table's current schema.
///
/// The loader hands one out from `table.updateSchema()` and wraps each method
/// to return the builder, so a chain reads as one sentence. Nothing is checked
/// while recording: `commit()` replays the recording onto a fresh core
/// `SchemaUpdate`, which is what makes the operations apply to the schema the
/// table has *then* and report the first failure with its core message.
#[napi(js_name = "SchemaUpdate")]
#[derive(Default)]
pub struct JsSchemaUpdate {
    /// The recorded operations, in call order.
    ops: Vec<SchemaOp>,
}

#[napi]
impl JsSchemaUpdate {
    /// Start an empty recording.
    #[napi(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a new column under `parent` - `""` for the root, a dotted path
    /// for a nested struct.
    #[napi]
    pub fn add_column(&mut self, parent: String, field: FieldInput<'_>) -> Result<()> {
        let field = field_from_input(field)?;
        self.ops.push(SchemaOp::AddColumn { parent, field });
        Ok(())
    }

    /// Record the removal of the column at `path`, retiring its identifier.
    #[napi]
    pub fn drop_column(&mut self, path: String) {
        self.ops.push(SchemaOp::DropColumn { path });
    }

    /// Record a rename of the column at `path`; its identifier is kept.
    #[napi]
    pub fn rename_column(&mut self, path: String, name: String) {
        self.ops.push(SchemaOp::RenameColumn { path, name });
    }

    /// Record a new `iceberg:doc` documentation string on the column at `path`.
    #[napi]
    pub fn update_doc(&mut self, path: String, doc: String) {
        self.ops.push(SchemaOp::UpdateDoc { path, doc });
    }

    /// Record that the column at `path` becomes optional.
    #[napi]
    pub fn make_nullable(&mut self, path: String) {
        self.ops.push(SchemaOp::MakeNullable { path });
    }

    /// Record a type promotion on the column at `path`.
    #[napi]
    pub fn update_type(&mut self, path: String, data_type: DataTypeInput<'_>) -> Result<()> {
        let data_type = data_type_from_input(data_type)?;
        self.ops.push(SchemaOp::UpdateType { path, data_type });
        Ok(())
    }
}

/// What one predicate let a scan leave alone.
#[napi(object)]
pub struct ScanPlanCounts {
    /// Data files the scan will open.
    pub tasks: i64,
    /// Live data files the metadata excluded.
    pub files_skipped: i64,
    /// Manifests that had to be read.
    pub manifests_read: i64,
    /// Manifests excluded on their summary alone, never opened.
    pub manifests_skipped: i64,
    /// Rows the planned files hold, as the manifests counted them.
    pub record_count: i64,
}

/// A warehouse folder of namespaces of Iceberg tables.
///
/// The catalog is storage and nothing else: a dotted name like `"nyc.taxis"`
/// names the folder `nyc/taxis` under the warehouse handle, and constructing
/// one touches nothing at all. There is no service in between, so two catalogs
/// over the same folder see the same tables.
#[napi(js_name = "Catalog")]
pub struct JsCatalog {
    inner: CoreCatalog<Holder>,
}

#[napi]
impl JsCatalog {
    /// Describe a catalog over a warehouse folder, touching nothing.
    ///
    /// `warehouse` accepts whatever names a location - a path or URL string, a
    /// native `Url`, or a handle - the same inputs `Table.create`'s root takes.
    #[napi(constructor)]
    pub fn new(warehouse: LocationInput<'_>) -> Result<Self> {
        Ok(Self {
            inner: CoreCatalog::new(folder_from_input(warehouse)?),
        })
    }

    /// Create the named table, writing its first metadata document.
    ///
    /// `schema` is a root `Field`, a field expression, or an array of child
    /// `Field`s assembled under a root named `row`. Unnumbered columns are
    /// numbered, and the partition spec is derived from the columns the schema
    /// itself marks - a schema that marks none produces an unpartitioned
    /// table.
    #[napi]
    pub fn create_table(&self, name: String, schema: TableSchemaInput<'_>) -> Result<JsTable> {
        self.inner
            .create_table(&name, schema_from_input(schema)?)
            .map(JsTable::from_core)
            .map_err(napi_error)
    }

    /// Open the named table.
    #[napi]
    pub fn table(&self, name: String) -> Result<JsTable> {
        self.inner
            .table(&name)
            .map(JsTable::from_core)
            .map_err(napi_error)
    }

    /// Return whether the named table exists.
    #[napi]
    pub fn has_table(&self, name: String) -> Result<bool> {
        self.inner.has_table(&name).map_err(napi_error)
    }

    /// Open the named table if it exists, creating it otherwise.
    ///
    /// An existing table is opened as it is - `schema` describes only the
    /// table this call would create.
    #[napi]
    pub fn open_or_create_table(
        &self,
        name: String,
        schema: TableSchemaInput<'_>,
    ) -> Result<JsTable> {
        self.inner
            .open_or_create_table(&name, schema_from_input(schema)?)
            .map(JsTable::from_core)
            .map_err(napi_error)
    }

    /// Append `data` to the named table, creating it on first write.
    ///
    /// A table that is not there yet takes its schema from the reader, so a
    /// caller who only has rows and a name needs nothing else. Returns the
    /// table so the caller can keep going.
    #[napi]
    pub fn append(&self, name: String, data: &mut JsBatchReader) -> Result<JsTable> {
        self.inner
            .append(&name, data.take()?)
            .map(JsTable::from_core)
            .map_err(napi_error)
    }

    /// Replace the named table's rows with `data`, creating it on first write.
    ///
    /// An existing table keeps its previous snapshot readable; only the
    /// current pointer moves. Returns the table so the caller can keep going.
    #[napi]
    pub fn overwrite(&self, name: String, data: &mut JsBatchReader) -> Result<JsTable> {
        self.inner
            .overwrite(&name, data.take()?)
            .map(JsTable::from_core)
            .map_err(napi_error)
    }

    /// List the namespaces one level below `parent`, as sorted dotted names.
    ///
    /// Omitting `parent` lists the warehouse's own child folders. A parent
    /// that does not exist lists nothing rather than failing.
    #[napi]
    pub fn list_namespaces(&self, parent: Option<String>) -> Result<Vec<String>> {
        self.inner
            .list_namespaces(parent.as_deref())
            .map_err(napi_error)
    }

    /// List the tables in a namespace, as sorted dotted names.
    ///
    /// A namespace that does not exist lists nothing rather than failing.
    #[napi]
    pub fn list_tables(&self, namespace: String) -> Result<Vec<String>> {
        self.inner.list_tables(&namespace).map_err(napi_error)
    }

    /// One namespace as a view: `catalog.namespace('analytics')`.
    ///
    /// The view exists whether or not the folder does, exactly as a handle
    /// describes a location without proof, so asking for one never fails.
    #[napi]
    pub fn namespace(&self, reference: Reference<JsCatalog>, name: String) -> JsNamespace {
        JsNamespace {
            catalog: reference,
            name,
        }
    }
}

/// One namespace of a catalog: the first half of `catalog[ns][table]`.
///
/// `get` opens a table, `set` gets-or-creates - a schema opens the table,
/// creating it when absent, and rows replace the table's rows, creating it
/// from their own schema on first write - and `has`, `tables`, and
/// `namespaces` answer the map questions.
#[napi(js_name = "Namespace")]
pub struct JsNamespace {
    catalog: Reference<JsCatalog>,
    name: String,
}

#[napi]
impl JsNamespace {
    /// The namespace's dotted name.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.name.clone()
    }

    /// Open the named table.
    #[napi]
    pub fn table(&self, name: String) -> Result<JsTable> {
        self.catalog
            .inner
            .namespace(&self.name)
            .table(&name)
            .map(JsTable::from_core)
            .map_err(napi_error)
    }

    /// Open the named table, as a map reads one.
    #[napi]
    pub fn get(&self, name: String) -> Result<JsTable> {
        self.table(name)
    }

    /// Return whether the named table exists here.
    #[napi]
    pub fn has(&self, name: String) -> Result<bool> {
        self.catalog
            .inner
            .namespace(&self.name)
            .has_table(&name)
            .map_err(napi_error)
    }

    /// Open the named table, creating it with `schema` when absent.
    #[napi]
    pub fn open_or_create_table(&self, name: String, schema: &JsField) -> Result<JsTable> {
        self.catalog
            .inner
            .namespace(&self.name)
            .open_or_create_table(&name, schema.inner.clone())
            .map(JsTable::from_core)
            .map_err(napi_error)
    }

    /// The setter half of the map-like spelling, as raw IPC crossings.
    ///
    /// The loader widens this to `set(name, schemaOrRows)`: a schema opens
    /// the table, creating it when absent; rows replace the table's rows,
    /// creating it from their own schema on first write.
    #[napi(js_name = "_setIpc", skip_typescript)]
    pub fn set_ipc(&self, name: String, rows: Buffer) -> Result<JsTable> {
        let reader = crate::arrow::JsBatchReader::decoded(rows.as_ref(), "row")?.take()?;
        self.catalog
            .inner
            .namespace(&self.name)
            .overwrite(&name, reader)
            .map(JsTable::from_core)
            .map_err(napi_error)
    }

    /// This namespace's tables, as bare names.
    #[napi]
    pub fn tables(&self) -> Result<Vec<String>> {
        self.catalog
            .inner
            .namespace(&self.name)
            .list_tables()
            .map_err(napi_error)
    }

    /// The namespaces one level below this one, as bare names.
    #[napi]
    pub fn namespaces(&self) -> Result<Vec<String>> {
        self.catalog
            .inner
            .namespace(&self.name)
            .list_namespaces()
            .map_err(napi_error)
    }
}

/// Number every column of a schema, so an Iceberg table can carry it.
///
/// Returns a copy: a Field is a value here, and numbering one in place would
/// change a schema another table already holds.
#[napi(js_name = "icebergAssignFieldIdsNative", skip_typescript)]
pub fn iceberg_assign_field_ids(schema: &JsField, start: Option<i32>) -> Result<JsField> {
    let mut schema: CoreField = schema.inner.clone();
    assign_field_ids(&mut schema, start.unwrap_or(1)).map_err(napi_error)?;
    Ok(JsField::from_core(schema))
}

/// Read an Iceberg schema document as a root Field.
#[napi(js_name = "icebergSchemaFromJsonNative", skip_typescript)]
pub fn iceberg_schema_from_json(name: String, document: &JsCodecValue) -> Result<JsField> {
    schema_from_json(&name, &document.inner)
        .map(JsField::from_core)
        .map_err(napi_error)
}

/// Write a root Field as an Iceberg schema document.
#[napi(js_name = "icebergSchemaToJsonNative", skip_typescript)]
pub fn iceberg_schema_to_json(schema: &JsField) -> Result<JsCodecValue> {
    schema_to_json(&schema.inner)
        .map(JsCodecValue::from_core)
        .map_err(napi_error)
}

/// Check one type change against the promotions Iceberg allows.
///
/// Returns nothing on a legal promotion and throws the core message naming
/// both sides for every other change.
#[napi(js_name = "icebergCanPromoteNative", skip_typescript)]
pub fn iceberg_can_promote(from_type: DataTypeInput<'_>, to_type: DataTypeInput<'_>) -> Result<()> {
    can_promote(
        &data_type_from_input(from_type)?,
        &data_type_from_input(to_type)?,
    )
    .map_err(napi_error)
}
