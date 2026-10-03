//! Apache Iceberg tables, reached from JavaScript through one handle.
//!
//! A table is a folder: `metadata/` holds the JSON documents and the Avro
//! manifests, `data/` holds the Parquet files, and every one of them is a child
//! of the handle the table was built from. Nothing here opens a path, so the
//! same JavaScript works over a local directory today and over an object store
//! the moment a backend for one exists.

use std::collections::HashMap;

use napi::bindgen_prelude::{
    BigInt, Buffer, ClassInstance, Either, Either4, Null, Result, Undefined,
};
use napi_derive::napi;
use yggdryl::iceberg::{
    Compaction as CoreCompaction, DataFile, FormatVersion, IcebergCatalog as CoreCatalog,
    IcebergNamespace as CoreNamespace, IcebergOptions as CoreIcebergOptions,
    IcebergTable as CoreTable, ManifestContent, ManifestFile, PartitionField as CorePartitionField,
    PartitionSpec as CorePartitionSpec, ScanPlan as CoreScanPlan, SchemaUpdate as CoreSchemaUpdate,
    Snapshot, SnapshotRef, WriteStaging, assign_field_ids, can_promote, last_column_id,
    schema_from_json, schema_into_json,
};
use yggdryl::{
    Catalog as CoreWarehouseCatalog, Field as CoreField, Handle, IntoObjectPath as _,
    Namespace as CoreWarehouseNamespace, ObjectValue, Scalar as CoreScalar,
    Table as CoreWarehouseTable,
};

use crate::datatype::{DataTypeInput, dtype_from_input};
use crate::enums::{JsMimeType, MimeTypeInput, mime_type_from_input};
use crate::field::{JsField, MetadataEntry};
use crate::iobase::{JsIOBase, LocationInput, folder_from_input, site_from_input};
use crate::iomedia::JsBatchReader;
use crate::napi_error;
use crate::text::codec::JsScalar;
use crate::uri::PartitionEntry;
use crate::warehouse::{
    JsObjectIterator, JsWarehouseCatalog, JsWarehouseNamespace, JsWarehouseNamespaces,
    JsWarehouseTable, JsWarehouseTables, ObjectOptions, ObjectOutput, ObjectPathInput, Stated,
    object_path,
};

/// What a new table partitions by: a spec, the `PARTITION:by` entries one is
/// read from, `null` for none, or nothing for the schema's own declaration.
///
/// `null` and an omitted argument mean two things here, so neither is folded
/// into an `Option`.
pub type PartitionInput<'a> =
    Either4<ClassInstance<'a, JsPartitionSpec>, Vec<String>, Null, Undefined>;

/// A native `Field` or the field expression naming one.
pub type FieldInput<'a> = Either<ClassInstance<'a, JsField>, String>;

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

/// Number a schema the way `Table.create` needs it.
///
/// Numbering continues above the highest identifier already assigned, so a
/// numbered schema keeps every id it came with and a plain schema arrives with
/// none and leaves with all of them. It happens at this boundary because the
/// spec builder resolves `partitionBy` names to identifiers.
fn numbered_schema(mut schema: CoreField) -> Result<CoreField> {
    let start = last_column_id(&schema)
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
/// getters carry, so the object and the setters spell the same eleven things.
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
    /// How many partition groups a commit writes at once.
    pub write_parallelism: Option<u32>,
    /// Where a commit stages its files: `off`, or a local folder URL or path.
    pub write_staging: Option<String>,
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
    if let Some(threads) = input.write_parallelism {
        options
            .set_write_parallelism(threads as usize)
            .map_err(napi_error)?;
    }
    if let Some(staging) = input.write_staging {
        options
            .set_write_staging(WriteStaging::from_str(&staging).map_err(napi_error)?)
            .map_err(napi_error)?;
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

    /// How many large-enough files justify a parallel scan. Default: 2.
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

    /// How many partition groups a commit writes at once. Default: the
    /// resolved `readParallelism`; 1 writes them one after another. The
    /// manifest lists a commit's files in partition-group order whatever the
    /// value.
    #[napi(getter)]
    pub fn write_parallelism(&self) -> Result<u32> {
        u32::try_from(self.inner.write_parallelism())
            .map_err(|_| napi::Error::from_reason("writeParallelism exceeds a JavaScript u32"))
    }

    /// Set how many partition groups a commit writes at once.
    ///
    /// # Errors
    ///
    /// Throws the core's typed error naming the value when the count is zero,
    /// which would write nothing at all.
    #[napi(setter)]
    pub fn set_write_parallelism(&mut self, threads: u32) -> Result<()> {
        self.inner
            .set_write_parallelism(threads as usize)
            .map_err(napi_error)
    }

    /// Where a commit stages its files before uploading them: `"off"`, or
    /// a local folder URL. `null` - the default - is the table's own: the
    /// temporary folder for a remote root, off for a local one.
    #[napi(getter)]
    pub fn write_staging(&self) -> Option<String> {
        self.inner.write_staging().map(ToString::to_string)
    }

    /// Set where a commit stages its files: `"off"`, a local folder URL, or
    /// a local path.
    ///
    /// # Errors
    ///
    /// Throws the core's typed error naming the key when the folder is not
    /// local, which could hold no staging file.
    #[napi(setter)]
    pub fn set_write_staging(&mut self, staging: String) -> Result<()> {
        self.inner
            .set_write_staging(WriteStaging::from_str(&staging).map_err(napi_error)?)
            .map_err(napi_error)
    }

    /// The recorded size below which a file does not count toward justifying a
    /// parallel scan, in bytes. Default: 64 KiB.
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
    table: &mut CoreTable<Handle>,
    options: Option<CoreIcebergOptions>,
    operation: impl FnOnce(&mut CoreTable<Handle>) -> Result<R>,
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

/// Resolve what a caller partitioned by.
///
/// An array is the `PARTITION:by` entries the schema root would declare - a
/// bare column an identity partition, `days(ts)`, `minutes(ts, 15)` or
/// `truncate(name, 4) as prefix` a derived one - declared on a copy of the
/// root and read into a spec by the core's one rule, which refuses an entry
/// no spec can hold by naming it. An omitted argument is the schema's own
/// declaration read by that rule - a schema declaring nothing unpartitioned -
/// and `null` is unpartitioned whatever the schema declares.
fn partition_spec(value: PartitionInput<'_>, schema: &CoreField) -> Result<CorePartitionSpec> {
    match value {
        Either4::A(spec) => Ok(spec.inner.clone()),
        Either4::C(Null) => Ok(CorePartitionSpec::unpartitioned()),
        Either4::D(()) => CorePartitionSpec::from_schema(0, schema).map_err(napi_error),
        Either4::B(entries) => {
            let mut root = schema.clone();
            root.as_partition_mut()
                .set_by_texts(&entries)
                .map_err(napi_error)?;
            CorePartitionSpec::from_schema(0, &root).map_err(napi_error)
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
/// as the native [`Scalar`](crate::text::codec::JsScalar) the manifest recorded.
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
#[napi(js_name = "IcebergTable")]
pub struct JsTable {
    inner: CoreTable<Handle>,
}

impl JsTable {
    const fn from_core(inner: CoreTable<Handle>) -> Self {
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
    /// `partitionBy` takes a [`PartitionSpec`](JsPartitionSpec) or the
    /// `PARTITION:by` entries to partition on: a bare column - `venue` - is an
    /// identity partition, and an epoch function over a column - `days(ts)`,
    /// `hours(ts)`, `minutes(ts, 15)`, `weeks(ts)`, `quarters(ts)` - or
    /// `truncate(name, 4)` is a derived one, named by its alias
    /// (`days(ts) as day`) or `{source}_{function}` (`ts_day`). An entry no
    /// spec can hold is refused, naming it. Omitted, the schema's own
    /// `PARTITION:by` declaration is read the same way - a schema declaring
    /// nothing is unpartitioned - and `null` is unpartitioned whatever the
    /// schema declares. Unnumbered schema columns are numbered automatically,
    /// so a plain schema works as it is; a schema that already carries field
    /// identifiers keeps every one of them.
    #[napi(
        factory,
        ts_args_type = "root: LocationInput, schema: Field, partitionBy?: PartitionInput | null, version?: number | undefined | null"
    )]
    pub fn create(
        root: LocationInput<'_>,
        schema: &JsField,
        partition_by: PartitionInput<'_>,
        version: Option<u32>,
    ) -> Result<Self> {
        let schema = numbered_schema(schema.inner.clone())?;
        let spec = partition_spec(partition_by, &schema)?;
        CoreTable::create(
            Handle::from(folder_from_input(root)?),
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
        CoreTable::open(Handle::from(folder_from_input(root)?))
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Open the table if it exists, creating it otherwise.
    ///
    /// Like [`create`](Self::create), `partitionBy` is a spec, the
    /// `PARTITION:by` entries one is read from, `null` for none, or - omitted -
    /// the schema's own declaration, and unnumbered schema columns are
    /// numbered automatically; an existing table is opened as it is and
    /// `schema` describes only the table this call would create.
    #[napi(
        factory,
        ts_args_type = "root: LocationInput, schema: Field, partitionBy?: PartitionInput | null, version?: number | undefined | null"
    )]
    pub fn open_or_create(
        root: LocationInput<'_>,
        schema: &JsField,
        partition_by: PartitionInput<'_>,
        version: Option<u32>,
    ) -> Result<Self> {
        let schema = numbered_schema(schema.inner.clone())?;
        let spec = partition_spec(partition_by, &schema)?;
        CoreTable::open_or_create(
            Handle::from(folder_from_input(root)?),
            format_version(version)?,
            schema,
            spec,
        )
        .map(Self::from_core)
        .map_err(napi_error)
    }

    /// The Iceberg table a warehouse `Table` holds, refused by name when its
    /// implementation is another.
    #[napi(factory)]
    pub fn from(table: &JsWarehouseTable) -> Result<Self> {
        match &table.inner {
            CoreWarehouseTable::Iceberg(table) => Ok(Self::from_core((**table).clone())),
            other => Err(napi_error(format!(
                "expected an Iceberg table, got `{other}` held by another implementation"
            ))),
        }
    }

    /// This table as the warehouse `Table` it is: what a warehouse registers
    /// and a plan reads.
    #[napi]
    pub fn into_table(&self) -> JsWarehouseTable {
        JsWarehouseTable::from_core(CoreWarehouseTable::from(self.inner.clone()))
    }

    /// The last part of the path: the table's own name.
    #[napi(getter)]
    pub fn name(&self) -> String {
        ObjectValue::name(&self.inner).to_owned()
    }

    /// The parts, from the catalog's name down to the table's own; a table
    /// opened by its location alone stands under its folder's name.
    #[napi(getter)]
    pub fn path(&self) -> Vec<String> {
        ObjectValue::path(&self.inner)
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Whether both describe the same table: the path, the location, what
    /// was stated - never what was read.
    #[napi]
    pub fn equals(&self, other: &JsTable) -> bool {
        self.inner == other.inner
    }

    /// The folder the table lives in.
    ///
    /// Taken from the table's own root handle rather than from its recorded
    /// location, because a location does not say which backend it belongs to:
    /// a table on a foreign Arrow file system must hand back a folder on that
    /// file system, not the local path its URL happens to spell.
    #[napi(getter)]
    pub fn root(&self) -> Result<JsIOBase> {
        let root = self.inner.root().get().map_err(napi_error)?;
        if let Some(holder) = crate::iobase::fs_folder_holder(root) {
            return Ok(JsIOBase::from_core(holder));
        }
        JsIOBase::folder_at(&self.location()?)
    }

    /// The table's base location, as a URI.
    #[napi(getter)]
    pub fn location(&self) -> Result<String> {
        Ok(self
            .inner
            .metadata()
            .map_err(napi_error)?
            .location()
            .to_owned())
    }

    /// A stable identifier for the table itself, not for any one version.
    #[napi(getter)]
    pub fn table_uuid(&self) -> Result<String> {
        Ok(self
            .inner
            .metadata()
            .map_err(napi_error)?
            .table_uuid()
            .to_owned())
    }

    /// Which revision of the specification the metadata is written to.
    #[napi(getter)]
    pub fn format_version(&self) -> Result<i32> {
        Ok(self
            .inner
            .metadata()
            .map_err(napi_error)?
            .format_version()
            .number())
    }

    /// The version number of the current metadata document.
    #[napi(getter)]
    pub fn version(&self) -> Result<u32> {
        self.inner.metadata_version().map_err(napi_error)
    }

    /// The effective properties: the parent's, then the free-form table
    /// properties the metadata document carries, then what was stated for
    /// the table, a later entry replacing an earlier one by name.
    #[napi(getter)]
    pub fn properties(&self) -> Result<HashMap<String, String>> {
        Ok(ObjectValue::properties(&self.inner)
            .map_err(napi_error)?
            .iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect())
    }

    /// The name of the current metadata document.
    #[napi(getter)]
    pub fn metadata_file_name(&self) -> Result<String> {
        self.inner.metadata_file_name().map_err(napi_error)
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
            .map_err(napi_error)?
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
    pub fn current_snapshot(&self) -> Result<Option<JsSnapshot>> {
        Ok(self
            .inner
            .current_snapshot()
            .map_err(napi_error)?
            .map(snapshot_view))
    }

    /// Every schema the table has had, oldest first.
    #[napi(getter)]
    pub fn schemas(&self) -> Result<Vec<JsField>> {
        Ok(self
            .inner
            .metadata()
            .map_err(napi_error)?
            .schemas()
            .iter()
            .cloned()
            .map(JsField::from_core)
            .collect())
    }

    /// Every retained snapshot, oldest first.
    #[napi(getter)]
    pub fn snapshots(&self) -> Result<Vec<JsSnapshot>> {
        Ok(self
            .inner
            .metadata()
            .map_err(napi_error)?
            .snapshots()
            .iter()
            .map(snapshot_view)
            .collect())
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
        let metadata = self.inner.metadata().map_err(napi_error)?;
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
    /// `filter` is a `Filter`, a `Term`, or the text of a predicate, which
    /// parses. It is the whole expression language rather than equality
    /// pairs: ranges, null tests, `in` lists, and nested paths. Planning
    /// prunes with the metadata chain, and only the conjuncts it could not
    /// settle are tested against the rows.
    #[napi]
    pub fn scan_matching(
        &self,
        filter: napi::bindgen_prelude::Either3<
            napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsFilter>,
            napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsTerm>,
            String,
        >,
        field: Option<&JsField>,
    ) -> Result<JsBatchReader> {
        let root_name = self.root_name()?;
        let filter = crate::expression::filter_from_input(filter)?;
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
        filter: napi::bindgen_prelude::Either3<
            napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsFilter>,
            napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsTerm>,
            String,
        >,
    ) -> Result<ScanPlanCounts> {
        let filter = crate::expression::filter_from_input(filter)?;
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
        filters: Option<ScanFilters>,
        field: Option<FieldInput<'_>>,
        options: Option<&JsIcebergOptions>,
    ) -> Result<JsBatchReader> {
        let root_name = self.root_name()?;
        let pairs = filter_pairs(filters);
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
            table.commit_append(batches).map_err(napi_error)
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
            table.commit_overwrite(batches).map_err(napi_error)
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
        filters: Option<ScanFilters>,
        batches: &mut JsBatchReader,
        options: Option<&JsIcebergOptions>,
    ) -> Result<()> {
        let pairs = filter_pairs(filters);
        let batches = batches.take()?;
        with_call_options(&mut self.inner, call_options(options), |table| {
            table
                .commit_overwrite_where(&borrowed_pairs(&pairs), batches)
                .map_err(napi_error)
        })
    }

    /// Merge `batches` into the stored rows, matching on `mergeBy`: a
    /// `Selector`, the text of one, or the key column names.
    ///
    /// A row whose key is already stored updates it and a row whose key is not
    /// appends. Only the files whose recorded bounds could hold an incoming key
    /// are read and rewritten - the rest are carried into the new snapshot
    /// untouched - so an upsert costs the files it can actually change. A
    /// non-empty `mergeBy` is required because nothing else identifies a
    /// row.
    ///
    /// `safe` decides what a cast that cannot convert a value does: the
    /// default nulls it, and `false` throws instead. `options` configures this
    /// one write, exactly as on [`append`](Self::append).
    #[napi]
    pub fn merge(
        &mut self,
        batches: &mut JsBatchReader,
        merge_by: napi::bindgen_prelude::Either4<
            napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsSelector>,
            napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsTerm>,
            String,
            Vec<
                napi::bindgen_prelude::Either<
                    napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsTerm>,
                    String,
                >,
            >,
        >,
        safe: Option<bool>,
        options: Option<&JsIcebergOptions>,
    ) -> Result<()> {
        let keys = crate::expression::selector_from_input(merge_by)?;
        let batches = batches.take()?;
        with_call_options(&mut self.inner, call_options(options), |table| {
            table
                .commit_merge(batches, &keys, safe.unwrap_or(true))
                .map_err(napi_error)
        })
    }

    /// Merge `batches` into the rows `filters` selects, on `mergeBy`.
    ///
    /// [`merge`](Self::merge) narrowed to a part of the table first: the
    /// filters decide which files are candidates at all, and the match-key
    /// statistics then decide which of those are actually read. `options`
    /// configures this one write, exactly as on [`append`](Self::append).
    #[napi]
    pub fn merge_where(
        &mut self,
        filters: Option<ScanFilters>,
        batches: &mut JsBatchReader,
        merge_by: napi::bindgen_prelude::Either4<
            napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsSelector>,
            napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsTerm>,
            String,
            Vec<
                napi::bindgen_prelude::Either<
                    napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsTerm>,
                    String,
                >,
            >,
        >,
        safe: Option<bool>,
        options: Option<&JsIcebergOptions>,
    ) -> Result<()> {
        let pairs = filter_pairs(filters);
        let keys = crate::expression::selector_from_input(merge_by)?;
        let batches = batches.take()?;
        with_call_options(&mut self.inner, call_options(options), |table| {
            table
                .commit_merge_where(
                    &borrowed_pairs(&pairs),
                    batches,
                    &keys,
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
            .remove_snapshot_ref(&name)
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
            .fast_forward_branch(&name, snapshot_id)
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

    /// The explicit override this handle holds, if any: what a per-call
    /// property bag with no options beside it is set on a copy of.
    #[napi(js_name = "_explicitOptionsNative", skip_typescript)]
    pub fn explicit_options_native(&self) -> Option<JsIcebergOptions> {
        self.inner.explicit_options().map(|inner| JsIcebergOptions {
            inner: inner.clone(),
        })
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
        let target = self.inner.target_file_size_bytes().map_err(napi_error)?;
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
            .commit_metadata_changes(|metadata| {
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

    /// The native half of `updateSchema()`: a core `SchemaUpdate` started
    /// from the metadata the table holds now.
    #[napi(js_name = "_updateSchemaNative", skip_typescript)]
    pub fn update_schema_native(&self) -> Result<JsSchemaUpdate> {
        CoreSchemaUpdate::from_metadata(self.inner.metadata().map_err(napi_error)?)
            .map(|inner| JsSchemaUpdate { inner })
            .map_err(napi_error)
    }

    /// The native half of `updateSchema().commit()`.
    ///
    /// The core replays the recording onto the metadata each commit attempt
    /// reads, so a commit beaten by another writer rebases onto the winner's
    /// schema rather than dropping its columns. A recording with nothing in
    /// it commits nothing and answers the current schema's identifier. The
    /// wrapper needs no refresh: a failed commit leaves the table as it was.
    #[napi(js_name = "_commitSchemaUpdateNative", skip_typescript)]
    pub fn commit_schema_update(&mut self, update: &JsSchemaUpdate) -> Result<i32> {
        self.inner.update_schema(&update.inner).map_err(napi_error)
    }

    /// The dotted path, as the plan grammar spells it and as every
    /// warehouse object prints.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.into_table().js_string()
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

/// A recording of column operations against a table's schema.
///
/// The loader hands one out from `table.updateSchema()` and wraps each method
/// to return the builder, so a chain reads as one sentence. It is the core
/// `SchemaUpdate` itself: nothing is checked while recording, and `commit()`
/// replays the recording onto the schema the table has *then*, reporting the
/// first failure with its core message.
#[napi(js_name = "SchemaUpdate")]
pub struct JsSchemaUpdate {
    inner: CoreSchemaUpdate,
}

#[napi]
impl JsSchemaUpdate {
    /// Record a new column under `parent` - `""` for the root, a dotted path
    /// for a nested struct.
    #[napi]
    pub fn add_column(&mut self, parent: String, field: FieldInput<'_>) -> Result<()> {
        self.inner.add_column(&parent, field_from_input(field)?);
        Ok(())
    }

    /// Record the removal of the column at `path`, retiring its identifier.
    #[napi]
    pub fn drop_column(&mut self, path: String) {
        self.inner.drop_column(&path);
    }

    /// Record a rename of the column at `path`; its identifier is kept.
    #[napi]
    pub fn rename_column(&mut self, path: String, name: String) {
        self.inner.rename_column(&path, name);
    }

    /// Record a new `ICEBERG:doc` documentation string on the column at `path`.
    #[napi]
    pub fn update_doc(&mut self, path: String, doc: String) {
        self.inner.update_doc(&path, doc);
    }

    /// Record that the column at `path` becomes optional.
    #[napi]
    pub fn make_nullable(&mut self, path: String) {
        self.inner.make_nullable(&path);
    }

    /// Record a type promotion on the column at `path`.
    #[napi]
    pub fn update_type(&mut self, path: String, dtype: DataTypeInput<'_>) -> Result<()> {
        self.inner.update_type(&path, dtype_from_input(dtype)?);
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

/// A warehouse folder of namespaces of Iceberg tables: the implementation a
/// warehouse `Catalog` holds when it is one.
///
/// Namespaces nest to any depth, each a folder; `metadata/catalog.json` and
/// `metadata/namespace.json` keep the stored properties; a table is a folder
/// laid out as one. The catalog is a description - constructing one touches
/// nothing - and every question is asked of the store when it is asked,
/// through the members every `Catalog` has: `namespaces()`, `tables()`,
/// `children()`, `get`, `resolve`, `table`, `namespace`, `createNamespace`
/// and `createTable` here redirect to the same object [`intoCatalog`](Self::into_catalog)
/// answers.
#[napi(js_name = "IcebergCatalog")]
pub struct JsCatalog {
    inner: CoreCatalog,
}

impl JsCatalog {
    /// The generic catalog this one is, which the shared members answer through.
    fn generic(&self) -> JsWarehouseCatalog {
        JsWarehouseCatalog::from_core(CoreWarehouseCatalog::from(self.inner.clone()))
    }
}

#[napi]
impl JsCatalog {
    /// The catalog `name` over the warehouse folder `location` names,
    /// touching nothing: a handle binds the folder, a location or an
    /// identifier names what opens on first use. `options.description` and
    /// `options.properties` are what the catalog states, which its folder
    /// and every object under it open with.
    #[napi(constructor)]
    pub fn new(
        name: String,
        location: LocationInput<'_>,
        options: Option<ObjectOptions<'_>>,
    ) -> Result<Self> {
        let stated = Stated::read(options)?;
        stated.only("IcebergCatalog", false, false, false)?;
        let mut inner = match site_from_input(location)? {
            Either::A(holder) => CoreCatalog::bound(name, holder),
            Either::B(uri) => CoreCatalog::new(name, uri).map_err(napi_error)?,
        }
        .with_properties(stated.properties);
        if let Some(description) = stated.description {
            inner = inner.with_description(description);
        }
        Ok(Self { inner })
    }

    /// Create the catalog `name` in the folder `location` names, writing its
    /// `metadata/catalog.json`; the write is what creates the folder, and a
    /// folder already holding anything is a conflict.
    #[napi(factory)]
    pub fn create(name: String, location: LocationInput<'_>) -> Result<Self> {
        CoreCatalog::create(name, folder_from_input(location)?)
            .map(|inner| Self { inner })
            .map_err(napi_error)
    }

    /// The catalog `name` over the folder `location` names, created when the
    /// folder is not there yet; a table or a file in its place is refused by
    /// name.
    #[napi(factory)]
    pub fn open_or_create(name: String, location: LocationInput<'_>) -> Result<Self> {
        CoreCatalog::open_or_create(name, folder_from_input(location)?)
            .map(|inner| Self { inner })
            .map_err(napi_error)
    }

    /// The Iceberg catalog a warehouse `Catalog` holds, refused by name when
    /// its implementation is another.
    #[napi(factory)]
    pub fn from(catalog: &JsWarehouseCatalog) -> Result<Self> {
        match &catalog.inner {
            CoreWarehouseCatalog::Iceberg(catalog) => Ok(Self {
                inner: (**catalog).clone(),
            }),
            other => Err(napi_error(format!(
                "expected an Iceberg catalog, got `{other}` held by another implementation"
            ))),
        }
    }

    /// This catalog as the warehouse `Catalog` it is: what a warehouse
    /// registers and a plan resolves against.
    #[napi]
    pub fn into_catalog(&self) -> JsWarehouseCatalog {
        self.generic()
    }

    /// The catalog's name, the first part of every path under it.
    #[napi(getter)]
    pub fn name(&self) -> String {
        ObjectValue::name(&self.inner).to_owned()
    }

    /// The path: the name alone.
    #[napi(getter)]
    pub fn path(&self) -> Vec<String> {
        ObjectValue::path(&self.inner)
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// What the store says this catalog is, when it says anything.
    #[napi(getter)]
    pub fn description(&self) -> Option<String> {
        ObjectValue::description(&self.inner).map(str::to_owned)
    }

    /// The warehouse folder's location.
    #[napi(getter)]
    pub fn url(&self) -> Option<crate::uri::JsUrl> {
        ObjectValue::url(&self.inner)
            .cloned()
            .map(crate::uri::JsUrl::from_core)
    }

    /// The effective properties: what `metadata/catalog.json` keeps, then
    /// what was stated, a later entry replacing an earlier one by name.
    #[napi(getter)]
    pub fn properties(&self) -> Result<HashMap<String, String>> {
        properties_map(&self.inner)
    }

    /// Set and remove the properties `metadata/catalog.json` keeps, as one
    /// write.
    ///
    /// `updates` is a mapping of properties to set and `removes` lists the
    /// keys to drop, in that order. Passing neither writes nothing at all.
    /// Keys under the reserved `ICEBERG:` prefix are refused by name.
    #[napi]
    pub fn update_properties(
        &self,
        updates: Option<PropertyUpdates>,
        removes: Option<Vec<String>>,
    ) -> Result<()> {
        update_object_properties(&self.inner, updates, removes)
    }

    /// How many namespace levels sit under the catalog: none stated, since
    /// namespaces nest to any depth.
    #[napi(getter)]
    pub fn namespace_levels(&self) -> Option<u32> {
        None
    }

    /// The namespaces one level down, as the lazy map-like view every
    /// catalog answers.
    #[napi]
    pub fn namespaces(&self) -> JsWarehouseNamespaces {
        self.generic().namespaces()
    }

    /// The tables one level down, as the lazy map-like view every catalog
    /// answers.
    #[napi]
    pub fn tables(&self) -> JsWarehouseTables {
        self.generic().tables()
    }

    /// Its children, one at a time in the store's order.
    #[napi]
    pub fn children(&self) -> JsObjectIterator {
        self.generic().children()
    }

    /// The child called `name`, one level down.
    #[napi]
    pub fn get(&self, name: String) -> Result<ObjectOutput> {
        self.generic().get(name)
    }

    /// The object a path below the catalog names, descending through `get`.
    #[napi]
    pub fn resolve(&self, path: ObjectPathInput) -> Result<ObjectOutput> {
        self.generic().resolve(path)
    }

    /// The table a path below the catalog names, or its absence.
    #[napi]
    pub fn table(&self, path: ObjectPathInput) -> Result<JsWarehouseTable> {
        self.generic().table(path)
    }

    /// The namespace a path below the catalog names, or its absence.
    #[napi]
    pub fn namespace(&self, path: ObjectPathInput) -> Result<JsWarehouseNamespace> {
        self.generic().namespace(path)
    }

    /// Create the namespace `name` under the catalog, writing its
    /// `metadata/namespace.json` with `properties`.
    #[napi(
        ts_args_type = "name: string, properties?: Record<string, string | number | boolean> | null"
    )]
    pub fn create_namespace(
        &self,
        name: String,
        properties: Option<napi::bindgen_prelude::Object<'_>>,
    ) -> Result<JsWarehouseNamespace> {
        self.generic().create_namespace(name, properties)
    }

    /// Create the table `name` under the catalog, `field` its row schema,
    /// numbered where it is not, its partition spec read from the schema's
    /// own `PARTITION:by` declaration.
    #[napi(
        ts_args_type = "name: string, field: Field | string, properties?: Record<string, string | number | boolean> | null"
    )]
    pub fn create_table(
        &self,
        name: String,
        field: crate::iceberg::FieldInput<'_>,
        properties: Option<napi::bindgen_prelude::Object<'_>>,
    ) -> Result<JsWarehouseTable> {
        self.generic().create_table(name, field, properties)
    }

    /// Whether both describe the same catalog: the name, the location, what
    /// was stated.
    #[napi]
    pub fn equals(&self, other: &JsCatalog) -> bool {
        self.inner == other.inner
    }

    /// The name, as the dotted path of every object under it starts.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.generic().js_string()
    }
}

/// One namespace of an Iceberg catalog - a folder under the warehouse, its
/// `metadata/namespace.json` the stored properties - as the implementation a
/// warehouse `Namespace` holds when it is one.
#[napi(js_name = "IcebergNamespace")]
pub struct JsNamespace {
    inner: CoreNamespace,
}

impl JsNamespace {
    /// The generic namespace this one is, which the shared members answer through.
    fn generic(&self) -> JsWarehouseNamespace {
        JsWarehouseNamespace::from_core(CoreWarehouseNamespace::from(self.inner.clone()))
    }
}

#[napi]
impl JsNamespace {
    /// The namespace at `path` - dotted text or parts, its catalog's name
    /// first - over the folder `location` names, touching nothing: a handle
    /// binds the folder, a location or an identifier names what opens on
    /// first use. `options.properties` is what the namespace states.
    #[napi(constructor)]
    pub fn new(
        path: ObjectPathInput,
        location: LocationInput<'_>,
        options: Option<ObjectOptions<'_>>,
    ) -> Result<Self> {
        let stated = Stated::read(options)?;
        stated.only("IcebergNamespace", false, false, false)?;
        if stated.description.is_some() {
            return Err(napi_error(
                "expected no `description` option on an IcebergNamespace, got one",
            ));
        }
        let path = object_path(path).map_err(napi_error)?;
        let inner = match site_from_input(location)? {
            Either::A(holder) => CoreNamespace::bound(path, holder),
            Either::B(uri) => CoreNamespace::new(path, uri),
        }
        .map_err(napi_error)?
        .with_properties(stated.properties);
        Ok(Self { inner })
    }

    /// The Iceberg namespace a warehouse `Namespace` holds, refused by name
    /// when its implementation is another.
    #[napi(factory)]
    pub fn from(namespace: &JsWarehouseNamespace) -> Result<Self> {
        match &namespace.inner {
            CoreWarehouseNamespace::Iceberg(namespace) => Ok(Self {
                inner: (**namespace).clone(),
            }),
            other => Err(napi_error(format!(
                "expected an Iceberg namespace, got `{other}` held by another implementation"
            ))),
        }
    }

    /// This namespace as the warehouse `Namespace` it is.
    #[napi]
    pub fn into_namespace(&self) -> JsWarehouseNamespace {
        self.generic()
    }

    /// The last part of the path.
    #[napi(getter)]
    pub fn name(&self) -> String {
        ObjectValue::name(&self.inner).to_owned()
    }

    /// The parts, from the catalog's name down to this namespace's own.
    #[napi(getter)]
    pub fn path(&self) -> Vec<String> {
        ObjectValue::path(&self.inner)
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// The folder's location.
    #[napi(getter)]
    pub fn url(&self) -> Option<crate::uri::JsUrl> {
        ObjectValue::url(&self.inner)
            .cloned()
            .map(crate::uri::JsUrl::from_core)
    }

    /// The effective properties: the parent's, then what
    /// `metadata/namespace.json` keeps, then what was stated.
    #[napi(getter)]
    pub fn properties(&self) -> Result<HashMap<String, String>> {
        properties_map(&self.inner)
    }

    /// Set and remove the properties `metadata/namespace.json` keeps, as one
    /// write; keys under the reserved `ICEBERG:` prefix are refused by name.
    #[napi]
    pub fn update_properties(
        &self,
        updates: Option<PropertyUpdates>,
        removes: Option<Vec<String>>,
    ) -> Result<()> {
        update_object_properties(&self.inner, updates, removes)
    }

    /// The namespaces one level down, as the lazy map-like view.
    #[napi]
    pub fn namespaces(&self) -> JsWarehouseNamespaces {
        self.generic().namespaces()
    }

    /// The tables one level down, as the lazy map-like view.
    #[napi]
    pub fn tables(&self) -> JsWarehouseTables {
        self.generic().tables()
    }

    /// Its children, one at a time in the store's order.
    #[napi]
    pub fn children(&self) -> JsObjectIterator {
        self.generic().children()
    }

    /// The child called `name`, one level down.
    #[napi]
    pub fn get(&self, name: String) -> Result<ObjectOutput> {
        self.generic().get(name)
    }

    /// The object a path below the namespace names, descending through `get`.
    #[napi]
    pub fn resolve(&self, path: ObjectPathInput) -> Result<ObjectOutput> {
        self.generic().resolve(path)
    }

    /// The table a path below the namespace names, or its absence.
    #[napi]
    pub fn table(&self, path: ObjectPathInput) -> Result<JsWarehouseTable> {
        self.generic().table(path)
    }

    /// The namespace a path below this one names, or its absence.
    #[napi]
    pub fn namespace(&self, path: ObjectPathInput) -> Result<JsWarehouseNamespace> {
        self.generic().namespace(path)
    }

    /// Create the namespace `name` under this one, writing its document.
    #[napi(
        ts_args_type = "name: string, properties?: Record<string, string | number | boolean> | null"
    )]
    pub fn create_namespace(
        &self,
        name: String,
        properties: Option<napi::bindgen_prelude::Object<'_>>,
    ) -> Result<JsWarehouseNamespace> {
        self.generic().create_namespace(name, properties)
    }

    /// Create the table `name` under this namespace, `field` its row schema.
    #[napi(
        ts_args_type = "name: string, field: Field | string, properties?: Record<string, string | number | boolean> | null"
    )]
    pub fn create_table(
        &self,
        name: String,
        field: crate::iceberg::FieldInput<'_>,
        properties: Option<napi::bindgen_prelude::Object<'_>>,
    ) -> Result<JsWarehouseTable> {
        self.generic().create_table(name, field, properties)
    }

    /// Whether both describe the same namespace.
    #[napi]
    pub fn equals(&self, other: &JsNamespace) -> bool {
        self.inner == other.inner
    }

    /// The dotted path, as the plan grammar spells it.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.generic().js_string()
    }
}

/// An object's effective properties, as a plain map.
fn properties_map(object: &dyn ObjectValue) -> Result<HashMap<String, String>> {
    Ok(object
        .properties()
        .map_err(napi_error)?
        .iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect())
}

/// Apply property updates and removals to what an object's store keeps; a
/// call given neither writes nothing.
fn update_object_properties(
    object: &dyn ObjectValue,
    updates: Option<PropertyUpdates>,
    removes: Option<Vec<String>>,
) -> Result<()> {
    let (updates, removes) = property_changes(updates, removes);
    if updates.is_empty() && removes.is_empty() {
        return Ok(());
    }
    let updates: yggdryl::Properties = updates.into_iter().collect();
    // The names as the core spells them, which is what the parts intake
    // answers for parts given as they are.
    let removes = removes
        .iter()
        .map(String::as_str)
        .collect::<Vec<&str>>()
        .into_object_path()
        .map_err(napi_error)?;
    object
        .update_properties(&updates, &removes)
        .map_err(napi_error)
}

/// Number every column of a schema from `start`, so a table can carry it:
/// the native half of `iceberg.assignFieldIds`.
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
#[napi(js_name = "icebergSchemaIntoJsonNative", skip_typescript)]
pub fn iceberg_schema_into_json(schema: &JsField) -> Result<JsScalar> {
    schema_into_json(&schema.inner)
        .map(JsScalar::from_core)
        .map_err(napi_error)
}

/// Check one type change against the promotions Iceberg allows.
///
/// Returns nothing on a legal promotion and throws the core message naming
/// both sides for every other change.
#[napi(js_name = "icebergCanPromoteNative", skip_typescript)]
pub fn iceberg_can_promote(from_type: DataTypeInput<'_>, to_type: DataTypeInput<'_>) -> Result<()> {
    can_promote(&dtype_from_input(from_type)?, &dtype_from_input(to_type)?).map_err(napi_error)
}
