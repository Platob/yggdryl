//! Manifest lists, manifests, and the data files they describe.
//!
//! Official Iceberg 0.10.1 owns metadata/schema builders and, after bounded
//! input checks, manifest/list readers. Yggdryl owns [`crate::io::IOBase`]
//! publication, the public Arrow 59 boundary, data-file writes, and
//! deterministic manifest/list writers.
//!
//! Iceberg puts two levels of indirection between a snapshot and its rows. A
//! snapshot names one *manifest list*; each row of that list is a *manifest*;
//! each row of a manifest is one *data file* with its partition tuple and its
//! statistics. Writes use [`crate::avro`] through the table's handle, never a
//! directly opened path. The official writers remain unused because they
//! materialize unbounded output, produce random or order-dependent Avro bytes,
//! and encode Iceberg UUID partitions as Avro strings instead of `fixed[16]`.
//!
//! A narrow read repair handles the official parser's
//! Avro-conversion failure for a manifest declaring UUID as `fixed[16]`. It
//! removes only the unsupported UUID annotation in a bounded temporary view,
//! preserves the 16 physical bytes, and retries the official parser.
//!
//! The Avro schemas are built rather than stored, because the `partition`
//! column's shape comes from the table's partition spec. Everything else is the
//! specification's fixed field numbering, which is what lets another
//! implementation read a manifest this module wrote: an Avro reader matches
//! fields by `field-id`, not by name or position.

use std::hash::{Hash, Hasher};

use iceberg_official::spec::{
    DataContentType as OfficialDataContentType, DataFile as OfficialDataFile,
    DataFileFormat as OfficialDataFileFormat, FormatVersion as OfficialFormatVersion,
    Literal as OfficialLiteral, Manifest as OfficialManifest,
    ManifestContentType as OfficialManifestContent, ManifestEntry as OfficialManifestEntry,
    ManifestFile as OfficialManifestFile, ManifestList as OfficialManifestList,
    ManifestMetadata as OfficialManifestMetadata, ManifestStatus as OfficialManifestStatus,
    PartitionSpec as OfficialPartitionSpec, PrimitiveLiteral as OfficialPrimitiveLiteral,
    PrimitiveType as OfficialPrimitiveType, StructType as OfficialStructType,
    Transform as OfficialTransform, Type as OfficialType,
};
use smol_str::{SmolStr, format_smolstr};

use super::FormatVersion;
use super::partition::{PartitionField, PartitionSpec, Transform};
use crate::io::IOBase;
use crate::{Error, Field, MimeType, Result, Scalar, TimeUnit, Timezone};

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

/// One data file, its partition tuple, and the statistics its writer reported.
#[derive(Clone, Debug)]
pub struct DataFile {
    /// Zero for rows, one for position deletes, two for equality deletes.
    pub content: i32,
    /// The file's location, as a URI.
    pub file_path: SmolStr,
    /// The encoding the file uses.
    pub mime_type: MimeType,
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
    /// Implementation-specific encryption key metadata.
    pub key_metadata: Option<Vec<u8>>,
    /// Byte offsets a reader may split the file at.
    pub split_offsets: Vec<i64>,
    /// Schema field identifiers used by an equality-delete file.
    pub equality_ids: Option<Vec<i32>>,
    /// The sort order the file was written in, when one applies.
    pub sort_order_id: Option<i32>,
    /// First assigned row identifier, added in v3 for row lineage.
    pub first_row_id: Option<i64>,
    /// Data file referenced by position-delete metadata or a deletion vector.
    pub referenced_data_file: Option<SmolStr>,
    /// Byte offset of referenced v3 content.
    pub content_offset: Option<i64>,
    /// Byte length of referenced v3 content.
    pub content_size_in_bytes: Option<i64>,
}

#[derive(Eq, Hash, Ord, PartialEq, PartialOrd)]
struct DataFileIdentity<'a> {
    content: i32,
    file_path: &'a SmolStr,
    mime_type: &'a MimeType,
    partition: &'a [Scalar],
    record_count: i64,
    file_size_in_bytes: i64,
    column_sizes: Vec<&'a (i32, i64)>,
    value_counts: Vec<&'a (i32, i64)>,
    null_value_counts: Vec<&'a (i32, i64)>,
    nan_value_counts: Vec<&'a (i32, i64)>,
    lower_bounds: Vec<&'a (i32, Vec<u8>)>,
    upper_bounds: Vec<&'a (i32, Vec<u8>)>,
    key_metadata: &'a Option<Vec<u8>>,
    split_offsets: &'a [i64],
    equality_ids: &'a Option<Vec<i32>>,
    sort_order_id: Option<i32>,
    first_row_id: Option<i64>,
    referenced_data_file: &'a Option<SmolStr>,
    content_offset: Option<i64>,
    content_size_in_bytes: Option<i64>,
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
            mime_type: &self.mime_type,
            partition: &self.partition,
            record_count: self.record_count,
            file_size_in_bytes: self.file_size_in_bytes,
            column_sizes: crate::generic::sorted_pairs(&self.column_sizes),
            value_counts: crate::generic::sorted_pairs(&self.value_counts),
            null_value_counts: crate::generic::sorted_pairs(&self.null_value_counts),
            nan_value_counts: crate::generic::sorted_pairs(&self.nan_value_counts),
            lower_bounds: crate::generic::sorted_pairs(&self.lower_bounds),
            upper_bounds: crate::generic::sorted_pairs(&self.upper_bounds),
            key_metadata: &self.key_metadata,
            split_offsets: &self.split_offsets,
            equality_ids: &self.equality_ids,
            sort_order_id: self.sort_order_id,
            first_row_id: self.first_row_id,
            referenced_data_file: &self.referenced_data_file,
            content_offset: self.content_offset,
            content_size_in_bytes: self.content_size_in_bytes,
        }
    }
}

impl Default for DataFile {
    fn default() -> Self {
        Self {
            content: 0,
            file_path: SmolStr::new_static(""),
            mime_type: MimeType::PARQUET,
            partition: Vec::new(),
            record_count: 0,
            file_size_in_bytes: 0,
            column_sizes: Vec::new(),
            value_counts: Vec::new(),
            null_value_counts: Vec::new(),
            nan_value_counts: Vec::new(),
            lower_bounds: Vec::new(),
            upper_bounds: Vec::new(),
            key_metadata: None,
            split_offsets: Vec::new(),
            equality_ids: None,
            sort_order_id: None,
            first_row_id: None,
            referenced_data_file: None,
            content_offset: None,
            content_size_in_bytes: None,
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
    /// The snapshot id always inherits when absent. Sequence numbers inherit
    /// for added entries and for initial-sequence manifests; other entries must
    /// carry both sequence numbers explicitly.
    pub fn inherit(&mut self, manifest: &ManifestFile) -> Result<()> {
        self.snapshot_id.get_or_insert(manifest.added_snapshot_id);
        if self.status == EntryStatus::Added || manifest.sequence_number == 0 {
            self.sequence_number.get_or_insert(manifest.sequence_number);
            self.file_sequence_number
                .get_or_insert(manifest.sequence_number);
        }
        if self.sequence_number.is_none() || self.file_sequence_number.is_none() {
            return Err(invalid(format_smolstr!(
                "expected sequence numbers on {:?} entry for {:?}",
                self.status,
                self.data_file.file_path
            )));
        }
        Ok(())
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
    pub added_files_count: Option<i32>,
    /// Files the manifest marks existing.
    pub existing_files_count: Option<i32>,
    /// Files the manifest marks deleted.
    pub deleted_files_count: Option<i32>,
    /// Rows in the added files.
    pub added_rows_count: Option<i64>,
    /// Rows in the existing files.
    pub existing_rows_count: Option<i64>,
    /// Rows in the deleted files.
    pub deleted_rows_count: Option<i64>,
    /// One summary per partition field of the manifest's spec, in spec order.
    pub partitions: Vec<FieldSummary>,
    /// Implementation-specific encryption metadata for the manifest file.
    pub key_metadata: Option<Vec<u8>>,
    /// First assigned row identifier, added in v3 for row lineage.
    pub first_row_id: Option<i64>,
}

impl ManifestFile {
    /// Return a deterministic hash of this complete manifest-file description.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }

    /// Assign this manifest's v3 row range from the manifest-list cursor.
    ///
    /// This mirrors Apache Iceberg's manifest-list writer without adopting its
    /// buffered async `FileWrite` boundary. An already assigned data manifest
    /// keeps its range; an unassigned one reserves every added and existing
    /// row. Delete manifests never carry row ids.
    fn assign_first_row_id(&mut self, next_row_id: &mut Option<i64>) -> Result<()> {
        if self.content == ManifestContent::Deletes {
            self.first_row_id = None;
            return Ok(());
        }
        match (*next_row_id, self.first_row_id) {
            (Some(_), Some(first)) if first < 0 => Err(invalid(format_smolstr!(
                "expected a non-negative first_row_id, got {first}"
            ))),
            (Some(_), Some(_)) | (None, None) => Ok(()),
            (None, Some(first)) => Err(invalid(format_smolstr!(
                "expected a manifest-list first-row-id for assigned data manifest {:?}, got {first}",
                self.manifest_path
            ))),
            (Some(first), None) => {
                let existing = required_non_negative_count(
                    self.existing_rows_count,
                    "existing_rows_count",
                    &self.manifest_path,
                )?;
                let added = required_non_negative_count(
                    self.added_rows_count,
                    "added_rows_count",
                    &self.manifest_path,
                )?;
                let next = first
                    .checked_add(existing)
                    .and_then(|value| value.checked_add(added))
                    .ok_or_else(|| {
                        invalid(format_smolstr!(
                            "row id overflow for manifest {:?}: {first} + {existing} + {added}",
                            self.manifest_path
                        ))
                    })?;
                self.first_row_id = Some(first);
                *next_row_id = Some(next);
                Ok(())
            }
        }
    }
}

fn required_non_negative_count(count: Option<i64>, name: &str, path: &str) -> Result<i64> {
    match count {
        Some(count) if count >= 0 => Ok(count),
        Some(count) => Err(invalid(format_smolstr!(
            "expected non-negative {name} for manifest {path:?}, got {count}"
        ))),
        None => Err(invalid(format_smolstr!(
            "expected required {name} for manifest {path:?}, got null"
        ))),
    }
}

/// Read every entry of the manifest a handle holds.
///
/// # Errors
///
/// Returns an error when the bytes are not an Avro manifest or an entry is
/// missing a field the specification requires.
pub fn read_manifest<H: IOBase + ?Sized>(handle: &H) -> Result<Vec<ManifestEntry>> {
    let bytes = manifest_bytes(handle)?;
    let manifest = parse_manifest(&bytes)?;
    let (entries, metadata) = manifest.into_parts();
    let partition_type = metadata
        .partition_spec()
        .partition_type(metadata.schema())
        .map_err(Error::from_iceberg)?;
    entries
        .into_iter()
        .map(|entry| match std::sync::Arc::try_unwrap(entry) {
            Ok(entry) => entry_from_official_owned(entry, &partition_type),
            // Parsed manifests currently own every entry uniquely. Keep the
            // borrowed fallback safe if the official representation later
            // starts sharing entries internally.
            Err(entry) => entry_from_official(&entry, &partition_type),
        })
        .collect()
}

/// Read the partition spec a manifest declares in its Avro header.
///
/// # Errors
///
/// Returns an error when the header's `partition-spec` is not a spec document.
pub fn read_manifest_spec<H: IOBase + ?Sized>(handle: &H) -> Result<PartitionSpec> {
    let blocks = crate::avro::read_blocks(handle)?;
    let metadata = blocks
        .metadata_bytes()
        .iter()
        .map(|(key, value)| (key.to_string(), value.clone()))
        .collect();
    let metadata = OfficialManifestMetadata::parse(&metadata).map_err(Error::from_iceberg)?;
    partition_spec_from_official(metadata.partition_spec())
}

/// Recover the manifest-list row absent from a v1 direct-manifest snapshot.
pub(crate) fn read_v1_direct_manifest_file<H: IOBase + ?Sized>(
    handle: &H,
    manifest_path: &str,
    snapshot_id: i64,
) -> Result<ManifestFile> {
    let bytes = manifest_bytes(handle)?;
    let manifest = parse_manifest(&bytes)?;
    let (entries, metadata) = manifest.into_parts();
    let mut added_files = 0_i32;
    let mut existing_files = 0_i32;
    let mut deleted_files = 0_i32;
    let mut added_rows = 0_i64;
    let mut existing_rows = 0_i64;
    let mut deleted_rows = 0_i64;
    for entry in entries {
        let rows = signed_u64(entry.data_file().record_count(), "record count")?;
        let (files, records) = match entry.status {
            OfficialManifestStatus::Added => (&mut added_files, &mut added_rows),
            OfficialManifestStatus::Existing => (&mut existing_files, &mut existing_rows),
            OfficialManifestStatus::Deleted => (&mut deleted_files, &mut deleted_rows),
        };
        *files = files.checked_add(1).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected fewer than {} entries in v1 direct manifest {manifest_path:?}",
                i32::MAX
            ))
        })?;
        *records = records.checked_add(rows).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected record counts fitting i64 in v1 direct manifest {manifest_path:?}"
            ))
        })?;
    }
    let content = match metadata.content() {
        OfficialManifestContent::Data => ManifestContent::Data,
        OfficialManifestContent::Deletes => ManifestContent::Deletes,
    };
    Ok(ManifestFile {
        manifest_path: SmolStr::new(manifest_path),
        manifest_length: i64::try_from(bytes.len()).map_err(|_| {
            invalid(format_smolstr!(
                "expected v1 direct manifest length fitting i64, got {}",
                bytes.len()
            ))
        })?,
        partition_spec_id: metadata.partition_spec().spec_id(),
        content,
        sequence_number: 0,
        min_sequence_number: 0,
        added_snapshot_id: snapshot_id,
        added_files_count: Some(added_files),
        existing_files_count: Some(existing_files),
        deleted_files_count: Some(deleted_files),
        added_rows_count: Some(added_rows),
        existing_rows_count: Some(existing_rows),
        deleted_rows_count: Some(deleted_rows),
        // The direct v1 form has no manifest-list summaries. Omitting them is
        // conservative: planning opens the manifest instead of pruning it.
        partitions: Vec::new(),
        key_metadata: None,
        first_row_id: None,
    })
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

    let mut content = None;
    for entry in entries {
        let entry_content = match entry.data_file.content {
            0 => ManifestContent::Data,
            1 | 2 => ManifestContent::Deletes,
            other => {
                return Err(invalid(format_smolstr!(
                    "expected data-file content 0, 1, or 2, got {other}"
                )));
            }
        };
        if content.is_some_and(|current| current != entry_content) {
            return Err(invalid(SmolStr::new_static(
                "expected one manifest content kind, got mixed data and delete files",
            )));
        }
        content = Some(entry_content);
    }
    let content = content.unwrap_or_default();
    if version == FormatVersion::V1 && content == ManifestContent::Deletes {
        return Err(invalid(SmolStr::new_static(
            "expected only data files in an Iceberg v1 manifest, got delete files",
        )));
    }

    let mut rows = Vec::with_capacity(entries.len());
    for entry in entries {
        rows.push(entry_to_value(entry, version, &partition)?);
    }

    let schema_json = super::schema_into_json(schema)?;
    let schema_text = String::from_utf8_lossy(&crate::json::into_bytes(&schema_json)?).into_owned();
    let spec_text =
        String::from_utf8_lossy(&crate::json::into_bytes(&spec.clone().into_v1_json()?)?)
            .into_owned();
    let spec_id = spec.spec_id.to_string();
    let format_version = version.number().to_string();
    let content = match content {
        ManifestContent::Data => "data",
        ManifestContent::Deletes => "deletes",
    };
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
        ("content", content),
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
    let bytes = manifest_bytes(handle)?;
    let version = manifest_list_version(&bytes)?;
    OfficialManifestList::parse_with_version(&bytes, version)
        .map_err(Error::from_iceberg)?
        .consume_entries()
        .into_iter()
        .map(manifest_from_official)
        .collect()
}

/// Read one bounded object-container payload for the official byte parsers.
fn manifest_bytes<H: IOBase + ?Sized>(handle: &H) -> Result<Vec<u8>> {
    let limit = crate::Limits::default().max_input_bytes();
    let declared = handle.size();
    let declared_exceeds_limit = usize::try_from(declared).map_or(true, |size| size > limit);
    if declared_exceeds_limit {
        return Err(invalid(format_smolstr!(
            "expected an Iceberg object container of at most {limit} bytes, got {declared}"
        )));
    }

    let bytes = handle.read_all_bytes()?;
    if bytes.len() > limit {
        return Err(invalid(format_smolstr!(
            "expected an Iceberg object container of at most {limit} bytes, got {}",
            bytes.len()
        )));
    }
    Ok(bytes)
}

/// Validate collection semantics the official reader projects into maps.
fn parse_manifest(bytes: &[u8]) -> Result<OfficialManifest> {
    preflight_manifest_entries(bytes)?;
    match OfficialManifest::parse_avro(bytes) {
        Ok(manifest) => Ok(manifest),
        Err(error) => {
            // apache-avro 0.21 collapses string and fixed UUID schemas into
            // one length-prefixed node. Iceberg 0.10.1 therefore cannot read
            // the spec-required fixed(16) wire it declared. See
            // https://docs.rs/apache-avro/0.21.0/src/apache_avro/schema.rs.html.
            if error.message() != "Failure in conversion with avro" {
                return Err(Error::from_iceberg(error));
            }
            let Some(reader_view) = fixed_uuid_official_reader_view(bytes)? else {
                return Err(Error::from_iceberg(error));
            };
            OfficialManifest::parse_avro(&reader_view).map_err(|_| Error::from_iceberg(error))
        }
    }
}

/// Build an in-memory official-reader view of a fixed UUID manifest.
fn fixed_uuid_official_reader_view(bytes: &[u8]) -> Result<Option<Vec<u8>>> {
    let source = crate::io::Buffer::from(bytes);
    let container = crate::avro::read_container(&source)?;
    let schema = container.schema.into_json();
    if !contains_fixed_uuid(&schema) {
        return Ok(None);
    }

    let schema = strip_uuid_logical_type(&schema)?;
    let mut metadata = container.metadata;
    let encoded_schema = metadata
        .iter_mut()
        .find_map(|(name, value)| (name == "schema").then_some(value))
        .ok_or_else(|| {
            invalid(SmolStr::new_static(
                "expected schema metadata in a UUID-partitioned manifest",
            ))
        })?;
    let iceberg_schema = crate::json::from_utf8(encoded_schema)?;
    *encoded_schema = SmolStr::new(crate::json::into_utf8(&uuid_as_fixed(&iceberg_schema)?)?);

    let metadata_refs = metadata
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let mut output = crate::io::Buffer::new();
    crate::avro::write_container(&mut output, &schema, &metadata_refs, &container.rows)?;
    let output = output.into_bytes();
    let limit = crate::Limits::default().max_input_bytes();
    if output.len() > limit {
        return Err(invalid(format_smolstr!(
            "expected a UUID official-reader view of at most {limit} bytes, got {}",
            output.len()
        )));
    }
    Ok(Some(output))
}

/// Return whether an Avro schema contains Iceberg's fixed UUID wire node.
fn contains_fixed_uuid(value: &Scalar) -> bool {
    if value.get_key_str("type").and_then(Scalar::as_str) == Some("fixed")
        && value.get_key_str("size").and_then(Scalar::as_i64) == Some(16)
        && value.get_key_str("logicalType").and_then(Scalar::as_str) == Some("uuid")
    {
        return true;
    }
    value.as_mapping().map_or_else(
        || value.iter().any(contains_fixed_uuid),
        |entries| entries.iter().any(|(_, value)| contains_fixed_uuid(value)),
    )
}

/// Remove only the UUID annotation that apache-avro 0.21 cannot preserve.
fn strip_uuid_logical_type(value: &Scalar) -> Result<Scalar> {
    if let Some(values) = value.as_sequence() {
        return values
            .iter()
            .map(strip_uuid_logical_type)
            .collect::<Result<Vec<_>>>()
            .map(Scalar::from_sequence);
    }
    if value.as_record().is_none() && value.as_mapping().is_none() {
        return Ok(value.clone());
    }

    let fixed_uuid = value.get_key_str("type").and_then(Scalar::as_str) == Some("fixed")
        && value.get_key_str("size").and_then(Scalar::as_i64) == Some(16)
        && value.get_key_str("logicalType").and_then(Scalar::as_str) == Some("uuid");
    let mut output = value.clone();
    for key in value.keys() {
        if fixed_uuid && key == "logicalType" {
            output = object_without(&output, key)?;
        } else if let Some(child) = value.get_key_str(key) {
            output = object_with(&output, key, strip_uuid_logical_type(child)?)?;
        }
    }
    Ok(output)
}

/// Temporarily spell UUID as the equivalent physical Iceberg fixed type.
fn uuid_as_fixed(value: &Scalar) -> Result<Scalar> {
    if value.as_str() == Some("uuid") {
        return Ok(Scalar::from("fixed[16]"));
    }
    let Some(kind) = value.get_key_str("type").and_then(Scalar::as_str) else {
        return Ok(value.clone());
    };
    let mut output = value.clone();
    match kind {
        "struct" => {
            let fields = value
                .get_key_str("fields")
                .and_then(Scalar::as_sequence)
                .ok_or_else(|| invalid(SmolStr::new_static("expected Iceberg struct fields")))?;
            let fields = fields
                .iter()
                .map(|field| {
                    let field_type = field.get_key_str("type").ok_or_else(|| {
                        invalid(SmolStr::new_static("expected an Iceberg field type"))
                    })?;
                    object_with(field, "type", uuid_as_fixed(field_type)?)
                })
                .collect::<Result<Vec<_>>>()?;
            output = object_with(&output, "fields", Scalar::from_sequence(fields))?;
        }
        "list" => {
            let element = value.get_key_str("element").ok_or_else(|| {
                invalid(SmolStr::new_static("expected an Iceberg list element type"))
            })?;
            output = object_with(&output, "element", uuid_as_fixed(element)?)?;
        }
        "map" => {
            for name in ["key", "value"] {
                let child = value.get_key_str(name).ok_or_else(|| {
                    invalid(format_smolstr!("expected an Iceberg map {name} type"))
                })?;
                output = object_with(&output, name, uuid_as_fixed(child)?)?;
            }
        }
        _ => {}
    }
    Ok(output)
}

/// Replace one JSON-object member while preserving its scalar object shape.
fn object_with(value: &Scalar, key: &str, child: Scalar) -> Result<Scalar> {
    if value.as_record().is_some() {
        value.with_field(key, child)
    } else {
        value.with_key(key, child)
    }
}

/// Remove one JSON-object member while preserving its scalar object shape.
fn object_without(value: &Scalar, key: &str) -> Result<Scalar> {
    if value.as_record().is_some() {
        value.without_field(key)
    } else {
        value.without_key(key)
    }
}

/// Decode one bounded row at a time before map conversion can erase duplicates.
fn preflight_manifest_entries(bytes: &[u8]) -> Result<()> {
    use crate::avro::container;
    use crate::avro::datum::{Cursor, DatumCodec};

    let limits = crate::Limits::default();
    let mut cursor = Cursor::new(bytes);
    container::check_magic(cursor.take(container::MAGIC.len())?)?;
    let header = container::parse_header(&mut cursor, limits)?;
    let datum = DatumCodec {
        names: &header.schema.names,
        limits,
    };
    let mut rows = 0_usize;
    // DESIGN: retain only the current decompressed block and current row. The
    // official parse owns the final entries after this bounded preflight.
    let mut scratch = Vec::new();
    while !cursor.is_exhausted() {
        let count = cursor.long()?;
        let count = u64::try_from(count).map_err(|_| {
            invalid(format_smolstr!(
                "expected a non-negative Avro block count, got {count}"
            ))
        })?;
        let payload = cursor.bytes()?;
        let marker = cursor.take(container::SYNC_LEN)?;
        if marker != header.sync {
            return Err(invalid(SmolStr::new_static(
                "expected the header's synchronization marker after an Avro block",
            )));
        }
        let count = usize::try_from(count).unwrap_or(usize::MAX);
        let end = rows.checked_add(count).ok_or_else(|| {
            invalid(SmolStr::new_static(
                "expected a manifest row count fitting usize",
            ))
        })?;
        if count > limits.max_nodes() || end > limits.max_nodes() {
            return Err(invalid(format_smolstr!(
                "expected at most {} rows in a manifest",
                limits.max_nodes()
            )));
        }
        header.coding.load_into(payload, limits, &mut scratch)?;
        let mut block = Cursor::new(&scratch);
        for index in rows..end {
            let mut budget = datum.budget();
            let row = datum.decode(&header.schema.node, &mut block, 0, &mut budget)?;
            preflight_manifest_entry(&row, index)?;
        }
        if !block.is_exhausted() {
            return Err(invalid(format_smolstr!(
                "expected the block to end after {count} declared rows"
            )));
        }
        rows = end;
    }
    Ok(())
}

/// Reject lossy metric maps and invalid split offsets in one raw entry.
fn preflight_manifest_entry(row: &Scalar, index: usize) -> Result<()> {
    let file = row.get_key_str("data_file").ok_or_else(|| {
        invalid(format_smolstr!(
            "expected data_file in manifest entry {index}"
        ))
    })?;
    for name in [
        "column_sizes",
        "value_counts",
        "null_value_counts",
        "nan_value_counts",
    ] {
        preflight_metric_map(file, name, true, index)?;
    }
    for name in ["lower_bounds", "upper_bounds"] {
        preflight_metric_map(file, name, false, index)?;
    }
    preflight_split_offsets(file, index)
}

/// Validate one Iceberg integer-keyed map while it is still an Avro pair array.
fn preflight_metric_map(
    file: &Scalar,
    name: &'static str,
    counts: bool,
    entry_index: usize,
) -> Result<()> {
    let Some(value) = file.get_key_str(name) else {
        return Ok(());
    };
    if value.is_null() {
        return Ok(());
    }
    let entries = value.as_sequence().ok_or_else(|| {
        invalid(format_smolstr!(
            "expected {name} as an Avro pair array in manifest entry {entry_index}"
        ))
    })?;
    let mut ids = std::collections::HashSet::with_capacity(entries.len());
    for (pair_index, pair) in entries.iter().enumerate() {
        let id = pair
            .get_key_str("key")
            .and_then(Scalar::as_i64)
            .and_then(|id| i32::try_from(id).ok())
            .ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected an i32 field id in {name}[{pair_index}] of manifest entry \
                     {entry_index}"
                ))
            })?;
        if id <= 0 {
            return Err(invalid(format_smolstr!(
                "expected a positive field id in {name}[{pair_index}] of manifest entry \
                 {entry_index}, got {id}"
            )));
        }
        if !ids.insert(id) {
            return Err(invalid(format_smolstr!(
                "expected unique field ids in {name} of manifest entry {entry_index}, got \
                 duplicate {id}"
            )));
        }
        let value = pair.get_key_str("value").ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a value in {name}[{pair_index}] of manifest entry {entry_index}"
            ))
        })?;
        if counts {
            let count = value.as_i64().ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected an integer count in {name}[{pair_index}] of manifest entry \
                     {entry_index}"
                ))
            })?;
            if count < 0 {
                return Err(invalid(format_smolstr!(
                    "expected a non-negative count in {name}[{pair_index}] of manifest entry \
                     {entry_index}, got {count}"
                )));
            }
        } else if value.as_bytes().is_none() {
            return Err(invalid(format_smolstr!(
                "expected bytes in {name}[{pair_index}] of manifest entry {entry_index}"
            )));
        }
    }
    Ok(())
}

/// Validate split offsets before the official view owns them.
fn preflight_split_offsets(file: &Scalar, entry_index: usize) -> Result<()> {
    let Some(value) = file.get_key_str("split_offsets") else {
        return Ok(());
    };
    if value.is_null() {
        return Ok(());
    }
    let offsets = value.as_sequence().ok_or_else(|| {
        invalid(format_smolstr!(
            "expected split_offsets as an array in manifest entry {entry_index}"
        ))
    })?;
    let mut previous = None;
    for (offset_index, value) in offsets.iter().enumerate() {
        let offset = value.as_i64().ok_or_else(|| {
            invalid(format_smolstr!(
                "expected an integer in split_offsets[{offset_index}] of manifest entry \
                 {entry_index}"
            ))
        })?;
        if offset < 0 || previous.is_some_and(|previous| previous >= offset) {
            return Err(invalid(format_smolstr!(
                "expected non-negative strictly ascending split_offsets in manifest entry \
                 {entry_index}, got {offsets:?}"
            )));
        }
        previous = Some(offset);
    }
    Ok(())
}

/// Recover the table format version the manifest-list parser requires.
fn manifest_list_version(bytes: &[u8]) -> Result<OfficialFormatVersion> {
    use crate::avro::container;
    use crate::avro::datum::Cursor;

    let mut cursor = Cursor::new(bytes);
    container::check_magic(cursor.take(container::MAGIC.len())?)?;
    let (header, _) = container::parse_header_entries(&mut cursor, crate::Limits::default())?;
    let schema = container::header_entry(&header, container::SCHEMA_KEY).ok_or_else(|| {
        invalid(SmolStr::new_static(
            "expected a manifest list carrying an Avro schema",
        ))
    })?;
    let schema = crate::json::from_bytes(schema)?;
    let fields = schema
        .get_key_str("fields")
        .and_then(Scalar::as_sequence)
        .ok_or_else(|| {
            invalid(SmolStr::new_static(
                "expected a manifest-list Avro record schema",
            ))
        })?;
    let has = |name: &str| {
        fields.iter().any(|field| {
            field
                .get_key_str("name")
                .and_then(Scalar::as_str)
                .is_some_and(|candidate| candidate == name)
        })
    };

    let inferred = if has("first_row_id") {
        OfficialFormatVersion::V3
    } else if has("content") || has("added_files_count") {
        OfficialFormatVersion::V2
    } else {
        OfficialFormatVersion::V1
    };
    let Some(encoded) = container::header_entry(&header, "format-version") else {
        return Ok(inferred);
    };
    let version = std::str::from_utf8(encoded).map_err(|error| {
        invalid(format_smolstr!(
            "expected a UTF-8 manifest-list format-version, got {error}"
        ))
    })?;
    match version.trim() {
        // Iceberg has used both file-count spellings in v1. The official v2
        // reader carries aliases and v1 defaults, so it preserves the modern
        // spelling while the v1 reader remains authoritative for v1 rows.
        "1" if has("added_files_count") => Ok(OfficialFormatVersion::V2),
        "1" => Ok(OfficialFormatVersion::V1),
        "2" => Ok(OfficialFormatVersion::V2),
        "3" => Ok(OfficialFormatVersion::V3),
        other => Err(invalid(format_smolstr!(
            "expected a manifest-list format-version of 1, 2, or 3, got {other:?}"
        ))),
    }
}

/// Project one official manifest entry onto Yggdryl's stable public view.
fn entry_from_official(
    entry: &OfficialManifestEntry,
    partition_type: &OfficialStructType,
) -> Result<ManifestEntry> {
    Ok(ManifestEntry {
        status: match entry.status {
            OfficialManifestStatus::Existing => EntryStatus::Existing,
            OfficialManifestStatus::Added => EntryStatus::Added,
            OfficialManifestStatus::Deleted => EntryStatus::Deleted,
        },
        snapshot_id: entry.snapshot_id,
        sequence_number: entry.sequence_number,
        file_sequence_number: entry.file_sequence_number,
        data_file: data_file_from_official(entry.data_file(), partition_type)?,
    })
}

/// Project one uniquely owned official entry without cloning its wrapper.
fn entry_from_official_owned(
    entry: OfficialManifestEntry,
    partition_type: &OfficialStructType,
) -> Result<ManifestEntry> {
    let OfficialManifestEntry {
        status,
        snapshot_id,
        sequence_number,
        file_sequence_number,
        data_file,
    } = entry;
    Ok(ManifestEntry {
        status: match status {
            OfficialManifestStatus::Existing => EntryStatus::Existing,
            OfficialManifestStatus::Added => EntryStatus::Added,
            OfficialManifestStatus::Deleted => EntryStatus::Deleted,
        },
        snapshot_id,
        sequence_number,
        file_sequence_number,
        // iceberg-rust 0.10.1 has no consuming DataFile projection. Borrow
        // only for that final conversion after releasing the entry Arc.
        data_file: data_file_from_official(&data_file, partition_type)?,
    })
}

/// Return whether one MIME type belongs to Iceberg's file-format vocabulary.
pub(super) fn is_iceberg_mime_type(mime_type: &MimeType) -> bool {
    iceberg_file_format(mime_type).is_some()
}

/// Project one generic MIME type onto Iceberg's closed file-format vocabulary.
fn iceberg_file_format(mime_type: &MimeType) -> Option<OfficialDataFileFormat> {
    mime_type
        .extension()
        .and_then(|extension| extension.parse().ok())
}

/// Return the manifest token for one supported MIME type.
pub(super) fn file_format_name(mime_type: &MimeType) -> Result<&'static str> {
    let format = iceberg_file_format(mime_type).ok_or_else(|| {
        invalid(format_smolstr!(
            "expected an Iceberg MIME type for Parquet, Avro, ORC, or Puffin, got {mime_type}"
        ))
    })?;
    Ok(match format {
        OfficialDataFileFormat::Parquet => "PARQUET",
        OfficialDataFileFormat::Avro => "AVRO",
        OfficialDataFileFormat::Orc => "ORC",
        OfficialDataFileFormat::Puffin => "PUFFIN",
    })
}

/// Project an official data-file description onto the public MIME-based view.
fn data_file_from_official(
    file: &OfficialDataFile,
    partition_type: &OfficialStructType,
) -> Result<DataFile> {
    let fields = partition_type.fields();
    if file.partition().fields().len() != fields.len() {
        return Err(invalid(format_smolstr!(
            "expected {} partition values for {:?}, got {}",
            fields.len(),
            file.file_path(),
            file.partition().fields().len()
        )));
    }
    let partition = file
        .partition()
        .iter()
        .zip(fields)
        .map(|(value, field)| {
            value.map_or(Ok(Scalar::Null), |value| {
                scalar_from_official(value, &field.field_type)
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(DataFile {
        content: match file.content_type() {
            OfficialDataContentType::Data => 0,
            OfficialDataContentType::PositionDeletes => 1,
            OfficialDataContentType::EqualityDeletes => 2,
        },
        file_path: SmolStr::new(file.file_path()),
        mime_type: match file.file_format() {
            OfficialDataFileFormat::Parquet => MimeType::PARQUET,
            OfficialDataFileFormat::Avro => MimeType::AVRO,
            OfficialDataFileFormat::Orc => MimeType::ORC,
            OfficialDataFileFormat::Puffin => MimeType::PUFFIN,
        },
        partition,
        record_count: signed_u64(file.record_count(), "record count")?,
        file_size_in_bytes: signed_u64(file.file_size_in_bytes(), "file size")?,
        column_sizes: count_pairs(file.column_sizes(), "column size")?,
        value_counts: count_pairs(file.value_counts(), "value count")?,
        null_value_counts: count_pairs(file.null_value_counts(), "null count")?,
        nan_value_counts: count_pairs(file.nan_value_counts(), "NaN count")?,
        lower_bounds: bound_pairs(file.lower_bounds())?,
        upper_bounds: bound_pairs(file.upper_bounds())?,
        key_metadata: file.key_metadata().map(<[u8]>::to_vec),
        split_offsets: file.split_offsets().unwrap_or_default().to_vec(),
        equality_ids: file.equality_ids(),
        sort_order_id: file.sort_order_id(),
        first_row_id: file.first_row_id(),
        referenced_data_file: file.referenced_data_file().map(SmolStr::new),
        content_offset: file.content_offset(),
        content_size_in_bytes: file.content_size_in_bytes(),
    })
}

/// Preserve the physical scalar identity of an official partition literal.
fn scalar_from_official(value: &OfficialLiteral, data_type: &OfficialType) -> Result<Scalar> {
    let OfficialLiteral::Primitive(value) = value else {
        return Err(invalid(SmolStr::new_static(
            "expected a primitive Iceberg partition value",
        )));
    };
    let Some(data_type) = data_type.as_primitive_type() else {
        return Err(invalid(SmolStr::new_static(
            "expected a primitive Iceberg partition type",
        )));
    };
    match (data_type, value) {
        (OfficialPrimitiveType::Boolean, OfficialPrimitiveLiteral::Boolean(value)) => {
            Ok(Scalar::Bool(*value))
        }
        (OfficialPrimitiveType::Int, OfficialPrimitiveLiteral::Int(value)) => {
            Ok(Scalar::I32(*value))
        }
        (OfficialPrimitiveType::Long, OfficialPrimitiveLiteral::Long(value)) => {
            Ok(Scalar::I64(*value))
        }
        (OfficialPrimitiveType::Float, OfficialPrimitiveLiteral::Float(value)) => {
            Ok(Scalar::from(value.0))
        }
        (OfficialPrimitiveType::Double, OfficialPrimitiveLiteral::Double(value)) => {
            Ok(Scalar::from(value.0))
        }
        (OfficialPrimitiveType::Decimal { scale, .. }, OfficialPrimitiveLiteral::Int128(value)) => {
            Ok(Scalar::D128(
                *value,
                i8::try_from(*scale).map_err(|_| {
                    invalid(format_smolstr!(
                        "expected a decimal scale fitting i8, got {scale}"
                    ))
                })?,
            ))
        }
        (OfficialPrimitiveType::Date, OfficialPrimitiveLiteral::Int(value)) => {
            Ok(Scalar::date32(*value))
        }
        (OfficialPrimitiveType::Time, OfficialPrimitiveLiteral::Long(value)) => {
            Scalar::time64(*value, TimeUnit::Microsecond, Timezone::NAIVE)
        }
        (OfficialPrimitiveType::Timestamp, OfficialPrimitiveLiteral::Long(value)) => {
            Scalar::datetime64(*value, TimeUnit::Microsecond, Timezone::NAIVE)
        }
        (OfficialPrimitiveType::Timestamptz, OfficialPrimitiveLiteral::Long(value)) => {
            Scalar::datetime64(*value, TimeUnit::Microsecond, Timezone::UTC)
        }
        (OfficialPrimitiveType::TimestampNs, OfficialPrimitiveLiteral::Long(value)) => {
            Scalar::datetime64(*value, TimeUnit::Nanosecond, Timezone::NAIVE)
        }
        (OfficialPrimitiveType::TimestamptzNs, OfficialPrimitiveLiteral::Long(value)) => {
            Scalar::datetime64(*value, TimeUnit::Nanosecond, Timezone::UTC)
        }
        (OfficialPrimitiveType::String, OfficialPrimitiveLiteral::String(value)) => {
            Ok(Scalar::from(value.as_str()))
        }
        (OfficialPrimitiveType::Uuid, OfficialPrimitiveLiteral::UInt128(value)) => {
            Ok(Scalar::from(value.to_be_bytes().to_vec()))
        }
        (
            OfficialPrimitiveType::Fixed(_) | OfficialPrimitiveType::Binary,
            OfficialPrimitiveLiteral::Binary(value),
        ) => Ok(Scalar::from(value.clone())),
        _ => Err(invalid(format_smolstr!(
            "expected a partition value matching {data_type}, got {value:?}"
        ))),
    }
}

/// Convert one official count map into the stable sorted public representation.
fn count_pairs(
    values: &std::collections::HashMap<i32, u64>,
    name: &str,
) -> Result<Vec<(i32, i64)>> {
    let mut pairs = values
        .iter()
        .map(|(field_id, value)| Ok((*field_id, signed_u64(*value, name)?)))
        .collect::<Result<Vec<_>>>()?;
    pairs.sort_unstable_by_key(|(field_id, _)| *field_id);
    Ok(pairs)
}

/// Convert official bounds back to their canonical Iceberg byte representation.
fn bound_pairs(
    values: &std::collections::HashMap<i32, iceberg_official::spec::Datum>,
) -> Result<Vec<(i32, Vec<u8>)>> {
    let mut pairs = values
        .iter()
        .map(|(field_id, value)| {
            value
                .to_bytes()
                .map(|bytes| (*field_id, bytes.to_vec()))
                .map_err(Error::from_iceberg)
        })
        .collect::<Result<Vec<_>>>()?;
    pairs.sort_unstable_by_key(|(field_id, _)| *field_id);
    Ok(pairs)
}

/// Project the manifest header's official partition spec onto the public view.
fn partition_spec_from_official(spec: &OfficialPartitionSpec) -> Result<PartitionSpec> {
    let fields = spec
        .fields()
        .iter()
        .map(|field| {
            let transform = match field.transform {
                OfficialTransform::Identity => Transform::Identity,
                OfficialTransform::Bucket(count) => Transform::Bucket(count),
                OfficialTransform::Truncate(width) => Transform::Truncate(width),
                OfficialTransform::Year => Transform::Year,
                OfficialTransform::Month => Transform::Month,
                OfficialTransform::Day => Transform::Day,
                OfficialTransform::Hour => Transform::Hour,
                OfficialTransform::Void => Transform::Void,
                OfficialTransform::Unknown => Transform::Unknown,
            };
            Ok(PartitionField {
                source_id: field.source_id,
                field_id: field.field_id,
                name: SmolStr::new(&field.name),
                transform,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(PartitionSpec {
        spec_id: spec.spec_id(),
        fields,
    })
}

/// Project one official manifest-list row onto Yggdryl's public view.
fn manifest_from_official(manifest: OfficialManifestFile) -> Result<ManifestFile> {
    let files = |value: Option<u32>, name: &str| {
        value
            .map(|value| {
                i32::try_from(value).map_err(|_| {
                    invalid(format_smolstr!("expected {name} fitting i32, got {value}"))
                })
            })
            .transpose()
    };
    let rows =
        |value: Option<u64>, name: &str| value.map(|value| signed_u64(value, name)).transpose();
    Ok(ManifestFile {
        manifest_path: SmolStr::from(manifest.manifest_path),
        manifest_length: manifest.manifest_length,
        partition_spec_id: manifest.partition_spec_id,
        content: match manifest.content {
            OfficialManifestContent::Data => ManifestContent::Data,
            OfficialManifestContent::Deletes => ManifestContent::Deletes,
        },
        sequence_number: manifest.sequence_number,
        min_sequence_number: manifest.min_sequence_number,
        added_snapshot_id: manifest.added_snapshot_id,
        added_files_count: files(manifest.added_files_count, "added file count")?,
        existing_files_count: files(manifest.existing_files_count, "existing file count")?,
        deleted_files_count: files(manifest.deleted_files_count, "deleted file count")?,
        added_rows_count: rows(manifest.added_rows_count, "added row count")?,
        existing_rows_count: rows(manifest.existing_rows_count, "existing row count")?,
        deleted_rows_count: rows(manifest.deleted_rows_count, "deleted row count")?,
        partitions: manifest
            .partitions
            .unwrap_or_default()
            .into_iter()
            .map(|summary| FieldSummary {
                contains_null: summary.contains_null,
                contains_nan: summary.contains_nan,
                lower_bound: summary.lower_bound.map(|bytes| bytes.into_vec()),
                upper_bound: summary.upper_bound.map(|bytes| bytes.into_vec()),
            })
            .collect(),
        key_metadata: manifest.key_metadata,
        first_row_id: manifest
            .first_row_id
            .map(|value| signed_u64(value, "first row id"))
            .transpose()?,
    })
}

/// Convert an unsigned official count to the signed public representation.
fn signed_u64(value: u64, name: &str) -> Result<i64> {
    i64::try_from(value)
        .map_err(|_| invalid(format_smolstr!("expected {name} fitting i64, got {value}")))
}

/// Replace a handle's bytes with a manifest list naming `manifests`.
///
/// For v3, `first_row_id` is the snapshot's row-id cursor. Unassigned data
/// manifests receive contiguous ranges and the returned value is the next
/// unassigned id. Older versions return `None`.
///
/// # Errors
///
/// Returns an error when row lineage is incomplete or overflows, a row does
/// not fit the derived Avro schema, or the write fails.
pub fn write_manifest_list<H: IOBase + ?Sized>(
    handle: &mut H,
    version: FormatVersion,
    snapshot_id: i64,
    parent_snapshot_id: Option<i64>,
    sequence_number: i64,
    first_row_id: Option<i64>,
    manifests: &[ManifestFile],
) -> Result<Option<i64>> {
    if version < FormatVersion::V3 && first_row_id.is_some() {
        return Err(invalid(format_smolstr!(
            "expected no manifest-list first-row-id in Iceberg v{}, got {first_row_id:?}",
            version.number()
        )));
    }
    if first_row_id.is_some_and(|value| value < 0) {
        return Err(invalid(format_smolstr!(
            "expected a non-negative manifest-list first-row-id, got {first_row_id:?}"
        )));
    }
    let avro_schema = manifest_list_schema(version)?;
    let mut rows = Vec::with_capacity(manifests.len());
    let mut next_row_id = first_row_id;
    for manifest in manifests {
        let mut manifest = manifest.clone();
        if version >= FormatVersion::V3 {
            manifest.assign_first_row_id(&mut next_row_id)?;
        }
        rows.push(manifest_to_value(&manifest, version)?);
    }
    let snapshot_text = snapshot_id.to_string();
    let parent_text = parent_snapshot_id.map_or_else(|| "null".to_owned(), |id| id.to_string());
    let sequence_text = sequence_number.to_string();
    let first_row_text = first_row_id.map_or_else(|| "null".to_owned(), |id| id.to_string());
    let format_version = version.number().to_string();
    let mut metadata = vec![
        ("snapshot-id", snapshot_text.as_str()),
        ("parent-snapshot-id", parent_text.as_str()),
        ("format-version", format_version.as_str()),
    ];
    if version >= FormatVersion::V2 {
        metadata.insert(2, ("sequence-number", sequence_text.as_str()));
    }
    if version >= FormatVersion::V3 {
        metadata.push(("first-row-id", first_row_text.as_str()));
    }
    crate::avro::write_container(handle, &avro_schema, &metadata, &rows)?;
    Ok(next_row_id)
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
    let file_count_names = manifest_file_count_names(version);
    let counts: [(&str, i32, &str); 6] = [
        (file_count_names[0], 504, "int"),
        (file_count_names[1], 505, "int"),
        (file_count_names[2], 506, "int"),
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

fn manifest_file_count_names(version: FormatVersion) -> [&'static str; 3] {
    if version == FormatVersion::V1 {
        [
            "added_data_files_count",
            "existing_data_files_count",
            "deleted_data_files_count",
        ]
    } else {
        [
            "added_files_count",
            "existing_files_count",
            "deleted_files_count",
        ]
    }
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
        fields.push(optional(child.name(), id, partition_avro_type(child, id)?)?);
    }
    record(name, Scalar::from_sequence(fields))
}

/// Render one Iceberg partition primitive as its required Avro wire schema.
fn partition_avro_type(field: &Field, id: i32) -> Result<Scalar> {
    let mut primitive = super::PrimitiveType::from_data_type(field.data_type())?;
    if primitive == super::PrimitiveType::Fixed(16)
        && field.iceberg().get(super::schema::DECLARED_TYPE) == Some("uuid")
    {
        primitive = super::PrimitiveType::Uuid;
    }
    Ok(match primitive {
        super::PrimitiveType::Boolean => Scalar::from("boolean"),
        super::PrimitiveType::Int => Scalar::from("int"),
        super::PrimitiveType::Long => Scalar::from("long"),
        super::PrimitiveType::Float => Scalar::from("float"),
        super::PrimitiveType::Double => Scalar::from("double"),
        super::PrimitiveType::Date => avro_logical("int", "date")?,
        super::PrimitiveType::Time => avro_logical("long", "time-micros")?,
        super::PrimitiveType::Timestamp | super::PrimitiveType::Timestamptz => {
            avro_logical("long", "timestamp-micros")?
        }
        super::PrimitiveType::TimestampNs | super::PrimitiveType::TimestamptzNs => {
            avro_logical("long", "timestamp-nanos")?
        }
        super::PrimitiveType::String => Scalar::from("string"),
        super::PrimitiveType::Uuid => avro_fixed(id, "uuid", 16, Some("uuid"), None)?,
        super::PrimitiveType::Fixed(width) => avro_fixed(id, "fixed", width, None, None)?,
        super::PrimitiveType::Binary => Scalar::from("bytes"),
        super::PrimitiveType::Decimal { precision, scale } => {
            let width = OfficialType::decimal_required_bytes(u32::from(precision))
                .map_err(Error::from_iceberg)?;
            avro_fixed(
                id,
                "decimal",
                i32::try_from(width).map_err(|_| {
                    invalid(format_smolstr!(
                        "expected the Avro decimal width to fit i32, got {width}"
                    ))
                })?,
                Some("decimal"),
                Some((precision, scale)),
            )?
        }
        super::PrimitiveType::Unknown => {
            return Err(invalid(format_smolstr!(
                "expected an Avro-encodable partition type on {:?}, got unknown",
                field.name()
            )));
        }
    })
}

/// Build an Avro logical primitive.
fn avro_logical(physical: &str, logical: &str) -> Result<Scalar> {
    Scalar::from_mapping([
        (Scalar::from("type"), Scalar::from(physical)),
        (Scalar::from("logicalType"), Scalar::from(logical)),
    ])
}

/// Build a uniquely named Avro fixed value, optionally with a logical type.
fn avro_fixed(
    id: i32,
    kind: &str,
    width: i32,
    logical: Option<&str>,
    decimal: Option<(u8, i8)>,
) -> Result<Scalar> {
    let id = if id < 0 {
        format_smolstr!("n{}", id.unsigned_abs())
    } else {
        format_smolstr!("{id}")
    };
    let mut entries = vec![
        (Scalar::from("type"), Scalar::from("fixed")),
        (
            Scalar::from("name"),
            Scalar::from(format_smolstr!("partition_{id}_{kind}")),
        ),
        (Scalar::from("size"), Scalar::from(i64::from(width))),
    ];
    if let Some(logical) = logical {
        entries.push((Scalar::from("logicalType"), Scalar::from(logical)));
    }
    if let Some((precision, scale)) = decimal {
        entries.push((
            Scalar::from("precision"),
            Scalar::from(i64::from(precision)),
        ));
        entries.push((Scalar::from("scale"), Scalar::from(i64::from(scale))));
    }
    Scalar::from_mapping(entries)
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

/// Render one manifest entry as the Avro record it is stored as.
fn entry_to_value(
    entry: &ManifestEntry,
    version: FormatVersion,
    partition: &Field,
) -> Result<Scalar> {
    validate_entry_version(entry, version)?;
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
        Scalar::from(file_format_name(&file.mime_type)?),
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
    data_file.push((
        Scalar::from("key_metadata"),
        file.key_metadata
            .as_deref()
            .map_or(Scalar::Null, Scalar::from),
    ));
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
        data_file.push((
            Scalar::from("equality_ids"),
            file.equality_ids.as_ref().map_or(Scalar::Null, |ids| {
                Scalar::from_sequence(ids.iter().map(|id| Scalar::from(i64::from(*id))))
            }),
        ));
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
        data_file.push((
            Scalar::from("referenced_data_file"),
            file.referenced_data_file
                .as_ref()
                .map_or(Scalar::Null, |path| Scalar::from(path.clone())),
        ));
        data_file.push((
            Scalar::from("content_offset"),
            file.content_offset.map_or(Scalar::Null, Scalar::from),
        ));
        data_file.push((
            Scalar::from("content_size_in_bytes"),
            file.content_size_in_bytes
                .map_or(Scalar::Null, Scalar::from),
        ));
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

fn validate_entry_version(entry: &ManifestEntry, version: FormatVersion) -> Result<()> {
    let file = &entry.data_file;
    if !(0..=2).contains(&file.content) {
        return Err(invalid(format_smolstr!(
            "expected data-file content 0, 1, or 2, got {} for {:?}",
            file.content,
            file.file_path
        )));
    }
    if file.record_count < 0 || file.file_size_in_bytes < 0 {
        return Err(invalid(format_smolstr!(
            "expected non-negative record count and file size for {:?}, got {} and {}",
            file.file_path,
            file.record_count,
            file.file_size_in_bytes
        )));
    }
    for (name, counts) in [
        ("column_sizes", file.column_sizes.as_slice()),
        ("value_counts", file.value_counts.as_slice()),
        ("null_value_counts", file.null_value_counts.as_slice()),
        ("nan_value_counts", file.nan_value_counts.as_slice()),
    ] {
        validate_metric_counts(name, counts, &file.file_path)?;
    }
    for (name, bounds) in [
        ("lower_bounds", file.lower_bounds.as_slice()),
        ("upper_bounds", file.upper_bounds.as_slice()),
    ] {
        validate_metric_ids(name, bounds.iter().map(|(id, _)| *id), &file.file_path)?;
    }
    if file.split_offsets.iter().any(|offset| *offset < 0)
        || file.split_offsets.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(invalid(format_smolstr!(
            "expected non-negative ascending split_offsets for {:?}, got {:?}",
            file.file_path,
            file.split_offsets
        )));
    }
    match (file.content, file.equality_ids.as_deref()) {
        (2, Some(ids)) if !ids.is_empty() => {
            validate_metric_ids("equality_ids", ids.iter().copied(), &file.file_path)?;
        }
        (2, _) => {
            return Err(invalid(format_smolstr!(
                "expected non-empty equality_ids for equality-delete file {:?}",
                file.file_path
            )));
        }
        (_, Some(_)) => {
            return Err(invalid(format_smolstr!(
                "expected equality_ids only on equality-delete files, got content {} for {:?}",
                file.content,
                file.file_path
            )));
        }
        _ => {}
    }
    if file.content == 1 && file.sort_order_id.is_some() {
        return Err(invalid(format_smolstr!(
            "expected no sort_order_id on position-delete file {:?}",
            file.file_path
        )));
    }
    if file.first_row_id.is_some_and(|value| value < 0) {
        return Err(invalid(format_smolstr!(
            "expected a non-negative first_row_id for {:?}, got {:?}",
            file.file_path,
            file.first_row_id
        )));
    }
    if file.first_row_id.is_some() && file.content != 0 {
        return Err(invalid(format_smolstr!(
            "expected first_row_id only on a data file, got content {} for {:?}",
            file.content,
            file.file_path
        )));
    }
    let vector_fields = (
        file.referenced_data_file.is_some(),
        file.content_offset,
        file.content_size_in_bytes,
    );
    match (&file.mime_type, file.content, vector_fields) {
        (mime_type, 1, (true, Some(offset), Some(size)))
            if mime_type == &MimeType::PUFFIN && offset >= 0 && size >= 0 => {}
        (mime_type, _, _) if mime_type == &MimeType::PUFFIN => {
            return Err(invalid(format_smolstr!(
                "expected a Puffin position-delete file with referenced_data_file, non-negative content_offset, and non-negative content_size_in_bytes, got {:?}",
                file.file_path
            )));
        }
        (_, _, (false, None, None)) => {}
        _ => {
            return Err(invalid(format_smolstr!(
                "expected deletion-vector fields only on Puffin files, got {:?}",
                file.file_path
            )));
        }
    }
    if version >= FormatVersion::V2
        && entry.status != EntryStatus::Added
        && (entry.snapshot_id.is_none()
            || entry.sequence_number.is_none()
            || entry.file_sequence_number.is_none())
    {
        return Err(invalid(format_smolstr!(
            "expected snapshot and sequence numbers on {:?} entry for {:?}",
            entry.status,
            file.file_path
        )));
    }
    if version == FormatVersion::V1 {
        if file.content != 0 {
            return Err(invalid(format_smolstr!(
                "expected only data files in an Iceberg v1 manifest, got content {} for {:?}",
                file.content,
                file.file_path
            )));
        }
        if entry.sequence_number.is_some() || entry.file_sequence_number.is_some() {
            return Err(invalid(format_smolstr!(
                "expected no entry sequence numbers in an Iceberg v1 manifest for {:?}",
                file.file_path
            )));
        }
        if file.equality_ids.is_some() {
            return Err(invalid(format_smolstr!(
                "expected no equality_ids in an Iceberg v1 manifest for {:?}",
                file.file_path
            )));
        }
    }
    if version < FormatVersion::V3 {
        let unsupported = if file.first_row_id.is_some() {
            Some("first_row_id")
        } else if file.referenced_data_file.is_some() {
            Some("referenced_data_file")
        } else if file.content_offset.is_some() {
            Some("content_offset")
        } else if file.content_size_in_bytes.is_some() {
            Some("content_size_in_bytes")
        } else {
            None
        };
        if let Some(name) = unsupported {
            return Err(invalid(format_smolstr!(
                "expected no {name} in an Iceberg v{} manifest for {:?}",
                version.number(),
                file.file_path
            )));
        }
        if file.mime_type == MimeType::PUFFIN {
            return Err(invalid(format_smolstr!(
                "expected Puffin only in an Iceberg v3 manifest, got v{} for {:?}",
                version.number(),
                file.file_path
            )));
        }
    }
    Ok(())
}

fn validate_metric_counts(name: &str, values: &[(i32, i64)], path: &str) -> Result<()> {
    if let Some((id, value)) = values.iter().find(|(id, value)| *id <= 0 || *value < 0) {
        return Err(invalid(format_smolstr!(
            "expected positive field ids and non-negative {name} for {path:?}, got ({id}, {value})"
        )));
    }
    validate_metric_ids(name, values.iter().map(|(id, _)| *id), path)
}

fn validate_metric_ids(name: &str, ids: impl IntoIterator<Item = i32>, path: &str) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        if id <= 0 || !seen.insert(id) {
            return Err(invalid(format_smolstr!(
                "expected unique positive field ids in {name} for {path:?}, got {id}"
            )));
        }
    }
    Ok(())
}

/// Render one manifest list row as the Avro record it is stored as.
fn manifest_to_value(manifest: &ManifestFile, version: FormatVersion) -> Result<Scalar> {
    if manifest.manifest_length < 0
        || manifest.partition_spec_id < 0
        || manifest.sequence_number < 0
        || manifest.min_sequence_number < 0
    {
        return Err(invalid(format_smolstr!(
            "expected non-negative manifest length, spec id, and sequence numbers for {:?}",
            manifest.manifest_path
        )));
    }
    for (name, value) in [
        (
            "added_files_count",
            manifest.added_files_count.map(i64::from),
        ),
        (
            "existing_files_count",
            manifest.existing_files_count.map(i64::from),
        ),
        (
            "deleted_files_count",
            manifest.deleted_files_count.map(i64::from),
        ),
        ("added_rows_count", manifest.added_rows_count),
        ("existing_rows_count", manifest.existing_rows_count),
        ("deleted_rows_count", manifest.deleted_rows_count),
    ] {
        if value.is_some_and(|count| count < 0) {
            return Err(invalid(format_smolstr!(
                "expected a non-negative {name} for {:?}, got {value:?}",
                manifest.manifest_path
            )));
        }
    }
    if version == FormatVersion::V1
        && (manifest.content != ManifestContent::Data
            || manifest.sequence_number != 0
            || manifest.min_sequence_number != 0)
    {
        return Err(invalid(format_smolstr!(
            "expected a v1 data manifest with zero sequence numbers, got {:?} with {}/{}",
            manifest.content,
            manifest.sequence_number,
            manifest.min_sequence_number
        )));
    }
    if version < FormatVersion::V3 && manifest.first_row_id.is_some() {
        return Err(invalid(format_smolstr!(
            "expected no first_row_id in an Iceberg v{} manifest list",
            version.number()
        )));
    }
    if manifest.first_row_id.is_some_and(|value| value < 0) {
        return Err(invalid(format_smolstr!(
            "expected a non-negative first_row_id, got {:?}",
            manifest.first_row_id
        )));
    }
    if version >= FormatVersion::V2 {
        let required_counts = [
            ("added_files_count", manifest.added_files_count.is_some()),
            (
                "existing_files_count",
                manifest.existing_files_count.is_some(),
            ),
            (
                "deleted_files_count",
                manifest.deleted_files_count.is_some(),
            ),
            ("added_rows_count", manifest.added_rows_count.is_some()),
            (
                "existing_rows_count",
                manifest.existing_rows_count.is_some(),
            ),
            ("deleted_rows_count", manifest.deleted_rows_count.is_some()),
        ];
        if let Some((name, _)) = required_counts.iter().find(|(_, present)| !present) {
            return Err(invalid(format_smolstr!(
                "expected required {name} in Iceberg v{} manifest list, got null",
                version.number()
            )));
        }
    }
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
    let file_count_names = manifest_file_count_names(version);
    row.push((
        Scalar::from(file_count_names[0]),
        manifest
            .added_files_count
            .map_or(Scalar::Null, |value| Scalar::from(i64::from(value))),
    ));
    row.push((
        Scalar::from(file_count_names[1]),
        manifest
            .existing_files_count
            .map_or(Scalar::Null, |value| Scalar::from(i64::from(value))),
    ));
    row.push((
        Scalar::from(file_count_names[2]),
        manifest
            .deleted_files_count
            .map_or(Scalar::Null, |value| Scalar::from(i64::from(value))),
    ));
    row.push((
        Scalar::from("added_rows_count"),
        manifest.added_rows_count.map_or(Scalar::Null, Scalar::from),
    ));
    row.push((
        Scalar::from("existing_rows_count"),
        manifest
            .existing_rows_count
            .map_or(Scalar::Null, Scalar::from),
    ));
    row.push((
        Scalar::from("deleted_rows_count"),
        manifest
            .deleted_rows_count
            .map_or(Scalar::Null, Scalar::from),
    ));
    row.push((
        Scalar::from("partitions"),
        summaries_to_value(&manifest.partitions)?,
    ));
    row.push((
        Scalar::from("key_metadata"),
        manifest
            .key_metadata
            .as_deref()
            .map_or(Scalar::Null, Scalar::from),
    ));
    if version >= FormatVersion::V3 {
        row.push((
            Scalar::from("first_row_id"),
            manifest.first_row_id.map_or(Scalar::Null, Scalar::from),
        ));
    }
    Scalar::from_mapping(row)
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

/// Report a malformed Iceberg manifest document.
fn invalid(reason: SmolStr) -> Error {
    Error::Codec {
        format: "iceberg",
        position: 0,
        reason,
    }
}

/// Read the entries a read-only scan plan needs.
///
/// Parsing and validation are delegated to the official Iceberg manifest
/// implementation. After that succeeds, fields a read-only plan cannot use are
/// dropped to bound retained memory. Filter statistics survive only when
/// `with_stats` is set. Rewrites use [`read_manifest`] and keep every field.
///
/// # Errors
///
/// Returns an error when the bytes are not an Avro manifest or a row does not
/// decode.
pub fn read_manifest_for_plan<H: IOBase + ?Sized>(
    handle: &H,
    with_stats: bool,
) -> Result<Vec<ManifestEntry>> {
    let mut entries = read_manifest(handle)?;
    for entry in &mut entries {
        let file = &mut entry.data_file;
        file.column_sizes.clear();
        file.nan_value_counts.clear();
        file.key_metadata = None;
        file.split_offsets.clear();
        file.equality_ids = None;
        file.sort_order_id = None;
        file.referenced_data_file = None;
        file.content_offset = None;
        file.content_size_in_bytes = None;
        if !with_stats {
            file.value_counts.clear();
            file.null_value_counts.clear();
            file.lower_bounds.clear();
            file.upper_bounds.clear();
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod official_read_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::DataType;
    use crate::io::{Buffer, IOBase};

    struct OversizedHandle {
        handle: Buffer,
        declared_size: u64,
        reads: AtomicUsize,
    }

    struct CountingHandle {
        handle: Buffer,
        bytes_read: AtomicUsize,
    }

    impl crate::io::IOMedia for OversizedHandle {
        crate::impl_default_iomedia!();
    }

    impl IOBase for OversizedHandle {
        crate::delegate_iobase!(handle: pread, pwrite, capacity, reserve, truncate, url,
            media_type, set_media_type);

        fn size(&self) -> u64 {
            self.declared_size
        }

        fn read_all_bytes(&self) -> Result<Vec<u8>> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            Ok(Vec::new())
        }
    }

    impl crate::io::IOMedia for CountingHandle {
        crate::impl_default_iomedia!();
    }

    impl IOBase for CountingHandle {
        crate::delegate_iobase!(handle: pwrite, size, capacity, reserve, truncate, url,
            media_type, set_media_type);

        fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
            let read = self.handle.pread(offset, buffer)?;
            self.bytes_read.fetch_add(read, Ordering::Relaxed);
            Ok(read)
        }
    }

    fn field() -> Field {
        let mut field = DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Utf8.nullable_field("venue"),
        ])
        .unwrap()
        .required_field("row");
        super::super::schema::assign_field_ids(&mut field, 1).unwrap();
        field.insert_metadata("iceberg:schema-id", "0").unwrap();
        field
    }

    fn official_version(version: FormatVersion) -> OfficialFormatVersion {
        match version {
            FormatVersion::V1 => OfficialFormatVersion::V1,
            FormatVersion::V2 => OfficialFormatVersion::V2,
            FormatVersion::V3 => OfficialFormatVersion::V3,
        }
    }

    fn manifest(sequence_number: i64) -> ManifestFile {
        ManifestFile {
            manifest_path: "s3://warehouse/table/metadata/manifest.avro".into(),
            manifest_length: 1,
            partition_spec_id: 0,
            content: ManifestContent::Data,
            sequence_number,
            min_sequence_number: sequence_number,
            added_snapshot_id: 41,
            added_files_count: Some(1),
            existing_files_count: Some(0),
            deleted_files_count: Some(0),
            added_rows_count: Some(1),
            existing_rows_count: Some(0),
            deleted_rows_count: Some(0),
            partitions: Vec::new(),
            key_metadata: None,
            first_row_id: None,
        }
    }

    fn entry(status: EntryStatus) -> ManifestEntry {
        ManifestEntry {
            status,
            snapshot_id: None,
            sequence_number: None,
            file_sequence_number: None,
            data_file: DataFile {
                file_path: "s3://warehouse/table/data/part.parquet".into(),
                record_count: 1,
                file_size_in_bytes: 1,
                ..DataFile::default()
            },
        }
    }

    fn replace_data_file_schema_field(
        schema: &Scalar,
        name: &str,
        replacement_type: Option<Scalar>,
    ) -> Scalar {
        let fields = schema.get_key_str("fields").unwrap().as_sequence().unwrap();
        let fields = fields.iter().map(|field| {
            if field.get_key_str("name").and_then(Scalar::as_str) != Some("data_file") {
                return field.clone();
            }
            let data_type = field.get_key_str("type").unwrap();
            let data_fields = data_type
                .get_key_str("fields")
                .unwrap()
                .as_sequence()
                .unwrap();
            let data_fields = data_fields.iter().filter_map(|child| {
                if child.get_key_str("name").and_then(Scalar::as_str) != Some(name) {
                    return Some(child.clone());
                }
                replacement_type
                    .as_ref()
                    .map(|kind| child.with_key("type", kind.clone()).unwrap())
            });
            let data_type = data_type
                .with_key("fields", Scalar::from_sequence(data_fields))
                .unwrap();
            field.with_key("type", data_type).unwrap()
        });
        schema
            .with_key("fields", Scalar::from_sequence(fields))
            .unwrap()
    }

    fn with_data_file_value(row: &Scalar, name: &str, value: Option<Scalar>) -> Scalar {
        let data_file = row.get_key_str("data_file").unwrap();
        let data_file = value.map_or_else(
            || data_file.without_key(name).unwrap(),
            |value| data_file.with_key(name, value).unwrap(),
        );
        row.with_key("data_file", data_file).unwrap()
    }

    fn write_nonconforming_manifest(
        handle: &mut Buffer,
        avro_schema: &Scalar,
        field: &Field,
        spec: &PartitionSpec,
        row: Scalar,
    ) {
        let schema_json = super::super::schema_into_json(field).unwrap();
        let schema_text =
            String::from_utf8(crate::json::into_bytes(&schema_json).unwrap()).unwrap();
        let spec_text = String::from_utf8(
            crate::json::into_bytes(&spec.clone().into_v1_json().unwrap()).unwrap(),
        )
        .unwrap();
        let spec_id = spec.spec_id.to_string();
        let metadata = [
            ("schema", schema_text.as_str()),
            ("schema-id", "0"),
            ("partition-spec", spec_text.as_str()),
            ("partition-spec-id", spec_id.as_str()),
            ("format-version", "2"),
            ("content", "data"),
        ];
        crate::avro::write_container(handle, avro_schema, &metadata, &[row]).unwrap();
    }

    fn manifest_header_parts(
        field: &Field,
        spec: &PartitionSpec,
        version: FormatVersion,
    ) -> (Scalar, String, String) {
        let partition = spec.partition_field(field).unwrap();
        let avro_schema = manifest_entry_schema(version, &partition).unwrap();
        let schema = super::super::schema_into_json(field).unwrap();
        let schema = crate::json::into_utf8(&schema).unwrap();
        let partition_spec = spec.clone().into_v1_json().unwrap();
        let partition_spec = crate::json::into_utf8(&partition_spec).unwrap();
        (avro_schema, schema, partition_spec)
    }

    fn with_invalid_utf8_metadata(handle: &Buffer, key: &str) -> Buffer {
        use crate::avro::container;
        use crate::avro::datum::Cursor;

        let mut bytes = handle.as_slice().to_vec();
        let position = {
            let mut cursor = Cursor::new(&bytes);
            container::check_magic(cursor.take(container::MAGIC.len()).unwrap()).unwrap();
            let mut position = None;
            loop {
                let count = cursor.long().unwrap();
                if count == 0 {
                    break;
                }
                if count < 0 {
                    cursor.long().unwrap();
                }
                for _ in 0..count.unsigned_abs() {
                    let candidate = std::str::from_utf8(cursor.bytes().unwrap()).unwrap();
                    let value = cursor.bytes().unwrap();
                    if candidate == key {
                        assert!(!value.is_empty(), "metadata value must not be empty");
                        assert!(position.is_none(), "metadata key must be unique");
                        position = Some(cursor.position - value.len());
                    }
                }
            }
            position.expect("metadata key is present")
        };
        bytes[position] = 0xff;
        Buffer::from(bytes)
    }

    fn avro_header_end(bytes: &[u8]) -> usize {
        use crate::avro::container;
        use crate::avro::datum::Cursor;

        let mut cursor = Cursor::new(bytes);
        container::check_magic(cursor.take(container::MAGIC.len()).unwrap()).unwrap();
        container::parse_header_entries(&mut cursor, crate::Limits::default()).unwrap();
        cursor.position
    }

    fn manifest_with_data_file_value(name: &str, value: Scalar) -> Buffer {
        let field = field();
        let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
        let partition = spec.partition_field(&field).unwrap();
        let schema = manifest_entry_schema(FormatVersion::V2, &partition).unwrap();
        let input = ManifestEntry::added(
            41,
            DataFile {
                file_path: "s3://warehouse/table/data/part.parquet".into(),
                partition: vec![Scalar::from("XNAS")],
                record_count: 7,
                file_size_in_bytes: 128,
                ..DataFile::default()
            },
        );
        let row = with_data_file_value(
            &entry_to_value(&input, FormatVersion::V2, &partition).unwrap(),
            name,
            Some(value),
        );
        let mut handle = Buffer::new();
        write_nonconforming_manifest(&mut handle, &schema, &field, &spec, row);
        handle
    }

    fn metric_pairs(entries: impl IntoIterator<Item = (i64, Scalar)>) -> Scalar {
        Scalar::from_sequence(entries.into_iter().map(|(id, value)| {
            Scalar::from_mapping([
                (Scalar::from("key"), Scalar::from(id)),
                (Scalar::from("value"), value),
            ])
            .unwrap()
        }))
    }

    fn assert_preflight_error(handle: &Buffer, expected: &str) {
        let error = read_manifest(handle).unwrap_err();
        let Error::Codec { format, reason, .. } = error else {
            panic!("expected a raw Iceberg preflight error, got {error}");
        };
        assert_eq!(format, "iceberg");
        assert!(reason.contains(expected), "{reason}");
    }

    #[test]
    fn manifest_entry_inheritance_matches_official_rules() {
        let current = manifest(7);
        let mut added = entry(EntryStatus::Added);
        added.inherit(&current).unwrap();
        assert_eq!(added.snapshot_id, Some(41));
        assert_eq!(added.sequence_number, Some(7));
        assert_eq!(added.file_sequence_number, Some(7));

        let mut existing = entry(EntryStatus::Existing);
        existing.sequence_number = Some(3);
        existing.file_sequence_number = Some(4);
        existing.inherit(&current).unwrap();
        assert_eq!(existing.snapshot_id, Some(41));
        assert_eq!(existing.sequence_number, Some(3));
        assert_eq!(existing.file_sequence_number, Some(4));

        for status in [EntryStatus::Existing, EntryStatus::Deleted] {
            let mut initial = entry(status);
            initial.inherit(&manifest(0)).unwrap();
            assert_eq!(initial.snapshot_id, Some(41));
            assert_eq!(initial.sequence_number, Some(0));
            assert_eq!(initial.file_sequence_number, Some(0));

            let mut missing = entry(status);
            assert!(missing.inherit(&current).is_err());
            assert_eq!(missing.snapshot_id, Some(41));
        }
    }

    #[test]
    fn manifest_rejects_oversized_declared_input_before_reading() {
        let limit = crate::Limits::default().max_input_bytes();
        let handle = OversizedHandle {
            handle: Buffer::new(),
            declared_size: u64::try_from(limit).unwrap() + 1,
            reads: AtomicUsize::new(0),
        };

        let error = read_manifest(&handle).unwrap_err();
        assert!(error.to_string().contains("at most"));
        assert!(error.to_string().contains(&(limit + 1).to_string()));
        assert_eq!(handle.reads.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn manifest_spec_reads_only_the_bounded_avro_header() {
        let field = field();
        let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
        let entry = ManifestEntry::added(
            41,
            DataFile {
                file_path: "s3://warehouse/table/data/part.parquet".into(),
                partition: vec![Scalar::from("XNAS")],
                record_count: 1,
                file_size_in_bytes: 1,
                ..DataFile::default()
            },
        );
        let mut buffer = Buffer::new();
        let entries = (0..4_096_u64)
            .map(|index| {
                let mut entry = entry.clone();
                let mut state = index.wrapping_add(0x9e37_79b9_7f4a_7c15);
                let mut key = Vec::with_capacity(32);
                for _ in 0..4 {
                    state ^= state >> 12;
                    state ^= state << 25;
                    state ^= state >> 27;
                    state = state.wrapping_mul(0x2545_f491_4f6c_dd1d);
                    key.extend_from_slice(&state.to_le_bytes());
                }
                entry.data_file.key_metadata = Some(key);
                entry
            })
            .collect::<Vec<_>>();
        write_manifest(&mut buffer, FormatVersion::V2, &field, &spec, &entries).unwrap();
        assert!(buffer.size() > 64 * 1024);
        let handle = CountingHandle {
            handle: buffer,
            bytes_read: AtomicUsize::new(0),
        };

        assert_eq!(read_manifest_spec(&handle).unwrap(), spec);
        let bytes_read = handle.bytes_read.load(Ordering::Relaxed);
        assert!(
            bytes_read < 72 * 1024,
            "header read used {bytes_read} bytes"
        );
        assert!(u64::try_from(bytes_read).unwrap() < handle.size());
    }

    #[test]
    fn manifest_spec_ignores_truncated_and_corrupt_bodies() {
        let field = field();
        let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
        let entry = ManifestEntry::added(
            41,
            DataFile {
                file_path: "s3://warehouse/table/data/part.parquet".into(),
                partition: vec![Scalar::from("XNAS")],
                record_count: 1,
                file_size_in_bytes: 1,
                ..DataFile::default()
            },
        );
        let mut encoded = Buffer::new();
        write_manifest(&mut encoded, FormatVersion::V2, &field, &spec, &[entry]).unwrap();
        let header_end = avro_header_end(encoded.as_slice());
        assert!(header_end < encoded.as_slice().len());

        let truncated = Buffer::from(encoded.as_slice()[..=header_end].to_vec());
        assert_eq!(read_manifest_spec(&truncated).unwrap(), spec);
        assert!(read_manifest(&truncated).is_err());

        let mut corrupt = encoded.as_slice().to_vec();
        *corrupt.last_mut().unwrap() ^= 0xff;
        let corrupt = Buffer::from(corrupt);
        assert_eq!(read_manifest_spec(&corrupt).unwrap(), spec);
        assert!(read_manifest(&corrupt).is_err());
    }

    #[test]
    fn manifest_spec_rejects_missing_and_malformed_required_metadata() {
        let field = field();
        let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
        let (avro_schema, schema, partition_spec) =
            manifest_header_parts(&field, &spec, FormatVersion::V1);

        let mut missing_schema = Buffer::new();
        crate::avro::write_container(
            &mut missing_schema,
            &avro_schema,
            &[("partition-spec", partition_spec.as_str())],
            &[],
        )
        .unwrap();
        let message = read_manifest_spec(&missing_schema).unwrap_err().to_string();
        assert!(message.contains("schema is required"), "{message}");

        let mut missing_spec = Buffer::new();
        crate::avro::write_container(
            &mut missing_spec,
            &avro_schema,
            &[("schema", schema.as_str())],
            &[],
        )
        .unwrap();
        let message = read_manifest_spec(&missing_spec).unwrap_err().to_string();
        assert!(message.contains("partition-spec is required"), "{message}");

        let mut malformed_schema = Buffer::new();
        crate::avro::write_container(
            &mut malformed_schema,
            &avro_schema,
            &[("schema", "{"), ("partition-spec", partition_spec.as_str())],
            &[],
        )
        .unwrap();
        let message = read_manifest_spec(&malformed_schema)
            .unwrap_err()
            .to_string();
        assert!(message.contains("parse schema"), "{message}");

        let mut malformed_spec = Buffer::new();
        crate::avro::write_container(
            &mut malformed_spec,
            &avro_schema,
            &[("schema", schema.as_str()), ("partition-spec", "{")],
            &[],
        )
        .unwrap();
        let message = read_manifest_spec(&malformed_spec).unwrap_err().to_string();
        assert!(message.contains("parse partition spec"), "{message}");
    }

    #[test]
    fn manifest_spec_rejects_invalid_utf8_required_metadata() {
        let field = field();
        let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
        let (avro_schema, schema, partition_spec) =
            manifest_header_parts(&field, &spec, FormatVersion::V1);
        let mut encoded = Buffer::new();
        crate::avro::write_container(
            &mut encoded,
            &avro_schema,
            &[
                ("schema", schema.as_str()),
                ("partition-spec", partition_spec.as_str()),
            ],
            &[],
        )
        .unwrap();

        for key in ["schema", "partition-spec"] {
            let malformed = with_invalid_utf8_metadata(&encoded, key);
            let message = read_manifest_spec(&malformed).unwrap_err().to_string();
            assert!(message.contains("parse"), "{key}: {message}");
        }
    }

    #[test]
    fn manifest_header_defaults_preserve_official_v1_semantics() {
        let field = field();
        let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
        let (avro_schema, schema, partition_spec) =
            manifest_header_parts(&field, &spec, FormatVersion::V1);
        let partition = spec.partition_field(&field).unwrap();
        let entry = ManifestEntry::added(
            41,
            DataFile {
                file_path: "s3://warehouse/table/data/part.parquet".into(),
                partition: vec![Scalar::from("XNAS")],
                record_count: 1,
                file_size_in_bytes: 1,
                ..DataFile::default()
            },
        );
        let row = entry_to_value(&entry, FormatVersion::V1, &partition).unwrap();
        let mut encoded = Buffer::new();
        crate::avro::write_container(
            &mut encoded,
            &avro_schema,
            &[
                ("schema", schema.as_str()),
                ("partition-spec", partition_spec.as_str()),
            ],
            &[row],
        )
        .unwrap();

        let official = OfficialManifest::parse_avro(encoded.as_slice()).unwrap();
        assert_eq!(official.metadata().schema_id(), 0);
        assert_eq!(official.metadata().partition_spec().spec_id(), 0);
        assert_eq!(
            official.metadata().format_version(),
            &OfficialFormatVersion::V1
        );
        assert_eq!(
            official.metadata().content(),
            &OfficialManifestContent::Data
        );

        let mut expected_spec = spec;
        expected_spec.spec_id = 0;
        assert_eq!(read_manifest_spec(&encoded).unwrap(), expected_spec);
        let decoded = read_manifest(&encoded).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].snapshot_id, Some(41));
        assert_eq!(decoded[0].sequence_number, Some(0));
        assert_eq!(decoded[0].file_sequence_number, Some(0));
        assert_eq!(decoded[0].data_file.content, 0);
    }

    #[test]
    fn planning_reader_rejects_missing_required_fields() {
        let field = field();
        let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
        let partition = spec.partition_field(&field).unwrap();
        let schema = replace_data_file_schema_field(
            &manifest_entry_schema(FormatVersion::V2, &partition).unwrap(),
            "record_count",
            None,
        );
        let input = ManifestEntry::added(
            41,
            DataFile {
                file_path: "s3://warehouse/table/data/part.parquet".into(),
                partition: vec![Scalar::from("XNAS")],
                record_count: 7,
                file_size_in_bytes: 128,
                ..DataFile::default()
            },
        );
        let row = with_data_file_value(
            &entry_to_value(&input, FormatVersion::V2, &partition).unwrap(),
            "record_count",
            None,
        );
        let mut handle = Buffer::new();
        write_nonconforming_manifest(&mut handle, &schema, &field, &spec, row);

        assert!(
            fixed_uuid_official_reader_view(handle.as_slice())
                .unwrap()
                .is_none()
        );
        assert!(read_manifest(&handle).is_err());
        assert!(read_manifest_for_plan(&handle, false).is_err());
    }

    #[test]
    fn planning_reader_rejects_malformed_statistics_members() {
        let field = field();
        let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
        let partition = spec.partition_field(&field).unwrap();
        let string_counts =
            Scalar::from_sequence([Scalar::from("null"), avro_map(119, 120, "string").unwrap()]);
        let schema = replace_data_file_schema_field(
            &manifest_entry_schema(FormatVersion::V2, &partition).unwrap(),
            "value_counts",
            Some(string_counts),
        );
        let input = ManifestEntry::added(
            41,
            DataFile {
                file_path: "s3://warehouse/table/data/part.parquet".into(),
                partition: vec![Scalar::from("XNAS")],
                record_count: 7,
                file_size_in_bytes: 128,
                ..DataFile::default()
            },
        );
        let malformed_counts = Scalar::from_sequence([Scalar::from_mapping([
            (Scalar::from("key"), Scalar::from(1_i64)),
            (Scalar::from("value"), Scalar::from("seven")),
        ])
        .unwrap()]);
        let row = with_data_file_value(
            &entry_to_value(&input, FormatVersion::V2, &partition).unwrap(),
            "value_counts",
            Some(malformed_counts),
        );
        let mut handle = Buffer::new();
        write_nonconforming_manifest(&mut handle, &schema, &field, &spec, row);

        assert!(read_manifest(&handle).is_err());
        assert!(read_manifest_for_plan(&handle, true).is_err());
    }

    #[test]
    fn raw_manifest_preflight_rejects_duplicate_metric_ids_before_official_maps() {
        for name in [
            "column_sizes",
            "value_counts",
            "null_value_counts",
            "nan_value_counts",
            "lower_bounds",
            "upper_bounds",
        ] {
            let value = if matches!(name, "lower_bounds" | "upper_bounds") {
                Scalar::from(1_i64.to_le_bytes().to_vec())
            } else {
                Scalar::from(1_i64)
            };
            let handle =
                manifest_with_data_file_value(name, metric_pairs([(1, value.clone()), (1, value)]));
            assert!(
                OfficialManifest::parse_avro(handle.as_slice()).is_ok(),
                "official map conversion should demonstrate the lossy duplicate case for {name}"
            );
            assert_preflight_error(&handle, &format!("unique field ids in {name}"));
        }
    }

    #[test]
    fn raw_manifest_preflight_rejects_negative_metric_counts() {
        for name in [
            "column_sizes",
            "value_counts",
            "null_value_counts",
            "nan_value_counts",
        ] {
            let handle =
                manifest_with_data_file_value(name, metric_pairs([(1, Scalar::from(-1_i64))]));
            assert_preflight_error(&handle, &format!("non-negative count in {name}"));
        }
    }

    #[test]
    fn raw_manifest_preflight_rejects_invalid_split_offsets() {
        for offsets in [vec![-1_i64], vec![4, 4], vec![8, 4]] {
            let handle = manifest_with_data_file_value(
                "split_offsets",
                Scalar::from_sequence(offsets.into_iter().map(Scalar::from)),
            );
            assert_preflight_error(&handle, "non-negative strictly ascending split_offsets");
        }
    }

    #[test]
    fn public_manifest_reads_are_official_parser_views() {
        let field = field();
        let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
        for version in [FormatVersion::V1, FormatVersion::V2, FormatVersion::V3] {
            let input = ManifestEntry::added(
                41,
                DataFile {
                    file_path: "s3://warehouse/table/data/part.parquet".into(),
                    partition: vec![Scalar::from("XNAS")],
                    record_count: 7,
                    file_size_in_bytes: 512,
                    value_counts: vec![(1, 7)],
                    null_value_counts: vec![(1, 0)],
                    lower_bounds: vec![(1, 1_i64.to_le_bytes().to_vec())],
                    upper_bounds: vec![(1, 7_i64.to_le_bytes().to_vec())],
                    key_metadata: Some(vec![1, 2, 3]),
                    split_offsets: vec![4],
                    sort_order_id: Some(0),
                    first_row_id: (version == FormatVersion::V3).then_some(70),
                    ..DataFile::default()
                },
            );
            let mut handle = Buffer::new();
            write_manifest(
                &mut handle,
                version,
                &field,
                &spec,
                std::slice::from_ref(&input),
            )
            .unwrap();

            let official = OfficialManifest::parse_avro(handle.as_slice()).unwrap();
            assert_eq!(
                *official.metadata().format_version(),
                official_version(version)
            );
            assert_eq!(official.entries().len(), 1);

            let output = read_manifest(&handle).unwrap();
            assert_eq!(output.len(), 1);
            assert_eq!(output[0].status, EntryStatus::Added);
            assert_eq!(output[0].snapshot_id, Some(41));
            assert_eq!(output[0].data_file, input.data_file);
            if version == FormatVersion::V1 {
                assert_eq!(output[0].sequence_number, Some(0));
                assert_eq!(output[0].file_sequence_number, Some(0));
            }
            assert_eq!(read_manifest_spec(&handle).unwrap(), spec);
        }
    }

    #[test]
    fn planning_reader_preserves_nonlexical_partition_spec_order() {
        let mut field = DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Utf8.required_field("z"),
            DataType::Utf8.required_field("a"),
        ])
        .unwrap()
        .required_field("row");
        super::super::schema::assign_field_ids(&mut field, 1).unwrap();
        field.insert_metadata("iceberg:schema-id", "0").unwrap();
        let spec = PartitionSpec::identity(3, &field, &["z", "a"]).unwrap();
        let input = ManifestEntry::added(
            41,
            DataFile {
                file_path: "s3://warehouse/table/data/part.parquet".into(),
                partition: vec![Scalar::from("Z"), Scalar::from("A")],
                record_count: 1,
                file_size_in_bytes: 128,
                column_sizes: vec![(1, 8)],
                value_counts: vec![(1, 1)],
                null_value_counts: vec![(1, 0)],
                lower_bounds: vec![(1, 1_i64.to_le_bytes().to_vec())],
                upper_bounds: vec![(1, 1_i64.to_le_bytes().to_vec())],
                split_offsets: vec![4],
                ..DataFile::default()
            },
        );
        let mut handle = Buffer::new();
        write_manifest(&mut handle, FormatVersion::V2, &field, &spec, &[input]).unwrap();

        let official_partition = read_manifest(&handle).unwrap()[0]
            .data_file
            .partition
            .clone();
        assert_eq!(
            official_partition,
            vec![Scalar::from("Z"), Scalar::from("A")]
        );
        for with_stats in [false, true] {
            let planned = read_manifest_for_plan(&handle, with_stats).unwrap();
            assert_eq!(planned[0].data_file.partition, official_partition);
            assert!(planned[0].data_file.column_sizes.is_empty());
            assert!(planned[0].data_file.split_offsets.is_empty());
            assert_eq!(
                planned[0].data_file.value_counts,
                if with_stats { vec![(1, 1)] } else { Vec::new() }
            );
        }
    }

    #[test]
    fn official_manifest_reads_preserve_unknown_partition_transforms() {
        let field = field();
        let mut spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
        spec.fields[0].name = "venue_opaque".into();
        spec.fields[0].transform = Transform::Unknown;
        let input = ManifestEntry::added(
            41,
            DataFile {
                file_path: "s3://warehouse/table/data/part.parquet".into(),
                partition: vec![Scalar::from("opaque")],
                record_count: 1,
                file_size_in_bytes: 128,
                ..DataFile::default()
            },
        );
        let mut handle = Buffer::new();
        write_manifest(&mut handle, FormatVersion::V2, &field, &spec, &[input]).unwrap();

        assert_eq!(read_manifest_spec(&handle).unwrap(), spec);
        assert_eq!(
            read_manifest(&handle).unwrap()[0].data_file.partition,
            vec![Scalar::from("opaque")]
        );
    }

    #[test]
    fn official_uuid_partition_literals_use_the_core_fixed_binary_shape() {
        let document = crate::json::from_utf8(
            r#"{"type":"struct","schema-id":0,"fields":[
                {"id":1,"name":"id","required":true,"type":"long"},
                {"id":2,"name":"token","required":true,"type":"uuid"}
            ]}"#,
        )
        .unwrap();
        let field = super::super::schema::schema_from_json("row", &document).unwrap();
        let spec = PartitionSpec::identity(4, &field, &["token"]).unwrap();
        let token = 0x0db3_e2a8_9d1d_42b9_aa7b_74eb_e558_dcebu128
            .to_be_bytes()
            .to_vec();
        let input = ManifestEntry::added(
            41,
            DataFile {
                file_path: "s3://warehouse/table/data/part.parquet".into(),
                partition: vec![Scalar::from(token.clone())],
                record_count: 1,
                file_size_in_bytes: 128,
                ..DataFile::default()
            },
        );
        let mut handle = Buffer::new();
        write_manifest(&mut handle, FormatVersion::V2, &field, &spec, &[input]).unwrap();
        let container = crate::avro::read_container(&handle).unwrap();
        assert!(contains_fixed_uuid(&container.schema.into_json()));
        assert_eq!(
            container.rows[0]
                .path("data_file.partition.token")
                .and_then(Scalar::as_bytes),
            Some(token.as_slice())
        );
        let read = read_manifest(&handle).unwrap();
        assert_eq!(read[0].data_file.partition, vec![Scalar::from(token)]);

        let mut rewritten = Buffer::new();
        write_manifest(&mut rewritten, FormatVersion::V2, &field, &spec, &read).unwrap();
        assert_eq!(read_manifest(&rewritten).unwrap(), read);
    }

    #[test]
    fn manifest_writer_rejects_non_iceberg_mime_types_without_writing() {
        let field = field();
        let spec = PartitionSpec::unpartitioned();
        let entry = ManifestEntry::added(
            41,
            DataFile {
                file_path: "s3://warehouse/table/data/part.json".into(),
                mime_type: MimeType::JSON,
                record_count: 1,
                file_size_in_bytes: 2,
                ..DataFile::default()
            },
        );
        let mut handle = Buffer::new();

        let message = write_manifest(&mut handle, FormatVersion::V2, &field, &spec, &[entry])
            .unwrap_err()
            .to_string();

        assert!(message.contains("Iceberg MIME type"), "{message}");
        assert!(message.contains(MimeType::JSON.as_str()), "{message}");
        assert!(handle.as_slice().is_empty());
        assert!(OfficialManifest::parse_avro(handle.as_slice()).is_err());
    }

    #[test]
    fn delete_manifest_metadata_is_official_and_mixed_content_is_refused() {
        let field = field();
        let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
        for version in [FormatVersion::V2, FormatVersion::V3] {
            let deleted = ManifestEntry::added(
                41,
                DataFile {
                    content: 2,
                    file_path: "s3://warehouse/table/data/delete.parquet".into(),
                    partition: vec![Scalar::from("XNAS")],
                    record_count: 1,
                    file_size_in_bytes: 128,
                    equality_ids: Some(vec![1]),
                    ..DataFile::default()
                },
            );
            let mut handle = Buffer::new();
            write_manifest(
                &mut handle,
                version,
                &field,
                &spec,
                std::slice::from_ref(&deleted),
            )
            .unwrap();
            let official = OfficialManifest::parse_avro(handle.as_slice()).unwrap();
            assert_eq!(
                official.metadata().content(),
                &OfficialManifestContent::Deletes
            );
            assert_eq!(read_manifest(&handle).unwrap(), vec![deleted.clone()]);

            let data = ManifestEntry::added(
                41,
                DataFile {
                    file_path: "s3://warehouse/table/data/part.parquet".into(),
                    partition: vec![Scalar::from("XNAS")],
                    record_count: 1,
                    file_size_in_bytes: 128,
                    ..DataFile::default()
                },
            );
            let message =
                write_manifest(&mut Buffer::new(), version, &field, &spec, &[data, deleted])
                    .unwrap_err()
                    .to_string();
            assert!(message.contains("mixed data and delete files"), "{message}");
        }
    }

    #[test]
    fn versioned_manifest_fields_are_preserved_or_rejected_before_write() {
        let field = field();
        let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
        let vector = ManifestEntry::added(
            41,
            DataFile {
                content: 1,
                file_path: "s3://warehouse/table/data/deletes.puffin".into(),
                mime_type: MimeType::PUFFIN,
                partition: vec![Scalar::from("XNAS")],
                record_count: 1,
                file_size_in_bytes: 128,
                referenced_data_file: Some("s3://warehouse/table/data/part.parquet".into()),
                content_offset: Some(4),
                content_size_in_bytes: Some(64),
                ..DataFile::default()
            },
        );
        let mut handle = Buffer::new();
        write_manifest(
            &mut handle,
            FormatVersion::V3,
            &field,
            &spec,
            std::slice::from_ref(&vector),
        )
        .unwrap();
        assert_eq!(read_manifest(&handle).unwrap(), vec![vector.clone()]);

        let message = write_manifest(
            &mut Buffer::new(),
            FormatVersion::V2,
            &field,
            &spec,
            &[vector],
        )
        .unwrap_err()
        .to_string();
        assert!(
            message.contains("referenced_data_file") && message.contains("v2"),
            "{message}"
        );

        let v1_delete = ManifestEntry::added(
            41,
            DataFile {
                content: 2,
                file_path: "s3://warehouse/table/data/delete.parquet".into(),
                partition: vec![Scalar::from("XNAS")],
                record_count: 1,
                file_size_in_bytes: 128,
                equality_ids: Some(vec![1]),
                ..DataFile::default()
            },
        );
        let message = write_manifest(
            &mut Buffer::new(),
            FormatVersion::V1,
            &field,
            &spec,
            &[v1_delete],
        )
        .unwrap_err()
        .to_string();
        assert!(
            message.contains("only data files") && message.contains("v1"),
            "{message}"
        );

        let manifest = ManifestFile {
            manifest_path: "s3://warehouse/table/metadata/delete.avro".into(),
            manifest_length: 1,
            partition_spec_id: 0,
            content: ManifestContent::Deletes,
            sequence_number: 1,
            min_sequence_number: 1,
            added_snapshot_id: 41,
            added_files_count: Some(0),
            existing_files_count: Some(0),
            deleted_files_count: Some(0),
            added_rows_count: Some(0),
            existing_rows_count: Some(0),
            deleted_rows_count: Some(0),
            partitions: Vec::new(),
            key_metadata: None,
            first_row_id: None,
        };
        let message = write_manifest_list(
            &mut Buffer::new(),
            FormatVersion::V1,
            41,
            None,
            0,
            None,
            &[manifest],
        )
        .unwrap_err()
        .to_string();
        assert!(message.contains("v1 data manifest"), "{message}");
    }

    #[test]
    fn public_manifest_list_reads_are_official_parser_views() {
        for version in [FormatVersion::V1, FormatVersion::V2, FormatVersion::V3] {
            let input = ManifestFile {
                manifest_path: "s3://warehouse/table/metadata/manifest.avro".into(),
                manifest_length: 1_024,
                partition_spec_id: 3,
                content: ManifestContent::Data,
                sequence_number: if version == FormatVersion::V1 { 0 } else { 5 },
                min_sequence_number: if version == FormatVersion::V1 { 0 } else { 2 },
                added_snapshot_id: 41,
                added_files_count: Some(1),
                existing_files_count: Some(2),
                deleted_files_count: Some(3),
                added_rows_count: Some(7),
                existing_rows_count: Some(11),
                deleted_rows_count: Some(13),
                partitions: vec![FieldSummary {
                    contains_null: false,
                    contains_nan: Some(false),
                    lower_bound: Some(vec![1]),
                    upper_bound: Some(vec![9]),
                }],
                key_metadata: Some(vec![3, 1, 4]),
                first_row_id: (version == FormatVersion::V3).then_some(70),
            };
            let mut handle = Buffer::new();
            write_manifest_list(
                &mut handle,
                version,
                41,
                None,
                5,
                (version == FormatVersion::V3).then_some(70),
                std::slice::from_ref(&input),
            )
            .unwrap();

            let official = OfficialManifestList::parse_with_version(
                handle.as_slice(),
                official_version(version),
            )
            .unwrap();
            assert_eq!(official.entries().len(), 1);
            assert_eq!(read_manifest_list(&handle).unwrap(), vec![input]);

            let mut unknown = read_manifest_list(&handle).unwrap().remove(0);
            unknown.added_files_count = None;
            unknown.existing_files_count = None;
            unknown.deleted_files_count = None;
            unknown.added_rows_count = None;
            unknown.existing_rows_count = None;
            unknown.deleted_rows_count = None;
            let mut handle = Buffer::new();
            if version == FormatVersion::V1 {
                write_manifest_list(&mut handle, version, 41, None, 5, None, &[unknown.clone()])
                    .unwrap();
                assert_eq!(read_manifest_list(&handle).unwrap(), vec![unknown]);
            } else {
                let message = write_manifest_list(
                    &mut handle,
                    version,
                    41,
                    None,
                    5,
                    (version == FormatVersion::V3).then_some(70),
                    &[unknown],
                )
                .unwrap_err()
                .to_string();
                assert!(
                    message.contains("required added_files_count") && message.contains("got null"),
                    "{message}"
                );
            }
        }
    }

    #[test]
    fn v3_manifest_lists_assign_contiguous_row_ranges_and_header() {
        let manifest = |name: &str, added: i64, existing: i64| ManifestFile {
            manifest_path: format!("s3://warehouse/table/metadata/{name}").into(),
            manifest_length: 1_024,
            partition_spec_id: 0,
            content: ManifestContent::Data,
            sequence_number: 5,
            min_sequence_number: 5,
            added_snapshot_id: 41,
            added_files_count: Some(1),
            existing_files_count: Some(1),
            deleted_files_count: Some(0),
            added_rows_count: Some(added),
            existing_rows_count: Some(existing),
            deleted_rows_count: Some(0),
            partitions: Vec::new(),
            key_metadata: None,
            first_row_id: None,
        };
        let manifests = [manifest("a.avro", 3, 2), manifest("b.avro", 7, 0)];
        let mut handle = Buffer::new();
        assert_eq!(
            write_manifest_list(
                &mut handle,
                FormatVersion::V3,
                41,
                None,
                5,
                Some(10),
                &manifests,
            )
            .unwrap(),
            Some(22)
        );

        let container = crate::avro::read_container(&handle).unwrap();
        assert_eq!(container.get("first-row-id"), Some("10"));
        let read = read_manifest_list(&handle).unwrap();
        assert_eq!(read[0].first_row_id, Some(10));
        assert_eq!(read[1].first_row_id, Some(15));
    }

    #[test]
    fn v3_row_range_assignment_fails_before_writing() {
        let manifest = ManifestFile {
            manifest_path: "s3://warehouse/table/metadata/a.avro".into(),
            manifest_length: 1,
            partition_spec_id: 0,
            content: ManifestContent::Data,
            sequence_number: 1,
            min_sequence_number: 1,
            added_snapshot_id: 41,
            added_files_count: Some(1),
            existing_files_count: Some(0),
            deleted_files_count: Some(0),
            added_rows_count: None,
            existing_rows_count: Some(0),
            deleted_rows_count: Some(0),
            partitions: Vec::new(),
            key_metadata: None,
            first_row_id: None,
        };
        let mut handle = Buffer::new();
        let message = write_manifest_list(
            &mut handle,
            FormatVersion::V3,
            41,
            None,
            1,
            Some(0),
            &[manifest],
        )
        .unwrap_err()
        .to_string();
        assert!(message.contains("added_rows_count") && message.contains("null"));
        assert!(handle.as_slice().is_empty());
    }
}
