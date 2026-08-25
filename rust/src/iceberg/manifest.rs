//! Manifest lists, manifests, and the data files they describe.
//!
//! Iceberg puts two levels of indirection between a snapshot and its rows. A
//! snapshot names one *manifest list*; each row of that list is a *manifest*;
//! each row of a manifest is one *data file* with its partition tuple and its
//! statistics. Both levels are Avro, both are read and written here through
//! [`crate::avro`], and both are reached through the handle the table was
//! constructed from - never by opening a path.
//!
//! The Avro schemas are built rather than stored, because the `partition`
//! column's shape comes from the table's partition spec. Everything else is the
//! specification's fixed field numbering, which is what lets another
//! implementation read a manifest this module wrote: an Avro reader matches
//! fields by `field-id`, not by name or position.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use smol_str::{SmolStr, format_smolstr};

use super::FormatVersion;
use super::partition::PartitionSpec;
use crate::io::IOBase;
use crate::{Error, Field, Result, Scalar};

/// What a manifest's entries describe.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ManifestContent {
    /// Data files holding rows.
    #[default]
    Data,
    /// Delete files removing rows.
    Deletes,
}

impl ManifestContent {
    /// Return the integer Iceberg stores for this content.
    pub const fn code(self) -> i32 {
        match self {
            Self::Data => 0,
            Self::Deletes => 1,
        }
    }

    /// Read the content one stored integer names.
    ///
    /// # Errors
    ///
    /// Returns an error naming the value when it is neither zero nor one.
    pub fn from_code(code: i64) -> Result<Self> {
        match code {
            0 => Ok(Self::Data),
            1 => Ok(Self::Deletes),
            other => Err(invalid(format_smolstr!(
                "expected a manifest content of 0 (data) or 1 (deletes), got {other}"
            ))),
        }
    }
}

/// What one snapshot did to a data file.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum EntryStatus {
    /// Carried over from an earlier snapshot.
    #[default]
    Existing,
    /// Added by the snapshot that wrote this manifest.
    Added,
    /// Removed by the snapshot that wrote this manifest.
    Deleted,
}

impl EntryStatus {
    /// Return the integer Iceberg stores for this status.
    pub const fn code(self) -> i32 {
        match self {
            Self::Existing => 0,
            Self::Added => 1,
            Self::Deleted => 2,
        }
    }

    /// Read the status one stored integer names.
    ///
    /// # Errors
    ///
    /// Returns an error naming the value when it is outside zero through two.
    pub fn from_code(code: i64) -> Result<Self> {
        match code {
            0 => Ok(Self::Existing),
            1 => Ok(Self::Added),
            2 => Ok(Self::Deleted),
            other => Err(invalid(format_smolstr!(
                "expected a manifest entry status of 0 (existing), 1 (added), or 2 (deleted), got \
                 {other}"
            ))),
        }
    }
}

/// The encoding one data file uses.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum FileFormat {
    /// Apache Parquet, which is what this module writes.
    #[default]
    Parquet,
    /// Apache Avro.
    Avro,
    /// Apache ORC.
    Orc,
}

impl FromStr for FileFormat {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.to_ascii_uppercase().as_str() {
            "PARQUET" => Ok(Self::Parquet),
            "AVRO" => Ok(Self::Avro),
            "ORC" => Ok(Self::Orc),
            other => Err(invalid(format_smolstr!(
                "expected an Iceberg file format of PARQUET, AVRO, or ORC, got {other:?}"
            ))),
        }
    }
}

impl fmt::Display for FileFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Parquet => "PARQUET",
            Self::Avro => "AVRO",
            Self::Orc => "ORC",
        })
    }
}

/// One data file, its partition tuple, and the statistics its writer reported.
#[derive(Clone, Debug, Default)]
pub struct DataFile {
    /// Zero for rows, one for position deletes, two for equality deletes.
    pub content: i32,
    /// The file's location, as a URI.
    pub file_path: SmolStr,
    /// The encoding the file uses.
    pub file_format: FileFormat,
    /// One value per partition field of the spec, in spec order.
    pub partition: Vec<Scalar>,
    /// Rows in the file.
    pub record_count: i64,
    /// Size of the file in bytes.
    pub file_size_in_bytes: i64,
    /// Stored bytes per column, keyed by field id.
    pub column_sizes: Vec<(i32, i64)>,
    /// Values per column, keyed by field id.
    pub value_counts: Vec<(i32, i64)>,
    /// Nulls per column, keyed by field id.
    pub null_value_counts: Vec<(i32, i64)>,
    /// Not-a-number values per column, keyed by field id.
    pub nan_value_counts: Vec<(i32, i64)>,
    /// Serialized minimum per column, keyed by field id.
    pub lower_bounds: Vec<(i32, Vec<u8>)>,
    /// Serialized maximum per column, keyed by field id.
    pub upper_bounds: Vec<(i32, Vec<u8>)>,
    /// Byte offsets a reader may split the file at.
    pub split_offsets: Vec<i64>,
    /// The sort order the file was written in, when one applies.
    pub sort_order_id: Option<i32>,
    /// First assigned row identifier, added in v3 for row lineage.
    pub first_row_id: Option<i64>,
}

#[derive(Eq, Hash, Ord, PartialEq, PartialOrd)]
struct DataFileIdentity<'a> {
    content: i32,
    file_path: &'a SmolStr,
    file_format: FileFormat,
    partition: &'a [Scalar],
    record_count: i64,
    file_size_in_bytes: i64,
    column_sizes: Vec<&'a (i32, i64)>,
    value_counts: Vec<&'a (i32, i64)>,
    null_value_counts: Vec<&'a (i32, i64)>,
    nan_value_counts: Vec<&'a (i32, i64)>,
    lower_bounds: Vec<&'a (i32, Vec<u8>)>,
    upper_bounds: Vec<&'a (i32, Vec<u8>)>,
    split_offsets: &'a [i64],
    sort_order_id: Option<i32>,
    first_row_id: Option<i64>,
}

impl DataFile {
    /// Return a deterministic hash of this complete data-file description.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }

    fn identity(&self) -> DataFileIdentity<'_> {
        DataFileIdentity {
            content: self.content,
            file_path: &self.file_path,
            file_format: self.file_format,
            partition: &self.partition,
            record_count: self.record_count,
            file_size_in_bytes: self.file_size_in_bytes,
            column_sizes: crate::generic::sorted_pairs(&self.column_sizes),
            value_counts: crate::generic::sorted_pairs(&self.value_counts),
            null_value_counts: crate::generic::sorted_pairs(&self.null_value_counts),
            nan_value_counts: crate::generic::sorted_pairs(&self.nan_value_counts),
            lower_bounds: crate::generic::sorted_pairs(&self.lower_bounds),
            upper_bounds: crate::generic::sorted_pairs(&self.upper_bounds),
            split_offsets: &self.split_offsets,
            sort_order_id: self.sort_order_id,
            first_row_id: self.first_row_id,
        }
    }
}

impl PartialEq for DataFile {
    fn eq(&self, other: &Self) -> bool {
        self.identity() == other.identity()
    }
}

impl Eq for DataFile {}

impl PartialOrd for DataFile {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DataFile {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.identity().cmp(&other.identity())
    }
}

impl Hash for DataFile {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identity().hash(state);
    }
}

/// One manifest row: a data file plus what the snapshot did to it.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ManifestEntry {
    /// Whether the snapshot added, kept, or removed the file.
    pub status: EntryStatus,
    /// The snapshot that produced this entry.
    pub snapshot_id: Option<i64>,
    /// Commit order of the data, inherited from the manifest when absent.
    pub sequence_number: Option<i64>,
    /// Commit order of the file, inherited from the manifest when absent.
    pub file_sequence_number: Option<i64>,
    /// The file itself.
    pub data_file: DataFile,
}

impl ManifestEntry {
    /// Return a deterministic hash of this complete manifest entry.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }

    /// Describe a newly written data file.
    pub const fn added(snapshot_id: i64, data_file: DataFile) -> Self {
        Self {
            status: EntryStatus::Added,
            snapshot_id: Some(snapshot_id),
            sequence_number: None,
            file_sequence_number: None,
            data_file,
        }
    }

    /// Carry an entry an earlier snapshot wrote into a new manifest.
    ///
    /// A rewritten manifest keeps the commit order the file was first written
    /// with, because that order is what tells a reader which data a delete file
    /// applies to. Inheritance only fills in an entry the *current* snapshot
    /// added, so an existing entry has to spell its numbers out.
    pub fn existing(&self) -> Self {
        Self {
            status: EntryStatus::Existing,
            snapshot_id: self.snapshot_id,
            sequence_number: self.sequence_number,
            file_sequence_number: self.file_sequence_number,
            data_file: self.data_file.clone(),
        }
    }

    /// Fill in the numbers the manifest records rather than the entry.
    ///
    /// An added entry leaves its snapshot and sequence numbers null so one
    /// manifest can be reused by the commit that adds it; the manifest list row
    /// carries them instead. A reader has to apply that inheritance before the
    /// numbers mean anything, and this is where it happens - once, in the one
    /// place manifests are read.
    pub const fn inherit(&mut self, manifest: &ManifestFile) {
        if self.snapshot_id.is_none() {
            self.snapshot_id = Some(manifest.added_snapshot_id);
        }
        if self.sequence_number.is_none() {
            self.sequence_number = Some(manifest.sequence_number);
        }
        if self.file_sequence_number.is_none() {
            self.file_sequence_number = Some(manifest.sequence_number);
        }
    }

    /// Return whether this entry still contributes rows to a scan.
    pub const fn is_live(&self) -> bool {
        !matches!(self.status, EntryStatus::Deleted)
    }
}

/// What one manifest holds for one partition field, across all its files.
///
/// This is the level a planner prunes at first: a manifest whose summary for a
/// partition column excludes a value cannot name a file that holds it, so the
/// whole manifest is skipped without being read.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FieldSummary {
    /// Whether any file in the manifest has a null value for the field.
    pub contains_null: bool,
    /// Whether any file has a not-a-number value, when the writer knew.
    pub contains_nan: Option<bool>,
    /// Serialized minimum across the manifest's files, when one applies.
    pub lower_bound: Option<Vec<u8>>,
    /// Serialized maximum across the manifest's files, when one applies.
    pub upper_bound: Option<Vec<u8>>,
}

impl FieldSummary {
    /// Return a deterministic hash of this complete field summary.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }
}

/// One row of a manifest list: a manifest and what it summarizes.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ManifestFile {
    /// The manifest's location, as a URI.
    pub manifest_path: SmolStr,
    /// Size of the manifest in bytes.
    pub manifest_length: i64,
    /// The partition spec the manifest's entries were written under.
    pub partition_spec_id: i32,
    /// Whether the manifest lists data files or delete files.
    pub content: ManifestContent,
    /// Commit order assigned when the manifest was added.
    pub sequence_number: i64,
    /// Lowest commit order of any entry in the manifest.
    pub min_sequence_number: i64,
    /// The snapshot that added the manifest.
    pub added_snapshot_id: i64,
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
    /// One summary per partition field of the manifest's spec, in spec order.
    pub partitions: Vec<FieldSummary>,
    /// First assigned row identifier, added in v3 for row lineage.
    pub first_row_id: Option<i64>,
}

impl ManifestFile {
    /// Return a deterministic hash of this complete manifest-file description.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }
}

/// Read every entry of the manifest a handle holds.
///
/// # Errors
///
/// Returns an error when the bytes are not an Avro manifest or an entry is
/// missing a field the specification requires.
pub fn read_manifest<H: IOBase + ?Sized>(handle: &H) -> Result<Vec<ManifestEntry>> {
    let container = crate::avro::read_container(handle)?;
    let mut entries = Vec::with_capacity(container.rows.len());
    for row in &container.rows {
        entries.push(entry_from_value(row)?);
    }
    Ok(entries)
}

/// Read the partition spec a manifest declares in its Avro header.
///
/// # Errors
///
/// Returns an error when the header's `partition-spec` is not a spec document.
pub fn read_manifest_spec<H: IOBase + ?Sized>(handle: &H) -> Result<PartitionSpec> {
    let container = crate::avro::read_container(handle)?;
    let Some(encoded) = container.get("partition-spec") else {
        return Ok(PartitionSpec::unpartitioned());
    };
    let mut spec = PartitionSpec::from_json(&crate::json::from_utf8(encoded)?)?;
    if let Some(id) = container
        .get("partition-spec-id")
        .and_then(|id| id.parse::<i32>().ok())
    {
        spec.spec_id = id;
    }
    Ok(spec)
}

/// Replace a handle's bytes with a manifest listing `entries`.
///
/// The manifest carries the table schema and the partition spec in its Avro
/// header, which is what makes it self-describing: a reader recovers the
/// partition tuple's shape without consulting the table metadata again.
///
/// # Errors
///
/// Returns an error when the schema or spec cannot be projected, when an entry
/// does not fit the derived Avro schema, or when the write fails.
pub fn write_manifest<H: IOBase + ?Sized>(
    handle: &mut H,
    version: FormatVersion,
    schema: &Field,
    spec: &PartitionSpec,
    entries: &[ManifestEntry],
) -> Result<()> {
    let partition = spec.partition_field(schema)?;
    let avro_schema = manifest_entry_schema(version, &partition)?;

    let mut rows = Vec::with_capacity(entries.len());
    for entry in entries {
        rows.push(entry_to_value(entry, version, &partition)?);
    }

    let schema_json = super::schema_to_json(schema)?;
    let schema_text = String::from_utf8_lossy(&crate::json::into_bytes(&schema_json)?).into_owned();
    let spec_text =
        String::from_utf8_lossy(&crate::json::into_bytes(&spec.clone().into_v1_json()?)?)
            .into_owned();
    let spec_id = spec.spec_id.to_string();
    let format_version = version.number().to_string();
    let metadata = [
        ("schema", schema_text.as_str()),
        (
            "schema-id",
            schema
                .iceberg()
                .get(super::schema::SCHEMA_ID)
                .unwrap_or("0"),
        ),
        ("partition-spec", spec_text.as_str()),
        ("partition-spec-id", spec_id.as_str()),
        ("format-version", format_version.as_str()),
        ("content", "data"),
    ];
    crate::avro::write_container(handle, &avro_schema, &metadata, &rows)
}

/// Read every manifest a manifest list names.
///
/// # Errors
///
/// Returns an error when the bytes are not an Avro manifest list or a row is
/// missing a field the specification requires.
pub fn read_manifest_list<H: IOBase + ?Sized>(handle: &H) -> Result<Vec<ManifestFile>> {
    let container = crate::avro::read_container(handle)?;
    let mut manifests = Vec::with_capacity(container.rows.len());
    for row in &container.rows {
        manifests.push(manifest_from_value(row)?);
    }
    Ok(manifests)
}

/// Replace a handle's bytes with a manifest list naming `manifests`.
///
/// # Errors
///
/// Returns an error when a row does not fit the derived Avro schema or when
/// the write fails.
pub fn write_manifest_list<H: IOBase + ?Sized>(
    handle: &mut H,
    version: FormatVersion,
    snapshot_id: i64,
    parent_snapshot_id: Option<i64>,
    sequence_number: i64,
    manifests: &[ManifestFile],
) -> Result<()> {
    let avro_schema = manifest_list_schema(version)?;
    let mut rows = Vec::with_capacity(manifests.len());
    for manifest in manifests {
        rows.push(manifest_to_value(manifest, version)?);
    }
    let snapshot_text = snapshot_id.to_string();
    let parent_text = parent_snapshot_id.map_or_else(|| "null".to_owned(), |id| id.to_string());
    let sequence_text = sequence_number.to_string();
    let format_version = version.number().to_string();
    let mut metadata = vec![
        ("snapshot-id", snapshot_text.as_str()),
        ("parent-snapshot-id", parent_text.as_str()),
        ("format-version", format_version.as_str()),
    ];
    if version >= FormatVersion::V2 {
        metadata.insert(2, ("sequence-number", sequence_text.as_str()));
    }
    crate::avro::write_container(handle, &avro_schema, &metadata, &rows)
}

/// Build the Avro schema one manifest's entries are written against.
///
/// # Errors
///
/// Returns an error only when a mapping cannot be built.
fn manifest_entry_schema(version: FormatVersion, partition: &Field) -> Result<Scalar> {
    let mut data_file = Vec::new();
    if version >= FormatVersion::V2 {
        data_file.push(required("content", 134, Scalar::from("int"))?);
    }
    data_file.push(required("file_path", 100, Scalar::from("string"))?);
    data_file.push(required("file_format", 101, Scalar::from("string"))?);
    data_file.push(required(
        "partition",
        102,
        partition_record(partition, "r102")?,
    )?);
    data_file.push(required("record_count", 103, Scalar::from("long"))?);
    data_file.push(required("file_size_in_bytes", 104, Scalar::from("long"))?);
    if version == FormatVersion::V1 {
        // v1 required a block size that no reader has ever used.
        data_file.push(required("block_size_in_bytes", 105, Scalar::from("long"))?);
    }
    data_file.push(optional("column_sizes", 108, avro_map(117, 118, "long")?)?);
    data_file.push(optional("value_counts", 109, avro_map(119, 120, "long")?)?);
    data_file.push(optional(
        "null_value_counts",
        110,
        avro_map(121, 122, "long")?,
    )?);
    data_file.push(optional(
        "nan_value_counts",
        137,
        avro_map(138, 139, "long")?,
    )?);
    data_file.push(optional("lower_bounds", 125, avro_map(126, 127, "bytes")?)?);
    data_file.push(optional("upper_bounds", 128, avro_map(129, 130, "bytes")?)?);
    data_file.push(optional("key_metadata", 131, Scalar::from("bytes"))?);
    data_file.push(optional(
        "split_offsets",
        132,
        avro_array(133, Scalar::from("long"))?,
    )?);
    if version >= FormatVersion::V2 {
        data_file.push(optional(
            "equality_ids",
            135,
            avro_array(136, Scalar::from("long"))?,
        )?);
    }
    data_file.push(optional("sort_order_id", 140, Scalar::from("int"))?);
    if version >= FormatVersion::V3 {
        data_file.push(optional("first_row_id", 142, Scalar::from("long"))?);
        data_file.push(optional(
            "referenced_data_file",
            143,
            Scalar::from("string"),
        )?);
        data_file.push(optional("content_offset", 144, Scalar::from("long"))?);
        data_file.push(optional(
            "content_size_in_bytes",
            145,
            Scalar::from("long"),
        )?);
    }

    let mut fields = vec![required("status", 0, Scalar::from("int"))?];
    if version == FormatVersion::V1 {
        fields.push(required("snapshot_id", 1, Scalar::from("long"))?);
    } else {
        fields.push(optional("snapshot_id", 1, Scalar::from("long"))?);
        fields.push(optional("sequence_number", 3, Scalar::from("long"))?);
        fields.push(optional("file_sequence_number", 4, Scalar::from("long"))?);
    }
    fields.push(required(
        "data_file",
        2,
        record("r2", Scalar::from_sequence(data_file))?,
    )?);

    record("manifest_entry", Scalar::from_sequence(fields))
}

/// Build the Avro schema a manifest list is written against.
///
/// # Errors
///
/// Returns an error only when a mapping cannot be built.
fn manifest_list_schema(version: FormatVersion) -> Result<Scalar> {
    let summary = record(
        "r508",
        Scalar::from_sequence(vec![
            required("contains_null", 509, Scalar::from("boolean"))?,
            optional("contains_nan", 518, Scalar::from("boolean"))?,
            optional("lower_bound", 510, Scalar::from("bytes"))?,
            optional("upper_bound", 511, Scalar::from("bytes"))?,
        ]),
    )?;

    let mut fields = vec![
        required("manifest_path", 500, Scalar::from("string"))?,
        required("manifest_length", 501, Scalar::from("long"))?,
        required("partition_spec_id", 502, Scalar::from("int"))?,
    ];
    if version >= FormatVersion::V2 {
        fields.push(required("content", 517, Scalar::from("int"))?);
        fields.push(required("sequence_number", 515, Scalar::from("long"))?);
        fields.push(required("min_sequence_number", 516, Scalar::from("long"))?);
    }
    fields.push(required("added_snapshot_id", 503, Scalar::from("long"))?);
    let counts: [(&str, i32, &str); 6] = [
        ("added_files_count", 504, "int"),
        ("existing_files_count", 505, "int"),
        ("deleted_files_count", 506, "int"),
        ("added_rows_count", 512, "long"),
        ("existing_rows_count", 513, "long"),
        ("deleted_rows_count", 514, "long"),
    ];
    for (name, id, kind) in counts {
        // v1 left every count optional; v2 made them all required.
        if version == FormatVersion::V1 {
            fields.push(optional(name, id, Scalar::from(kind))?);
        } else {
            fields.push(required(name, id, Scalar::from(kind))?);
        }
    }
    fields.push(optional("partitions", 507, avro_array(508, summary)?)?);
    fields.push(optional("key_metadata", 519, Scalar::from("bytes"))?);
    if version >= FormatVersion::V3 {
        fields.push(optional("first_row_id", 520, Scalar::from("long"))?);
    }

    record("manifest_file", Scalar::from_sequence(fields))
}

/// Build the Avro record a partition tuple is stored as.
fn partition_record(partition: &Field, name: &str) -> Result<Scalar> {
    let mut fields = Vec::with_capacity(partition.field_len());
    for child in partition.fields() {
        let id = child.parquet_field_id()?.ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a field id on the partition column {:?}",
                child.name()
            ))
        })?;
        // Every partition value is optional, because a spec may retire a field
        // and `void` produces nothing else.
        fields.push(optional(
            child.name(),
            id,
            Scalar::from(super::PrimitiveType::from_data_type(child.data_type())?.to_string()),
        )?);
    }
    record(name, Scalar::from_sequence(fields))
}

/// Build one Avro record schema.
fn record(name: &str, fields: Scalar) -> Result<Scalar> {
    Scalar::from_mapping([
        (Scalar::from("type"), Scalar::from("record")),
        (Scalar::from("name"), Scalar::from(name)),
        (Scalar::from("fields"), fields),
    ])
}

/// Build one required Avro record field carrying its Iceberg field id.
fn required(name: &str, id: i32, schema: Scalar) -> Result<Scalar> {
    Scalar::from_mapping([
        (Scalar::from("name"), Scalar::from(name)),
        (Scalar::from("field-id"), Scalar::from(i64::from(id))),
        (Scalar::from("type"), schema),
    ])
}

/// Build one optional Avro record field: a null-first union defaulting to null.
fn optional(name: &str, id: i32, schema: Scalar) -> Result<Scalar> {
    Scalar::from_mapping([
        (Scalar::from("name"), Scalar::from(name)),
        (Scalar::from("field-id"), Scalar::from(i64::from(id))),
        (
            Scalar::from("type"),
            Scalar::from_sequence([Scalar::from("null"), schema]),
        ),
        (Scalar::from("default"), Scalar::Null),
    ])
}

/// Build an Avro array with the element id a reader needs.
fn avro_array(element_id: i32, items: Scalar) -> Result<Scalar> {
    Scalar::from_mapping([
        (Scalar::from("type"), Scalar::from("array")),
        (
            Scalar::from("element-id"),
            Scalar::from(i64::from(element_id)),
        ),
        (Scalar::from("items"), items),
    ])
}

/// Build the array-of-pairs Iceberg uses where Avro cannot key a map by an int.
fn avro_map(key_id: i32, value_id: i32, value_type: &str) -> Result<Scalar> {
    let entry = Scalar::from_mapping([
        (Scalar::from("type"), Scalar::from("record")),
        (
            Scalar::from("name"),
            Scalar::from(format_smolstr!("k{key_id}_v{value_id}")),
        ),
        (
            Scalar::from("fields"),
            Scalar::from_sequence([
                Scalar::from_mapping([
                    (Scalar::from("name"), Scalar::from("key")),
                    (Scalar::from("type"), Scalar::from("int")),
                    (Scalar::from("field-id"), Scalar::from(i64::from(key_id))),
                ])?,
                Scalar::from_mapping([
                    (Scalar::from("name"), Scalar::from("value")),
                    (Scalar::from("type"), Scalar::from(value_type)),
                    (Scalar::from("field-id"), Scalar::from(i64::from(value_id))),
                ])?,
            ]),
        ),
    ])?;
    Scalar::from_mapping([
        (Scalar::from("type"), Scalar::from("array")),
        (Scalar::from("items"), entry),
        (Scalar::from("logicalType"), Scalar::from("map")),
    ])
}

/// Render an integer-keyed statistics map as the pair array Avro stores.
fn pairs<V: Clone + Into<Scalar>>(entries: &[(i32, V)]) -> Result<Scalar> {
    if entries.is_empty() {
        return Ok(Scalar::Null);
    }
    let mut rows = Vec::with_capacity(entries.len());
    for (key, value) in entries {
        rows.push(Scalar::from_mapping([
            (Scalar::from("key"), Scalar::from(i64::from(*key))),
            (Scalar::from("value"), value.clone().into()),
        ])?);
    }
    Ok(Scalar::from_sequence(rows))
}

/// Read a pair array back into an integer-keyed statistics map.
fn from_pairs(value: Option<&Scalar>) -> Vec<(i32, Scalar)> {
    value
        .map(|value| {
            value
                .sequence_iter()
                .filter_map(|row| {
                    let key = row.get_key_str("key")?.as_i64()?;
                    Some((i32::try_from(key).ok()?, row.get_key_str("value")?.clone()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Render one manifest entry as the Avro record it is stored as.
fn entry_to_value(
    entry: &ManifestEntry,
    version: FormatVersion,
    partition: &Field,
) -> Result<Scalar> {
    let file = &entry.data_file;
    if file.partition.len() != partition.field_len() {
        return Err(invalid(format_smolstr!(
            "expected {} partition values for {:?}, got {}",
            partition.field_len(),
            file.file_path,
            file.partition.len()
        )));
    }
    let mut tuple = Vec::with_capacity(file.partition.len());
    for (column, value) in partition.fields().iter().zip(&file.partition) {
        tuple.push((Scalar::from(column.name()), value.clone()));
    }

    let mut data_file = Vec::new();
    if version >= FormatVersion::V2 {
        data_file.push((
            Scalar::from("content"),
            Scalar::from(i64::from(file.content)),
        ));
    }
    data_file.push((
        Scalar::from("file_path"),
        Scalar::from(file.file_path.clone()),
    ));
    data_file.push((
        Scalar::from("file_format"),
        Scalar::from(file.file_format.to_string()),
    ));
    data_file.push((Scalar::from("partition"), Scalar::from_mapping(tuple)?));
    data_file.push((
        Scalar::from("record_count"),
        Scalar::from(file.record_count),
    ));
    data_file.push((
        Scalar::from("file_size_in_bytes"),
        Scalar::from(file.file_size_in_bytes),
    ));
    if version == FormatVersion::V1 {
        data_file.push((Scalar::from("block_size_in_bytes"), Scalar::from(0_i64)));
    }
    data_file.push((Scalar::from("column_sizes"), pairs(&file.column_sizes)?));
    data_file.push((Scalar::from("value_counts"), pairs(&file.value_counts)?));
    data_file.push((
        Scalar::from("null_value_counts"),
        pairs(&file.null_value_counts)?,
    ));
    data_file.push((
        Scalar::from("nan_value_counts"),
        pairs(&file.nan_value_counts)?,
    ));
    data_file.push((Scalar::from("lower_bounds"), pairs(&file.lower_bounds)?));
    data_file.push((Scalar::from("upper_bounds"), pairs(&file.upper_bounds)?));
    data_file.push((Scalar::from("key_metadata"), Scalar::Null));
    data_file.push((
        Scalar::from("split_offsets"),
        if file.split_offsets.is_empty() {
            Scalar::Null
        } else {
            Scalar::from_sequence(
                file.split_offsets
                    .iter()
                    .map(|offset| Scalar::from(*offset)),
            )
        },
    ));
    if version >= FormatVersion::V2 {
        data_file.push((Scalar::from("equality_ids"), Scalar::Null));
    }
    data_file.push((
        Scalar::from("sort_order_id"),
        file.sort_order_id
            .map_or(Scalar::Null, |id| Scalar::from(i64::from(id))),
    ));
    if version >= FormatVersion::V3 {
        data_file.push((
            Scalar::from("first_row_id"),
            file.first_row_id.map_or(Scalar::Null, Scalar::from),
        ));
        data_file.push((Scalar::from("referenced_data_file"), Scalar::Null));
        data_file.push((Scalar::from("content_offset"), Scalar::Null));
        data_file.push((Scalar::from("content_size_in_bytes"), Scalar::Null));
    }

    let mut row = vec![(
        Scalar::from("status"),
        Scalar::from(i64::from(entry.status.code())),
    )];
    row.push((
        Scalar::from("snapshot_id"),
        entry.snapshot_id.map_or(Scalar::Null, Scalar::from),
    ));
    if version >= FormatVersion::V2 {
        row.push((
            Scalar::from("sequence_number"),
            entry.sequence_number.map_or(Scalar::Null, Scalar::from),
        ));
        row.push((
            Scalar::from("file_sequence_number"),
            entry
                .file_sequence_number
                .map_or(Scalar::Null, Scalar::from),
        ));
    }
    row.push((Scalar::from("data_file"), Scalar::from_mapping(data_file)?));
    Scalar::from_mapping(row)
}

/// Read one manifest entry back from its Avro record.
fn entry_from_value(row: &Scalar) -> Result<ManifestEntry> {
    let status = EntryStatus::from_code(
        row.get_key_str("status")
            .and_then(Scalar::as_i64)
            .ok_or_else(|| invalid(SmolStr::new_static("expected a manifest entry \"status\"")))?,
    )?;
    let file = row.get_key_str("data_file").ok_or_else(|| {
        invalid(SmolStr::new_static(
            "expected a manifest entry \"data_file\"",
        ))
    })?;

    let file_path = file
        .get_key_str("file_path")
        .and_then(Scalar::as_str)
        .ok_or_else(|| invalid(SmolStr::new_static("expected a data file \"file_path\"")))?;
    let partition = file
        .get_key_str("partition")
        .map(|tuple| {
            if let Some(record) = tuple.as_record() {
                record.values().cloned().collect()
            } else {
                tuple
                    .mapping_iter()
                    .map(|(_, value)| value.clone())
                    .collect()
            }
        })
        .unwrap_or_default();

    Ok(ManifestEntry {
        status,
        snapshot_id: row.get_key_str("snapshot_id").and_then(Scalar::as_i64),
        sequence_number: row.get_key_str("sequence_number").and_then(Scalar::as_i64),
        file_sequence_number: row
            .get_key_str("file_sequence_number")
            .and_then(Scalar::as_i64),
        data_file: DataFile {
            content: file
                .get_key_str("content")
                .and_then(Scalar::as_i64)
                .and_then(|code| i32::try_from(code).ok())
                .unwrap_or_default(),
            file_path: SmolStr::new(file_path),
            file_format: FileFormat::from_str(
                file.get_key_str("file_format")
                    .and_then(Scalar::as_str)
                    .unwrap_or("PARQUET"),
            )?,
            partition,
            record_count: file
                .get_key_str("record_count")
                .and_then(Scalar::as_i64)
                .unwrap_or_default(),
            file_size_in_bytes: file
                .get_key_str("file_size_in_bytes")
                .and_then(Scalar::as_i64)
                .unwrap_or_default(),
            column_sizes: counts(file.get_key_str("column_sizes")),
            value_counts: counts(file.get_key_str("value_counts")),
            null_value_counts: counts(file.get_key_str("null_value_counts")),
            nan_value_counts: counts(file.get_key_str("nan_value_counts")),
            lower_bounds: bounds(file.get_key_str("lower_bounds")),
            upper_bounds: bounds(file.get_key_str("upper_bounds")),
            split_offsets: file
                .get_key_str("split_offsets")
                .map(|offsets| offsets.sequence_iter().filter_map(Scalar::as_i64).collect())
                .unwrap_or_default(),
            sort_order_id: file
                .get_key_str("sort_order_id")
                .and_then(Scalar::as_i64)
                .and_then(|id| i32::try_from(id).ok()),
            first_row_id: file.get_key_str("first_row_id").and_then(Scalar::as_i64),
        },
    })
}

/// Read a pair array as integer counts.
fn counts(value: Option<&Scalar>) -> Vec<(i32, i64)> {
    from_pairs(value)
        .into_iter()
        .filter_map(|(key, value)| Some((key, value.as_i64()?)))
        .collect()
}

/// Read a pair array as serialized bounds.
fn bounds(value: Option<&Scalar>) -> Vec<(i32, Vec<u8>)> {
    from_pairs(value)
        .into_iter()
        .filter_map(|(key, value)| Some((key, value.as_bytes()?.to_vec())))
        .collect()
}

/// Render one manifest list row as the Avro record it is stored as.
fn manifest_to_value(manifest: &ManifestFile, version: FormatVersion) -> Result<Scalar> {
    let mut row = vec![
        (
            Scalar::from("manifest_path"),
            Scalar::from(manifest.manifest_path.clone()),
        ),
        (
            Scalar::from("manifest_length"),
            Scalar::from(manifest.manifest_length),
        ),
        (
            Scalar::from("partition_spec_id"),
            Scalar::from(i64::from(manifest.partition_spec_id)),
        ),
    ];
    if version >= FormatVersion::V2 {
        row.push((
            Scalar::from("content"),
            Scalar::from(i64::from(manifest.content.code())),
        ));
        row.push((
            Scalar::from("sequence_number"),
            Scalar::from(manifest.sequence_number),
        ));
        row.push((
            Scalar::from("min_sequence_number"),
            Scalar::from(manifest.min_sequence_number),
        ));
    }
    row.push((
        Scalar::from("added_snapshot_id"),
        Scalar::from(manifest.added_snapshot_id),
    ));
    row.push((
        Scalar::from("added_files_count"),
        Scalar::from(i64::from(manifest.added_files_count)),
    ));
    row.push((
        Scalar::from("existing_files_count"),
        Scalar::from(i64::from(manifest.existing_files_count)),
    ));
    row.push((
        Scalar::from("deleted_files_count"),
        Scalar::from(i64::from(manifest.deleted_files_count)),
    ));
    row.push((
        Scalar::from("added_rows_count"),
        Scalar::from(manifest.added_rows_count),
    ));
    row.push((
        Scalar::from("existing_rows_count"),
        Scalar::from(manifest.existing_rows_count),
    ));
    row.push((
        Scalar::from("deleted_rows_count"),
        Scalar::from(manifest.deleted_rows_count),
    ));
    row.push((
        Scalar::from("partitions"),
        summaries_to_value(&manifest.partitions)?,
    ));
    row.push((Scalar::from("key_metadata"), Scalar::Null));
    if version >= FormatVersion::V3 {
        row.push((
            Scalar::from("first_row_id"),
            manifest.first_row_id.map_or(Scalar::Null, Scalar::from),
        ));
    }
    Scalar::from_mapping(row)
}

/// Read one manifest list row back from its Avro record.
fn manifest_from_value(row: &Scalar) -> Result<ManifestFile> {
    let manifest_path = row
        .get_key_str("manifest_path")
        .and_then(Scalar::as_str)
        .ok_or_else(|| {
            invalid(SmolStr::new_static(
                "expected a manifest list \"manifest_path\"",
            ))
        })?;
    // v1 named the file counts `added_data_files_count`; v2 renamed them.
    let count = |primary: &str, legacy: &str| -> i32 {
        row.get_key_str(primary)
            .or_else(|| row.get_key_str(legacy))
            .and_then(Scalar::as_i64)
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or_default()
    };
    Ok(ManifestFile {
        manifest_path: SmolStr::new(manifest_path),
        manifest_length: row
            .get_key_str("manifest_length")
            .and_then(Scalar::as_i64)
            .unwrap_or_default(),
        partition_spec_id: row
            .get_key_str("partition_spec_id")
            .and_then(Scalar::as_i64)
            .and_then(|id| i32::try_from(id).ok())
            .unwrap_or_default(),
        content: ManifestContent::from_code(
            row.get_key_str("content")
                .and_then(Scalar::as_i64)
                .unwrap_or_default(),
        )?,
        sequence_number: row
            .get_key_str("sequence_number")
            .and_then(Scalar::as_i64)
            .unwrap_or_default(),
        min_sequence_number: row
            .get_key_str("min_sequence_number")
            .and_then(Scalar::as_i64)
            .unwrap_or_default(),
        added_snapshot_id: row
            .get_key_str("added_snapshot_id")
            .and_then(Scalar::as_i64)
            .unwrap_or_default(),
        added_files_count: count("added_files_count", "added_data_files_count"),
        existing_files_count: count("existing_files_count", "existing_data_files_count"),
        deleted_files_count: count("deleted_files_count", "deleted_data_files_count"),
        added_rows_count: row
            .get_key_str("added_rows_count")
            .and_then(Scalar::as_i64)
            .unwrap_or_default(),
        existing_rows_count: row
            .get_key_str("existing_rows_count")
            .and_then(Scalar::as_i64)
            .unwrap_or_default(),
        deleted_rows_count: row
            .get_key_str("deleted_rows_count")
            .and_then(Scalar::as_i64)
            .unwrap_or_default(),
        partitions: summaries_from_value(row.get_key_str("partitions")),
        first_row_id: row.get_key_str("first_row_id").and_then(Scalar::as_i64),
    })
}

/// Render the per-partition-field summaries a manifest list row carries.
fn summaries_to_value(summaries: &[FieldSummary]) -> Result<Scalar> {
    if summaries.is_empty() {
        return Ok(Scalar::Null);
    }
    let mut rows = Vec::with_capacity(summaries.len());
    for summary in summaries {
        let bound = |bytes: Option<&Vec<u8>>| {
            bytes.map_or(Scalar::Null, |bytes| Scalar::from(bytes.as_slice()))
        };
        rows.push(Scalar::from_mapping([
            (
                Scalar::from("contains_null"),
                Scalar::Bool(summary.contains_null),
            ),
            (
                Scalar::from("contains_nan"),
                summary.contains_nan.map_or(Scalar::Null, Scalar::Bool),
            ),
            (
                Scalar::from("lower_bound"),
                bound(summary.lower_bound.as_ref()),
            ),
            (
                Scalar::from("upper_bound"),
                bound(summary.upper_bound.as_ref()),
            ),
        ])?);
    }
    Ok(Scalar::from_sequence(rows))
}

/// Read the per-partition-field summaries back, in spec order.
fn summaries_from_value(value: Option<&Scalar>) -> Vec<FieldSummary> {
    value
        .map(|value| {
            value
                .sequence_iter()
                .map(|row| FieldSummary {
                    contains_null: row
                        .get_key_str("contains_null")
                        .and_then(Scalar::as_bool)
                        .unwrap_or_default(),
                    contains_nan: row.get_key_str("contains_nan").and_then(Scalar::as_bool),
                    lower_bound: row
                        .get_key_str("lower_bound")
                        .and_then(Scalar::as_bytes)
                        .map(<[u8]>::to_vec),
                    upper_bound: row
                        .get_key_str("upper_bound")
                        .and_then(Scalar::as_bytes)
                        .map(<[u8]>::to_vec),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Report a malformed Iceberg manifest document.
fn invalid(reason: SmolStr) -> Error {
    Error::Codec {
        format: "iceberg",
        position: 0,
        reason,
    }
}

/// The compiled planning resolutions, keyed by the fingerprint of the raw
/// schema bytes.
///
/// A scan reads many manifests written by one writer, so the JSON parse, the
/// schema parse, and the resolution build happen once per writer schema
/// rather than once per manifest. The map is small by construction - a table
/// rarely sees more than a handful of writer schemas - and cleared outright
/// if it ever grows past its cap, which is simpler than an eviction order
/// nothing would exercise.
type PlanningPlans =
    std::collections::HashMap<(u64, bool), std::sync::Arc<crate::avro::Resolution>>;
static PLANNING_PLANS: std::sync::Mutex<Option<PlanningPlans>> = std::sync::Mutex::new(None);

/// How many compiled planning resolutions are retained.
const PLANNING_PLAN_CAP: usize = 64;

/// Read the entries a read-only scan plan needs, skipping the rest.
///
/// The reader schema is the writer's own schema projected down to the
/// planning columns - status, the sequence numbers, and the data file's
/// identity, partition tuple, and sizes - so decoding runs through a compiled
/// resolution plan whose skip steps jump every statistics map the plan will
/// never consult. When `with_stats` is set, which is a *filtered* plan, the
/// value counts, null counts, and bounds survive, because file pruning reads
/// them. This path serves reads only: a rewrite carries entries into new
/// manifests and must decode them whole with [`read_manifest`], because a
/// carried entry has to keep its statistics.
///
/// # Errors
///
/// Returns an error when the bytes are not an Avro manifest or a row does not
/// decode.
pub fn read_manifest_for_plan<H: IOBase + ?Sized>(
    handle: &H,
    with_stats: bool,
) -> Result<Vec<ManifestEntry>> {
    use crate::avro::container;
    use crate::avro::datum::Cursor;

    let limits = crate::Limits::default();
    let bytes = handle.read_all_bytes()?;
    if bytes.len() > limits.max_input_bytes() {
        return Err(invalid(format_smolstr!(
            "expected a manifest of at most {} bytes, got {}",
            limits.max_input_bytes(),
            bytes.len()
        )));
    }
    let mut cursor = Cursor::new(&bytes);
    container::check_magic(cursor.take(container::MAGIC.len())?)?;
    let (header, sync) = container::parse_header_entries(&mut cursor, limits)?;
    let schema_bytes =
        container::header_entry(&header, container::SCHEMA_KEY).ok_or_else(|| {
            invalid(SmolStr::new_static(
                "expected an Avro header carrying \"avro.schema\"",
            ))
        })?;
    let coding = match container::header_entry(&header, container::CODEC_KEY) {
        Some(value) => {
            crate::avro::container::BlockCoding::from_name(&String::from_utf8_lossy(value))?
        }
        None => crate::avro::container::BlockCoding::Shared(crate::Codec::Identity),
    };
    let plan = planning_plan(schema_bytes, with_stats, limits)?;

    let mut entries = Vec::new();
    // DESIGN: one decompression buffer for every block of every manifest this
    // call reads; its capacity carries across blocks so a wide manifest costs
    // one allocation, not one per block.
    let mut scratch = Vec::new();
    while !cursor.is_exhausted() {
        let count = cursor.long()?;
        let count = u64::try_from(count).map_err(|_| {
            invalid(format_smolstr!(
                "expected a non-negative Avro block count, got {count}"
            ))
        })?;
        let payload = cursor.bytes()?;
        let marker = cursor.take(16)?;
        if marker != sync {
            return Err(invalid(SmolStr::new_static(
                "expected the header's synchronization marker after an Avro block",
            )));
        }
        if count as usize > limits.max_nodes() {
            return Err(invalid(format_smolstr!(
                "expected at most {} rows in a block",
                limits.max_nodes()
            )));
        }
        coding.load_into(payload, limits, &mut scratch)?;
        let mut block = Cursor::new(&scratch);
        for _ in 0..count {
            let mut budget = limits.max_nodes();
            let row = plan.decode(&mut block, limits, &mut budget)?;
            entries.push(entry_from_value(&row)?);
        }
        if !block.is_exhausted() {
            return Err(invalid(format_smolstr!(
                "expected the block to end after {count} declared rows"
            )));
        }
    }
    Ok(entries)
}

/// Compile or fetch the planning resolution for one writer schema.
fn planning_plan(
    schema_bytes: &[u8],
    with_stats: bool,
    limits: crate::Limits,
) -> Result<std::sync::Arc<crate::avro::Resolution>> {
    let key = (crate::avro::schema::rabin(schema_bytes), with_stats);
    if let Ok(mut guard) = PLANNING_PLANS.lock() {
        if let Some(plan) = guard.as_ref().and_then(|plans| plans.get(&key)) {
            return Ok(plan.clone());
        }
        let plans = guard.get_or_insert_with(std::collections::HashMap::new);
        if plans.len() >= PLANNING_PLAN_CAP {
            plans.clear();
        }
    }
    let writer_json = crate::json::from_bytes_with_limits(schema_bytes, limits)?;
    let writer = crate::avro::Schema::from_json_with_limits(&writer_json, limits)?;
    let reader_json = planning_schema(&writer_json, with_stats);
    // Projection drops whole fields, and Avro lets a legal schema define a
    // named type inside one field and reference it by bare name from another;
    // a projection that drops the defining field orphans the reference. Such
    // a schema degrades to the identity plan - a full decode - rather than
    // failing the scan.
    let projected = crate::avro::Schema::from_json_with_limits(&reader_json, limits)
        .and_then(|reader| crate::avro::Resolution::from_schemas(&writer, &reader));
    let plan = std::sync::Arc::new(match projected {
        Ok(plan) => plan,
        Err(_) => crate::avro::Resolution::from_schemas(&writer, &writer)?,
    });
    if let Ok(mut guard) = PLANNING_PLANS.lock() {
        guard
            .get_or_insert_with(std::collections::HashMap::new)
            .insert(key, plan.clone());
    }
    Ok(plan)
}

/// Project a manifest-entry schema down to what a plan reads.
///
/// A schema whose shape is not the expected entry record is returned whole,
/// so an unusual writer degrades to a full decode rather than an error.
fn planning_schema(writer_json: &Scalar, with_stats: bool) -> Scalar {
    const ENTRY_KEEP: [&str; 4] = [
        "status",
        "snapshot_id",
        "sequence_number",
        "file_sequence_number",
    ];
    const FILE_KEEP: [&str; 6] = [
        "content",
        "file_path",
        "file_format",
        "partition",
        "record_count",
        "file_size_in_bytes",
    ];
    const FILE_STATS: [&str; 4] = [
        "value_counts",
        "null_value_counts",
        "lower_bounds",
        "upper_bounds",
    ];

    let whole = || writer_json.clone();
    let Some(fields) = writer_json
        .get_key_str("fields")
        .and_then(Scalar::as_sequence)
    else {
        return whole();
    };
    let mut kept = Vec::new();
    for field in fields {
        let Some(name) = field.get_key_str("name").and_then(Scalar::as_str) else {
            return whole();
        };
        if name == "data_file" {
            let Some(declared) = field.get_key_str("type") else {
                return whole();
            };
            let Some(filtered) = filter_record_fields(declared, &|child| {
                FILE_KEEP.contains(&child) || (with_stats && FILE_STATS.contains(&child))
            }) else {
                return whole();
            };
            let rebuilt = if field.as_record().is_some() {
                field.with_field("type", filtered)
            } else {
                field.with_key("type", filtered)
            };
            let Ok(rebuilt) = rebuilt else {
                return whole();
            };
            kept.push(rebuilt);
        } else if ENTRY_KEEP.contains(&name) {
            kept.push(field.clone());
        }
    }
    let fields = Scalar::from_sequence(kept);
    if writer_json.as_record().is_some() {
        writer_json.with_field("fields", fields)
    } else {
        writer_json.with_key("fields", fields)
    }
    .unwrap_or_else(|_| whole())
}

/// Rebuild a record schema JSON keeping only the fields `keep` accepts.
fn filter_record_fields(record: &Scalar, keep: &dyn Fn(&str) -> bool) -> Option<Scalar> {
    let fields = record.get_key_str("fields")?.as_sequence()?;
    let mut kept = Vec::new();
    for field in fields {
        if keep(field.get_key_str("name")?.as_str()?) {
            kept.push(field.clone());
        }
    }
    let fields = Scalar::from_sequence(kept);
    if record.as_record().is_some() {
        record.with_field("fields", fields)
    } else {
        record.with_key("fields", fields)
    }
    .ok()
}
