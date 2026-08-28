//! Table metadata: the JSON document that is the table.
//!
//! Everything else in a table - manifests, data files, the directory layout -
//! is reachable only from this one document, which is why committing a change
//! means writing a new one. It exists in three format versions, and the
//! differences are small but load-bearing:
//!
//! - **v1** stores the current schema as `schema` and the current partition
//!   spec as a bare `partition-spec` array, and has no sequence numbers.
//! - **v2** makes `schemas`/`partition-specs` the authority, adds
//!   `last-sequence-number`, and numbers every snapshot.
//! - **v3** adds row lineage: `next-row-id` on the table and `first-row-id` /
//!   `added-rows` on each snapshot.
//!
//! Reading accepts all three and normalizes the singular forms into the plural
//! ones, so the rest of the module never asks which version it is looking at.
//! Writing emits exactly what the declared version requires.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::{Hash, Hasher};

use iceberg_official::TableUpdate as OfficialTableUpdate;
use iceberg_official::spec::{
    EncryptedKey as OfficialEncryptedKey, FormatVersion as OfficialFormatVersion,
    Operation as OfficialOperation, PartitionStatisticsFile as OfficialPartitionStatisticsFile,
    PrimitiveType as OfficialPrimitiveType, Schema as OfficialSchema, Snapshot as OfficialSnapshot,
    SnapshotReference as OfficialSnapshotReference, SnapshotRetention as OfficialSnapshotRetention,
    SortOrder as OfficialSortOrder, StatisticsFile as OfficialStatisticsFile,
    Summary as OfficialSummary, TableMetadataBuildResult as OfficialTableMetadataBuildResult,
    TableMetadataBuilder as OfficialTableMetadataBuilder,
    TableProperties as OfficialTableProperties, Transform as OfficialTransform,
    Type as OfficialType, UnboundPartitionSpec as OfficialUnboundPartitionSpec,
};
use smol_str::{SmolStr, format_smolstr};

use super::partition::PartitionSpec;
use super::snapshot::{MAIN_BRANCH, Snapshot, SnapshotRef};
use super::{Transform, schema_from_json, schema_to_json};
use crate::{Error, Field, Result, Scalar};

/// Which revision of the Iceberg table specification a table is written to.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum FormatVersion {
    /// The original format: one schema, one spec, no sequence numbers.
    V1,
    /// Row-level deletes, sequence numbers, and multiple schemas and specs.
    #[default]
    V2,
    /// Row lineage, nanosecond temporals, and default values.
    V3,
}

impl FormatVersion {
    /// Return the integer a metadata document stores.
    pub const fn number(self) -> i32 {
        match self {
            Self::V1 => 1,
            Self::V2 => 2,
            Self::V3 => 3,
        }
    }

    /// Read the version one stored integer names.
    ///
    /// # Errors
    ///
    /// Returns an error naming the value when it is not 1, 2, or 3.
    pub fn from_number(number: i64) -> Result<Self> {
        match number {
            1 => Ok(Self::V1),
            2 => Ok(Self::V2),
            3 => Ok(Self::V3),
            other => Err(invalid(format_smolstr!(
                "expected an Iceberg format version of 1, 2, or 3, got {other}"
            ))),
        }
    }
}

/// One column a table's rows are sorted by.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SortField {
    /// Identifier of the schema field sorted on.
    pub source_id: i32,
    /// How the value is transformed before comparison.
    pub transform: Transform,
    /// Either `asc` or `desc`.
    pub direction: SmolStr,
    /// Either `nulls-first` or `nulls-last`.
    pub null_order: SmolStr,
}

impl SortField {
    /// Return a deterministic hash of this complete sort field.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }
}

/// An identified ordering a table's writers maintain.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SortOrder {
    /// Identifier of this order within the table.
    pub order_id: i64,
    /// The sort columns, most significant first.
    pub fields: Vec<SortField>,
}

impl SortOrder {
    /// Return a deterministic hash of this complete sort order.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }

    /// The unsorted order, which every table has as order zero.
    pub const fn unsorted() -> Self {
        Self {
            order_id: 0,
            fields: Vec::new(),
        }
    }

    /// Read one sort order object.
    ///
    /// # Errors
    ///
    /// Returns an error when a sort field names a transform Iceberg does not.
    pub fn from_json(document: &Scalar) -> Result<Self> {
        let order_id = document
            .get_key_str("order-id")
            .and_then(Scalar::as_i64)
            .ok_or_else(|| {
                invalid(SmolStr::new_static(
                    "expected a 64-bit integer sort order \"order-id\"",
                ))
            })?;
        let entries = document
            .get_key_str("fields")
            .and_then(Scalar::as_sequence)
            .ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected a sort field array on sort order {order_id}"
                ))
            })?;
        let mut fields = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            let source_id = entry
                .get_key_str("source-id")
                .and_then(Scalar::as_i64)
                .and_then(|id| i32::try_from(id).ok())
                .ok_or_else(|| {
                    invalid(format_smolstr!(
                        "expected a 32-bit integer source-id on sort order {order_id} field {index}"
                    ))
                })?;
            let transform = entry
                .get_key_str("transform")
                .and_then(Scalar::as_str)
                .ok_or_else(|| {
                    invalid(format_smolstr!(
                        "expected a transform string on sort order {order_id} field {index}"
                    ))
                })?;
            let direction = entry
                .get_key_str("direction")
                .and_then(Scalar::as_str)
                .ok_or_else(|| {
                    invalid(format_smolstr!(
                        "expected a direction string on sort order {order_id} field {index}"
                    ))
                })?;
            let null_order = entry
                .get_key_str("null-order")
                .and_then(Scalar::as_str)
                .ok_or_else(|| {
                    invalid(format_smolstr!(
                        "expected a null-order string on sort order {order_id} field {index}"
                    ))
                })?;
            fields.push(SortField {
                source_id,
                transform: Transform::from_str(transform)?,
                direction: SmolStr::new(direction),
                null_order: SmolStr::new(null_order),
            });
        }
        let order = Self { order_id, fields };
        order.validate_shape()?;
        Ok(order)
    }

    fn validate_shape(&self) -> Result<()> {
        if self.order_id < 0 {
            return Err(invalid(format_smolstr!(
                "expected a non-negative sort order id, got {}",
                self.order_id
            )));
        }
        if self.fields.is_empty() {
            if self.order_id != 0 {
                return Err(invalid(format_smolstr!(
                    "expected only sort order 0 to be unsorted, got empty order {}",
                    self.order_id
                )));
            }
        } else if self.order_id == 0 {
            return Err(invalid(SmolStr::new_static(
                "expected sort order 0 to be unsorted",
            )));
        }
        for field in &self.fields {
            if field.source_id <= 0 {
                return Err(invalid(format_smolstr!(
                    "expected a positive sort source-id, got {}",
                    field.source_id
                )));
            }
            if !matches!(field.direction.as_str(), "asc" | "desc") {
                return Err(invalid(format_smolstr!(
                    "expected a sort direction of asc or desc, got {:?}",
                    crate::text::elide_to(&field.direction, 64)
                )));
            }
            if !matches!(field.null_order.as_str(), "nulls-first" | "nulls-last") {
                return Err(invalid(format_smolstr!(
                    "expected a null order of nulls-first or nulls-last, got {:?}",
                    crate::text::elide_to(&field.null_order, 64)
                )));
            }
        }
        Ok(())
    }

    /// Write one sort order object.
    ///
    /// # Errors
    ///
    /// Returns an error only when the mapping cannot be built.
    pub fn into_json(self) -> Result<Scalar> {
        self.validate_shape()?;
        let mut fields = Vec::with_capacity(self.fields.len());
        for field in &self.fields {
            fields.push(Scalar::from_mapping([
                (
                    Scalar::from("source-id"),
                    Scalar::from(i64::from(field.source_id)),
                ),
                (
                    Scalar::from("transform"),
                    Scalar::from(field.transform.to_string()),
                ),
                (
                    Scalar::from("direction"),
                    Scalar::from(field.direction.clone()),
                ),
                (
                    Scalar::from("null-order"),
                    Scalar::from(field.null_order.clone()),
                ),
            ])?);
        }
        Scalar::from_mapping([
            (Scalar::from("order-id"), Scalar::from(self.order_id)),
            (Scalar::from("fields"), Scalar::from_sequence(fields)),
        ])
    }
}

/// The complete state of an Iceberg table at one point in time.
#[derive(Clone, Debug)]
pub struct TableMetadata {
    /// Which revision of the specification this document is written to.
    pub(super) format_version: FormatVersion,
    /// A stable identifier for the table itself, not for any one version.
    pub(super) table_uuid: SmolStr,
    /// The table's base location, as a URI.
    pub(super) location: SmolStr,
    /// Highest assigned sequence number, absent in v1.
    pub(super) last_sequence_number: i64,
    /// When this document was written, in milliseconds since the Unix epoch.
    pub(super) last_updated_ms: i64,
    /// Highest assigned column identifier.
    pub(super) last_column_id: i32,
    /// Every schema the table has had, by identifier.
    pub(super) schemas: Vec<Field>,
    /// The schema new data is written against.
    pub(super) current_schema_id: i32,
    /// Every partition spec the table has had.
    pub(super) partition_specs: Vec<PartitionSpec>,
    /// The spec new data is written against.
    pub(super) default_spec_id: i32,
    /// Highest assigned partition field identifier.
    pub(super) last_partition_id: i32,
    /// Every sort order the table has had.
    pub(super) sort_orders: Vec<SortOrder>,
    /// The order new data is written in.
    pub(super) default_sort_order_id: i64,
    /// Free-form table properties.
    pub(super) properties: Vec<(SmolStr, SmolStr)>,
    /// The snapshot a reader sees, when the table has one.
    pub(super) current_snapshot_id: Option<i64>,
    /// Every retained snapshot, oldest first.
    pub(super) snapshots: Vec<Snapshot>,
    /// When each snapshot became current, oldest first.
    pub(super) snapshot_log: Vec<(i64, i64)>,
    /// Every previous metadata document, oldest first.
    pub(super) metadata_log: Vec<(i64, SmolStr)>,
    /// Named branches and tags.
    pub(super) refs: Vec<(SmolStr, SnapshotRef)>,
    /// Snapshot-level Puffin statistics descriptors retained by the official
    /// metadata model.
    statistics: Vec<Scalar>,
    /// Partition statistics descriptors retained by the official metadata
    /// model.
    partition_statistics: Vec<Scalar>,
    /// V3 encryption keys retained by the official metadata model.
    encryption_keys: Vec<Scalar>,
    /// Next unassigned row identifier, required in v3.
    pub(super) next_row_id: Option<i64>,
}

/// Complete semantic metadata identity. Iceberg models schemas, specs, sort
/// orders, properties, snapshots, and refs as keyed collections, so wire
/// order is excluded. The two history logs are timelines and retain order.
#[derive(Eq, Hash, Ord, PartialEq, PartialOrd)]
struct TableMetadataIdentity<'a> {
    format_version: FormatVersion,
    table_uuid: &'a SmolStr,
    location: &'a SmolStr,
    last_sequence_number: i64,
    last_updated_ms: i64,
    last_column_id: i32,
    schemas: Vec<&'a Field>,
    current_schema_id: i32,
    partition_specs: Vec<&'a PartitionSpec>,
    default_spec_id: i32,
    last_partition_id: i32,
    sort_orders: Vec<&'a SortOrder>,
    default_sort_order_id: i64,
    properties: Vec<&'a (SmolStr, SmolStr)>,
    current_snapshot_id: Option<i64>,
    snapshots: Vec<&'a Snapshot>,
    snapshot_log: &'a [(i64, i64)],
    metadata_log: &'a [(i64, SmolStr)],
    refs: Vec<&'a (SmolStr, SnapshotRef)>,
    statistics: Vec<&'a Scalar>,
    partition_statistics: Vec<&'a Scalar>,
    encryption_keys: Vec<&'a Scalar>,
    next_row_id: Option<i64>,
}

impl TableMetadata {
    /// Iceberg specification revision of this document.
    pub const fn format_version(&self) -> FormatVersion {
        self.format_version
    }

    /// Stable table identifier.
    pub fn table_uuid(&self) -> &str {
        &self.table_uuid
    }

    /// Canonical table location.
    pub fn location(&self) -> &str {
        &self.location
    }

    /// Highest assigned sequence number.
    pub const fn last_sequence_number(&self) -> i64 {
        self.last_sequence_number
    }

    /// Document update time in Unix epoch milliseconds.
    pub const fn last_updated_ms(&self) -> i64 {
        self.last_updated_ms
    }

    /// Highest assigned column identifier.
    pub const fn last_column_id(&self) -> i32 {
        self.last_column_id
    }

    /// Retained schemas.
    pub fn schemas(&self) -> &[Field] {
        &self.schemas
    }

    /// Current schema identifier.
    pub const fn current_schema_id(&self) -> i32 {
        self.current_schema_id
    }

    /// Retained partition specs.
    pub fn partition_specs(&self) -> &[PartitionSpec] {
        &self.partition_specs
    }

    /// Default partition-spec identifier.
    pub const fn default_spec_id(&self) -> i32 {
        self.default_spec_id
    }

    /// Highest assigned partition-field identifier.
    pub const fn last_partition_id(&self) -> i32 {
        self.last_partition_id
    }

    /// Retained sort orders.
    pub fn sort_orders(&self) -> &[SortOrder] {
        &self.sort_orders
    }

    /// Default sort-order identifier.
    pub const fn default_sort_order_id(&self) -> i64 {
        self.default_sort_order_id
    }

    /// Sorted table properties.
    pub fn properties(&self) -> &[(SmolStr, SmolStr)] {
        &self.properties
    }

    /// Current snapshot identifier.
    pub const fn current_snapshot_id(&self) -> Option<i64> {
        self.current_snapshot_id
    }

    /// Retained snapshots, oldest first.
    pub fn snapshots(&self) -> &[Snapshot] {
        &self.snapshots
    }

    /// Current-snapshot history, oldest first.
    pub fn snapshot_log(&self) -> &[(i64, i64)] {
        &self.snapshot_log
    }

    /// Previous metadata files, oldest first.
    pub fn metadata_log(&self) -> &[(i64, SmolStr)] {
        &self.metadata_log
    }

    /// Sorted branch and tag references.
    pub fn refs(&self) -> &[(SmolStr, SnapshotRef)] {
        &self.refs
    }

    /// Next unassigned v3 row identifier.
    pub const fn next_row_id(&self) -> Option<i64> {
        self.next_row_id
    }

    /// Apply one official metadata-builder operation and replace this view
    /// only after the complete result validates.
    fn apply_official_update<F>(
        &mut self,
        current_file_location: Option<String>,
        update: F,
    ) -> Result<()>
    where
        F: FnOnce(
            OfficialTableMetadataBuilder,
        ) -> iceberg_official::Result<OfficialTableMetadataBuilder>,
    {
        self.apply_official_update_with_v1_manifests(
            current_file_location,
            super::official::V1SnapshotManifests::default(),
            update,
        )
    }

    fn apply_official_update_with_v1_manifests<F>(
        &mut self,
        current_file_location: Option<String>,
        additional_v1_manifests: super::official::V1SnapshotManifests,
        update: F,
    ) -> Result<()>
    where
        F: FnOnce(
            OfficialTableMetadataBuilder,
        ) -> iceberg_official::Result<OfficialTableMetadataBuilder>,
    {
        self.apply_official_update_result(
            current_file_location,
            additional_v1_manifests,
            update,
            |_| Ok(()),
        )
    }

    /// Apply one official update and extract its authoritative assigned ids
    /// before publishing the converted local view.
    fn apply_official_update_result<F, G, T>(
        &mut self,
        current_file_location: Option<String>,
        additional_v1_manifests: super::official::V1SnapshotManifests,
        update: F,
        extract: G,
    ) -> Result<T>
    where
        F: FnOnce(
            OfficialTableMetadataBuilder,
        ) -> iceberg_official::Result<OfficialTableMetadataBuilder>,
        G: FnOnce(&OfficialTableMetadataBuildResult) -> Result<T>,
    {
        let document = self.clone().into_json_document()?;
        let (metadata, mut v1_manifests) = super::official::parse_table_metadata(&document)?;
        for (snapshot_id, manifests) in additional_v1_manifests.into_entries() {
            v1_manifests.insert(snapshot_id, manifests);
        }
        let builder =
            update(metadata.into_builder(current_file_location)).map_err(Error::from_iceberg)?;
        let built = builder.build().map_err(Error::from_iceberg)?;
        let extracted = extract(&built)?;
        let document = super::official::table_metadata_document(&built.metadata, &v1_manifests)?;
        let mut replacement = Self::from_normalized_json(&document)?;
        // Apache Iceberg schemas do not carry Yggdryl's inert root protocol
        // properties. Preserve them across metadata-builder updates while the
        // official model remains authoritative for Iceberg schema state.
        for schema in &mut replacement.schemas {
            let schema_id = field_schema_id(schema);
            if let Some(previous) = self
                .schemas
                .iter()
                .find(|candidate| field_schema_id(candidate) == schema_id)
            {
                schema.update_metadata(
                    previous
                        .metadata_iter()
                        .map(|(key, value)| (key.to_owned(), value.to_owned())),
                )?;
            }
        }
        *self = replacement;
        Ok(extracted)
    }

    /// Let the official builder finalize timestamps, metadata history, and
    /// configured history expiry immediately before publication.
    pub(super) fn finalize_official(
        &mut self,
        current_file_location: Option<String>,
    ) -> Result<()> {
        self.apply_official_update(current_file_location, Ok)
    }

    /// Resolve metadata-file compression with Apache Iceberg's property parser.
    pub(super) fn metadata_compression_codec(
        &self,
    ) -> Result<iceberg_official::compression::CompressionCodec> {
        let key = OfficialTableProperties::PROPERTY_METADATA_COMPRESSION_CODEC;
        let properties = self
            .property(key)
            .map(|value| HashMap::from([(key.to_owned(), value.to_owned())]))
            .unwrap_or_default();
        OfficialTableProperties::try_from(&properties)
            .map(|properties| properties.metadata_compression_codec)
            .map_err(Error::from_iceberg)
    }

    fn identity(&self) -> TableMetadataIdentity<'_> {
        TableMetadataIdentity {
            format_version: self.format_version,
            table_uuid: &self.table_uuid,
            location: &self.location,
            last_sequence_number: self.last_sequence_number,
            last_updated_ms: self.last_updated_ms,
            last_column_id: self.last_column_id,
            schemas: crate::generic::sorted_values(&self.schemas),
            current_schema_id: self.current_schema_id,
            partition_specs: crate::generic::sorted_values(&self.partition_specs),
            default_spec_id: self.default_spec_id,
            last_partition_id: self.last_partition_id,
            sort_orders: crate::generic::sorted_values(&self.sort_orders),
            default_sort_order_id: self.default_sort_order_id,
            properties: crate::generic::sorted_pairs(&self.properties),
            current_snapshot_id: self.current_snapshot_id,
            snapshots: crate::generic::sorted_values(&self.snapshots),
            snapshot_log: &self.snapshot_log,
            metadata_log: &self.metadata_log,
            refs: crate::generic::sorted_pairs(&self.refs),
            statistics: crate::generic::sorted_values(&self.statistics),
            partition_statistics: crate::generic::sorted_values(&self.partition_statistics),
            encryption_keys: crate::generic::sorted_values(&self.encryption_keys),
            next_row_id: self.next_row_id,
        }
    }

    /// Return a deterministic hash of the complete semantic table document.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(&self.identity())
    }

    /// Describe a new, empty table.
    ///
    /// The table has a schema, a spec, and no snapshot, which is exactly what
    /// a freshly created Iceberg table is: reading it must yield no rows rather
    /// than fail.
    ///
    /// # Errors
    ///
    /// Returns an error when the schema is not a valid non-null struct root or
    /// a column identifier would overflow.
    pub fn new(
        format_version: FormatVersion,
        location: impl Into<SmolStr>,
        mut schema: Field,
        spec: PartitionSpec,
    ) -> Result<Self> {
        schema.validate_struct_root()?;
        // Iceberg resolves a column by identifier, so every column needs one
        // before this document can be written. Numbering continues above the
        // highest identifier already assigned - exactly as [`Self::add_schema`]
        // numbers an evolution - so a schema that arrives numbered keeps every
        // id it came with and a plain Arrow schema needs no ceremony first.
        let start = super::last_field_id(&schema)?.saturating_add(1);
        super::assign_field_ids(&mut schema, start)?;
        let last_column_id = super::last_field_id(&schema)?;
        if schema.iceberg().get(super::schema::SCHEMA_ID).is_none() {
            schema.iceberg_mut().insert(super::schema::SCHEMA_ID, "0")?;
        }
        // The schema says how the table is laid out, so the columns the spec
        // partitions on are marked on it rather than only named beside it.
        let schema = spec.mark_partitions(&schema)?;
        let last_partition_id = spec.last_field_id();
        let current_schema_id = schema
            .iceberg()
            .get(super::schema::SCHEMA_ID)
            .and_then(|id| id.parse::<i32>().ok())
            .unwrap_or_default();
        let mut metadata = Self {
            format_version,
            table_uuid: uuid(),
            location: location.into(),
            last_sequence_number: 0,
            last_updated_ms: now_ms(),
            last_column_id,
            schemas: vec![schema],
            current_schema_id,
            default_spec_id: spec.spec_id,
            partition_specs: vec![spec],
            last_partition_id,
            sort_orders: vec![SortOrder::unsorted()],
            default_sort_order_id: 0,
            properties: Vec::new(),
            current_snapshot_id: None,
            snapshots: Vec::new(),
            snapshot_log: Vec::new(),
            metadata_log: Vec::new(),
            refs: Vec::new(),
            statistics: Vec::new(),
            partition_statistics: Vec::new(),
            encryption_keys: Vec::new(),
            next_row_id: (format_version >= FormatVersion::V3).then_some(0),
        };
        metadata.finalize_official(None)?;
        Ok(metadata)
    }

    /// Return the schema new data is written against.
    ///
    /// # Errors
    ///
    /// Returns an error when no schema carries `current-schema-id`.
    pub fn current_schema(&self) -> Result<&Field> {
        self.schema_by_id(self.current_schema_id).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a schema with id {}, got {} schemas",
                self.current_schema_id,
                self.schemas.len()
            ))
        })
    }

    /// Return one schema by identifier.
    pub fn schema_by_id(&self, schema_id: i32) -> Option<&Field> {
        self.schemas.iter().find(|schema| {
            schema
                .iceberg()
                .get(super::schema::SCHEMA_ID)
                .and_then(|id| id.parse::<i32>().ok())
                .unwrap_or_default()
                == schema_id
        })
    }

    /// Return the snapshot a reader sees, when the table has one.
    ///
    /// A table with snapshots can still have no current one - a table that was
    /// just created, or one rolled back past its first commit - so this is an
    /// `Option` rather than a failure.
    pub fn current_snapshot(&self) -> Option<&Snapshot> {
        let current = self.current_snapshot_id?;
        self.snapshot_by_id(current)
    }

    /// Return one snapshot by identifier.
    pub fn snapshot_by_id(&self, snapshot_id: i64) -> Option<&Snapshot> {
        self.snapshots
            .iter()
            .find(|snapshot| snapshot.snapshot_id == snapshot_id)
    }

    /// Return the partition spec new data is written against.
    ///
    /// # Errors
    ///
    /// Returns an error when no spec carries `default-spec-id`.
    pub fn default_spec(&self) -> Result<&PartitionSpec> {
        self.spec_by_id(self.default_spec_id).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a partition spec with id {}, got {} specs",
                self.default_spec_id,
                self.partition_specs.len()
            ))
        })
    }

    /// Return one partition spec by identifier.
    pub fn spec_by_id(&self, spec_id: i32) -> Option<&PartitionSpec> {
        self.partition_specs
            .iter()
            .find(|spec| spec.spec_id == spec_id)
    }

    /// Return one table property.
    pub fn property(&self, key: &str) -> Option<&str> {
        self.properties
            .iter()
            .find_map(|(name, value)| (name == key).then(|| value.as_str()))
    }

    /// Add a schema, numbering any unnumbered column, and return its canonical id.
    ///
    /// This is what schema evolution is at the metadata level: the old schema
    /// stays, so a snapshot written under it still reads correctly, and the
    /// caller chooses when the new one becomes current with
    /// [`Self::set_current_schema`]. A column that carries no identifier is
    /// numbered above `last-column-id` - above every identifier the table has
    /// ever assigned, not merely the ones still in use - which is why an added
    /// column can never be confused with a dropped one.
    ///
    /// # Errors
    ///
    /// Returns an error when the schema is not a valid non-null struct root or
    /// a column identifier would overflow.
    pub fn add_schema(&mut self, mut schema: Field) -> Result<i32> {
        schema.validate_struct_root()?;
        // A fully numbered reusable schema remains valid at i32::MAX. Missing
        // ids still fail inside the checked depth-first assignment.
        let start = self.last_column_id.saturating_add(1);
        schema.assign_parquet_field_ids(start)?;
        validate_schema_evolution(
            self.current_schema()?,
            &schema,
            self.last_column_id,
            self.format_version,
        )?;
        let official = official_schema(&schema)?;
        let reuses_existing = self
            .schemas
            .iter()
            .map(official_schema)
            .collect::<Result<Vec<_>>>()?
            .iter()
            .any(|existing| official_schemas_same(existing, &official));
        if !reuses_existing && self.schemas.iter().map(field_schema_id).max() == Some(i32::MAX) {
            return Err(invalid(SmolStr::new_static(
                "expected an available signed 32-bit schema id, got i32::MAX already assigned",
            )));
        }
        let target = official.clone();
        let mut updated = self.clone();
        let schema_id = updated.apply_official_update_result(
            None,
            super::official::V1SnapshotManifests::default(),
            |builder| builder.add_schema(official),
            |built| {
                let mut ids: Vec<i32> = built
                    .metadata
                    .schemas_iter()
                    .filter(|candidate| official_schemas_same(candidate, &target))
                    .map(|candidate| candidate.schema_id())
                    .collect();
                ids.sort_unstable();
                match ids.as_slice() {
                    [schema_id] => Ok(*schema_id),
                    [] => Err(invalid(SmolStr::new_static(
                        "official Iceberg schema update returned no matching schema",
                    ))),
                    _ => Err(invalid(format_smolstr!(
                        "official Iceberg schema update returned ambiguous matching ids {ids:?}"
                    ))),
                }
            },
        )?;
        if let Some(added) = updated
            .schemas
            .iter_mut()
            .find(|candidate| field_schema_id(candidate) == schema_id)
        {
            added.update_metadata(
                schema
                    .metadata_iter()
                    .map(|(key, value)| (key.to_owned(), value.to_owned())),
            )?;
            added
                .iceberg_mut()
                .insert(super::schema::SCHEMA_ID, schema_id.to_string())?;
        }
        *self = updated;
        Ok(schema_id)
    }

    /// Read a table metadata document of any format version.
    ///
    /// # Errors
    ///
    /// Returns an error when a required key is missing, when the format
    /// version is not one this build implements, or when a nested document is
    /// malformed.
    pub fn from_json(document: &Scalar) -> Result<Self> {
        validate_versioned_document(document)?;
        let normalized = super::official::normalize_table_metadata(document)?;
        Self::from_normalized_json(&normalized)
    }

    /// Build the Yggdryl view from an official, normalized document.
    fn from_normalized_json(document: &Scalar) -> Result<Self> {
        let format_version = FormatVersion::from_number(
            document
                .get_key_str("format-version")
                .and_then(Scalar::as_i64)
                .ok_or_else(|| {
                    invalid(SmolStr::new_static(
                        "expected a table metadata \"format-version\"",
                    ))
                })?,
        )?;
        let location = document
            .get_key_str("location")
            .and_then(Scalar::as_str)
            .ok_or_else(|| {
                invalid(SmolStr::new_static(
                    "expected a table metadata \"location\"",
                ))
            })?;

        // v1 stores the current schema as `schema`; v2 made `schemas` the
        // authority. Reading accepts both and normalizes to the plural.
        let mut schemas = Vec::new();
        for entry in document
            .get_key_str("schemas")
            .map(Scalar::sequence_iter)
            .unwrap_or_default()
        {
            schemas.push(schema_from_json("row", entry)?);
        }
        if schemas.is_empty() {
            if let Some(schema) = document.get_key_str("schema") {
                schemas.push(schema_from_json("row", schema)?);
            }
        }
        if schemas.is_empty() {
            return Err(invalid(SmolStr::new_static(
                "expected a table metadata \"schemas\" array or a v1 \"schema\" object",
            )));
        }
        schemas.sort_by_key(|schema| {
            schema
                .iceberg()
                .get(super::schema::SCHEMA_ID)
                .and_then(|id| id.parse::<i32>().ok())
                .unwrap_or_default()
        });

        let mut partition_specs = Vec::new();
        for entry in document
            .get_key_str("partition-specs")
            .map(Scalar::sequence_iter)
            .unwrap_or_default()
        {
            partition_specs.push(PartitionSpec::from_json(entry)?);
        }
        if partition_specs.is_empty() {
            partition_specs.push(match document.get_key_str("partition-spec") {
                Some(spec) => PartitionSpec::from_json(spec)?,
                None => PartitionSpec::unpartitioned(),
            });
        }
        partition_specs.sort_by_key(|spec| spec.spec_id);

        // A document records the layout in its spec; a Field records it on the
        // columns. Marking them here is what makes a table read back with the
        // same schema it was created with, marks included.
        let default_spec_id = document
            .get_key_str("default-spec-id")
            .and_then(Scalar::as_i64)
            .and_then(|id| i32::try_from(id).ok())
            .unwrap_or_default();
        if let Some(spec) = partition_specs
            .iter()
            .find(|spec| spec.spec_id == default_spec_id)
        {
            for schema in &mut schemas {
                *schema = spec.mark_partitions(schema)?;
            }
        }

        let mut sort_orders = Vec::new();
        for entry in document
            .get_key_str("sort-orders")
            .map(Scalar::sequence_iter)
            .unwrap_or_default()
        {
            sort_orders.push(SortOrder::from_json(entry)?);
        }
        if sort_orders.is_empty() {
            sort_orders.push(SortOrder::unsorted());
        }
        sort_orders.sort_by_key(|order| order.order_id);

        let mut snapshots = Vec::new();
        for entry in document
            .get_key_str("snapshots")
            .map(Scalar::sequence_iter)
            .unwrap_or_default()
        {
            snapshots.push(Snapshot::from_json(entry)?);
        }

        let mut refs = Vec::new();
        if let Some(entries) = document.get_key_str("refs") {
            if let Some(record) = entries.as_record() {
                for (name, entry) in record {
                    refs.push((name.clone(), SnapshotRef::from_json(entry)?));
                }
            } else if let Some(mapping) = entries.as_mapping() {
                for (name, entry) in mapping {
                    if let Some(name) = name.as_str() {
                        refs.push((SmolStr::new(name), SnapshotRef::from_json(entry)?));
                    }
                }
            }
        }
        refs.sort_by(|left, right| left.0.cmp(&right.0));

        let mut properties: Vec<(SmolStr, SmolStr)> = document
            .get_key_str("properties")
            .map(|entries| {
                if let Some(record) = entries.as_record() {
                    record
                        .iter()
                        .map(|(key, value)| (key.clone(), super::value::scalar_text(value)))
                        .collect()
                } else {
                    entries
                        .mapping_iter()
                        .filter_map(|(key, value)| {
                            Some((
                                SmolStr::new(key.as_str()?),
                                super::value::scalar_text(value),
                            ))
                        })
                        .collect()
                }
            })
            .unwrap_or_default();
        properties.sort_by(|left, right| left.0.cmp(&right.0));

        let mut statistics = sequence(document, "statistics");
        statistics.sort();
        let mut partition_statistics = sequence(document, "partition-statistics");
        partition_statistics.sort();
        let mut encryption_keys = sequence(document, "encryption-keys");
        encryption_keys.sort();
        let mut snapshot_log = log_entries(document, "snapshot-log", "snapshot-id");
        // Updating `main` retention through the official builder currently
        // records the unchanged head again. Consecutive identical heads carry
        // no time-travel information, so keep the first transition only.
        snapshot_log.dedup_by_key(|(_, snapshot_id)| *snapshot_id);
        let log_positions: HashMap<i64, usize> = snapshot_log
            .iter()
            .enumerate()
            .map(|(position, (_, snapshot_id))| (*snapshot_id, position))
            .collect();
        // Official metadata stores snapshots in a map. Restore the public
        // oldest-first view by commit time; the log breaks timestamp ties and
        // snapshot id makes branch-only snapshots deterministic.
        snapshots.sort_by_key(|snapshot| {
            (
                snapshot.timestamp_ms,
                log_positions
                    .get(&snapshot.snapshot_id)
                    .copied()
                    .unwrap_or(usize::MAX),
                snapshot.snapshot_id,
            )
        });

        // A table with no snapshot spells that as an absent key or as -1.
        let current_snapshot_id = document
            .get_key_str("current-snapshot-id")
            .and_then(Scalar::as_i64)
            .filter(|id| *id >= 0);

        let metadata = Self {
            format_version,
            table_uuid: SmolStr::new(
                document
                    .get_key_str("table-uuid")
                    .and_then(Scalar::as_str)
                    .unwrap_or_default(),
            ),
            location: SmolStr::new(location),
            last_sequence_number: document
                .get_key_str("last-sequence-number")
                .and_then(Scalar::as_i64)
                .unwrap_or_default(),
            last_updated_ms: document
                .get_key_str("last-updated-ms")
                .and_then(Scalar::as_i64)
                .unwrap_or_default(),
            last_column_id: document
                .get_key_str("last-column-id")
                .and_then(Scalar::as_i64)
                .and_then(|id| i32::try_from(id).ok())
                .unwrap_or_default(),
            current_schema_id: document
                .get_key_str("current-schema-id")
                .and_then(Scalar::as_i64)
                .and_then(|id| i32::try_from(id).ok())
                .unwrap_or_default(),
            schemas,
            default_spec_id,
            last_partition_id: document
                .get_key_str("last-partition-id")
                .and_then(Scalar::as_i64)
                .and_then(|id| i32::try_from(id).ok())
                // v1 could omit the key, so it is recovered from the specs the
                // way Iceberg's own reader recovers it.
                .unwrap_or_else(|| {
                    partition_specs
                        .iter()
                        .map(PartitionSpec::last_field_id)
                        .max()
                        .unwrap_or(super::FIRST_PARTITION_ID - 1)
                }),
            partition_specs,
            default_sort_order_id: document
                .get_key_str("default-sort-order-id")
                .and_then(Scalar::as_i64)
                .ok_or_else(|| {
                    invalid(SmolStr::new_static(
                        "expected a 64-bit integer table metadata \"default-sort-order-id\"",
                    ))
                })?,
            sort_orders,
            properties,
            current_snapshot_id,
            snapshots,
            snapshot_log,
            metadata_log: metadata_log(document),
            refs,
            statistics,
            partition_statistics,
            encryption_keys,
            next_row_id: document.get_key_str("next-row-id").and_then(Scalar::as_i64),
        };
        metadata.validate()?;
        Ok(metadata)
    }

    /// Write this table metadata as the document its format version requires.
    ///
    /// # Errors
    ///
    /// Returns an error when a schema has no field identifiers or a nested
    /// document cannot be built.
    pub fn into_json(self) -> Result<Scalar> {
        self.validate()?;
        self.into_json_document()
    }

    /// Render the local metadata view without re-entering official validation.
    fn into_json_document(self) -> Result<Scalar> {
        let mut entries: Vec<(Scalar, Scalar)> = vec![
            (
                Scalar::from("format-version"),
                Scalar::from(i64::from(self.format_version.number())),
            ),
            (
                Scalar::from("table-uuid"),
                Scalar::from(self.table_uuid.clone()),
            ),
            (
                Scalar::from("location"),
                Scalar::from(self.location.clone()),
            ),
        ];
        if self.format_version >= FormatVersion::V2 {
            entries.push((
                Scalar::from("last-sequence-number"),
                Scalar::from(self.last_sequence_number),
            ));
        }
        entries.push((
            Scalar::from("last-updated-ms"),
            Scalar::from(self.last_updated_ms),
        ));
        entries.push((
            Scalar::from("last-column-id"),
            Scalar::from(i64::from(self.last_column_id)),
        ));

        let mut schemas = Vec::with_capacity(self.schemas.len());
        for schema in &self.schemas {
            schemas.push(schema_to_json(schema)?);
        }
        if self.format_version == FormatVersion::V1 {
            // A v1 reader that predates `schemas` still needs the singular key.
            entries.push((
                Scalar::from("schema"),
                schema_to_json(self.current_schema()?)?,
            ));
        }
        entries.push((Scalar::from("schemas"), Scalar::from_sequence(schemas)));
        entries.push((
            Scalar::from("current-schema-id"),
            Scalar::from(i64::from(self.current_schema_id)),
        ));

        let mut specs = Vec::with_capacity(self.partition_specs.len());
        for spec in &self.partition_specs {
            specs.push(spec.clone().into_json()?);
        }
        if self.format_version == FormatVersion::V1 {
            entries.push((
                Scalar::from("partition-spec"),
                self.default_spec()?.clone().into_v1_json()?,
            ));
        }
        entries.push((
            Scalar::from("partition-specs"),
            Scalar::from_sequence(specs),
        ));
        entries.push((
            Scalar::from("default-spec-id"),
            Scalar::from(i64::from(self.default_spec_id)),
        ));
        entries.push((
            Scalar::from("last-partition-id"),
            Scalar::from(i64::from(self.last_partition_id)),
        ));

        let mut orders = Vec::with_capacity(self.sort_orders.len());
        for order in &self.sort_orders {
            orders.push(order.clone().into_json()?);
        }
        entries.push((Scalar::from("sort-orders"), Scalar::from_sequence(orders)));
        entries.push((
            Scalar::from("default-sort-order-id"),
            Scalar::from(self.default_sort_order_id),
        ));

        entries.push((
            Scalar::from("properties"),
            Scalar::from_mapping(
                self.properties
                    .iter()
                    .map(|(key, value)| (Scalar::from(key.clone()), Scalar::from(value.clone()))),
            )?,
        ));

        if let Some(current) = self.current_snapshot_id {
            entries.push((Scalar::from("current-snapshot-id"), Scalar::from(current)));
        }
        let mut snapshots = Vec::with_capacity(self.snapshots.len());
        for snapshot in &self.snapshots {
            snapshots.push(snapshot.clone().into_json(self.format_version)?);
        }
        entries.push((Scalar::from("snapshots"), Scalar::from_sequence(snapshots)));

        entries.push((
            Scalar::from("snapshot-log"),
            Scalar::from_sequence(
                self.snapshot_log
                    .iter()
                    .map(|(timestamp, snapshot_id)| {
                        Scalar::from_mapping([
                            (Scalar::from("timestamp-ms"), Scalar::from(*timestamp)),
                            (Scalar::from("snapshot-id"), Scalar::from(*snapshot_id)),
                        ])
                    })
                    .collect::<Result<Vec<_>>>()?,
            ),
        ));
        entries.push((
            Scalar::from("metadata-log"),
            Scalar::from_sequence(
                self.metadata_log
                    .iter()
                    .map(|(timestamp, file)| {
                        Scalar::from_mapping([
                            (Scalar::from("timestamp-ms"), Scalar::from(*timestamp)),
                            (Scalar::from("metadata-file"), Scalar::from(file.clone())),
                        ])
                    })
                    .collect::<Result<Vec<_>>>()?,
            ),
        ));

        if self.format_version >= FormatVersion::V2 {
            let mut refs = Vec::with_capacity(self.refs.len());
            for (name, reference) in &self.refs {
                refs.push((Scalar::from(name.clone()), reference.clone().into_json()?));
            }
            entries.push((Scalar::from("refs"), Scalar::from_mapping(refs)?));
        }

        if !self.statistics.is_empty() {
            entries.push((
                Scalar::from("statistics"),
                Scalar::from_sequence(self.statistics),
            ));
        }
        if !self.partition_statistics.is_empty() {
            entries.push((
                Scalar::from("partition-statistics"),
                Scalar::from_sequence(self.partition_statistics),
            ));
        }
        if self.format_version >= FormatVersion::V3 && !self.encryption_keys.is_empty() {
            entries.push((
                Scalar::from("encryption-keys"),
                Scalar::from_sequence(self.encryption_keys),
            ));
        }

        if self.format_version >= FormatVersion::V3 {
            let next_row_id = self.next_row_id.ok_or_else(|| {
                invalid(SmolStr::new_static(
                    "expected next-row-id in Iceberg v3 metadata, got none",
                ))
            })?;
            if next_row_id < 0 {
                return Err(invalid(format_smolstr!(
                    "expected a non-negative next-row-id in Iceberg v3 metadata, got {next_row_id}"
                )));
            }
            entries.push((Scalar::from("next-row-id"), Scalar::from(next_row_id)));
        }

        Scalar::from_mapping(entries)
    }

    /// Make `snapshot` the current one, recording it in the log and on `main`.
    pub fn set_current_snapshot(&mut self, snapshot: Snapshot) -> Result<()> {
        let mut v1_manifests = super::official::V1SnapshotManifests::default();
        if let Some(manifests) = snapshot.manifests.clone() {
            v1_manifests.insert(snapshot.snapshot_id, manifests);
        }
        let snapshot = official_snapshot(snapshot, self.format_version)?;
        self.apply_official_update_with_v1_manifests(None, v1_manifests, |builder| {
            builder.set_branch_snapshot(snapshot, MAIN_BRANCH)
        })
    }

    /// Set one table property, returning the value it replaces.
    ///
    /// Keys are unique and exposed in deterministic lexical order.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is empty; the properties are unchanged.
    pub fn set_property(
        &mut self,
        key: impl Into<SmolStr>,
        value: impl Into<SmolStr>,
    ) -> Result<Option<SmolStr>> {
        let key = key.into();
        if key.is_empty() {
            return Err(invalid(SmolStr::new_static(
                "expected a non-empty property key, got \"\"",
            )));
        }
        let value = value.into();
        let previous = self.property(&key).map(SmolStr::new);
        let property = HashMap::from([(key.to_string(), value.to_string())]);
        self.apply_official_update(None, |builder| builder.set_properties(property))?;
        Ok(previous)
    }

    /// Remove one table property, returning the value it held.
    ///
    /// # Errors
    ///
    /// Returns an error when Apache Iceberg reserves the key; the metadata is
    /// unchanged.
    pub fn remove_property(&mut self, key: &str) -> Result<Option<SmolStr>> {
        let previous = self.property(key).map(SmolStr::new);
        let key = key.to_owned();
        self.apply_official_update(None, |builder| builder.remove_properties(&[key]))?;
        Ok(previous)
    }

    /// Borrow Apache-compatible statistics-file objects.
    pub fn statistics(&self) -> &[Scalar] {
        &self.statistics
    }

    /// Add or replace one snapshot's statistics through Apache's builder.
    pub fn set_statistics(&mut self, value: Scalar) -> Result<Option<Scalar>> {
        let statistics: OfficialStatisticsFile = official_from_scalar(&value)?;
        let snapshot_id = statistics.snapshot_id;
        let previous = scalar_by_i64(&self.statistics, "snapshot-id", snapshot_id).cloned();
        self.apply_official_update(None, |builder| Ok(builder.set_statistics(statistics)))?;
        Ok(previous)
    }

    /// Remove and return one snapshot's statistics object.
    pub fn remove_statistics(&mut self, snapshot_id: i64) -> Result<Option<Scalar>> {
        let previous = scalar_by_i64(&self.statistics, "snapshot-id", snapshot_id).cloned();
        self.apply_official_update(None, |builder| Ok(builder.remove_statistics(snapshot_id)))?;
        Ok(previous)
    }

    /// Borrow Apache-compatible partition-statistics objects.
    pub fn partition_statistics(&self) -> &[Scalar] {
        &self.partition_statistics
    }

    /// Add or replace one snapshot's partition statistics through Apache's builder.
    pub fn set_partition_statistics(&mut self, value: Scalar) -> Result<Option<Scalar>> {
        let statistics: OfficialPartitionStatisticsFile = official_from_scalar(&value)?;
        let snapshot_id = statistics.snapshot_id;
        let previous =
            scalar_by_i64(&self.partition_statistics, "snapshot-id", snapshot_id).cloned();
        self.apply_official_update(None, |builder| {
            Ok(builder.set_partition_statistics(statistics))
        })?;
        Ok(previous)
    }

    /// Remove and return one snapshot's partition-statistics object.
    pub fn remove_partition_statistics(&mut self, snapshot_id: i64) -> Result<Option<Scalar>> {
        let previous =
            scalar_by_i64(&self.partition_statistics, "snapshot-id", snapshot_id).cloned();
        self.apply_official_update(None, |builder| {
            Ok(builder.remove_partition_statistics(snapshot_id))
        })?;
        Ok(previous)
    }

    /// Borrow Apache-compatible v3 encryption-key objects.
    pub fn encryption_keys(&self) -> &[Scalar] {
        &self.encryption_keys
    }

    /// Add one v3 encryption key through Apache's builder.
    pub fn add_encryption_key(&mut self, value: Scalar) -> Result<bool> {
        self.require_v3_encryption()?;
        let key: OfficialEncryptedKey = official_from_scalar(&value)?;
        let key_id = key.key_id().to_owned();
        let inserted = scalar_by_str(&self.encryption_keys, "key-id", &key_id).is_none();
        self.apply_official_update(None, |builder| Ok(builder.add_encryption_key(key)))?;
        Ok(inserted)
    }

    /// Remove and return one v3 encryption-key object.
    pub fn remove_encryption_key(&mut self, key_id: &str) -> Result<Option<Scalar>> {
        self.require_v3_encryption()?;
        let previous = scalar_by_str(&self.encryption_keys, "key-id", key_id).cloned();
        let key_id = key_id.to_owned();
        self.apply_official_update(None, |builder| Ok(builder.remove_encryption_key(&key_id)))?;
        Ok(previous)
    }

    fn require_v3_encryption(&self) -> Result<()> {
        if self.format_version >= FormatVersion::V3 {
            return Ok(());
        }
        Err(invalid(format_smolstr!(
            "expected Iceberg v3 metadata for encryption keys, got v{}",
            self.format_version.number()
        )))
    }

    /// Replace the table's base location.
    ///
    /// Apache Iceberg canonicalizes trailing separators.
    ///
    /// # Errors
    ///
    /// Returns an error when the resulting metadata does not validate; the
    /// metadata is unchanged.
    pub fn set_location(&mut self, location: impl Into<SmolStr>) -> Result<()> {
        let location = location.into().to_string();
        self.apply_official_update(None, |builder| Ok(builder.set_location(location)))
    }

    /// Replace the table's UUID, validating the canonical 8-4-4-4-12 shape.
    ///
    /// # Errors
    ///
    /// Returns an error naming the input when it is not hyphenated hex of that
    /// shape; the stored UUID is unchanged.
    pub fn assign_uuid(&mut self, uuid: impl Into<SmolStr>) -> Result<()> {
        let uuid = uuid.into();
        let parsed = uuid::Uuid::parse_str(&uuid).map_err(|_| {
            invalid(format_smolstr!(
                "expected a UUID shaped 8-4-4-4-12 hex, got {:?}",
                crate::text::elide_to(&uuid, 64)
            ))
        })?;
        self.apply_official_update(None, |builder| Ok(builder.assign_uuid(parsed)))
    }

    /// Raise the format version, which is the only direction it can move.
    ///
    /// Upgrading to [`FormatVersion::V3`] initializes `next-row-id` to zero
    /// when the table does not carry one yet, because v3 requires it.
    ///
    /// # Errors
    ///
    /// Returns an error naming both versions when `version` is below the
    /// current one; the metadata is unchanged.
    pub fn upgrade_format_version(&mut self, version: FormatVersion) -> Result<()> {
        let version = match version {
            FormatVersion::V1 => OfficialFormatVersion::V1,
            FormatVersion::V2 => OfficialFormatVersion::V2,
            FormatVersion::V3 => OfficialFormatVersion::V3,
        };
        self.apply_official_update(None, |builder| builder.upgrade_format_version(version))
    }

    /// Make one already-added schema the one new data is written against.
    ///
    /// # Errors
    ///
    /// Returns an error naming the id when no schema carries it.
    pub fn set_current_schema(&mut self, schema_id: i32) -> Result<()> {
        self.apply_official_update(None, |builder| builder.set_current_schema(schema_id))
    }

    /// Add a partition spec and return the identifier selected by Iceberg.
    ///
    /// `last-partition-id` stays monotone: it grows to cover the new spec's
    /// highest field and never shrinks, so a retired partition field id is
    /// never reassigned.
    ///
    /// # Errors
    ///
    /// Returns an error when the spec cannot bind to the current schema; the
    /// specs are unchanged.
    pub fn add_spec(&mut self, spec: PartitionSpec) -> Result<i32> {
        let equivalent = self
            .partition_specs
            .iter()
            .any(|existing| partition_specs_compatible(existing, &spec));
        if !equivalent
            && self
                .partition_specs
                .iter()
                .map(|existing| existing.spec_id)
                .max()
                == Some(i32::MAX)
        {
            return Err(invalid(format_smolstr!(
                "expected a partition spec id below {}, got {}",
                i32::MAX,
                i32::MAX
            )));
        }
        let official = official_partition_spec(&spec)?;
        self.apply_official_update_result(
            None,
            super::official::V1SnapshotManifests::default(),
            |builder| builder.add_partition_spec(official),
            |built| {
                built
                    .changes
                    .iter()
                    .rev()
                    .find_map(|change| match change {
                        OfficialTableUpdate::AddSpec { spec } => spec.spec_id(),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        invalid(SmolStr::new_static(
                            "official Iceberg partition update returned no assigned spec id",
                        ))
                    })
            },
        )
    }

    /// Make one already-added spec the one new data is written against.
    ///
    /// Every schema is re-marked with the spec's partition columns, the same
    /// way [`Self::new`] and [`Self::from_json`] mark them, so a schema keeps
    /// reporting the layout its rows are stored in.
    ///
    /// # Errors
    ///
    /// Returns an error naming the id when no spec carries it, or a marking
    /// failure; the metadata is unchanged on error.
    pub fn set_default_spec(&mut self, spec_id: i32) -> Result<()> {
        self.apply_official_update(None, |builder| builder.set_default_partition_spec(spec_id))
    }

    /// Add a sort order and return the identifier selected by Iceberg.
    ///
    /// # Errors
    ///
    /// Returns an error when the order cannot bind to the current schema; the
    /// orders are unchanged.
    pub fn add_sort_order(&mut self, order: SortOrder) -> Result<i64> {
        let reuses_existing = self
            .sort_orders
            .iter()
            .any(|existing| existing.fields == order.fields);
        if !reuses_existing
            && self
                .sort_orders
                .iter()
                .map(|existing| existing.order_id)
                .max()
                == Some(i64::MAX)
        {
            return Err(invalid(SmolStr::new_static(
                "expected an available signed 64-bit sort order id, got i64::MAX already assigned",
            )));
        }
        let official = official_sort_order(&order)?;
        self.apply_official_update_result(
            None,
            super::official::V1SnapshotManifests::default(),
            |builder| builder.add_sort_order(official),
            |built| {
                built
                    .changes
                    .iter()
                    .rev()
                    .find_map(|change| match change {
                        OfficialTableUpdate::AddSortOrder { sort_order } => {
                            Some(sort_order.order_id)
                        }
                        _ => None,
                    })
                    .ok_or_else(|| {
                        invalid(SmolStr::new_static(
                            "official Iceberg sort update returned no assigned order id",
                        ))
                    })
            },
        )
    }

    /// Make one already-added sort order the one new data is written in.
    ///
    /// # Errors
    ///
    /// Returns an error naming the id when no order carries it.
    pub fn set_default_sort_order(&mut self, order_id: i64) -> Result<()> {
        self.apply_official_update(None, |builder| builder.set_default_sort_order(order_id))
    }

    /// Point a named branch or tag at one retained snapshot.
    ///
    /// The reserved `main` branch is the current snapshot, so pointing it
    /// somewhere also moves `current-snapshot-id` and records the move in the
    /// snapshot log, the same way [`Self::set_current_snapshot`] does.
    ///
    /// # Errors
    ///
    /// Returns an error when the snapshot is not retained, or when `main` is
    /// given anything but a branch; the refs are unchanged.
    pub fn set_snapshot_ref(
        &mut self,
        name: impl Into<SmolStr>,
        reference: SnapshotRef,
    ) -> Result<()> {
        if self.format_version == FormatVersion::V1 {
            return Err(invalid(SmolStr::new_static(
                "expected Iceberg v2 or v3 metadata for snapshot refs, got v1",
            )));
        }
        let name = name.into();
        let reference = official_snapshot_reference(&reference)?;
        self.apply_official_update(None, |builder| builder.set_ref(name.as_str(), reference))
    }

    /// Remove one named reference, returning what it pointed at.
    ///
    /// Removing the reserved `main` branch clears `current-snapshot-id`, which
    /// keeps the two spellings of "what a reader sees" agreeing: a table whose
    /// main branch is gone has no current snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the resulting metadata does not validate; the
    /// metadata is unchanged.
    pub fn remove_snapshot_ref(&mut self, name: &str) -> Result<Option<SnapshotRef>> {
        let previous = self.ref_by_name(name).cloned();
        if previous.is_none() {
            return Ok(None);
        }
        self.apply_official_update(None, |builder| Ok(builder.remove_ref(name)))?;
        Ok(previous)
    }

    /// Return one named reference, when the table has it.
    pub fn ref_by_name(&self, name: &str) -> Option<&SnapshotRef> {
        self.refs
            .iter()
            .find_map(|(existing, reference)| (existing == name).then_some(reference))
    }

    /// Create a branch at one retained snapshot.
    ///
    /// ```
    /// use yggdryl::iceberg::{FormatVersion, PartitionSpec, Snapshot, TableMetadata};
    /// use yggdryl::iceberg::assign_field_ids;
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut schema = DataType::from_fields([DataType::Int64.required_field("id")])?
    ///     .required_field("row");
    /// assign_field_ids(&mut schema, 1)?;
    /// let mut metadata = TableMetadata::new(
    ///     FormatVersion::V2,
    ///     "file:///tmp/branches",
    ///     schema,
    ///     PartitionSpec::unpartitioned(),
    /// )?;
    /// let timestamp_ms = metadata.last_updated_ms() + 1;
    /// metadata.set_current_snapshot(Snapshot {
    ///     snapshot_id: 7,
    ///     parent_snapshot_id: None,
    ///     sequence_number: Some(1),
    ///     timestamp_ms,
    ///     manifest_list: "".into(),
    ///     manifests: None,
    ///     summary: vec![("operation".into(), "append".into())],
    ///     schema_id: Some(0),
    ///     encryption_key_id: None,
    ///     first_row_id: None,
    ///     added_rows: None,
    /// })?;
    ///
    /// metadata.create_branch("dev", 7)?;
    /// metadata.create_tag("v1", 7)?;
    /// assert!(metadata.ref_by_name("dev").unwrap().is_branch());
    /// assert!(metadata.ref_by_name("v1").unwrap().is_tag());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the table already has a ref with this name or
    /// when the snapshot is not retained; the refs are unchanged.
    pub fn create_branch(&mut self, name: impl Into<SmolStr>, snapshot_id: i64) -> Result<()> {
        let name = name.into();
        self.expect_no_ref(&name)?;
        self.set_snapshot_ref(name, SnapshotRef::branch(snapshot_id))
    }

    /// Create a tag at one retained snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the table already has a ref with this name or
    /// when the snapshot is not retained; the refs are unchanged.
    pub fn create_tag(&mut self, name: impl Into<SmolStr>, snapshot_id: i64) -> Result<()> {
        let name = name.into();
        self.expect_no_ref(&name)?;
        self.set_snapshot_ref(name, SnapshotRef::tag(snapshot_id))
    }

    /// Rename one reference, keeping what it points at and how it is retained.
    ///
    /// Renaming a branch *to* `main` goes through the same reserved-name rules
    /// as pointing `main` anywhere: the branch's snapshot becomes the current
    /// one, and a tag is refused.
    ///
    /// # Errors
    ///
    /// Returns an error when `from` is the reserved `main` branch, names no
    /// ref, or `to` names one the table already has; the refs are unchanged.
    pub fn rename_ref(&mut self, from: &str, to: impl Into<SmolStr>) -> Result<()> {
        let to = to.into();
        if from == MAIN_BRANCH {
            return Err(invalid(SmolStr::new_static(
                "expected a renameable ref, got the reserved \"main\" branch",
            )));
        }
        let Some(reference) = self.ref_by_name(from) else {
            return Err(invalid(format_smolstr!(
                "expected a ref named {:?}, got {} refs",
                crate::text::elide_to(from, 64),
                self.refs.len()
            )));
        };
        self.expect_no_ref(&to)?;
        let reference = official_snapshot_reference(reference)?;
        self.apply_official_update(None, |builder| {
            let builder = builder.set_ref(to.as_str(), reference)?;
            Ok(builder.remove_ref(from))
        })
    }

    /// Move a branch forward to a descendant of its current head.
    ///
    /// Fast-forwarding is the one branch move that cannot lose history, which
    /// is why it is checked: the target must reach the current head by walking
    /// parent ids. Moving `main` keeps `current-snapshot-id` and the snapshot
    /// log in step, exactly as [`Self::set_snapshot_ref`] does.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is not a branch, the target is not
    /// retained, or the target does not descend from the branch's head; the
    /// refs are unchanged.
    pub fn fast_forward_branch(&mut self, name: &str, to_snapshot_id: i64) -> Result<()> {
        let Some(reference) = self.ref_by_name(name) else {
            return Err(invalid(format_smolstr!(
                "expected a branch named {:?}, got {} refs",
                crate::text::elide_to(name, 64),
                self.refs.len()
            )));
        };
        if !reference.is_branch() {
            return Err(invalid(format_smolstr!(
                "expected a branch named {:?}, got a {:?}",
                crate::text::elide_to(name, 64),
                crate::text::elide_to(&reference.kind, 64)
            )));
        }
        let mut moved = reference.clone();
        let head = moved.snapshot_id;
        if self.snapshot_by_id(to_snapshot_id).is_none() {
            return Err(invalid(format_smolstr!(
                "expected a retained snapshot for ref {:?}, got unknown snapshot id \
                 {to_snapshot_id}",
                crate::text::elide_to(name, 64)
            )));
        }
        if !self.descends_from(to_snapshot_id, head) {
            return Err(invalid(format_smolstr!(
                "expected {to_snapshot_id} to descend from {head}"
            )));
        }
        moved.snapshot_id = to_snapshot_id;
        self.set_snapshot_ref(SmolStr::new(name), moved)
    }

    /// Expire snapshots with Apache Iceberg's complete retention contract.
    ///
    /// `older_than_ms` and `retain_last` override the corresponding table
    /// defaults when present. Per-branch settings override both. Explicit
    /// `snapshot_ids` are unioned with age-based selection, but a retained ref
    /// head or the current snapshot cannot be named. `main` never ages out;
    /// tags retain only their target; branches retain their selected ancestry;
    /// and recent unreferenced snapshots survive until the default cutoff.
    /// `gc.enabled=false` refuses the complete operation.
    ///
    /// Returns the removed snapshot ids, sorted; a table with nothing old
    /// returns an empty list. Statistics metadata for removed snapshots is
    /// removed in the same atomic metadata update.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid consulted retention properties, an
    /// explicit zero retain count, a protected explicit id, disabled garbage
    /// collection, or an official metadata-builder failure. Nothing is
    /// removed on error.
    pub fn expire_snapshots(
        &mut self,
        older_than_ms: Option<i64>,
        retain_last: Option<usize>,
        snapshot_ids: &[i64],
    ) -> Result<Vec<i64>> {
        let mut property_keys = vec![OfficialTableProperties::PROPERTY_GC_ENABLED];
        if retain_last.is_none() {
            property_keys.push(OfficialTableProperties::PROPERTY_MIN_SNAPSHOTS_TO_KEEP);
        }
        if older_than_ms.is_none() {
            property_keys.push(OfficialTableProperties::PROPERTY_MAX_SNAPSHOT_AGE_MS);
        }
        if self
            .refs
            .iter()
            .any(|(name, reference)| name != MAIN_BRANCH && reference.max_ref_age_ms.is_none())
        {
            property_keys.push(OfficialTableProperties::PROPERTY_MAX_REF_AGE_MS);
        }
        let properties: HashMap<String, String> = property_keys
            .into_iter()
            .filter_map(|key| {
                self.property(key)
                    .map(|value| (key.to_owned(), value.to_owned()))
            })
            .collect();
        let properties =
            OfficialTableProperties::try_from(&properties).map_err(Error::from_iceberg)?;
        if !properties.gc_enabled {
            return Err(invalid(SmolStr::new_static(
                "Cannot expire snapshots: gc.enabled is false",
            )));
        }
        if retain_last == Some(0) {
            return Err(invalid(SmolStr::new_static(
                "expected retain_last to be at least 1, got 0",
            )));
        }

        let now = now_ms();
        let default_cutoff =
            older_than_ms.unwrap_or_else(|| now.saturating_sub(properties.max_snapshot_age_ms));
        let default_minimum = retain_last.unwrap_or(properties.min_snapshots_to_keep);

        // Match Apache's expiry planner: age refs first, then resolve every
        // retained branch independently. `main` never ages out.
        let mut expired_refs: Vec<SmolStr> = self
            .refs
            .iter()
            .filter(|(name, reference)| {
                if name == MAIN_BRANCH {
                    return false;
                }
                let limit = reference
                    .max_ref_age_ms
                    .unwrap_or(properties.max_ref_age_ms);
                self.snapshot_by_id(reference.snapshot_id)
                    .is_some_and(|snapshot| now.saturating_sub(snapshot.timestamp_ms) > limit)
            })
            .map(|(name, _)| name.clone())
            .collect();
        expired_refs.sort();
        let expired_ref_names: HashSet<&str> = expired_refs.iter().map(SmolStr::as_str).collect();

        let retained_refs: Vec<&SnapshotRef> = self
            .refs
            .iter()
            .filter(|(name, _)| !expired_ref_names.contains(name.as_str()))
            .map(|(_, reference)| reference)
            .collect();
        let mut ref_heads: HashSet<i64> = retained_refs
            .iter()
            .map(|reference| reference.snapshot_id)
            .collect();
        ref_heads.extend(self.current_snapshot_id);

        let existing_ids: HashSet<i64> = self
            .snapshots
            .iter()
            .map(|snapshot| snapshot.snapshot_id)
            .collect();
        let mut removing = HashSet::new();
        for snapshot_id in snapshot_ids {
            if ref_heads.contains(snapshot_id) {
                let reason = if self.current_snapshot_id == Some(*snapshot_id) {
                    format_smolstr!("cannot expire current snapshot {snapshot_id}")
                } else {
                    let names: Vec<&str> = self
                        .refs
                        .iter()
                        .filter(|(_, reference)| reference.snapshot_id == *snapshot_id)
                        .map(|(name, _)| name.as_str())
                        .collect();
                    format_smolstr!(
                        "cannot expire snapshot {snapshot_id}; retained refs [{}] still name it",
                        names.join(", ")
                    )
                };
                return Err(invalid(reason));
            }
            if existing_ids.contains(snapshot_id) {
                removing.insert(*snapshot_id);
            }
        }

        let mut retained = ref_heads.clone();
        let mut referenced = ref_heads;
        let mut branches = Vec::new();
        for reference in retained_refs {
            if reference.is_branch() {
                let minimum = reference
                    .min_snapshots_to_keep
                    .and_then(|count| usize::try_from(count).ok())
                    .unwrap_or(default_minimum);
                let cutoff = reference
                    .max_snapshot_age_ms
                    .map_or(default_cutoff, |age| now.saturating_sub(age));
                branches.push((reference.snapshot_id, minimum, cutoff));
            } else {
                referenced.insert(reference.snapshot_id);
            }
        }
        if let Some(current) = self.current_snapshot_id
            && !branches.iter().any(|(head, _, _)| *head == current)
        {
            branches.push((current, default_minimum, default_cutoff));
        }

        for (head, minimum, cutoff) in branches {
            let mut position = 0_usize;
            let mut cursor = Some(head);
            while let Some(id) = cursor {
                // A corrupt parent chain could cycle; the walk is bounded by
                // the ancestors a table can actually hold.
                let Some(snapshot) = self
                    .snapshot_by_id(id)
                    .filter(|_| position < self.snapshots.len())
                else {
                    break;
                };
                referenced.insert(id);
                if position < minimum || snapshot.timestamp_ms >= cutoff {
                    retained.insert(id);
                }
                position += 1;
                cursor = snapshot.parent_snapshot_id;
            }
        }

        // A young orphan may be adopted by a concurrent branch later. Apache
        // keeps it until the explicit default cutoff even though no current
        // ancestry reaches it.
        for snapshot in &self.snapshots {
            if !referenced.contains(&snapshot.snapshot_id)
                && snapshot.timestamp_ms >= default_cutoff
            {
                retained.insert(snapshot.snapshot_id);
            }
        }

        removing.extend(
            self.snapshots
                .iter()
                .map(|snapshot| snapshot.snapshot_id)
                .filter(|id| !retained.contains(id)),
        );
        let mut removed: Vec<i64> = removing.into_iter().collect();
        removed.sort_unstable();
        if !expired_refs.is_empty() || !removed.is_empty() {
            let expired_refs: Vec<String> = expired_refs.into_iter().map(String::from).collect();
            let remove_statistics: Vec<i64> = removed
                .iter()
                .copied()
                .filter(|id| scalar_by_i64(&self.statistics, "snapshot-id", *id).is_some())
                .collect();
            let remove_partition_statistics: Vec<i64> = removed
                .iter()
                .copied()
                .filter(|id| {
                    scalar_by_i64(&self.partition_statistics, "snapshot-id", *id).is_some()
                })
                .collect();
            self.apply_official_update(None, |mut builder| {
                for name in &expired_refs {
                    builder = builder.remove_ref(name);
                }
                builder = builder.remove_snapshots(&removed);
                for snapshot_id in &remove_statistics {
                    builder = builder.remove_statistics(*snapshot_id);
                }
                for snapshot_id in &remove_partition_statistics {
                    builder = builder.remove_partition_statistics(*snapshot_id);
                }
                Ok(builder)
            })?;
        }
        Ok(removed)
    }

    /// Refuse a name the table already has a reference under.
    fn expect_no_ref(&self, name: &str) -> Result<()> {
        let Some(existing) = self.ref_by_name(name) else {
            return Ok(());
        };
        Err(invalid(format_smolstr!(
            "expected no ref named {:?}, got a {:?}",
            crate::text::elide_to(name, 64),
            crate::text::elide_to(&existing.kind, 64)
        )))
    }

    /// Return whether one snapshot reaches another by walking parent ids.
    fn descends_from(&self, descendant: i64, ancestor: i64) -> bool {
        let mut cursor = Some(descendant);
        let mut steps = 0_usize;
        while let Some(id) = cursor {
            if id == ancestor {
                return true;
            }
            // A corrupt parent chain could cycle; the walk is bounded by the
            // ancestors a table can actually hold.
            steps += 1;
            if steps > self.snapshots.len() {
                return false;
            }
            cursor = self
                .snapshot_by_id(id)
                .and_then(|snapshot| snapshot.parent_snapshot_id);
        }
        false
    }

    /// Check that this document's cross-references resolve.
    ///
    /// This is what [`Self::from_json`] runs after reading and what a commit
    /// runs before writing: the current schema, spec, sort order, and snapshot
    /// ids resolve; the current schema's field ids are unique and non-zero
    /// with `last-column-id` at or above them; `last-partition-id` covers
    /// every spec; and every named ref points at a retained snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error naming the first identifier that does not hold.
    pub fn validate(&self) -> Result<()> {
        match self.format_version {
            FormatVersion::V1 if self.last_sequence_number != 0 => {
                return Err(invalid(format_smolstr!(
                    "expected no last sequence number in Iceberg v1 metadata, got {}",
                    self.last_sequence_number
                )));
            }
            FormatVersion::V2 | FormatVersion::V3 if self.last_sequence_number < 0 => {
                return Err(invalid(format_smolstr!(
                    "expected a non-negative last-sequence-number in Iceberg v{} metadata, got {}",
                    self.format_version.number(),
                    self.last_sequence_number
                )));
            }
            _ => {}
        }
        match (self.format_version, self.next_row_id) {
            (FormatVersion::V3, Some(value)) if value >= 0 => {}
            (FormatVersion::V3, None) => {
                return Err(invalid(SmolStr::new_static(
                    "expected a non-negative next-row-id in Iceberg v3 metadata, got none",
                )));
            }
            (FormatVersion::V3, Some(value)) => {
                return Err(invalid(format_smolstr!(
                    "expected a non-negative next-row-id in Iceberg v3 metadata, got {value}"
                )));
            }
            (_, None) => {}
            (version, Some(value)) => {
                return Err(invalid(format_smolstr!(
                    "expected no next-row-id in Iceberg v{} metadata, got {value}",
                    version.number()
                )));
            }
        }
        if self.format_version == FormatVersion::V1 {
            let refs_are_derived_main = match (self.current_snapshot_id, self.refs.as_slice()) {
                (_, []) => true,
                (Some(snapshot_id), [(name, reference)]) => {
                    name == MAIN_BRANCH && *reference == SnapshotRef::branch(snapshot_id)
                }
                _ => false,
            };
            if !refs_are_derived_main {
                return Err(invalid(format_smolstr!(
                    "expected no refs or only the official derived main ref in Iceberg v1 metadata, got {} refs",
                    self.refs.len()
                )));
            }
        }
        if self.format_version < FormatVersion::V3 && !self.encryption_keys.is_empty() {
            return Err(invalid(format_smolstr!(
                "expected no encryption-keys in Iceberg v{} metadata, got {}",
                self.format_version.number(),
                self.encryption_keys.len()
            )));
        }
        for snapshot in &self.snapshots {
            snapshot.validate_for_version(self.format_version)?;
            if let Some(sequence) = snapshot.sequence_number
                && sequence > self.last_sequence_number
            {
                return Err(invalid(format_smolstr!(
                    "expected snapshot {} sequence-number at most {}, got {}",
                    snapshot.snapshot_id,
                    self.last_sequence_number,
                    sequence
                )));
            }
            if let Some(schema_id) = snapshot.schema_id
                && self.schema_by_id(schema_id).is_none()
            {
                return Err(invalid(format_smolstr!(
                    "expected schema id {schema_id} for snapshot {}, got {} retained schemas",
                    snapshot.snapshot_id,
                    self.schemas.len()
                )));
            }
        }
        ensure_unique(self.schemas.iter().map(field_schema_id), "schema id")?;
        ensure_unique(
            self.partition_specs.iter().map(|spec| spec.spec_id),
            "partition spec id",
        )?;
        ensure_unique(
            self.sort_orders.iter().map(|order| order.order_id),
            "sort order id",
        )?;
        ensure_unique(
            self.snapshots.iter().map(|snapshot| snapshot.snapshot_id),
            "snapshot id",
        )?;
        ensure_unique(self.refs.iter().map(|(name, _)| name), "snapshot ref name")?;
        ensure_unique(
            self.properties.iter().map(|(name, _)| name),
            "property name",
        )?;
        validate_schema_history(&self.schemas, self.last_column_id, self.format_version)?;
        validate_partition_spec_history(&self.partition_specs, &self.schemas)?;
        validate_sort_order_history(&self.sort_orders, &self.schemas)?;
        let schema = self.current_schema()?;
        let mut ids = Vec::new();
        collect_field_ids(schema, &mut ids)?;
        if ids.contains(&0) {
            return Err(invalid(format_smolstr!(
                "expected non-zero field ids in schema {}, got 0",
                self.current_schema_id
            )));
        }
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        for pair in sorted.windows(2) {
            if pair[0] == pair[1] {
                return Err(invalid(format_smolstr!(
                    "expected unique field ids in schema {}, got {} more than once",
                    self.current_schema_id,
                    pair[0]
                )));
            }
        }
        if let Some(highest) = sorted.last().copied() {
            if self.last_column_id < highest {
                return Err(invalid(format_smolstr!(
                    "expected a last-column-id of at least {highest}, got {}",
                    self.last_column_id
                )));
            }
        }
        self.default_spec()?;
        for spec in &self.partition_specs {
            if !spec.fields.is_empty() && spec.last_field_id() > self.last_partition_id {
                return Err(invalid(format_smolstr!(
                    "expected a last-partition-id of at least {} for partition spec {}, got {}",
                    spec.last_field_id(),
                    spec.spec_id,
                    self.last_partition_id
                )));
            }
        }
        if !self
            .sort_orders
            .iter()
            .any(|order| order.order_id == self.default_sort_order_id)
        {
            return Err(invalid(format_smolstr!(
                "expected a sort order with id {}, got {} orders",
                self.default_sort_order_id,
                self.sort_orders.len()
            )));
        }
        if let Some(current) = self.current_snapshot_id {
            if self.snapshot_by_id(current).is_none() {
                return Err(invalid(format_smolstr!(
                    "expected a snapshot with id {current}, got {} snapshots",
                    self.snapshots.len()
                )));
            }
        }
        for (name, reference) in &self.refs {
            if let Err(error) = reference.validate() {
                return Err(invalid(format_smolstr!(
                    "expected a valid snapshot ref {:?}, got {error}",
                    crate::text::elide_to(name, 64)
                )));
            }
            if self.snapshot_by_id(reference.snapshot_id).is_none() {
                return Err(invalid(format_smolstr!(
                    "expected a retained snapshot for ref {:?}, got unknown snapshot id {}",
                    crate::text::elide_to(name, 64),
                    reference.snapshot_id
                )));
            }
        }
        let document = self.clone().into_json_document()?;
        super::official::validate_table_metadata(&document)
    }
}

impl PartialEq for TableMetadata {
    fn eq(&self, other: &Self) -> bool {
        self.identity() == other.identity()
    }
}

impl Eq for TableMetadata {}

impl PartialOrd for TableMetadata {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TableMetadata {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.identity().cmp(&other.identity())
    }
}

impl Hash for TableMetadata {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identity().hash(state);
    }
}

/// Convert the public Yggdryl snapshot view into the official metadata type.
fn official_snapshot(snapshot: Snapshot, version: FormatVersion) -> Result<OfficialSnapshot> {
    snapshot.validate_for_version(version)?;
    let manifest_list = if snapshot.manifests.is_some() {
        super::official::v1_manifest_list(snapshot.snapshot_id)
    } else {
        snapshot.manifest_list.to_string()
    };
    let mut summary: HashMap<String, String> = snapshot
        .summary
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    let operation = match summary.remove("operation").as_deref() {
        Some("append") => OfficialOperation::Append,
        Some("replace") => OfficialOperation::Replace,
        Some("overwrite") => OfficialOperation::Overwrite,
        Some("delete") => OfficialOperation::Delete,
        Some(other) => {
            return Err(invalid(format_smolstr!(
                "expected an Iceberg snapshot operation, got {:?}",
                crate::text::elide_to(other, 64)
            )));
        }
        None => {
            return Err(invalid(SmolStr::new_static(
                "expected an Iceberg snapshot summary operation",
            )));
        }
    };
    let sequence_number = match version {
        FormatVersion::V1 => 0,
        FormatVersion::V2 | FormatVersion::V3 => snapshot.sequence_number.ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a sequence-number on Iceberg v{} snapshot {}",
                version.number(),
                snapshot.snapshot_id
            ))
        })?,
    };
    let builder = OfficialSnapshot::builder()
        .with_snapshot_id(snapshot.snapshot_id)
        .with_parent_snapshot_id(snapshot.parent_snapshot_id)
        .with_sequence_number(sequence_number)
        .with_timestamp_ms(snapshot.timestamp_ms)
        .with_manifest_list(manifest_list)
        .with_summary(OfficialSummary {
            operation,
            additional_properties: summary,
        })
        .schema_id_opt(snapshot.schema_id)
        .with_encryption_key_id(snapshot.encryption_key_id.map(String::from));
    let row_range = match (snapshot.first_row_id, snapshot.added_rows) {
        (Some(first), Some(added)) => {
            let first = u64::try_from(first).map_err(|_| {
                invalid(format_smolstr!(
                    "expected a non-negative first-row-id, got {first}"
                ))
            })?;
            let added = u64::try_from(added).map_err(|_| {
                invalid(format_smolstr!(
                    "expected a non-negative added-rows, got {added}"
                ))
            })?;
            Some((first, added))
        }
        (None, None) => None,
        _ => {
            return Err(invalid(SmolStr::new_static(
                "expected first-row-id and added-rows together",
            )));
        }
    };
    Ok(match row_range {
        Some((first, added)) => builder.with_row_range(first, added).build(),
        None => builder.build(),
    })
}

/// Convert the public Yggdryl reference view into the official metadata type.
fn official_snapshot_reference(reference: &SnapshotRef) -> Result<OfficialSnapshotReference> {
    reference.validate()?;
    let retention = match reference.kind.as_str() {
        "branch" => OfficialSnapshotRetention::branch(
            reference.min_snapshots_to_keep,
            reference.max_snapshot_age_ms,
            reference.max_ref_age_ms,
        ),
        "tag"
            if reference.min_snapshots_to_keep.is_none()
                && reference.max_snapshot_age_ms.is_none() =>
        {
            OfficialSnapshotRetention::Tag {
                max_ref_age_ms: reference.max_ref_age_ms,
            }
        }
        other => {
            return Err(invalid(format_smolstr!(
                "expected an Iceberg branch or tag, got {:?}",
                crate::text::elide_to(other, 64)
            )));
        }
    };
    Ok(OfficialSnapshotReference::new(
        reference.snapshot_id,
        retention,
    ))
}

fn field_schema_id(schema: &Field) -> i32 {
    schema
        .iceberg()
        .get(super::schema::SCHEMA_ID)
        .and_then(|id| id.parse().ok())
        .unwrap_or_default()
}

fn official_schema(schema: &Field) -> Result<OfficialSchema> {
    let document = schema_to_json(schema)?;
    let bytes = crate::json::into_bytes(&document)?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Match the identity Apache's builder uses, excluding its assigned schema id.
fn official_schemas_same(left: &OfficialSchema, right: &OfficialSchema) -> bool {
    left.as_struct() == right.as_struct()
        && left.identifier_field_ids().collect::<HashSet<_>>()
            == right.identifier_field_ids().collect::<HashSet<_>>()
}

fn official_from_scalar<T: serde::de::DeserializeOwned>(value: &Scalar) -> Result<T> {
    let bytes = crate::json::into_bytes(value)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn scalar_by_i64<'a>(values: &'a [Scalar], key: &str, expected: i64) -> Option<&'a Scalar> {
    values
        .iter()
        .find(|value| value.get_key_str(key).and_then(Scalar::as_i64) == Some(expected))
}

fn scalar_by_str<'a>(values: &'a [Scalar], key: &str, expected: &str) -> Option<&'a Scalar> {
    values
        .iter()
        .find(|value| value.get_key_str(key).and_then(Scalar::as_str) == Some(expected))
}

fn official_partition_spec(spec: &PartitionSpec) -> Result<OfficialUnboundPartitionSpec> {
    let mut builder = OfficialUnboundPartitionSpec::builder();
    for field in &spec.fields {
        let transform = field
            .transform
            .to_string()
            .parse::<OfficialTransform>()
            .map_err(Error::from_iceberg)?;
        builder = builder
            .add_partition_field(field.source_id, &field.name, transform)
            .map_err(Error::from_iceberg)?;
    }
    Ok(builder.build())
}

/// Match Apache's compatibility rule; field and spec ids are assigned state.
fn partition_specs_compatible(left: &PartitionSpec, right: &PartitionSpec) -> bool {
    left.fields.len() == right.fields.len()
        && left.fields.iter().zip(&right.fields).all(|(left, right)| {
            left.source_id == right.source_id
                && left.name == right.name
                && left.transform == right.transform
        })
}

fn official_sort_order(order: &SortOrder) -> Result<OfficialSortOrder> {
    let document = order.clone().into_json()?;
    let bytes = crate::json::into_bytes(&document)?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Collect every field identifier below a schema root, depth first.
fn collect_field_ids(node: &Field, ids: &mut Vec<i32>) -> Result<()> {
    for index in 0..node.data_type().field_len() {
        let Some(child) = node.data_type().get_field(index) else {
            continue;
        };
        if let Some(id) = child.parquet_field_id()? {
            ids.push(id);
        }
        collect_field_ids(child, ids)?;
    }
    Ok(())
}

/// Prevent direct metadata updates from reusing retired ids or changing an
/// existing column incompatibly. Apache's low-level builder intentionally
/// leaves this compatibility check to its caller.
fn validate_schema_evolution(
    current: &Field,
    candidate: &Field,
    last_id: i32,
    version: FormatVersion,
) -> Result<()> {
    let mut current_parents = HashMap::new();
    collect_field_parents(current, None, &mut current_parents)?;
    let mut candidate_parents = HashMap::new();
    collect_field_parents(candidate, None, &mut candidate_parents)?;
    let current = official_schema(current)?;
    let candidate = official_schema(candidate)?;

    for (id, parent) in candidate_parents {
        let new = candidate.field_by_id(id).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected candidate schema field id {id}, got no field"
            ))
        })?;
        if version < FormatVersion::V3
            && (new.initial_default.is_some() || new.write_default.is_some())
        {
            return Err(invalid(format_smolstr!(
                "expected no field defaults before Iceberg v3, got defaults on field id {id}"
            )));
        }
        if id > last_id {
            if parent.is_none_or(|parent_id| parent_id <= last_id)
                && new.required
                && new.initial_default.is_none()
            {
                return Err(invalid(format_smolstr!(
                    "expected an initial-default on new required field id {id} ({:?})",
                    new.name
                )));
            }
            continue;
        }
        let old_parent = current_parents.get(&id).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a fresh field id above {last_id}, got retired field id {id}"
            ))
        })?;
        if *old_parent != parent {
            return Err(invalid(format_smolstr!(
                "expected field id {id} to remain under parent {old_parent:?}, got {parent:?}"
            )));
        }
        let old = current.field_by_id(id).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected current schema field id {id}, got no field"
            ))
        })?;
        if old.initial_default != new.initial_default {
            return Err(invalid(format_smolstr!(
                "expected immutable initial-default on field id {id} ({:?})",
                new.name
            )));
        }
        if !old.required && new.required {
            return Err(invalid(format_smolstr!(
                "expected field id {id} to stay optional, got required field {:?}",
                new.name
            )));
        }
        if !official_type_can_promote(&old.field_type, &new.field_type) {
            return Err(invalid(format_smolstr!(
                "expected an Iceberg-legal promotion for field id {id}, got {} to {}",
                old.field_type,
                new.field_type
            )));
        }
    }
    Ok(())
}

fn collect_field_parents(
    node: &Field,
    parent: Option<i32>,
    fields: &mut HashMap<i32, Option<i32>>,
) -> Result<()> {
    for index in 0..node.data_type().field_len() {
        let Some(child) = node.data_type().get_field(index) else {
            continue;
        };
        let id = child.parquet_field_id()?.ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a PARQUET:field_id on {:?}",
                child.name()
            ))
        })?;
        if fields.insert(id, parent).is_some() {
            return Err(invalid(format_smolstr!(
                "expected unique field ids, got {id} more than once"
            )));
        }
        collect_field_parents(child, Some(id), fields)?;
    }
    Ok(())
}

fn official_type_can_promote(from: &OfficialType, to: &OfficialType) -> bool {
    if from == to {
        return true;
    }
    match (from, to) {
        (
            OfficialType::Primitive(OfficialPrimitiveType::Int),
            OfficialType::Primitive(OfficialPrimitiveType::Long),
        )
        | (
            OfficialType::Primitive(OfficialPrimitiveType::Float),
            OfficialType::Primitive(OfficialPrimitiveType::Double),
        ) => true,
        (
            OfficialType::Primitive(OfficialPrimitiveType::Decimal { precision, scale }),
            OfficialType::Primitive(OfficialPrimitiveType::Decimal {
                precision: to_precision,
                scale: to_scale,
            }),
        ) => to_scale == scale && to_precision >= precision && *to_precision <= 38,
        (OfficialType::Struct(_), OfficialType::Struct(_))
        | (OfficialType::List(_), OfficialType::List(_))
        | (OfficialType::Map(_), OfficialType::Map(_)) => true,
        _ => false,
    }
}

/// Reject known versioned keys before serde can ignore them as unknown.
fn validate_versioned_document(document: &Scalar) -> Result<()> {
    let version = FormatVersion::from_number(
        document
            .get_key_str("format-version")
            .and_then(Scalar::as_i64)
            .ok_or_else(|| {
                invalid(SmolStr::new_static(
                    "expected a table metadata \"format-version\"",
                ))
            })?,
    )?;
    let forbidden = match version {
        FormatVersion::V1 => [
            Some("last-sequence-number"),
            Some("next-row-id"),
            Some("encryption-keys"),
            None,
        ],
        FormatVersion::V2 => [Some("next-row-id"), Some("encryption-keys"), None, None],
        FormatVersion::V3 => [None, None, None, None],
    };
    if let Some(key) = forbidden
        .into_iter()
        .flatten()
        .find(|key| document.get_key_str(key).is_some())
    {
        return Err(invalid(format_smolstr!(
            "expected no {key} in Iceberg v{} metadata",
            version.number()
        )));
    }
    if version == FormatVersion::V1 {
        validate_v1_wire_refs(document)?;
    }
    for (collection, id) in [
        ("schemas", "schema-id"),
        ("partition-specs", "spec-id"),
        ("sort-orders", "order-id"),
        ("snapshots", "snapshot-id"),
        ("statistics", "snapshot-id"),
        ("partition-statistics", "snapshot-id"),
    ] {
        reject_duplicate_document_ids(document, collection, id)?;
    }
    reject_duplicate_document_string_ids(document, "encryption-keys", "key-id")?;
    for snapshot in document
        .get_key_str("snapshots")
        .map(Scalar::sequence_iter)
        .unwrap_or_default()
    {
        Snapshot::from_json(snapshot)?.validate_for_version(version)?;
    }
    Ok(())
}

/// Accept the derived `refs.main` field emitted by PyIceberg for v1,
/// but only when it exactly matches the main ref the official Rust parser
/// derives from `current-snapshot-id`. Other v1 refs would otherwise be
/// ignored by serde and disappear during normalization.
fn validate_v1_wire_refs(document: &Scalar) -> Result<()> {
    let Some(entries) = document.get_key_str("refs") else {
        return Ok(());
    };
    let refs = if let Some(record) = entries.as_record() {
        record
            .iter()
            .map(|(name, value)| Ok((name.clone(), SnapshotRef::from_json(value)?)))
            .collect::<Result<Vec<_>>>()?
    } else if let Some(mapping) = entries.as_mapping() {
        mapping
            .iter()
            .map(|(name, value)| {
                let name = name.as_str().ok_or_else(|| {
                    invalid(SmolStr::new_static(
                        "expected string snapshot ref names in Iceberg v1 metadata",
                    ))
                })?;
                Ok((SmolStr::new(name), SnapshotRef::from_json(value)?))
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        return Err(invalid(SmolStr::new_static(
            "expected refs to be an object in Iceberg v1 metadata",
        )));
    };
    let current_snapshot_id = document
        .get_key_str("current-snapshot-id")
        .and_then(Scalar::as_i64)
        .filter(|id| *id >= 0);
    let refs_are_derived_main = match (current_snapshot_id, refs.as_slice()) {
        (_, []) => true,
        (Some(snapshot_id), [(name, reference)]) => {
            name == MAIN_BRANCH && *reference == SnapshotRef::branch(snapshot_id)
        }
        _ => false,
    };
    if refs_are_derived_main {
        return Ok(());
    }
    Err(invalid(format_smolstr!(
        "expected only main to match current-snapshot-id in Iceberg v1 refs, got {} refs",
        refs.len()
    )))
}

fn reject_duplicate_document_string_ids(
    document: &Scalar,
    collection: &str,
    key: &str,
) -> Result<()> {
    let mut seen = HashSet::new();
    for entry in document
        .get_key_str(collection)
        .map(Scalar::sequence_iter)
        .unwrap_or_default()
    {
        let Some(id) = entry.get_key_str(key).and_then(Scalar::as_str) else {
            continue;
        };
        if !seen.insert(id) {
            return Err(invalid(format_smolstr!(
                "expected unique {key} values in {collection}, got {:?} more than once",
                crate::text::elide_to(id, 64)
            )));
        }
    }
    Ok(())
}

fn reject_duplicate_document_ids(document: &Scalar, collection: &str, key: &str) -> Result<()> {
    let mut seen = HashSet::new();
    for entry in document
        .get_key_str(collection)
        .map(Scalar::sequence_iter)
        .unwrap_or_default()
    {
        let Some(id) = entry.get_key_str(key).and_then(Scalar::as_i64) else {
            continue;
        };
        if !seen.insert(id) {
            return Err(invalid(format_smolstr!(
                "expected unique {key} values in {collection}, got {id} more than once"
            )));
        }
    }
    Ok(())
}

fn ensure_unique<T: Copy + Eq + std::hash::Hash + fmt::Debug>(
    values: impl IntoIterator<Item = T>,
    name: &str,
) -> Result<()> {
    let mut seen = HashSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(invalid(format_smolstr!(
                "expected unique {name}s, got {value:?} more than once"
            )));
        }
    }
    Ok(())
}

fn validate_partition_spec_history(specs: &[PartitionSpec], schemas: &[Field]) -> Result<()> {
    for spec in specs {
        spec.validate_shape()?;
        let document = spec.clone().into_json()?;
        let official: OfficialUnboundPartitionSpec = official_from_scalar(&document)?;
        let mut last_error = None;
        let mut matched = false;
        for schema in schemas {
            let schema = std::sync::Arc::new(official_schema(schema)?);
            match official.clone().bind(schema) {
                Ok(_) => {
                    matched = true;
                    break;
                }
                Err(error) => last_error = Some(error.to_string()),
            }
        }
        if !matched {
            let reason = last_error.as_deref().unwrap_or("no retained schemas");
            return Err(invalid(format_smolstr!(
                "expected partition spec {} to bind to a retained schema, got {}",
                spec.spec_id,
                crate::text::elide_to(reason, 160)
            )));
        }
    }
    Ok(())
}

fn validate_sort_order_history(orders: &[SortOrder], schemas: &[Field]) -> Result<()> {
    for order in orders {
        order.validate_shape()?;
        let official = official_sort_order(order)?;
        let mut last_error = None;
        let mut matched = false;
        for schema in schemas {
            let schema = official_schema(schema)?;
            let result = OfficialSortOrder::builder()
                .with_order_id(official.order_id)
                .with_fields(official.fields.clone())
                .build(&schema);
            match result {
                Ok(_) => {
                    matched = true;
                    break;
                }
                Err(error) => last_error = Some(error.to_string()),
            }
        }
        if !matched {
            let reason = last_error.as_deref().unwrap_or("no retained schemas");
            return Err(invalid(format_smolstr!(
                "expected sort order {} to bind to a retained schema, got {}",
                order.order_id,
                crate::text::elide_to(reason, 160)
            )));
        }
    }
    Ok(())
}

fn validate_schema_history(
    schemas: &[Field],
    last_column_id: i32,
    version: FormatVersion,
) -> Result<()> {
    let mut schemas: Vec<&Field> = schemas.iter().collect();
    schemas.sort_by_key(|schema| field_schema_id(schema));
    let Some(first) = schemas.first().copied() else {
        return Ok(());
    };
    validate_schema_version(first, version)?;
    let mut highest = first.max_parquet_field_id()?.unwrap_or_default();
    for pair in schemas.windows(2) {
        validate_schema_evolution(pair[0], pair[1], highest, version)?;
        highest = highest.max(pair[1].max_parquet_field_id()?.unwrap_or_default());
    }
    if highest > last_column_id {
        return Err(invalid(format_smolstr!(
            "expected last-column-id of at least {highest}, got {last_column_id}"
        )));
    }
    Ok(())
}

fn validate_schema_version(schema: &Field, version: FormatVersion) -> Result<()> {
    if version >= FormatVersion::V3 {
        return Ok(());
    }
    let schema = official_schema(schema)?;
    for id in 1..=schema.highest_field_id() {
        let Some(field) = schema.field_by_id(id) else {
            continue;
        };
        if field.initial_default.is_some() || field.write_default.is_some() {
            return Err(invalid(format_smolstr!(
                "expected no field defaults before Iceberg v3, got defaults on field id {id}"
            )));
        }
    }
    Ok(())
}

/// Read a `snapshot-log`-shaped array of timestamped identifiers.
fn log_entries(document: &Scalar, key: &str, value_key: &str) -> Vec<(i64, i64)> {
    document
        .get_key_str(key)
        .map(Scalar::sequence_iter)
        .unwrap_or_default()
        .filter_map(|entry| {
            Some((
                entry.get_key_str("timestamp-ms")?.as_i64()?,
                entry.get_key_str(value_key)?.as_i64()?,
            ))
        })
        .collect()
}

/// Read the `metadata-log` array of timestamped previous documents.
fn metadata_log(document: &Scalar) -> Vec<(i64, SmolStr)> {
    document
        .get_key_str("metadata-log")
        .map(Scalar::sequence_iter)
        .unwrap_or_default()
        .filter_map(|entry| {
            Some((
                entry.get_key_str("timestamp-ms")?.as_i64()?,
                SmolStr::new(entry.get_key_str("metadata-file")?.as_str()?),
            ))
        })
        .collect()
}

/// Clone one optional metadata array from an official normalized document.
fn sequence(document: &Scalar, key: &str) -> Vec<Scalar> {
    document
        .get_key_str(key)
        .map(Scalar::sequence_iter)
        .unwrap_or_default()
        .cloned()
        .collect()
}

/// Return the current wall-clock time in milliseconds since the Unix epoch.
pub(super) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

/// Produce the time-ordered UUID used by official Iceberg table creation.
pub(super) fn uuid() -> SmolStr {
    SmolStr::new(uuid::Uuid::now_v7().hyphenated().to_string())
}

/// Report a malformed Iceberg table metadata document.
fn invalid(reason: SmolStr) -> Error {
    Error::Codec {
        format: "iceberg",
        position: 0,
        reason,
    }
}

#[cfg(test)]
mod strict_metadata_tests {
    use super::{FormatVersion, SortOrder, TableMetadata};
    use crate::iceberg::{PartitionSpec, Snapshot, SnapshotRef};
    use crate::{DataType, Scalar};
    use smol_str::SmolStr;

    fn document(version: FormatVersion) -> Scalar {
        let schema = DataType::from_fields([DataType::Int64.required_field("id")])
            .unwrap()
            .required_field("row");
        TableMetadata::new(
            version,
            "file:///tmp/strict-metadata",
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap()
        .into_json()
        .unwrap()
    }

    #[test]
    fn normalization_cannot_hide_duplicate_statistics_or_key_ids() {
        for collection in ["statistics", "partition-statistics"] {
            let duplicates =
                crate::json::from_utf8(r#"[{"snapshot-id":7},{"snapshot-id":7}]"#).unwrap();
            let candidate = document(FormatVersion::V2)
                .with_key(collection, duplicates)
                .unwrap();
            let message = TableMetadata::from_json(&candidate)
                .unwrap_err()
                .to_string();
            assert!(message.contains(collection), "{message}");
            assert!(message.contains("more than once"), "{message}");
        }

        let duplicates = crate::json::from_utf8(r#"[{"key-id":"k"},{"key-id":"k"}]"#).unwrap();
        let candidate = document(FormatVersion::V3)
            .with_key("encryption-keys", duplicates)
            .unwrap();
        let message = TableMetadata::from_json(&candidate)
            .unwrap_err()
            .to_string();
        assert!(message.contains("encryption-keys"), "{message}");
        assert!(message.contains("more than once"), "{message}");
    }

    #[test]
    fn sequence_counters_must_be_non_negative() {
        let candidate = document(FormatVersion::V2)
            .with_key("last-sequence-number", -1_i64)
            .unwrap();
        let message = TableMetadata::from_json(&candidate)
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("non-negative last-sequence-number"),
            "{message}"
        );
    }

    #[test]
    fn v1_accepts_only_the_derived_main_ref_emitted_by_pyiceberg() {
        let mut metadata = TableMetadata::from_json(&document(FormatVersion::V1)).unwrap();
        metadata
            .set_current_snapshot(Snapshot {
                snapshot_id: 7,
                parent_snapshot_id: None,
                sequence_number: None,
                timestamp_ms: metadata.last_updated_ms + 1,
                manifest_list: SmolStr::new_static("file:///tmp/manifest-list.avro"),
                manifests: None,
                summary: vec![(
                    SmolStr::new_static("operation"),
                    SmolStr::new_static("append"),
                )],
                schema_id: Some(metadata.current_schema_id),
                encryption_key_id: None,
                first_row_id: None,
                added_rows: None,
            })
            .unwrap();
        let document = metadata.into_json().unwrap();
        assert!(document.get_key_str("refs").is_none());

        let empty = document
            .clone()
            .with_key("refs", crate::json::from_utf8("{}").unwrap())
            .unwrap();
        assert!(TableMetadata::from_json(&empty).is_ok());

        let refs = Scalar::from_mapping([(
            Scalar::from("main"),
            SnapshotRef::branch(7).into_json().unwrap(),
        )])
        .unwrap();
        let candidate = document.clone().with_key("refs", refs).unwrap();
        let loaded = TableMetadata::from_json(&candidate).unwrap();
        assert!(loaded.refs.is_empty());

        let wrong_refs = Scalar::from_mapping([(
            Scalar::from("main"),
            SnapshotRef::branch(8).into_json().unwrap(),
        )])
        .unwrap();
        let message = TableMetadata::from_json(&document.with_key("refs", wrong_refs).unwrap())
            .unwrap_err()
            .to_string();
        assert!(message.contains("current-snapshot-id"), "{message}");
    }

    #[test]
    fn sort_order_json_has_no_implicit_fields_or_options() {
        for text in [
            r#"{"order-id":0}"#,
            r#"{"order-id":1,"fields":[{"source-id":1,"transform":"identity","direction":"asc"}]}"#,
            r#"{"order-id":1,"fields":[{"source-id":2147483648,"transform":"identity","direction":"asc","null-order":"nulls-first"}]}"#,
            r#"{"order-id":0,"fields":[{"source-id":1,"transform":"identity","direction":"asc","null-order":"nulls-first"}]}"#,
        ] {
            let value = crate::json::from_utf8(text).unwrap();
            assert!(SortOrder::from_json(&value).is_err(), "{text}");
        }
    }

    #[test]
    fn every_historical_layout_must_bind_to_a_retained_schema() {
        let mut partition_document = document(FormatVersion::V2);
        let mut specs: Vec<Scalar> = partition_document
            .get_key_str("partition-specs")
            .unwrap()
            .sequence_iter()
            .cloned()
            .collect();
        specs.push(
            crate::json::from_utf8(
                r#"{"spec-id":1,"fields":[{"source-id":999,"field-id":1000,"name":"missing","transform":"identity"}]}"#,
            )
            .unwrap(),
        );
        partition_document = partition_document
            .with_key("partition-specs", Scalar::from_sequence(specs))
            .unwrap();
        let message = TableMetadata::from_json(&partition_document)
            .unwrap_err()
            .to_string();
        assert!(message.contains("partition spec 1"), "{message}");
        assert!(message.contains("retained schema"), "{message}");

        let mut sort_document = document(FormatVersion::V2);
        let mut orders: Vec<Scalar> = sort_document
            .get_key_str("sort-orders")
            .unwrap()
            .sequence_iter()
            .cloned()
            .collect();
        orders.push(
            crate::json::from_utf8(
                r#"{"order-id":1,"fields":[{"source-id":999,"transform":"identity","direction":"asc","null-order":"nulls-first"}]}"#,
            )
            .unwrap(),
        );
        sort_document = sort_document
            .with_key("sort-orders", Scalar::from_sequence(orders))
            .unwrap();
        let message = TableMetadata::from_json(&sort_document)
            .unwrap_err()
            .to_string();
        assert!(message.contains("sort order 1"), "{message}");
        assert!(message.contains("retained schema"), "{message}");
    }
}
