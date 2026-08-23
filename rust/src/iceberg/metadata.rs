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

use std::hash::{Hash, Hasher};

use smol_str::{SmolStr, format_smolstr};

use super::partition::PartitionSpec;
use super::snapshot::{MAIN_BRANCH, Snapshot, SnapshotRef};
use super::{Transform, schema_from_json, schema_to_json};
use crate::{Error, Field, Result, Value};

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
    pub order_id: i32,
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
    pub fn from_json(document: &Value) -> Result<Self> {
        let order_id = document
            .get_key_str("order-id")
            .and_then(Value::as_i64)
            .and_then(|id| i32::try_from(id).ok())
            .unwrap_or_default();
        let mut fields = Vec::new();
        for entry in document
            .get_key_str("fields")
            .map(Value::sequence_iter)
            .unwrap_or_default()
        {
            fields.push(SortField {
                source_id: entry
                    .get_key_str("source-id")
                    .and_then(Value::as_i64)
                    .and_then(|id| i32::try_from(id).ok())
                    .unwrap_or_default(),
                transform: Transform::from_str(
                    entry
                        .get_key_str("transform")
                        .and_then(Value::as_str)
                        .unwrap_or("identity"),
                )?,
                direction: SmolStr::new(
                    entry
                        .get_key_str("direction")
                        .and_then(Value::as_str)
                        .unwrap_or("asc"),
                ),
                null_order: SmolStr::new(
                    entry
                        .get_key_str("null-order")
                        .and_then(Value::as_str)
                        .unwrap_or("nulls-first"),
                ),
            });
        }
        Ok(Self { order_id, fields })
    }

    /// Write one sort order object.
    ///
    /// # Errors
    ///
    /// Returns an error only when the mapping cannot be built.
    pub fn into_json(self) -> Result<Value> {
        let mut fields = Vec::with_capacity(self.fields.len());
        for field in &self.fields {
            fields.push(Value::from_mapping([
                (
                    Value::from("source-id"),
                    Value::from(i64::from(field.source_id)),
                ),
                (
                    Value::from("transform"),
                    Value::from(field.transform.to_string()),
                ),
                (
                    Value::from("direction"),
                    Value::from(field.direction.clone()),
                ),
                (
                    Value::from("null-order"),
                    Value::from(field.null_order.clone()),
                ),
            ])?);
        }
        Value::from_mapping([
            (
                Value::from("order-id"),
                Value::from(i64::from(self.order_id)),
            ),
            (Value::from("fields"), Value::from_sequence(fields)),
        ])
    }
}

/// The complete state of an Iceberg table at one point in time.
#[derive(Clone, Debug)]
pub struct TableMetadata {
    /// Which revision of the specification this document is written to.
    pub format_version: FormatVersion,
    /// A stable identifier for the table itself, not for any one version.
    pub table_uuid: SmolStr,
    /// The table's base location, as a URI.
    pub location: SmolStr,
    /// Highest assigned sequence number, absent in v1.
    pub last_sequence_number: i64,
    /// When this document was written, in milliseconds since the Unix epoch.
    pub last_updated_ms: i64,
    /// Highest assigned column identifier.
    pub last_column_id: i32,
    /// Every schema the table has had, by identifier.
    pub schemas: Vec<Field>,
    /// The schema new data is written against.
    pub current_schema_id: i32,
    /// Every partition spec the table has had.
    pub partition_specs: Vec<PartitionSpec>,
    /// The spec new data is written against.
    pub default_spec_id: i32,
    /// Highest assigned partition field identifier.
    pub last_partition_id: i32,
    /// Every sort order the table has had.
    pub sort_orders: Vec<SortOrder>,
    /// The order new data is written in.
    pub default_sort_order_id: i32,
    /// Free-form table properties.
    pub properties: Vec<(SmolStr, SmolStr)>,
    /// The snapshot a reader sees, when the table has one.
    pub current_snapshot_id: Option<i64>,
    /// Every retained snapshot.
    pub snapshots: Vec<Snapshot>,
    /// When each snapshot became current, oldest first.
    pub snapshot_log: Vec<(i64, i64)>,
    /// Every previous metadata document, oldest first.
    pub metadata_log: Vec<(i64, SmolStr)>,
    /// Named branches and tags.
    pub refs: Vec<(SmolStr, SnapshotRef)>,
    /// Next unassigned row identifier, required in v3.
    pub next_row_id: Option<i64>,
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
    default_sort_order_id: i32,
    properties: Vec<&'a (SmolStr, SmolStr)>,
    current_snapshot_id: Option<i64>,
    snapshots: Vec<&'a Snapshot>,
    snapshot_log: &'a [(i64, i64)],
    metadata_log: &'a [(i64, SmolStr)],
    refs: Vec<&'a (SmolStr, SnapshotRef)>,
    next_row_id: Option<i64>,
}

impl TableMetadata {
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
        // The schema says how the table is laid out, so the columns the spec
        // partitions on are marked on it rather than only named beside it.
        let schema = spec.mark_partitions(&schema)?;
        let last_partition_id = spec.last_field_id();
        let current_schema_id = schema
            .iceberg()
            .get(super::schema::SCHEMA_ID)
            .and_then(|id| id.parse::<i32>().ok())
            .unwrap_or_default();
        Ok(Self {
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
            next_row_id: (format_version >= FormatVersion::V3).then_some(0),
        })
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

    /// Add a schema, numbering any unnumbered column, and return its fresh id.
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
        let start = self.last_column_id.checked_add(1).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a last-column-id below {}, got {}",
                i32::MAX,
                self.last_column_id
            ))
        })?;
        schema.assign_parquet_field_ids(start)?;
        let next_id = self
            .schemas
            .iter()
            .map(|existing| {
                existing
                    .iceberg()
                    .get(super::schema::SCHEMA_ID)
                    .and_then(|id| id.parse::<i32>().ok())
                    .unwrap_or_default()
            })
            .max()
            .map_or(0, |highest| highest + 1);
        schema
            .iceberg_mut()
            .insert(super::schema::SCHEMA_ID, next_id.to_string())?;
        self.last_column_id = self.last_column_id.max(super::last_field_id(&schema)?);
        self.schemas.push(schema);
        Ok(next_id)
    }

    /// Read a table metadata document of any format version.
    ///
    /// # Errors
    ///
    /// Returns an error when a required key is missing, when the format
    /// version is not one this build implements, or when a nested document is
    /// malformed.
    pub fn from_json(document: &Value) -> Result<Self> {
        let format_version = FormatVersion::from_number(
            document
                .get_key_str("format-version")
                .and_then(Value::as_i64)
                .ok_or_else(|| {
                    invalid(SmolStr::new_static(
                        "expected a table metadata \"format-version\"",
                    ))
                })?,
        )?;
        let location = document
            .get_key_str("location")
            .and_then(Value::as_str)
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
            .map(Value::sequence_iter)
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

        let mut partition_specs = Vec::new();
        for entry in document
            .get_key_str("partition-specs")
            .map(Value::sequence_iter)
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

        // A document records the layout in its spec; a Field records it on the
        // columns. Marking them here is what makes a table read back with the
        // same schema it was created with, marks included.
        let default_spec_id = document
            .get_key_str("default-spec-id")
            .and_then(Value::as_i64)
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
            .map(Value::sequence_iter)
            .unwrap_or_default()
        {
            sort_orders.push(SortOrder::from_json(entry)?);
        }
        if sort_orders.is_empty() {
            sort_orders.push(SortOrder::unsorted());
        }

        let mut snapshots = Vec::new();
        for entry in document
            .get_key_str("snapshots")
            .map(Value::sequence_iter)
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

        // A table with no snapshot spells that as an absent key or as -1.
        let current_snapshot_id = document
            .get_key_str("current-snapshot-id")
            .and_then(Value::as_i64)
            .filter(|id| *id >= 0);

        let metadata = Self {
            format_version,
            table_uuid: SmolStr::new(
                document
                    .get_key_str("table-uuid")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ),
            location: SmolStr::new(location),
            last_sequence_number: document
                .get_key_str("last-sequence-number")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            last_updated_ms: document
                .get_key_str("last-updated-ms")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            last_column_id: document
                .get_key_str("last-column-id")
                .and_then(Value::as_i64)
                .and_then(|id| i32::try_from(id).ok())
                .unwrap_or_default(),
            current_schema_id: document
                .get_key_str("current-schema-id")
                .and_then(Value::as_i64)
                .and_then(|id| i32::try_from(id).ok())
                .unwrap_or_default(),
            schemas,
            default_spec_id,
            last_partition_id: document
                .get_key_str("last-partition-id")
                .and_then(Value::as_i64)
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
                .and_then(Value::as_i64)
                .and_then(|id| i32::try_from(id).ok())
                .unwrap_or_default(),
            sort_orders,
            properties: document
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
                .unwrap_or_default(),
            current_snapshot_id,
            snapshots,
            snapshot_log: log_entries(document, "snapshot-log", "snapshot-id"),
            metadata_log: metadata_log(document),
            refs,
            next_row_id: document.get_key_str("next-row-id").and_then(Value::as_i64),
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
    pub fn into_json(self) -> Result<Value> {
        let mut entries: Vec<(Value, Value)> = vec![
            (
                Value::from("format-version"),
                Value::from(i64::from(self.format_version.number())),
            ),
            (
                Value::from("table-uuid"),
                Value::from(self.table_uuid.clone()),
            ),
            (Value::from("location"), Value::from(self.location.clone())),
        ];
        if self.format_version >= FormatVersion::V2 {
            entries.push((
                Value::from("last-sequence-number"),
                Value::from(self.last_sequence_number),
            ));
        }
        entries.push((
            Value::from("last-updated-ms"),
            Value::from(self.last_updated_ms),
        ));
        entries.push((
            Value::from("last-column-id"),
            Value::from(i64::from(self.last_column_id)),
        ));

        let mut schemas = Vec::with_capacity(self.schemas.len());
        for schema in &self.schemas {
            schemas.push(schema_to_json(schema)?);
        }
        if self.format_version == FormatVersion::V1 {
            // A v1 reader that predates `schemas` still needs the singular key.
            entries.push((
                Value::from("schema"),
                schema_to_json(self.current_schema()?)?,
            ));
        }
        entries.push((Value::from("schemas"), Value::from_sequence(schemas)));
        entries.push((
            Value::from("current-schema-id"),
            Value::from(i64::from(self.current_schema_id)),
        ));

        let mut specs = Vec::with_capacity(self.partition_specs.len());
        for spec in &self.partition_specs {
            specs.push(spec.clone().into_json()?);
        }
        if self.format_version == FormatVersion::V1 {
            entries.push((
                Value::from("partition-spec"),
                self.default_spec()?.clone().into_v1_json()?,
            ));
        }
        entries.push((Value::from("partition-specs"), Value::from_sequence(specs)));
        entries.push((
            Value::from("default-spec-id"),
            Value::from(i64::from(self.default_spec_id)),
        ));
        entries.push((
            Value::from("last-partition-id"),
            Value::from(i64::from(self.last_partition_id)),
        ));

        let mut orders = Vec::with_capacity(self.sort_orders.len());
        for order in &self.sort_orders {
            orders.push(order.clone().into_json()?);
        }
        entries.push((Value::from("sort-orders"), Value::from_sequence(orders)));
        entries.push((
            Value::from("default-sort-order-id"),
            Value::from(i64::from(self.default_sort_order_id)),
        ));

        entries.push((
            Value::from("properties"),
            Value::from_mapping(
                self.properties
                    .iter()
                    .map(|(key, value)| (Value::from(key.clone()), Value::from(value.clone()))),
            )?,
        ));

        if let Some(current) = self.current_snapshot_id {
            entries.push((Value::from("current-snapshot-id"), Value::from(current)));
        }
        let mut snapshots = Vec::with_capacity(self.snapshots.len());
        for snapshot in &self.snapshots {
            snapshots.push(snapshot.clone().into_json(self.format_version)?);
        }
        entries.push((Value::from("snapshots"), Value::from_sequence(snapshots)));

        entries.push((
            Value::from("snapshot-log"),
            Value::from_sequence(
                self.snapshot_log
                    .iter()
                    .map(|(timestamp, snapshot_id)| {
                        Value::from_mapping([
                            (Value::from("timestamp-ms"), Value::from(*timestamp)),
                            (Value::from("snapshot-id"), Value::from(*snapshot_id)),
                        ])
                    })
                    .collect::<Result<Vec<_>>>()?,
            ),
        ));
        entries.push((
            Value::from("metadata-log"),
            Value::from_sequence(
                self.metadata_log
                    .iter()
                    .map(|(timestamp, file)| {
                        Value::from_mapping([
                            (Value::from("timestamp-ms"), Value::from(*timestamp)),
                            (Value::from("metadata-file"), Value::from(file.clone())),
                        ])
                    })
                    .collect::<Result<Vec<_>>>()?,
            ),
        ));

        let mut refs = Vec::with_capacity(self.refs.len());
        for (name, reference) in &self.refs {
            refs.push((Value::from(name.clone()), reference.clone().into_json()?));
        }
        entries.push((Value::from("refs"), Value::from_mapping(refs)?));

        if self.format_version >= FormatVersion::V3 {
            entries.push((
                Value::from("next-row-id"),
                Value::from(self.next_row_id.unwrap_or_default()),
            ));
        }

        Value::from_mapping(entries)
    }

    /// Make `snapshot` the current one, recording it in the log and on `main`.
    pub fn set_current_snapshot(&mut self, snapshot: Snapshot) {
        self.last_updated_ms = snapshot.timestamp_ms;
        if let Some(sequence) = snapshot.sequence_number {
            self.last_sequence_number = self.last_sequence_number.max(sequence);
        }
        self.current_snapshot_id = Some(snapshot.snapshot_id);
        self.snapshot_log
            .push((snapshot.timestamp_ms, snapshot.snapshot_id));
        let reference = SnapshotRef::branch(snapshot.snapshot_id);
        match self.refs.iter_mut().find(|(name, _)| name == MAIN_BRANCH) {
            Some(entry) => entry.1 = reference,
            None => self
                .refs
                .push((SmolStr::new_static(MAIN_BRANCH), reference)),
        }
        self.snapshots.push(snapshot);
    }

    /// Set one table property, returning the value it replaces.
    ///
    /// Keys are unique and keep their insertion order, so a document written
    /// after repeated updates lists its properties in the order they first
    /// appeared.
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
        match self.properties.iter_mut().find(|(name, _)| *name == key) {
            Some(entry) => Ok(Some(std::mem::replace(&mut entry.1, value))),
            None => {
                self.properties.push((key, value));
                Ok(None)
            }
        }
    }

    /// Remove one table property, returning the value it held.
    pub fn remove_property(&mut self, key: &str) -> Option<SmolStr> {
        let index = self.properties.iter().position(|(name, _)| name == key)?;
        Some(self.properties.remove(index).1)
    }

    /// Replace the table's base location.
    pub fn set_location(&mut self, location: impl Into<SmolStr>) {
        self.location = location.into();
    }

    /// Replace the table's UUID, validating the canonical 8-4-4-4-12 shape.
    ///
    /// # Errors
    ///
    /// Returns an error naming the input when it is not hyphenated hex of that
    /// shape; the stored UUID is unchanged.
    pub fn assign_uuid(&mut self, uuid: impl Into<SmolStr>) -> Result<()> {
        let uuid = uuid.into();
        if !is_uuid_shaped(&uuid) {
            return Err(invalid(format_smolstr!(
                "expected a UUID shaped 8-4-4-4-12 hex, got {:?}",
                crate::text::elide_to(&uuid, 64)
            )));
        }
        self.table_uuid = uuid;
        Ok(())
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
        if version < self.format_version {
            return Err(invalid(format_smolstr!(
                "expected a format version of at least {}, got {}",
                self.format_version.number(),
                version.number()
            )));
        }
        self.format_version = version;
        if version >= FormatVersion::V3 && self.next_row_id.is_none() {
            self.next_row_id = Some(0);
        }
        Ok(())
    }

    /// Make one already-added schema the one new data is written against.
    ///
    /// # Errors
    ///
    /// Returns an error naming the id when no schema carries it.
    pub fn set_current_schema(&mut self, schema_id: i32) -> Result<()> {
        if self.schema_by_id(schema_id).is_none() {
            return Err(invalid(format_smolstr!(
                "expected a schema with id {schema_id}, got {} schemas",
                self.schemas.len()
            )));
        }
        self.current_schema_id = schema_id;
        Ok(())
    }

    /// Add a partition spec under the identifier it carries.
    ///
    /// `last-partition-id` stays monotone: it grows to cover the new spec's
    /// highest field and never shrinks, so a retired partition field id is
    /// never reassigned.
    ///
    /// # Errors
    ///
    /// Returns an error naming the id when the table already has a spec with
    /// it; the specs are unchanged.
    pub fn add_spec(&mut self, spec: PartitionSpec) -> Result<i32> {
        if self.spec_by_id(spec.spec_id).is_some() {
            return Err(invalid(format_smolstr!(
                "expected an unused partition spec id, got {} which the table already has",
                spec.spec_id
            )));
        }
        self.last_partition_id = self.last_partition_id.max(spec.last_field_id());
        let spec_id = spec.spec_id;
        self.partition_specs.push(spec);
        Ok(spec_id)
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
        let Some(spec) = self.spec_by_id(spec_id) else {
            return Err(invalid(format_smolstr!(
                "expected a partition spec with id {spec_id}, got {} specs",
                self.partition_specs.len()
            )));
        };
        let spec = spec.clone();
        let mut schemas = Vec::with_capacity(self.schemas.len());
        for schema in &self.schemas {
            schemas.push(spec.mark_partitions(schema)?);
        }
        self.schemas = schemas;
        self.default_spec_id = spec_id;
        Ok(())
    }

    /// Add a sort order under the identifier it carries.
    ///
    /// # Errors
    ///
    /// Returns an error naming the id when the table already has an order with
    /// it; the orders are unchanged.
    pub fn add_sort_order(&mut self, order: SortOrder) -> Result<i32> {
        if self
            .sort_orders
            .iter()
            .any(|existing| existing.order_id == order.order_id)
        {
            return Err(invalid(format_smolstr!(
                "expected an unused sort order id, got {} which the table already has",
                order.order_id
            )));
        }
        let order_id = order.order_id;
        self.sort_orders.push(order);
        Ok(order_id)
    }

    /// Make one already-added sort order the one new data is written in.
    ///
    /// # Errors
    ///
    /// Returns an error naming the id when no order carries it.
    pub fn set_default_sort_order(&mut self, order_id: i32) -> Result<()> {
        if !self
            .sort_orders
            .iter()
            .any(|order| order.order_id == order_id)
        {
            return Err(invalid(format_smolstr!(
                "expected a sort order with id {order_id}, got {} orders",
                self.sort_orders.len()
            )));
        }
        self.default_sort_order_id = order_id;
        Ok(())
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
        let name = name.into();
        if self.snapshot_by_id(reference.snapshot_id).is_none() {
            return Err(invalid(format_smolstr!(
                "expected a retained snapshot for ref {:?}, got unknown snapshot id {}",
                crate::text::elide_to(&name, 64),
                reference.snapshot_id
            )));
        }
        if name == MAIN_BRANCH {
            if reference.kind != "branch" {
                return Err(invalid(format_smolstr!(
                    "expected the reserved \"main\" ref to be a branch, got {:?}",
                    crate::text::elide_to(&reference.kind, 64)
                )));
            }
            if self.current_snapshot_id != Some(reference.snapshot_id) {
                self.last_updated_ms = now_ms();
                self.current_snapshot_id = Some(reference.snapshot_id);
                self.snapshot_log
                    .push((self.last_updated_ms, reference.snapshot_id));
            }
        }
        match self.refs.iter_mut().find(|(existing, _)| *existing == name) {
            Some(entry) => entry.1 = reference,
            None => self.refs.push((name, reference)),
        }
        Ok(())
    }

    /// Remove one named reference, returning what it pointed at.
    ///
    /// Removing the reserved `main` branch clears `current-snapshot-id`, which
    /// keeps the two spellings of "what a reader sees" agreeing: a table whose
    /// main branch is gone has no current snapshot.
    pub fn remove_snapshot_ref(&mut self, name: &str) -> Option<SnapshotRef> {
        let index = self
            .refs
            .iter()
            .position(|(existing, _)| existing == name)?;
        let (_, reference) = self.refs.remove(index);
        if name == MAIN_BRANCH {
            self.current_snapshot_id = None;
        }
        Some(reference)
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
    /// metadata.set_current_snapshot(Snapshot {
    ///     snapshot_id: 7,
    ///     parent_snapshot_id: None,
    ///     sequence_number: Some(1),
    ///     timestamp_ms: 0,
    ///     manifest_list: "".into(),
    ///     summary: Vec::new(),
    ///     schema_id: Some(0),
    ///     first_row_id: None,
    ///     added_rows: None,
    /// });
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
        let reference = reference.clone();
        self.set_snapshot_ref(to, reference)?;
        self.remove_snapshot_ref(from);
        Ok(())
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

    /// Expire snapshots, honoring what every reference says to retain.
    ///
    /// `older_than_ms` is the default age cutoff: a snapshot committed before
    /// it is old. What survives is exactly what the Iceberg retention rules
    /// name - every ref target; for each branch, its head's ancestors younger
    /// than the branch's own `max-snapshot-age-ms` when it has one (younger
    /// than `older_than_ms` otherwise) and at least `min-snapshots-to-keep`
    /// most recent ones; and the current snapshot, always. A tag keeps only
    /// its target. Before any of that, a reference older than its own
    /// `max-ref-age-ms` - measured from its snapshot's commit time - is
    /// removed, except `main`, which never expires. The snapshot log is
    /// trimmed with the removed snapshots, as [`Self::remove_snapshots`]
    /// trims it.
    ///
    /// Returns the removed snapshot ids, sorted; a table with nothing old
    /// returns an empty list.
    ///
    /// # Errors
    ///
    /// Returns an error only when the removal machinery refuses an id, which
    /// the retained set rules out; nothing is removed on error.
    pub fn expire_snapshots_older_than(&mut self, older_than_ms: i64) -> Result<Vec<i64>> {
        let now = now_ms();

        // Refs expire first, so a snapshot a dead ref pointed at is no longer
        // anchored when retention is computed.
        let expired: Vec<SmolStr> = self
            .refs
            .iter()
            .filter(|(name, reference)| {
                if name == MAIN_BRANCH {
                    return false;
                }
                let Some(limit) = reference.max_ref_age_ms else {
                    return false;
                };
                self.snapshot_by_id(reference.snapshot_id)
                    .is_some_and(|snapshot| now.saturating_sub(snapshot.timestamp_ms) > limit)
            })
            .map(|(name, _)| name.clone())
            .collect();
        for name in &expired {
            self.remove_snapshot_ref(name);
        }

        let mut retained: Vec<i64> = self.current_snapshot_id.into_iter().collect();
        for (_, reference) in &self.refs {
            retained.push(reference.snapshot_id);
            if !reference.is_branch() {
                continue;
            }
            let cutoff = match reference.max_snapshot_age_ms {
                Some(age_ms) => now.saturating_sub(age_ms),
                None => older_than_ms,
            };
            let keep_at_least = reference
                .min_snapshots_to_keep
                .and_then(|count| usize::try_from(count).ok())
                .unwrap_or(1);
            let mut position = 1_usize;
            let mut cursor = self
                .snapshot_by_id(reference.snapshot_id)
                .and_then(|head| head.parent_snapshot_id);
            while let Some(id) = cursor {
                position += 1;
                // A corrupt parent chain could cycle; the walk is bounded by
                // the ancestors a table can actually hold.
                let Some(snapshot) = self
                    .snapshot_by_id(id)
                    .filter(|_| position <= self.snapshots.len())
                else {
                    break;
                };
                if position <= keep_at_least || snapshot.timestamp_ms >= cutoff {
                    retained.push(id);
                }
                cursor = snapshot.parent_snapshot_id;
            }
        }

        let mut removed: Vec<i64> = self
            .snapshots
            .iter()
            .map(|snapshot| snapshot.snapshot_id)
            .filter(|id| !retained.contains(id))
            .collect();
        removed.sort_unstable();
        if !removed.is_empty() {
            self.remove_snapshots(&removed)?;
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

    /// Expire snapshots by id, trimming the snapshot log with them.
    ///
    /// An id the table does not hold is ignored, so a caller can expire a set
    /// without checking it first.
    ///
    /// # Errors
    ///
    /// Returns an error naming the snapshot - and the ref, when one points at
    /// it - when an id is the current snapshot or a named reference's target;
    /// nothing is removed on error.
    pub fn remove_snapshots(&mut self, ids: &[i64]) -> Result<()> {
        for id in ids {
            if self.current_snapshot_id == Some(*id) {
                return Err(invalid(format_smolstr!(
                    "expected non-current snapshots to remove, got the current snapshot {id}"
                )));
            }
            if let Some((name, _)) = self
                .refs
                .iter()
                .find(|(_, reference)| reference.snapshot_id == *id)
            {
                return Err(invalid(format_smolstr!(
                    "expected unreferenced snapshots to remove, got {id} which ref {name:?} \
                     points at"
                )));
            }
        }
        self.snapshots
            .retain(|snapshot| !ids.contains(&snapshot.snapshot_id));
        self.snapshot_log.retain(|(_, id)| !ids.contains(id));
        Ok(())
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
            if self.snapshot_by_id(reference.snapshot_id).is_none() {
                return Err(invalid(format_smolstr!(
                    "expected a retained snapshot for ref {:?}, got unknown snapshot id {}",
                    crate::text::elide_to(name, 64),
                    reference.snapshot_id
                )));
            }
            // The two branch retention limits describe ancestors, which only
            // a branch has, so anything else carrying one is malformed.
            if !reference.is_branch() {
                let branch_only = if reference.min_snapshots_to_keep.is_some() {
                    Some("min-snapshots-to-keep")
                } else if reference.max_snapshot_age_ms.is_some() {
                    Some("max-snapshot-age-ms")
                } else {
                    None
                };
                if let Some(key) = branch_only {
                    return Err(invalid(format_smolstr!(
                        "expected a branch for {key} on ref {:?}, got a {:?}",
                        crate::text::elide_to(name, 64),
                        crate::text::elide_to(&reference.kind, 64)
                    )));
                }
            }
        }
        Ok(())
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

/// Return whether text has the canonical hyphenated 8-4-4-4-12 UUID shape.
fn is_uuid_shaped(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    bytes.iter().enumerate().all(|(index, byte)| match index {
        8 | 13 | 18 | 23 => *byte == b'-',
        _ => byte.is_ascii_hexdigit(),
    })
}

/// Read a `snapshot-log`-shaped array of timestamped identifiers.
fn log_entries(document: &Value, key: &str, value_key: &str) -> Vec<(i64, i64)> {
    document
        .get_key_str(key)
        .map(Value::sequence_iter)
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
fn metadata_log(document: &Value) -> Vec<(i64, SmolStr)> {
    document
        .get_key_str("metadata-log")
        .map(Value::sequence_iter)
        .unwrap_or_default()
        .filter_map(|entry| {
            Some((
                entry.get_key_str("timestamp-ms")?.as_i64()?,
                SmolStr::new(entry.get_key_str("metadata-file")?.as_str()?),
            ))
        })
        .collect()
}

/// Return the current wall-clock time in milliseconds since the Unix epoch.
pub(super) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

/// Produce a random version-4 UUID for a newly created table.
///
/// A table identifier only has to be unique, so process-seeded hashing is
/// enough and avoids a dependency whose only job would be sixteen bytes.
pub(super) fn uuid() -> SmolStr {
    use std::hash::{BuildHasher, Hasher};

    let state = std::collections::hash_map::RandomState::new();
    let mut bytes = [0_u8; 16];
    for (half, chunk) in bytes.chunks_mut(8).enumerate() {
        let mut hasher = state.build_hasher();
        hasher.write_usize(half);
        hasher.write_i64(now_ms());
        chunk.copy_from_slice(&hasher.finish().to_le_bytes()[..chunk.len()]);
    }
    // Stamp the version and variant so the value is a well-formed UUIDv4.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format_smolstr!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Report a malformed Iceberg table metadata document.
fn invalid(reason: SmolStr) -> Error {
    Error::Codec {
        format: "iceberg",
        position: 0,
        reason,
    }
}
