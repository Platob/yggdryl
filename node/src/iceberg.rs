//! Apache Iceberg tables, reached from JavaScript through one handle.
//!
//! A table is a folder: `metadata/` holds the JSON documents and the Avro
//! manifests, `data/` holds the Parquet files, and every one of them is a child
//! of the handle the table was built from. Nothing here opens a path, so the
//! same JavaScript works over a local directory today and over an object store
//! the moment a backend for one exists.

use std::collections::HashMap;

use napi::bindgen_prelude::{
    BigInt, Buffer, ClassInstance, Either, Either3, Env, Reference, Result,
};
use napi_derive::napi;
use yggdryl::generic::Holder;
use yggdryl::iceberg::{
    Catalog as CoreCatalog, Compaction as CoreCompaction, DataFile, FormatVersion,
    IcebergOptions as CoreIcebergOptions, ManifestContent, ManifestFile, Names as CoreNames,
    Namespaces as CoreNamespaces, PartitionField as CorePartitionField,
    PartitionSpec as CorePartitionSpec, ScanPlan as CoreScanPlan, SchemaUpdate as CoreSchemaUpdate,
    Snapshot, SnapshotRef, Table as CoreTable, Tables as CoreTables, assign_field_ids, can_promote,
    last_field_id, schema_from_json, schema_to_json,
};
use yggdryl::{DataType as CoreDataType, Field as CoreField, Scalar as CoreScalar};

use crate::arrow::JsBatchReader;
use crate::codec::JsScalar;
use crate::datatype::{JsDataType, data_type_from_input};
use crate::field::{JsField, MetadataEntry};
use crate::io::{JsIOBase, LocationInput, folder_from_input};
use crate::media::{JsMimeType, MimeTypeInput, mime_type_from_input};
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

/// Normalize an `updateProperties` call's two optional arguments into the
/// pair lists every level of the hierarchy commits.
fn property_changes(
    updates: Option<PropertyUpdates>,
    removes: Option<Vec<String>>,
) -> (Vec<(String, String)>, Vec<String>) {
    let updates = match updates {
        None => Vec::new(),
        Some(Either::A(entries)) => entries
            .into_iter()
            .map(|entry| (entry.key, entry.value))
            .collect(),
        Some(Either::B(values)) => values.into_iter().collect(),
    };
    (updates, removes.unwrap_or_default())
}

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
pub(crate) fn field_from_input(value: FieldInput<'_>) -> Result<CoreField> {
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

/// Borrow owned filter pairs as the `(column, value)` slices the core takes.
fn borrowed_pairs(pairs: &[(String, String)]) -> Vec<(&str, &str)> {
    pairs
        .iter()
        .map(|(column, value)| (column.as_str(), value.as_str()))
        .collect()
}

/// The Iceberg option fields, as one JavaScript options object.
///
/// Every field is optional because an options value records only what was set
/// on it: a field left out is not "the default" but unresolved, and a table
/// still answers it from its own properties. The names are the ones the
/// getters carry, so the object and the setters spell the same ten things.
#[napi(object)]
pub struct IcebergOptionsInput<'env> {
    /// How many beaten commit attempts are retried.
    pub commit_retries: Option<u32>,
    /// The first commit retry wait, in milliseconds.
    pub commit_min_backoff_ms: Option<f64>,
    /// The largest commit retry wait, in milliseconds.
    pub commit_max_backoff_ms: Option<f64>,
    /// The total commit retry-delay budget, in milliseconds.
    pub commit_total_timeout_ms: Option<f64>,
    /// The size a data file aims for, in bytes.
    pub target_file_size: Option<f64>,
    /// How many data files a scan decodes at once.
    pub read_parallelism: Option<u32>,
    /// How many large-enough files justify a parallel scan.
    pub read_parallel_min_files: Option<u32>,
    /// The recorded size below which a file does not count toward that
    /// justification, in bytes.
    pub read_parallel_min_file_size: Option<f64>,
    /// After how many data commits an automatic compaction runs.
    pub compact_after_commits: Option<u32>,
    /// The MIME type for new data files. Table writes encode Parquet and Avro.
    pub data_mime_type: Option<MimeTypeInput<'env>>,
}

/// Apply every field one options object carried, in field order.
///
/// A field the object omits is left exactly as it was, so this is equally the
/// constructor's whole body and a partial update of a value already built.
///
/// # Errors
///
/// Throws the core's typed error for a value it refuses - a zero target size,
/// zero parallelism, or unsupported data MIME type - naming the value.
fn apply_options_input(
    options: &mut CoreIcebergOptions,
    input: IcebergOptionsInput<'_>,
) -> Result<()> {
    if let Some(retries) = input.commit_retries {
        options.set_commit_retries(retries);
    }
    if let Some(wait_ms) = input.commit_min_backoff_ms {
        options.set_commit_min_backoff_ms(crate::exact_u64(wait_ms, "commitMinBackoffMs")?);
    }
    if let Some(wait_ms) = input.commit_max_backoff_ms {
        options.set_commit_max_backoff_ms(crate::exact_u64(wait_ms, "commitMaxBackoffMs")?);
    }
    if let Some(timeout_ms) = input.commit_total_timeout_ms {
        options.set_commit_total_timeout_ms(crate::exact_u64(timeout_ms, "commitTotalTimeoutMs")?);
    }
    if let Some(bytes) = input.target_file_size {
        options
            .set_target_file_size_bytes(crate::exact_u64(bytes, "targetFileSize")?)
            .map_err(napi_error)?;
    }
    if let Some(threads) = input.read_parallelism {
        options
            .set_read_parallelism(threads as usize)
            .map_err(napi_error)?;
    }
    if let Some(files) = input.read_parallel_min_files {
        options.set_read_parallel_min_files(files as usize);
    }
    if let Some(bytes) = input.read_parallel_min_file_size {
        options.set_read_parallel_min_file_size_bytes(crate::exact_u64(
            bytes,
            "readParallelMinFileSize",
        )?);
    }
    if let Some(commits) = input.compact_after_commits {
        options.set_compact_after_commits(commits);
    }
    if let Some(mime_type) = input.data_mime_type {
        options
            .set_data_mime_type(mime_type_from_input(mime_type)?)
            .map_err(napi_error)?;
    }
    Ok(())
}

/// Configuration for one table's commits, writes, and reads.
///
/// The value records only what was set on it: every getter answers the field's
/// documented default when nothing was, and a table resolves each field as
/// explicit option, then table property, then that default. That three-layer
/// resolution is why an unset field is not the same as a field set to the
/// default - only the second one shadows the table's own property.
#[napi(js_name = "IcebergOptions")]
#[derive(Clone, Default)]
pub struct JsIcebergOptions {
    pub(crate) inner: CoreIcebergOptions,
}

// Byte counts and thread counts cross as JavaScript numbers, exact to 2^53 -
// the same contract `IOBase.size` already publishes - so the casts here are
// the boundary, not a loss.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_lossless
)]
#[napi]
impl JsIcebergOptions {
    /// Build an options value from the fields an object names, or an empty one.
    ///
    /// # Errors
    ///
    /// Throws the core's typed error for a value it refuses, naming it.
    #[napi(constructor)]
    pub fn new(options: Option<IcebergOptionsInput<'_>>) -> Result<Self> {
        let mut inner = CoreIcebergOptions::new();
        if let Some(input) = options {
            apply_options_input(&mut inner, input)?;
        }
        Ok(Self { inner })
    }

    /// How many beaten commit attempts are retried. Default: 4.
    #[napi(getter)]
    pub fn commit_retries(&self) -> u32 {
        self.inner.commit_retries()
    }

    /// Set how many beaten commit attempts are retried.
    #[napi(setter)]
    pub fn set_commit_retries(&mut self, retries: u32) {
        self.inner.set_commit_retries(retries);
    }

    /// The first commit retry wait, in milliseconds. Default: 100.
    #[napi(getter)]
    pub fn commit_min_backoff_ms(&self) -> Result<f64> {
        crate::exact_f64(self.inner.commit_min_backoff_ms(), "commitMinBackoffMs")
    }

    /// Set the first commit retry wait, in milliseconds.
    ///
    /// # Errors
    ///
    /// Throws when the wait is not a whole non-negative number of at most 2^53.
    #[napi(setter)]
    pub fn set_commit_min_backoff_ms(&mut self, wait_ms: f64) -> Result<()> {
        self.inner
            .set_commit_min_backoff_ms(crate::exact_u64(wait_ms, "commitMinBackoffMs")?);
        Ok(())
    }

    /// The largest commit retry wait, in milliseconds. Default: 60000.
    #[napi(getter)]
    pub fn commit_max_backoff_ms(&self) -> Result<f64> {
        crate::exact_f64(self.inner.commit_max_backoff_ms(), "commitMaxBackoffMs")
    }

    /// Set the largest commit retry wait, in milliseconds.
    ///
    /// # Errors
    ///
    /// Throws when the wait is not a whole non-negative number of at most 2^53.
    #[napi(setter)]
    pub fn set_commit_max_backoff_ms(&mut self, wait_ms: f64) -> Result<()> {
        self.inner
            .set_commit_max_backoff_ms(crate::exact_u64(wait_ms, "commitMaxBackoffMs")?);
        Ok(())
    }

    /// The total commit retry-delay budget, in milliseconds. Default: 1800000.
    #[napi(getter)]
    pub fn commit_total_timeout_ms(&self) -> Result<f64> {
        crate::exact_f64(self.inner.commit_total_timeout_ms(), "commitTotalTimeoutMs")
    }

    /// Set the total commit retry-delay budget, in milliseconds.
    ///
    /// # Errors
    ///
    /// Throws when the timeout is not a whole non-negative number of at most 2^53.
    #[napi(setter)]
    pub fn set_commit_total_timeout_ms(&mut self, timeout_ms: f64) -> Result<()> {
        self.inner
            .set_commit_total_timeout_ms(crate::exact_u64(timeout_ms, "commitTotalTimeoutMs")?);
        Ok(())
    }

    /// The size a data file aims for, in bytes. Default: 512 MiB.
    #[napi(getter)]
    pub fn target_file_size(&self) -> Result<f64> {
        crate::exact_f64(self.inner.target_file_size_bytes(), "targetFileSize")
    }

    /// Set the size a data file aims for, in bytes.
    ///
    /// # Errors
    ///
    /// Throws the core's typed error naming the value when the size is zero: a
    /// target no file can meet would roll one file per batch forever, so it is
    /// refused here rather than obeyed later.
    #[napi(setter)]
    pub fn set_target_file_size(&mut self, bytes: f64) -> Result<()> {
        self.inner
            .set_target_file_size_bytes(crate::exact_u64(bytes, "targetFileSize")?)
            .map_err(napi_error)
    }

    /// How many data files a scan decodes at once. Default: the host's own
    /// parallelism, kept in 1..=8.
    #[napi(getter)]
    pub fn read_parallelism(&self) -> Result<u32> {
        u32::try_from(self.inner.read_parallelism())
            .map_err(|_| napi::Error::from_reason("readParallelism exceeds a JavaScript u32"))
    }

    /// Set how many data files a scan decodes at once.
    ///
    /// # Errors
    ///
    /// Throws the core's typed error naming the value when the count is zero,
    /// which would read nothing at all.
    #[napi(setter)]
    pub fn set_read_parallelism(&mut self, threads: u32) -> Result<()> {
        self.inner
            .set_read_parallelism(threads as usize)
            .map_err(napi_error)
    }

    /// How many large-enough files justify a parallel scan. Default: 16.
    #[napi(getter)]
    pub fn read_parallel_min_files(&self) -> Result<u32> {
        u32::try_from(self.inner.read_parallel_min_files())
            .map_err(|_| napi::Error::from_reason("readParallelMinFiles exceeds a JavaScript u32"))
    }

    /// Set how many large-enough files justify a parallel scan.
    #[napi(setter)]
    pub fn set_read_parallel_min_files(&mut self, files: u32) {
        self.inner.set_read_parallel_min_files(files as usize);
    }

    /// The recorded size below which a file does not count toward justifying a
    /// parallel scan, in bytes. Default: 4 MiB.
    #[napi(getter)]
    pub fn read_parallel_min_file_size(&self) -> Result<f64> {
        crate::exact_f64(
            self.inner.read_parallel_min_file_size_bytes(),
            "readParallelMinFileSize",
        )
    }

    /// Set the size below which a file does not count toward justifying a
    /// parallel scan, in bytes.
    ///
    /// # Errors
    ///
    /// Throws when the size is not a whole non-negative number of at most 2^53.
    #[napi(setter)]
    pub fn set_read_parallel_min_file_size(&mut self, bytes: f64) -> Result<()> {
        self.inner
            .set_read_parallel_min_file_size_bytes(crate::exact_u64(
                bytes,
                "readParallelMinFileSize",
            )?);
        Ok(())
    }

    /// After how many data commits an automatic compaction runs; `null` - the
    /// default - never compacts on its own, and 0 reads as off.
    #[napi(getter)]
    pub fn compact_after_commits(&self) -> Option<u32> {
        self.inner.compact_after_commits()
    }

    /// Set after how many data commits an automatic compaction runs.
    #[napi(setter)]
    pub fn set_compact_after_commits(&mut self, commits: u32) {
        self.inner.set_compact_after_commits(commits);
    }

    /// The MIME type for new data files. Default: `MimeType.PARQUET`.
    ///
    /// Only what a write produces is decided here: a scan decodes each data
    /// file as the format its manifest entry records, so one table can mix
    /// formats and still read as one shape.
    #[napi(getter)]
    pub fn data_mime_type(&self) -> JsMimeType {
        JsMimeType::from_core(self.inner.data_mime_type())
    }

    /// Set the MIME type for new data files from a native value or parser input.
    ///
    /// # Errors
    ///
    /// Throws the core message naming the accepted formats and the input.
    #[napi(setter)]
    pub fn set_data_mime_type(&mut self, mime_type: MimeTypeInput<'_>) -> Result<()> {
        self.inner
            .set_data_mime_type(mime_type_from_input(mime_type)?)
            .map_err(napi_error)
    }

    /// Return whether every explicitly configured option is equal.
    #[napi]
    pub fn equals(&self, other: &JsIcebergOptions) -> bool {
        self.inner == other.inner
    }

    /// Compare the complete explicit configurations in the core's order.
    #[napi]
    pub fn compare(&self, other: &JsIcebergOptions) -> i32 {
        crate::ordering_value(self.inner.cmp(&other.inner))
    }

    /// Return deterministic hash bits for every explicitly configured option.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a detached copy of the current explicit configuration.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }
}

/// Run one table operation under per-call options, restoring the handle after.
///
/// The override is shadowed for exactly the length of the call - saved before,
/// put back after, whatever the operation did - so per-call options never leak
/// into the handle's own configuration.
fn with_call_options<R>(
    table: &mut CoreTable<Holder>,
    options: Option<CoreIcebergOptions>,
    operation: impl FnOnce(&mut CoreTable<Holder>) -> Result<R>,
) -> Result<R> {
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

/// Read the per-call options an optional argument carried.
fn call_options(options: Option<&JsIcebergOptions>) -> Option<CoreIcebergOptions> {
    options.map(|options| options.inner.clone())
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
#[napi(js_name = "PartitionField")]
#[derive(Clone)]
pub struct JsPartitionField {
    inner: CorePartitionField,
}

impl JsPartitionField {
    fn from_core(inner: CorePartitionField) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsPartitionField {
    /// Identifier of the schema column the value is derived from.
    #[napi(getter)]
    pub const fn source_id(&self) -> i32 {
        self.inner.source_id
    }

    /// Identifier of the partition field itself.
    #[napi(getter)]
    pub const fn field_id(&self) -> i32 {
        self.inner.field_id
    }

    /// The directory name this field writes.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name.to_string()
    }

    /// The transform applied to the source column.
    #[napi(getter)]
    pub fn transform(&self) -> String {
        self.inner.transform.to_string()
    }

    /// Return whether the complete core partition fields are equal.
    #[napi]
    pub fn equals(&self, other: &JsPartitionField) -> bool {
        self.inner == other.inner
    }

    /// Compare complete partition fields in the core's order.
    #[napi]
    pub fn compare(&self, other: &JsPartitionField) -> i32 {
        crate::ordering_value(self.inner.cmp(&other.inner))
    }

    /// Return deterministic hash bits for the complete partition field.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a detached clone of this immutable partition field.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }
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
#[napi(js_name = "Snapshot")]
#[derive(Clone)]
pub struct JsSnapshot {
    inner: Snapshot,
}

impl JsSnapshot {
    fn from_core(inner: Snapshot) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsSnapshot {
    /// Identifier of this snapshot, unique within the table.
    #[napi(getter)]
    pub fn snapshot_id(&self) -> BigInt {
        BigInt::from(self.inner.snapshot_id)
    }

    /// The snapshot this one was produced from, when there was one.
    #[napi(getter)]
    pub fn parent_snapshot_id(&self) -> Option<BigInt> {
        self.inner.parent_snapshot_id.map(BigInt::from)
    }

    /// Monotonic commit order, absent in v1 tables.
    #[napi(getter)]
    pub const fn sequence_number(&self) -> Option<i64> {
        self.inner.sequence_number
    }

    /// Wall-clock commit time in milliseconds since the Unix epoch.
    #[napi(getter)]
    pub const fn timestamp_ms(&self) -> i64 {
        self.inner.timestamp_ms
    }

    /// Location of the Avro manifest list this snapshot's manifests are in.
    #[napi(getter)]
    pub fn manifest_list(&self) -> String {
        self.inner.manifest_list.to_string()
    }

    /// Direct manifest locations carried by a v1 snapshot.
    #[napi(getter)]
    pub fn manifests(&self) -> Option<Vec<String>> {
        self.inner
            .manifests
            .as_ref()
            .map(|paths| paths.iter().map(ToString::to_string).collect::<Vec<_>>())
    }

    /// What the commit did, defaulting to `append`.
    #[napi(getter)]
    pub fn operation(&self) -> String {
        self.inner.operation().to_owned()
    }

    /// The commit summary, keyed by Iceberg's summary vocabulary.
    #[napi(getter)]
    pub fn summary(&self) -> HashMap<String, String> {
        self.inner
            .summary
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    /// The schema in effect when the snapshot was written.
    #[napi(getter)]
    pub const fn schema_id(&self) -> Option<i32> {
        self.inner.schema_id
    }

    /// V3 encryption key used by this snapshot, when encrypted.
    #[napi(getter)]
    pub fn encryption_key_id(&self) -> Option<String> {
        self.inner
            .encryption_key_id
            .as_ref()
            .map(ToString::to_string)
    }

    /// First row id assigned by a v3 snapshot, when row lineage is present.
    #[napi(getter)]
    pub fn first_row_id(&self) -> Option<BigInt> {
        self.inner.first_row_id.map(BigInt::from)
    }

    /// Rows this v3 snapshot added, when row lineage is present.
    #[napi(getter)]
    pub const fn added_rows(&self) -> Option<i64> {
        self.inner.added_rows
    }

    /// Return whether the complete core snapshots are equal.
    #[napi]
    pub fn equals(&self, other: &JsSnapshot) -> bool {
        self.inner == other.inner
    }

    /// Compare complete snapshots in the core's structural order.
    #[napi]
    pub fn compare(&self, other: &JsSnapshot) -> i32 {
        crate::ordering_value(self.inner.cmp(&other.inner))
    }

    /// Return deterministic hash bits for the complete snapshot.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a detached clone of this immutable snapshot.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }
}

/// One branch or tag, as the metadata records it.
///
/// A branch moves as commits land on it and a tag does not, which is the whole
/// of the difference: both are a name pointing at one retained snapshot, and
/// the retention fields are what expiration consults before dropping it.
#[napi(js_name = "SnapshotRef")]
#[derive(Clone)]
pub struct JsSnapshotRef {
    inner: SnapshotRef,
}

impl JsSnapshotRef {
    fn from_core(inner: SnapshotRef) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsSnapshotRef {
    /// The snapshot this reference names.
    #[napi(getter)]
    pub fn snapshot_id(&self) -> BigInt {
        BigInt::from(self.inner.snapshot_id)
    }

    /// Either `branch` or `tag`.
    #[napi(getter)]
    pub fn kind(&self) -> String {
        self.inner.kind.to_string()
    }

    /// Fewest snapshots expiration keeps on this branch, head included.
    #[napi(getter)]
    pub const fn min_snapshots_to_keep(&self) -> Option<i32> {
        self.inner.min_snapshots_to_keep
    }

    /// Oldest ancestor age expiration keeps on this branch, in milliseconds.
    #[napi(getter)]
    pub const fn max_snapshot_age_ms(&self) -> Option<i64> {
        self.inner.max_snapshot_age_ms
    }

    /// Age at which the reference itself expires, in milliseconds from its
    /// snapshot's commit time.
    #[napi(getter)]
    pub const fn max_ref_age_ms(&self) -> Option<i64> {
        self.inner.max_ref_age_ms
    }

    /// Return whether the complete core snapshot references are equal.
    #[napi]
    pub fn equals(&self, other: &JsSnapshotRef) -> bool {
        self.inner == other.inner
    }

    /// Compare complete snapshot references in the core's order.
    #[napi]
    pub fn compare(&self, other: &JsSnapshotRef) -> i32 {
        crate::ordering_value(self.inner.cmp(&other.inner))
    }

    /// Return deterministic hash bits for the complete snapshot reference.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a detached clone of this immutable snapshot reference.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }
}

/// One manifest of the current snapshot.
#[napi(js_name = "ManifestFile")]
#[derive(Clone)]
pub struct JsManifestFile {
    inner: ManifestFile,
}

impl JsManifestFile {
    fn from_core(inner: ManifestFile) -> Self {
        Self { inner }
    }
}

/// One partition-field summary carried by a manifest-list row.
#[napi(object)]
pub struct FieldSummaryView {
    /// Whether any file in the manifest has a null partition value.
    pub contains_null: bool,
    /// Whether any file has a NaN value, when the writer knew.
    pub contains_nan: Option<bool>,
    /// Serialized minimum across the manifest's files.
    pub lower_bound: Option<Buffer>,
    /// Serialized maximum across the manifest's files.
    pub upper_bound: Option<Buffer>,
}

#[napi]
impl JsManifestFile {
    /// The manifest's location, as a URI.
    #[napi(getter)]
    pub fn manifest_path(&self) -> String {
        self.inner.manifest_path.to_string()
    }

    /// Size of the manifest in bytes.
    #[napi(getter)]
    pub const fn manifest_length(&self) -> i64 {
        self.inner.manifest_length
    }

    /// The partition spec the manifest's entries were written under.
    #[napi(getter)]
    pub const fn partition_spec_id(&self) -> i32 {
        self.inner.partition_spec_id
    }

    /// Whether the manifest lists `data` files or `deletes`.
    #[napi(getter)]
    pub fn content(&self) -> String {
        manifest_content(self.inner.content)
    }

    /// Commit order assigned when the manifest was added.
    #[napi(getter)]
    pub const fn sequence_number(&self) -> i64 {
        self.inner.sequence_number
    }

    /// Lowest commit order of any entry in the manifest.
    #[napi(getter)]
    pub const fn min_sequence_number(&self) -> i64 {
        self.inner.min_sequence_number
    }

    /// The snapshot that added the manifest.
    #[napi(getter)]
    pub fn added_snapshot_id(&self) -> BigInt {
        BigInt::from(self.inner.added_snapshot_id)
    }

    /// Files the manifest marks added.
    #[napi(getter)]
    pub const fn added_files_count(&self) -> Option<i32> {
        self.inner.added_files_count
    }

    /// Files the manifest marks existing.
    #[napi(getter)]
    pub const fn existing_files_count(&self) -> Option<i32> {
        self.inner.existing_files_count
    }

    /// Files the manifest marks deleted.
    #[napi(getter)]
    pub const fn deleted_files_count(&self) -> Option<i32> {
        self.inner.deleted_files_count
    }

    /// Rows in the added files.
    #[napi(getter)]
    pub const fn added_rows_count(&self) -> Option<i64> {
        self.inner.added_rows_count
    }

    /// Rows in the existing files.
    #[napi(getter)]
    pub const fn existing_rows_count(&self) -> Option<i64> {
        self.inner.existing_rows_count
    }

    /// Rows in the deleted files.
    #[napi(getter)]
    pub const fn deleted_rows_count(&self) -> Option<i64> {
        self.inner.deleted_rows_count
    }

    /// Partition summaries in the partition spec's field order.
    #[napi(getter)]
    pub fn partitions(&self) -> Vec<FieldSummaryView> {
        self.inner
            .partitions
            .iter()
            .map(|summary| FieldSummaryView {
                contains_null: summary.contains_null,
                contains_nan: summary.contains_nan,
                lower_bound: summary.lower_bound.clone().map(Into::into),
                upper_bound: summary.upper_bound.clone().map(Into::into),
            })
            .collect()
    }

    /// Implementation-specific encryption metadata for the manifest file.
    #[napi(getter)]
    pub fn key_metadata(&self) -> Option<Buffer> {
        self.inner.key_metadata.clone().map(Into::into)
    }

    /// First row id assigned by a v3 manifest, when row lineage is present.
    #[napi(getter)]
    pub fn first_row_id(&self) -> Option<BigInt> {
        self.inner.first_row_id.map(BigInt::from)
    }

    /// Return whether the complete core manifest-list rows are equal.
    #[napi]
    pub fn equals(&self, other: &JsManifestFile) -> bool {
        self.inner == other.inner
    }

    /// Compare complete manifest-list rows in the core's order.
    #[napi]
    pub fn compare(&self, other: &JsManifestFile) -> i32 {
        crate::ordering_value(self.inner.cmp(&other.inner))
    }

    /// Return deterministic hash bits for the complete manifest-list row.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a detached clone of this immutable manifest-list row.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }
}

/// A bounded five-count report of what a scan decided before reading rows.
///
/// The core plan holds the data files themselves because a write needs them;
/// this view keeps only the counts because callers use a plan to check what
/// metadata pruning accomplished. Those five public counts are its complete
/// value identity, independent of the hidden paths and tasks that produced it.
#[napi(js_name = "ScanPlan")]
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct JsScanPlan {
    /// The rows the planned files hold, as the manifests counted them.
    record_count: i64,
    /// The data files the scan will open.
    files_planned: usize,
    /// Live files a read manifest listed that the filters excluded.
    files_skipped: usize,
    /// Manifests opened because their summaries allowed a match.
    manifests_read: usize,
    /// Manifests excluded on their summary alone, never opened.
    manifests_skipped: usize,
}

impl JsScanPlan {
    fn from_core(plan: CoreScanPlan) -> yggdryl::Result<Self> {
        Ok(Self {
            record_count: plan.record_count()?,
            files_planned: plan.tasks.len(),
            files_skipped: plan.files_skipped(),
            manifests_read: plan.manifests_read,
            manifests_skipped: plan.manifests_skipped(),
        })
    }

    fn identity_value(&self) -> CoreScalar {
        CoreScalar::from_sequence([
            CoreScalar::from(self.record_count),
            CoreScalar::from(u64::try_from(self.files_planned).unwrap_or(u64::MAX)),
            CoreScalar::from(u64::try_from(self.files_skipped).unwrap_or(u64::MAX)),
            CoreScalar::from(u64::try_from(self.manifests_read).unwrap_or(u64::MAX)),
            CoreScalar::from(u64::try_from(self.manifests_skipped).unwrap_or(u64::MAX)),
        ])
    }
}

// Counts cross as JavaScript numbers, exact to 2^53 - the same contract
// `IOBase.size` already publishes.
#[allow(clippy::cast_precision_loss)]
#[napi]
impl JsScanPlan {
    /// The rows the planned files hold, as their manifest entries record them.
    ///
    /// This is metadata arithmetic and never a read, so it answers for a table
    /// of any size in the time it takes to walk the manifests.
    #[napi(getter)]
    pub fn record_count(&self) -> i64 {
        self.record_count
    }

    /// How many data files the read would open.
    #[napi(getter)]
    pub fn files_planned(&self) -> f64 {
        self.files_planned as f64
    }

    /// How many data files the partition tuples and statistics excluded.
    #[napi(getter)]
    pub fn files_skipped(&self) -> f64 {
        self.files_skipped as f64
    }

    /// How many manifests had to be decoded to decide all of that.
    #[napi(getter)]
    pub fn manifests_read(&self) -> f64 {
        self.manifests_read as f64
    }

    /// How many manifests the manifest list's own summaries ruled out whole.
    ///
    /// A manifest skipped here is one that was never even read, which is the
    /// coarsest of the three levels of pruning and the cheapest.
    #[napi(getter)]
    pub fn manifests_skipped(&self) -> f64 {
        self.manifests_skipped as f64
    }

    /// Return whether all five public counts are equal.
    #[napi]
    pub fn equals(&self, other: &JsScanPlan) -> bool {
        self == other
    }

    /// Compare the five-count reports in their documented field order.
    #[napi]
    pub fn compare(&self, other: &JsScanPlan) -> i32 {
        crate::ordering_value(self.cmp(other))
    }

    /// Return deterministic hash bits for the complete five-count report.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.identity_value().stable_hash()
    }

    /// Make a detached copy of this immutable count report.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }
}

/// One live data file of the current snapshot, with the spec that placed it.
///
/// This is a class rather than a plain object because a partition value crosses
/// as the native [`Scalar`](crate::codec::JsScalar) the manifest recorded.
/// Rendering it as text here would have to spell a null `null`, which is exactly
/// what makes a directory name unable to answer the question.
#[napi(js_name = "DataFile")]
#[derive(Clone)]
pub struct JsDataFile {
    /// The manifest's record of the file.
    file: DataFile,
    /// Projection context naming the partition tuple's positions.
    ///
    /// This is not part of `DataFile` identity; a clone retains it so the
    /// `partitionNames` view stays identical.
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

    /// The file's generic MIME type.
    #[napi(getter)]
    pub fn mime_type(&self) -> JsMimeType {
        JsMimeType::from_core(self.file.mime_type.clone())
    }

    /// The partition tuple the manifest records, in spec order.
    #[napi(getter)]
    pub fn partition(&self) -> Vec<JsScalar> {
        self.file
            .partition
            .iter()
            .map(|value| JsScalar::from_core(value.clone()))
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

    /// Not-a-number values per column.
    #[napi(getter)]
    pub fn nan_value_counts(&self) -> Vec<FieldCount> {
        counts(&self.file.nan_value_counts)
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

    /// Implementation-specific encryption key metadata.
    #[napi(getter)]
    pub fn key_metadata(&self) -> Option<Buffer> {
        self.file.key_metadata.clone().map(Into::into)
    }

    /// Byte offsets a reader may split the file at.
    #[napi(getter)]
    pub fn split_offsets(&self) -> Vec<i64> {
        self.file.split_offsets.clone()
    }

    /// Field identifiers used by an equality-delete file.
    #[napi(getter)]
    pub fn equality_ids(&self) -> Option<Vec<i32>> {
        self.file.equality_ids.clone()
    }

    /// The sort order the file was written in, when one applies.
    #[napi(getter)]
    pub const fn sort_order_id(&self) -> Option<i32> {
        self.file.sort_order_id
    }

    /// First row identifier assigned to this v3 data file.
    #[napi(getter)]
    pub fn first_row_id(&self) -> Option<BigInt> {
        self.file.first_row_id.map(BigInt::from)
    }

    /// Data file referenced by position-delete metadata.
    #[napi(getter)]
    pub fn referenced_data_file(&self) -> Option<String> {
        self.file
            .referenced_data_file
            .as_ref()
            .map(ToString::to_string)
    }

    /// Byte offset of referenced v3 content.
    #[napi(getter)]
    pub const fn content_offset(&self) -> Option<i64> {
        self.file.content_offset
    }

    /// Byte length of referenced v3 content.
    #[napi(getter)]
    pub const fn content_size_in_bytes(&self) -> Option<i64> {
        self.file.content_size_in_bytes
    }

    /// Return the file's location, so a data file prints as where it is.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.file_path()
    }

    /// Return whether two views carry the same complete core data file.
    #[napi]
    pub fn equals(&self, other: &JsDataFile) -> bool {
        self.file == other.file
    }

    /// Compare two data files by the core's complete structural order.
    #[napi]
    pub fn compare(&self, other: &JsDataFile) -> i32 {
        crate::ordering_value(self.file.cmp(&other.file))
    }

    /// Return deterministic hash bits for the complete core data file.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.file.stable_hash()
    }

    /// Make a cheap detached clone of this immutable manifest file view.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
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

fn snapshot_ref_view(reference: SnapshotRef) -> JsSnapshotRef {
    JsSnapshotRef::from_core(reference)
}

fn snapshot_view(snapshot: &Snapshot) -> JsSnapshot {
    JsSnapshot::from_core(snapshot.clone())
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

fn manifest_view(manifest: &ManifestFile) -> JsManifestFile {
    JsManifestFile::from_core(manifest.clone())
}

/// How a table turns column values into the directories it writes.
#[napi(js_name = "PartitionSpec")]
#[derive(Clone)]
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
    /// Parse one already-inferred native JSON value through the core codec.
    #[napi(factory, js_name = "_fromScalarNative", skip_typescript)]
    pub fn from_scalar_native(value: &JsScalar) -> Result<Self> {
        CorePartitionSpec::from_json(&value.inner)
            .map(Self::from_core)
            .map_err(napi_error)
    }

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
    pub fn fields(&self) -> Vec<JsPartitionField> {
        self.inner
            .fields
            .iter()
            .cloned()
            .map(JsPartitionField::from_core)
            .collect()
    }

    /// Return whether this spec writes every row into one place.
    #[napi]
    pub fn is_unpartitioned(&self) -> bool {
        self.inner.is_unpartitioned()
    }

    /// Project the core's v2 document through the shared native Scalar.
    #[napi(js_name = "_intoScalarNative", skip_typescript)]
    pub fn into_scalar_native(&self) -> Result<JsScalar> {
        self.inner
            .clone()
            .into_json()
            .map(JsScalar::from_core)
            .map_err(napi_error)
    }

    /// Return whether two partition specifications are structurally equal.
    #[napi]
    pub fn equals(&self, other: &JsPartitionSpec) -> bool {
        self.inner == other.inner
    }

    /// Compare two specifications by the core's complete structural order.
    #[napi]
    pub fn compare(&self, other: &JsPartitionSpec) -> i32 {
        crate::ordering_value(self.inner.cmp(&other.inner))
    }

    /// Return deterministic hash bits for the complete specification.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a cheap detached clone of this immutable specification.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
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
    ///
    /// Taken from the table's own root handle rather than from its recorded
    /// location, because a location does not say which backend it belongs to:
    /// a table on a foreign Arrow file system must hand back a folder on that
    /// file system, not the local path its URL happens to spell.
    #[napi(getter)]
    pub fn root(&self) -> Result<JsIOBase> {
        if let Some(holder) = crate::io::arrow_folder_holder(self.inner.root()) {
            return Ok(JsIOBase::from_core(holder));
        }
        JsIOBase::folder_at(&self.location())
    }

    /// The table's base location, as a URI.
    #[napi(getter)]
    pub fn location(&self) -> String {
        self.inner.metadata().location().to_owned()
    }

    /// A stable identifier for the table itself, not for any one version.
    #[napi(getter)]
    pub fn table_uuid(&self) -> String {
        self.inner.metadata().table_uuid().to_owned()
    }

    /// Which revision of the specification the metadata is written to.
    #[napi(getter)]
    pub const fn format_version(&self) -> i32 {
        self.inner.metadata().format_version().number()
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
            .properties()
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
    pub fn spec(&self) -> Result<JsPartitionSpec> {
        self.inner
            .metadata()
            .default_spec()
            .cloned()
            .map(JsPartitionSpec::from_core)
            .map_err(napi_error)
    }

    /// The snapshot a reader sees, or `null` when the table has none.
    ///
    /// A freshly created or rolled-back table has snapshots but no current one,
    /// and reading it yields no rows rather than failing.
    #[napi(getter)]
    pub fn current_snapshot(&self) -> Option<JsSnapshot> {
        self.inner.current_snapshot().map(snapshot_view)
    }

    /// Every schema the table has had, oldest first.
    #[napi(getter)]
    pub fn schemas(&self) -> Vec<JsField> {
        self.inner
            .metadata()
            .schemas()
            .iter()
            .cloned()
            .map(JsField::from_core)
            .collect()
    }

    /// Every retained snapshot, oldest first.
    #[napi(getter)]
    pub fn snapshots(&self) -> Vec<JsSnapshot> {
        self.inner
            .metadata()
            .snapshots()
            .iter()
            .map(snapshot_view)
            .collect()
    }

    /// Every manifest the current snapshot points at.
    #[napi]
    pub fn manifests(&self) -> Result<Vec<JsManifestFile>> {
        Ok(self
            .inner
            .manifests()
            .map_err(napi_error)?
            .iter()
            .map(manifest_view)
            .collect())
    }

    /// Every manifest one retained snapshot points at.
    ///
    /// The manifest half of time travel: what
    /// [`manifests`](Self::manifests) answers for the present, this answers
    /// for any snapshot the table still retains.
    #[napi]
    pub fn manifests_at(&self, snapshot_id: SnapshotIdInput) -> Result<Vec<JsManifestFile>> {
        let snapshot_id = snapshot_id_from_input(snapshot_id)?;
        let metadata = self.inner.metadata();
        let snapshot = metadata.snapshot_by_id(snapshot_id).ok_or_else(|| {
            let retained: Vec<String> = metadata
                .snapshots()
                .iter()
                .map(|snapshot| snapshot.snapshot_id.to_string())
                .collect();
            napi_error(format!(
                "expected a retained snapshot id, got {snapshot_id}; the table retains [{}]",
                retained.join(", ")
            ))
        })?;
        Ok(self
            .inner
            .manifests_at(snapshot)
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
    /// schema evolved readable as one shape. `options` configures this one
    /// call and is put back afterwards, so the handle's own override survives.
    #[napi]
    pub fn scan(
        &mut self,
        field: Option<&JsField>,
        options: Option<&JsIcebergOptions>,
    ) -> Result<JsBatchReader> {
        let root_name = self.root_name()?;
        let field = field.map(|field| field.inner.clone());
        let reader = with_call_options(&mut self.inner, call_options(options), |table| {
            table.scan(field.as_ref()).map_err(napi_error)
        })?;
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
            record_count: plan.record_count().map_err(napi_error)?,
        })
    }

    /// Read the rows matching `filters`, keeping the columns `field` names.
    ///
    /// A filter on a partition column is answered by [`plan`](Self::plan)
    /// alone - every row of a file whose tuple matches holds that value - and a
    /// filter on any other column is applied to the rows the surviving files
    /// hold, because statistics bound a file rather than select a row. Either
    /// way the result is the same rows; what differs is how many files were
    /// opened to find them.
    #[napi]
    pub fn scan_where(
        &mut self,
        filters: ScanFilters,
        field: Option<FieldInput<'_>>,
        options: Option<&JsIcebergOptions>,
    ) -> Result<JsBatchReader> {
        let root_name = self.root_name()?;
        let pairs = filter_pairs(Some(filters));
        let field = field.map(field_from_input).transpose()?;
        let reader = with_call_options(&mut self.inner, call_options(options), |table| {
            table
                .scan_where(&borrowed_pairs(&pairs), field.as_ref())
                .map_err(napi_error)
        })?;
        Ok(JsBatchReader::from_core(reader, &root_name))
    }

    /// Read the rows a branch or tag names, as of the snapshot it points at.
    ///
    /// This is [`snapshotByRef`](Self::snapshot_by_ref) and
    /// [`scanAt`](Self::scan_at) in one call, with the same `filters` and
    /// `field` meanings. A name the table does not have is refused naming the
    /// refs it does.
    #[napi]
    pub fn scan_ref(
        &mut self,
        name: String,
        filters: Option<ScanFilters>,
        field: Option<FieldInput<'_>>,
        options: Option<&JsIcebergOptions>,
    ) -> Result<JsBatchReader> {
        let root_name = self.root_name()?;
        let pairs = filter_pairs(filters);
        let field = field.map(field_from_input).transpose()?;
        let reader = with_call_options(&mut self.inner, call_options(options), |table| {
            table
                .scan_ref(&name, &borrowed_pairs(&pairs), field.as_ref())
                .map_err(napi_error)
        })?;
        Ok(JsBatchReader::from_core(reader, &root_name))
    }

    /// Decide which data files `filters` would have a read open, and no more.
    ///
    /// Nothing here lists a directory and nothing opens a data file: the
    /// snapshot names a manifest list, whose summaries rule out whole
    /// manifests, whose entries carry the partition tuples and statistics that
    /// rule out single files. The returned [`ScanPlan`](JsScanPlan) reports
    /// what it skipped, so how much a filter actually saves is a number rather
    /// than a promise.
    #[napi]
    pub fn plan(&self, filters: Option<ScanFilters>) -> Result<JsScanPlan> {
        let pairs = filter_pairs(filters);
        let plan = self
            .inner
            .plan(&borrowed_pairs(&pairs))
            .map_err(napi_error)?;
        JsScanPlan::from_core(plan).map_err(napi_error)
    }

    /// Plan a scan of one retained snapshot rather than the current one.
    ///
    /// The planning half of time travel: the same three levels of pruning are
    /// walked over the snapshot's own manifest list, so a filtered read of
    /// history skips exactly what a filtered read of the present skips.
    #[napi]
    pub fn plan_at(
        &self,
        snapshot_id: SnapshotIdInput,
        filters: Option<ScanFilters>,
    ) -> Result<JsScanPlan> {
        let snapshot_id = snapshot_id_from_input(snapshot_id)?;
        let pairs = filter_pairs(filters);
        let plan = self
            .inner
            .plan_at(snapshot_id, &borrowed_pairs(&pairs))
            .map_err(napi_error)?;
        JsScanPlan::from_core(plan).map_err(napi_error)
    }

    /// Append `batches` as a new snapshot, keeping everything already stored.
    ///
    /// `options` configures this one write - `targetFileSize`,
    /// `commitRetries`, `dataMimeType`, and the rest - and the handle's own
    /// configuration is untouched.
    #[napi]
    pub fn append(
        &mut self,
        batches: &mut JsBatchReader,
        options: Option<&JsIcebergOptions>,
    ) -> Result<()> {
        let batches = batches.take()?;
        with_call_options(&mut self.inner, call_options(options), |table| {
            table.append(batches).map_err(napi_error)
        })
    }

    /// Replace every row with `batches` as a new snapshot.
    ///
    /// The previous snapshot stays readable; only the current pointer moves.
    /// `options` configures this one write, exactly as on
    /// [`append`](Self::append).
    #[napi]
    pub fn overwrite(
        &mut self,
        batches: &mut JsBatchReader,
        options: Option<&JsIcebergOptions>,
    ) -> Result<()> {
        let batches = batches.take()?;
        with_call_options(&mut self.inner, call_options(options), |table| {
            table.overwrite(batches).map_err(napi_error)
        })
    }

    /// Replace only the rows `filters` selects, keeping every other file.
    ///
    /// A file the filters exclude is carried into the new snapshot exactly as
    /// it is - same location, same statistics, same commit order - so
    /// overwriting one partition of a thousand rewrites one partition.
    ///
    /// An overwrite beaten by a concurrent commit does not rebase: what it
    /// keeps was planned against a snapshot the winner may have replaced, and
    /// `batches` is already consumed, so it throws a commit conflict naming
    /// both versions rather than risk losing rows.
    ///
    /// `options` configures this one write, exactly as on
    /// [`append`](Self::append).
    #[napi]
    pub fn overwrite_where(
        &mut self,
        filters: ScanFilters,
        batches: &mut JsBatchReader,
        options: Option<&JsIcebergOptions>,
    ) -> Result<()> {
        let pairs = filter_pairs(Some(filters));
        let batches = batches.take()?;
        with_call_options(&mut self.inner, call_options(options), |table| {
            table
                .overwrite_where(&borrowed_pairs(&pairs), batches)
                .map_err(napi_error)
        })
    }

    /// Merge `batches` into the stored rows, matching on `mergeByNames`.
    ///
    /// A row whose key is already stored updates it and a row whose key is not
    /// appends. Only the files whose recorded bounds could hold an incoming key
    /// are read and rewritten - the rest are carried into the new snapshot
    /// untouched - so an upsert costs the files it can actually change. A
    /// non-empty `mergeByNames` is required because nothing else identifies a
    /// row.
    ///
    /// `safe` decides what a cast that cannot convert a value does: the
    /// default nulls it, and `false` throws instead. `options` configures this
    /// one write, exactly as on [`append`](Self::append).
    #[napi]
    pub fn merge(
        &mut self,
        batches: &mut JsBatchReader,
        merge_by_names: Vec<String>,
        safe: Option<bool>,
        options: Option<&JsIcebergOptions>,
    ) -> Result<()> {
        let batches = batches.take()?;
        with_call_options(&mut self.inner, call_options(options), |table| {
            table
                .merge(batches, &merge_by_names, safe.unwrap_or(true))
                .map_err(napi_error)
        })
    }

    /// Merge `batches` into the rows `filters` selects, on `mergeByNames`.
    ///
    /// [`merge`](Self::merge) narrowed to a part of the table first: the
    /// filters decide which files are candidates at all, and the match-key
    /// statistics then decide which of those are actually read. `options`
    /// configures this one write, exactly as on [`append`](Self::append).
    #[napi]
    pub fn merge_where(
        &mut self,
        filters: ScanFilters,
        batches: &mut JsBatchReader,
        merge_by_names: Vec<String>,
        safe: Option<bool>,
        options: Option<&JsIcebergOptions>,
    ) -> Result<()> {
        let pairs = filter_pairs(Some(filters));
        let batches = batches.take()?;
        with_call_options(&mut self.inner, call_options(options), |table| {
            table
                .merge_where(
                    &borrowed_pairs(&pairs),
                    batches,
                    &merge_by_names,
                    safe.unwrap_or(true),
                )
                .map_err(napi_error)
        })
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
        &mut self,
        snapshot_id: SnapshotIdInput,
        filters: Option<ScanFilters>,
        schema: Option<FieldInput<'_>>,
        options: Option<&JsIcebergOptions>,
    ) -> Result<JsBatchReader> {
        let root_name = self.root_name()?;
        let snapshot_id = snapshot_id_from_input(snapshot_id)?;
        let pairs = filter_pairs(filters);
        let schema = schema.map(field_from_input).transpose()?;
        let reader = with_call_options(&mut self.inner, call_options(options), |table| {
            table
                .scan_at(snapshot_id, &borrowed_pairs(&pairs), schema.as_ref())
                .map_err(napi_error)
        })?;
        Ok(JsBatchReader::from_core(reader, &root_name))
    }

    /// Return the retained snapshot a branch or tag names.
    ///
    /// A name the table does not have is refused naming the refs it does.
    #[napi]
    pub fn snapshot_by_ref(&self, name: String) -> Result<JsSnapshot> {
        self.inner
            .snapshot_by_ref(&name)
            .map(snapshot_view)
            .map_err(napi_error)
    }

    /// Create a branch at one retained snapshot, as one metadata commit.
    ///
    /// Writing *to* a branch other than `main` remains future work; a branch is
    /// read with [`scanRef`](Self::scan_ref) and moved with
    /// [`fastForward`](Self::fast_forward).
    #[napi]
    pub fn create_branch(&mut self, name: String, snapshot_id: SnapshotIdInput) -> Result<()> {
        let snapshot_id = snapshot_id_from_input(snapshot_id)?;
        self.inner
            .create_branch(&name, snapshot_id)
            .map_err(napi_error)
    }

    /// Create a tag at one retained snapshot, as one metadata commit.
    ///
    /// A tag never moves, so it is what pins a snapshot against expiration.
    #[napi]
    pub fn create_tag(&mut self, name: String, snapshot_id: SnapshotIdInput) -> Result<()> {
        let snapshot_id = snapshot_id_from_input(snapshot_id)?;
        self.inner
            .create_tag(&name, snapshot_id)
            .map_err(napi_error)
    }

    /// Remove one branch or tag, returning what it pointed at.
    ///
    /// A name the table does not have is refused naming the refs it does,
    /// rather than committing nothing: dropping a ref that was never there is
    /// far more often a typo than a no-op.
    #[napi]
    pub fn remove_ref(&mut self, name: String) -> Result<JsSnapshotRef> {
        self.inner
            .remove_ref(&name)
            .map(snapshot_ref_view)
            .map_err(napi_error)
    }

    /// Move a branch forward to a descendant snapshot, as one metadata commit.
    ///
    /// The target must be retained and must reach the branch's head by walking
    /// parent ids, which is what makes a fast-forward unable to lose history.
    #[napi]
    pub fn fast_forward(&mut self, name: String, snapshot_id: SnapshotIdInput) -> Result<()> {
        let snapshot_id = snapshot_id_from_input(snapshot_id)?;
        self.inner
            .fast_forward(&name, snapshot_id)
            .map_err(napi_error)
    }

    /// Expire the snapshots retention no longer keeps, returning their ids.
    ///
    /// Omitted cutoff and retain count use table properties. Explicit snapshot
    /// ids join age-based selection; retained heads cannot be removed.
    /// Statistics metadata is removed, while physical files remain.
    #[napi]
    pub fn expire_snapshots(
        &mut self,
        older_than_ms: Option<f64>,
        retain_last: Option<f64>,
        snapshot_ids: Option<Vec<SnapshotIdInput>>,
    ) -> Result<Vec<BigInt>> {
        let older_than_ms = older_than_ms
            .map(|value| crate::exact_i64(value, "olderThanMs"))
            .transpose()?;
        let retain_last = retain_last
            .map(|value| {
                usize::try_from(crate::exact_u64(value, "retainLast")?)
                    .map_err(|_| napi_error("retainLast exceeds this platform's usize"))
            })
            .transpose()?;
        let snapshot_ids = snapshot_ids
            .unwrap_or_default()
            .into_iter()
            .map(snapshot_id_from_input)
            .collect::<Result<Vec<_>>>()?;
        Ok(self
            .inner
            .expire_snapshots(older_than_ms, retain_last, &snapshot_ids)
            .map_err(napi_error)?
            .into_iter()
            .map(BigInt::from)
            .collect())
    }

    /// Store an explicit options override every later call resolves first.
    ///
    /// A field the override sets shadows the table property of the same name,
    /// and a field it leaves unset still resolves property-then-default. The
    /// override lives on this handle alone - it is never written to the table;
    /// [`updateProperties`](Self::update_properties) is what stores a setting
    /// on the table itself.
    #[napi]
    pub fn set_options(&mut self, options: &JsIcebergOptions) {
        self.inner.set_options(options.inner.clone());
    }

    /// Resolve this table's effective options, field by field.
    ///
    /// Each field takes the nearest of three layers: the explicit override,
    /// then the table property of the same name, then the documented default.
    ///
    /// # Errors
    ///
    /// Throws naming the key and the value when a property no override shadows
    /// is present but does not parse - a configured setting is never silently
    /// replaced by the default.
    #[napi]
    pub fn options(&self) -> Result<JsIcebergOptions> {
        self.inner
            .options()
            .map(|inner| JsIcebergOptions { inner })
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
    pub fn compact(&mut self) -> Result<JsCompaction> {
        self.inner
            .compact()
            .map(JsCompaction::from_core)
            .map_err(napi_error)
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
        let (updates, removes) = property_changes(updates, removes);
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
                    metadata.remove_property(key)?;
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
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.location()
    }
}

/// What one `compact` call rewrote.
///
/// The sizes cross as numbers because a data file already reports
/// `fileSizeInBytes` as one, and the two must agree.
#[napi(js_name = "Compaction")]
#[derive(Clone)]
pub struct JsCompaction {
    inner: CoreCompaction,
}

impl JsCompaction {
    fn from_core(inner: CoreCompaction) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsCompaction {
    /// How many live data files were read and replaced.
    #[napi(getter)]
    pub fn files_before(&self) -> i64 {
        i64::try_from(self.inner.files_before).unwrap_or(i64::MAX)
    }

    /// How many data files the rewrite produced in their place.
    #[napi(getter)]
    pub fn files_after(&self) -> i64 {
        i64::try_from(self.inner.files_after).unwrap_or(i64::MAX)
    }

    /// The recorded size of the replaced files, in bytes.
    #[napi(getter)]
    pub const fn bytes_rewritten(&self) -> i64 {
        self.inner.bytes_rewritten
    }

    /// Return whether the complete core compaction reports are equal.
    #[napi]
    pub fn equals(&self, other: &JsCompaction) -> bool {
        self.inner == other.inner
    }

    /// Compare complete compaction reports in the core's order.
    #[napi]
    pub fn compare(&self, other: &JsCompaction) -> i32 {
        crate::ordering_value(self.inner.cmp(&other.inner))
    }

    /// Return deterministic hash bits for the complete compaction report.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a detached clone of this immutable compaction report.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }
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

    /// Open the table a dotted name addresses - the one-call spelling of
    /// `catalog.tables.get(name)`.
    #[napi]
    pub fn table(&self, name: String) -> Result<JsTable> {
        self.inner
            .table(&name)
            .map(JsTable::from_core)
            .map_err(napi_error)
    }

    /// Append `data` to the named table, creating it on first write.
    ///
    /// A table that is not there yet takes its schema from the reader, so a
    /// caller who only has rows and a name needs nothing else. Returns the
    /// table so the caller can keep going.
    #[napi]
    pub fn append(
        &self,
        name: String,
        data: &mut JsBatchReader,
        options: Option<&JsIcebergOptions>,
    ) -> Result<JsTable> {
        self.inner
            .tables()
            .append_with(&name, data.take()?, call_options(options))
            .map(JsTable::from_core)
            .map_err(napi_error)
    }

    /// Replace the named table's rows with `data`, creating it on first write.
    ///
    /// An existing table keeps its previous snapshot readable; only the
    /// current pointer moves. `options` configures this one write. Returns the
    /// table so the caller can keep going.
    #[napi]
    pub fn overwrite(
        &self,
        name: String,
        data: &mut JsBatchReader,
        options: Option<&JsIcebergOptions>,
    ) -> Result<JsTable> {
        self.inner
            .tables()
            .overwrite_with(&name, data.take()?, call_options(options))
            .map(JsTable::from_core)
            .map_err(napi_error)
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

    /// The catalog's namespaces, as a lazy map-like view.
    ///
    /// Building the view performs no I/O: `get`, `has`, `names`, and `size`
    /// each consult storage at the moment they are asked, which is why two
    /// views over one catalog observe each other's writes and why a view stays
    /// valid across a creation or a deletion. This is the one collection
    /// spelling - `catalog.namespaces.get('sales').tables.get('orders')`
    /// chains all the way to a table.
    #[napi(getter)]
    pub fn namespaces(&self, reference: Reference<JsCatalog>) -> JsNamespaces {
        JsNamespaces {
            catalog: reference,
            parent: None,
        }
    }

    /// The catalog's tables, as the same lazy view over dotted names.
    ///
    /// `catalog.tables.get('sales.eu.orders')` descends; an un-dotted name
    /// addresses a table directly under the warehouse root, and the listing
    /// questions answer exactly those.
    #[napi(getter)]
    pub fn tables(&self, reference: Reference<JsCatalog>) -> JsTables {
        JsTables {
            catalog: reference,
            namespace: None,
        }
    }

    /// The catalog's own properties, from `metadata/catalog.json`.
    ///
    /// Absent means empty - never an error a caller has to catch.
    #[napi]
    pub fn properties(&self) -> Result<HashMap<String, String>> {
        Ok(self
            .inner
            .properties()
            .map_err(napi_error)?
            .iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect())
    }

    /// Set and remove catalog properties as one transactional write.
    ///
    /// `updates` is a mapping of properties to set and `removes` lists the
    /// keys to drop, in that order. Passing neither writes nothing at all.
    /// Keys under the reserved `iceberg:` prefix are refused by name.
    #[napi]
    pub fn update_properties(
        &self,
        updates: Option<PropertyUpdates>,
        removes: Option<Vec<String>>,
    ) -> Result<()> {
        let (updates, removes) = property_changes(updates, removes);
        if updates.is_empty() && removes.is_empty() {
            return Ok(());
        }
        self.inner
            .update_properties(updates, removes)
            .map_err(napi_error)
    }
}

/// One namespace of a catalog: identity, plus its two collection views.
///
/// The namespace holds only its dotted name. Its tables are
/// [`tables`](Self::tables) and its child namespaces are
/// [`namespaces`](Self::namespaces), so access chains -
/// `catalog.namespaces.get('sales').tables.get('orders')` - and every
/// collection question has exactly one home: a namespace is a resource, and
/// the map verbs live on its collections, never on it.
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

    /// This namespace's tables, as a lazy map-like view.
    #[napi(getter)]
    pub fn tables(&self, env: Env) -> Result<JsTables> {
        Ok(JsTables {
            catalog: self.catalog.clone(env)?,
            namespace: Some(self.name.clone()),
        })
    }

    /// The namespaces one level below this one, as the same view shape the
    /// catalog itself answers - the cascade that reaches a nested namespace.
    #[napi(getter)]
    pub fn namespaces(&self, env: Env) -> Result<JsNamespaces> {
        Ok(JsNamespaces {
            catalog: self.catalog.clone(env)?,
            parent: Some(self.name.clone()),
        })
    }

    /// The namespace's properties, from `metadata/namespace.json`.
    ///
    /// Absent means empty - a namespace a table write brought into being
    /// carries no document and answers no properties, and that is not a
    /// failure.
    #[napi]
    pub fn properties(&self) -> Result<HashMap<String, String>> {
        Ok(self
            .catalog
            .inner
            .namespaces()
            .get(&self.name)
            .map_err(napi_error)?
            .properties()
            .map_err(napi_error)?
            .iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect())
    }

    /// Set and remove namespace properties as one transactional write.
    ///
    /// `updates` is a mapping of properties to set and `removes` lists the
    /// keys to drop, in that order. Passing neither writes nothing at all.
    /// Keys under the reserved `iceberg:` prefix are refused by name.
    #[napi]
    pub fn update_properties(
        &self,
        updates: Option<PropertyUpdates>,
        removes: Option<Vec<String>>,
    ) -> Result<()> {
        let (updates, removes) = property_changes(updates, removes);
        if updates.is_empty() && removes.is_empty() {
            return Ok(());
        }
        self.catalog
            .inner
            .namespaces()
            .get(&self.name)
            .map_err(napi_error)?
            .update_properties(updates, removes)
            .map_err(napi_error)
    }
}

/// The namespaces one level below a catalog or a namespace, as a lazy view.
///
/// JavaScript has no indexing hook a native class can answer, so the map
/// questions are spelled out: `get` and `has` for membership, `names` and
/// `size` for the whole collection, `create` and `openOrCreate` to add one.
/// None of it is cached - every answer is storage's, asked when the question
/// is - so a view built before a namespace existed finds it afterwards.
#[napi(js_name = "Namespaces")]
pub struct JsNamespaces {
    catalog: Reference<JsCatalog>,
    /// The parent namespace's dotted name; `null` is the warehouse root.
    parent: Option<String>,
}

impl JsNamespaces {
    /// The root view, which accepts the dotted spelling this value builds -
    /// the resolution rule lives in the core collection, not here.
    fn view(&self) -> CoreNamespaces<'_, Holder> {
        self.catalog.inner.namespaces()
    }

    /// Spell one child's full dotted name.
    fn dotted(&self, name: &str) -> String {
        match &self.parent {
            Some(parent) => format!("{parent}.{name}"),
            None => name.to_owned(),
        }
    }

    /// The names one level down, as the core's lazy iterator.
    fn level(&self) -> Result<CoreNames> {
        level_of(&self.catalog.inner, self.parent.as_deref(), false)
    }

    /// Wrap one child namespace's dotted name as the view of it.
    fn wrap(&self, env: Env, dotted: String) -> Result<JsNamespace> {
        Ok(JsNamespace {
            catalog: self.catalog.clone(env)?,
            name: dotted,
        })
    }
}

/// The names of one level, with an absent parent listing empty.
fn level_of(
    catalog: &CoreCatalog<Holder>,
    parent: Option<&str>,
    tables: bool,
) -> Result<CoreNames> {
    match parent {
        None if tables => Ok(catalog.tables().iter()),
        None => Ok(catalog.namespaces().iter()),
        Some(parent) => match catalog.namespaces().get(parent) {
            Ok(namespace) if tables => Ok(namespace.tables().iter()),
            Ok(namespace) => Ok(namespace.namespaces().iter()),
            // A parent that does not exist lists nothing rather than failing.
            Err(error) if error.is_absent() => Ok(CoreNames::empty()),
            Err(error) => Err(napi_error(error)),
        },
    }
}

// Counts cross as JavaScript numbers, exact to 2^53 - the same contract
// `IOBase.size` already publishes.
#[allow(clippy::cast_precision_loss)]
#[napi]
impl JsNamespaces {
    /// Open the named namespace.
    ///
    /// # Errors
    ///
    /// Throws naming the namespace when nothing is there, or when the name
    /// addresses a table instead - the two ways a chained lookup goes wrong,
    /// told apart rather than collapsed into "not found".
    #[napi]
    pub fn get(&self, env: Env, name: String) -> Result<JsNamespace> {
        let dotted = self
            .view()
            .get(&self.dotted(&name))
            .map_err(napi_error)?
            .name()
            .to_owned();
        self.wrap(env, dotted)
    }

    /// Return whether the named namespace exists, asked of storage now.
    ///
    /// A namespace is a folder that is not a table, so a table's name answers
    /// `false` here, and so does a location nothing occupies yet.
    #[napi]
    pub fn has(&self, name: String) -> Result<bool> {
        self.view()
            .contains(&self.dotted(&name))
            .map_err(napi_error)
    }

    /// The names one level down, lazily - the loader wires `Symbol.iterator`,
    /// `keys`, `values`, and `entries` over this, so `for...of` walks it.
    #[napi]
    pub fn keys(&self) -> Result<JsIcebergNames> {
        Ok(JsIcebergNames {
            names: self.level()?,
        })
    }

    /// The namespaces one level down, as sorted bare names.
    #[napi]
    pub fn names(&self) -> Result<Vec<String>> {
        self.level()?
            .collect::<yggdryl::Result<Vec<_>>>()
            .map_err(napi_error)
    }

    /// How many namespaces are one level down, right now.
    ///
    /// This drains the level's listing, so it costs the full listing.
    #[napi]
    pub fn size(&self) -> Result<f64> {
        let mut count = 0_u64;
        for name in self.level()? {
            name.map_err(napi_error)?;
            count += 1;
        }
        Ok(count as f64)
    }

    /// Create the named namespace, as the folder it is.
    ///
    /// # Errors
    ///
    /// Throws naming the namespace when one - or a table - is already there;
    /// [`openOrCreate`](Self::open_or_create) is the spelling that tolerates it.
    #[napi]
    pub fn create(&self, env: Env, name: String) -> Result<JsNamespace> {
        let dotted = self
            .view()
            .create(&self.dotted(&name))
            .map_err(napi_error)?
            .name()
            .to_owned();
        self.wrap(env, dotted)
    }

    /// Open the named namespace, creating its folder when absent.
    #[napi]
    pub fn open_or_create(&self, env: Env, name: String) -> Result<JsNamespace> {
        let dotted = self
            .view()
            .open_or_create(&self.dotted(&name))
            .map_err(napi_error)?
            .name()
            .to_owned();
        self.wrap(env, dotted)
    }
}

/// The names of one collection level, one at a time.
///
/// Built by `keys()` on `Namespaces` and `Tables`. It wraps the core names
/// iterator directly, so nothing is collected on the way across the boundary;
/// `next()` is the native half of the iteration protocol and the loader wraps
/// it so `for...of` yields strings. A failure throws at the entry it happened
/// on, after which the iterator is exhausted.
#[napi(js_name = "IcebergNames")]
pub struct JsIcebergNames {
    names: CoreNames,
}

#[napi]
impl JsIcebergNames {
    /// The next name, or `null` when the level is exhausted.
    #[napi]
    pub fn next(&mut self) -> Result<Option<String>> {
        self.names.next().transpose().map_err(napi_error)
    }
}

/// The tables of one namespace - or of the warehouse root - as a lazy view.
///
/// The same shape as [`Namespaces`](JsNamespaces), one level down: `get` opens
/// a [`Table`](JsTable) and the write conveniences that take a name create the
/// table on first write, from the incoming rows' own schema. At the root,
/// names may be fully dotted - `catalog.tables.get('sales.eu.orders')`
/// descends. Every answer comes from storage at call time, so the view is
/// never stale.
#[napi(js_name = "Tables")]
pub struct JsTables {
    catalog: Reference<JsCatalog>,
    /// The owning namespace's dotted name; `None` is the warehouse root.
    namespace: Option<String>,
}

impl JsTables {
    /// The root view, which accepts the dotted spelling this value builds.
    fn view(&self) -> CoreTables<'_, Holder> {
        self.catalog.inner.tables()
    }

    /// Spell one table's full dotted name under this namespace.
    fn dotted(&self, name: &str) -> String {
        match &self.namespace {
            Some(namespace) => format!("{namespace}.{name}"),
            None => name.to_owned(),
        }
    }

    /// The table names one level down, as the core's lazy iterator.
    fn level(&self) -> Result<CoreNames> {
        level_of(&self.catalog.inner, self.namespace.as_deref(), true)
    }
}

// Counts cross as JavaScript numbers, exact to 2^53 - the same contract
// `IOBase.size` already publishes.
#[allow(clippy::cast_precision_loss)]
#[napi]
impl JsTables {
    /// Open the named table.
    ///
    /// # Errors
    ///
    /// Throws naming the table when no table is there, and the metadata
    /// failure when its current document cannot be read.
    #[napi]
    pub fn get(&self, name: String) -> Result<JsTable> {
        self.view()
            .get(&self.dotted(&name))
            .map(JsTable::from_core)
            .map_err(napi_error)
    }

    /// Return whether the named table exists, asked of storage now.
    #[napi]
    pub fn has(&self, name: String) -> Result<bool> {
        self.view()
            .contains(&self.dotted(&name))
            .map_err(napi_error)
    }

    /// The names one level down, lazily - the loader wires `Symbol.iterator`,
    /// `keys`, `values`, and `entries` over this, so `for...of` walks it.
    #[napi]
    pub fn keys(&self) -> Result<JsIcebergNames> {
        Ok(JsIcebergNames {
            names: self.level()?,
        })
    }

    /// This namespace's tables, as sorted bare names.
    #[napi]
    pub fn names(&self) -> Result<Vec<String>> {
        self.level()?
            .collect::<yggdryl::Result<Vec<_>>>()
            .map_err(napi_error)
    }

    /// How many tables the namespace holds, right now.
    ///
    /// This drains the level's listing, so it costs the full listing.
    #[napi]
    pub fn size(&self) -> Result<f64> {
        let mut count = 0_u64;
        for name in self.level()? {
            name.map_err(napi_error)?;
            count += 1;
        }
        Ok(count as f64)
    }

    /// Create the named table, writing its first metadata document.
    ///
    /// `schema` is a root `Field`, a field expression, or an array of child
    /// `Field`s assembled under a root named `row`. Unnumbered columns are
    /// numbered, and the partition spec is derived from the columns the schema
    /// itself marks - a schema that marks none produces an unpartitioned table.
    ///
    /// # Errors
    ///
    /// Throws naming the table when one is already there.
    #[napi]
    pub fn create(&self, name: String, schema: TableSchemaInput<'_>) -> Result<JsTable> {
        self.view()
            .create(&self.dotted(&name), schema_from_input(schema)?)
            .map(JsTable::from_core)
            .map_err(napi_error)
    }

    /// Open the named table if it exists, creating it otherwise.
    ///
    /// An existing table is opened as it is - `schema` describes only the table
    /// this call would create.
    #[napi]
    pub fn open_or_create(&self, name: String, schema: TableSchemaInput<'_>) -> Result<JsTable> {
        self.view()
            .open_or_create(&self.dotted(&name), schema_from_input(schema)?)
            .map(JsTable::from_core)
            .map_err(napi_error)
    }

    /// Append `batches` to the named table, creating it on first write.
    ///
    /// A table that is not there yet takes its schema from the rows: partition
    /// marks riding the Arrow fields' metadata become the spec, so a marked
    /// schema lays its files out partitioned from the very first append.
    /// `options` configures this one write. Returns the table so the caller can
    /// keep going.
    #[napi]
    pub fn append(
        &self,
        name: String,
        batches: &mut JsBatchReader,
        options: Option<&JsIcebergOptions>,
    ) -> Result<JsTable> {
        self.view()
            .append_with(&self.dotted(&name), batches.take()?, call_options(options))
            .map(JsTable::from_core)
            .map_err(napi_error)
    }

    /// Replace the named table's rows with `batches`, creating it on first
    /// write.
    ///
    /// An existing table keeps its previous snapshot readable, which is what
    /// makes the overwrite reversible. `options` configures this one write.
    /// Returns the table so the caller can keep going.
    #[napi]
    pub fn overwrite(
        &self,
        name: String,
        batches: &mut JsBatchReader,
        options: Option<&JsIcebergOptions>,
    ) -> Result<JsTable> {
        self.view()
            .overwrite_with(&self.dotted(&name), batches.take()?, call_options(options))
            .map(JsTable::from_core)
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
pub fn iceberg_schema_from_json(name: String, document: &JsScalar) -> Result<JsField> {
    schema_from_json(&name, &document.inner)
        .map(JsField::from_core)
        .map_err(napi_error)
}

/// Write a root Field as an Iceberg schema document.
#[napi(js_name = "icebergSchemaToJsonNative", skip_typescript)]
pub fn iceberg_schema_to_json(schema: &JsField) -> Result<JsScalar> {
    schema_to_json(&schema.inner)
        .map(JsScalar::from_core)
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
