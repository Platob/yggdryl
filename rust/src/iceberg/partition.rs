//! Partition specs, their transforms, and the Hive layout they write.
//!
//! A partition spec says how a row's column values become the directory a data
//! file lands in, and which values a manifest records for that file. Iceberg
//! writes those directories in exactly the `column=value` shape
//! [`Url::hive_partitions`](crate::Url::hive_partitions) already reads, so a
//! table this module writes is also a lake the rest of the crate can walk.
//!
//! Apache Iceberg validates transform/source pairs and computes non-identity
//! partition values. Yggdryl keeps its Arrow 59 arrays at the I/O boundary and
//! passes one typed scalar at a time through that implementation while grouping
//! rows for its own data-file writer.

use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use iceberg_official::spec::{
    Datum as OfficialDatum, PrimitiveLiteral as OfficialLiteral,
    PrimitiveType as OfficialPrimitiveType, Transform as OfficialTransform, Type as OfficialType,
};
use iceberg_official::transform::{BoxedTransformFunction, create_transform_function};
use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Error, Field, Result, Scalar};

/// The identifier Iceberg assigns to the first partition field of a table.
pub const FIRST_PARTITION_ID: i32 = 1000;

/// The Iceberg property naming how a partition value is derived.
const TRANSFORM: &str = "transform";

/// The Iceberg property naming the schema column a partition field reads.
const SOURCE_ID: &str = "partition-source-id";

/// The Iceberg property naming the spec a partition tuple belongs to.
const SPEC_ID: &str = "spec-id";

/// How a source column value becomes a partition value.
///
/// ```
/// use yggdryl::iceberg::Transform;
///
/// # fn main() -> yggdryl::Result<()> {
/// assert_eq!(Transform::from_str("bucket[16]")?, Transform::Bucket(16));
/// assert_eq!(Transform::Bucket(16).to_string(), "bucket[16]");
///
/// // Invertibility is distinct from write support: bucket values are
/// // computed by Apache Iceberg even though they cannot restore the source.
/// assert!(Transform::Identity.is_invertible());
/// assert!(!Transform::Bucket(16).is_invertible());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Transform {
    /// The source value, unchanged.
    Identity,
    /// A hash of the source value, modulo a bucket count.
    Bucket(u32),
    /// The source value shortened to a width.
    Truncate(u32),
    /// Years since 1970, from a date or timestamp.
    Year,
    /// Months since 1970-01, from a date or timestamp.
    Month,
    /// Days since 1970-01-01, from a date or timestamp.
    Day,
    /// Hours since 1970-01-01T00, from a timestamp.
    Hour,
    /// Always null, which is how a spec retires a partition field.
    Void,
    /// A transform retained for metadata compatibility but not interpreted.
    Unknown,
}

impl Transform {
    /// Parse an Iceberg transform name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the vocabulary and the input.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Return whether the partition value can recover its source value.
    pub const fn is_invertible(self) -> bool {
        matches!(self, Self::Identity | Self::Void)
    }

    /// Return the datatype a partition value has, given its source column.
    ///
    /// # Errors
    ///
    /// Returns an error when the transform cannot apply to the source type.
    pub fn result_type(self, source: &DataType) -> Result<DataType> {
        self.validate_parameter()?;
        if self == Self::Void {
            return Ok(source.clone());
        }
        if self == Self::Unknown {
            return Ok(DataType::Utf8);
        }

        let transform = official_transform(self);
        let input = OfficialType::Primitive(official_primitive_type(source)?);
        transform.result_type(&input).map_err(Error::from_iceberg)?;

        Ok(match self {
            Self::Identity | Self::Truncate(_) => source.clone(),
            Self::Day => DataType::Date32,
            Self::Bucket(_) | Self::Year | Self::Month | Self::Hour => DataType::Int32,
            Self::Void | Self::Unknown => unreachable!("returned above"),
        })
    }

    /// Reject parameters the official scalar implementation cannot evaluate.
    fn validate_parameter(self) -> Result<()> {
        match self {
            Self::Bucket(0) => Err(invalid(SmolStr::new_static(
                "expected a positive bucket count, got bucket[0]",
            ))),
            Self::Bucket(count) if count > i32::MAX as u32 => Err(invalid(format_smolstr!(
                "expected a bucket count of at most {}, got bucket[{count}]",
                i32::MAX
            ))),
            Self::Truncate(0) => Err(invalid(SmolStr::new_static(
                "expected a positive truncate width, got truncate[0]",
            ))),
            _ => Ok(()),
        }
    }
}

impl FromStr for Transform {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let trimmed = value.trim();
        match trimmed {
            "identity" => return Ok(Self::Identity),
            "year" => return Ok(Self::Year),
            "month" => return Ok(Self::Month),
            "day" => return Ok(Self::Day),
            "hour" => return Ok(Self::Hour),
            "void" => return Ok(Self::Void),
            "unknown" => return Ok(Self::Unknown),
            _ => {}
        }
        if let Some(rest) = trimmed.strip_prefix("bucket") {
            return Ok(Self::Bucket(bracketed(rest, "bucket")?));
        }
        if let Some(rest) = trimmed.strip_prefix("truncate") {
            return Ok(Self::Truncate(bracketed(rest, "truncate")?));
        }
        Err(Error::Parse {
            target: "iceberg transform",
            position: 0,
            reason: format_smolstr!(
                "expected an Iceberg transform (identity, bucket[n], truncate[w], year, month, \
                 day, hour, void, unknown), got {trimmed:?}"
            ),
        })
    }
}

impl fmt::Display for Transform {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Identity => formatter.write_str("identity"),
            Self::Bucket(count) => write!(formatter, "bucket[{count}]"),
            Self::Truncate(width) => write!(formatter, "truncate[{width}]"),
            Self::Year => formatter.write_str("year"),
            Self::Month => formatter.write_str("month"),
            Self::Day => formatter.write_str("day"),
            Self::Hour => formatter.write_str("hour"),
            Self::Void => formatter.write_str("void"),
            Self::Unknown => formatter.write_str("unknown"),
        }
    }
}

/// Read `[n]` or `(n)` after a transform keyword.
fn bracketed(rest: &str, keyword: &str) -> Result<u32> {
    let trimmed = rest.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .or_else(|| {
            trimmed
                .strip_prefix('(')
                .and_then(|value| value.strip_suffix(')'))
        })
        .ok_or_else(|| Error::Parse {
            target: "iceberg transform",
            position: 0,
            reason: format_smolstr!("expected {keyword}[n], got {keyword}{rest}"),
        })?;
    inner.trim().parse::<u32>().map_err(|_| Error::Parse {
        target: "iceberg transform",
        position: 0,
        reason: format_smolstr!(
            "expected an integer {keyword} parameter, got {:?}",
            inner.trim()
        ),
    })
}

/// One partition column: a source column, a transform, and a name.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PartitionField {
    /// Identifier of the schema field this partitions on.
    pub source_id: i32,
    /// Identifier of the partition field itself, unique within a table.
    pub field_id: i32,
    /// The partition column's name, which is also its directory prefix.
    pub name: SmolStr,
    /// How the source value becomes the partition value.
    pub transform: Transform,
}

impl PartitionField {
    /// Return a deterministic hash of this complete partition field.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }

    /// Partition on a source column's value unchanged.
    pub fn identity(source_id: i32, field_id: i32, name: impl Into<SmolStr>) -> Self {
        Self {
            source_id,
            field_id,
            name: name.into(),
            transform: Transform::Identity,
        }
    }

    /// Read one partition field object.
    ///
    /// # Errors
    ///
    /// Returns an error when a required key is missing or a transform is not
    /// one Iceberg names.
    pub fn from_json(document: &Scalar) -> Result<Self> {
        Self::from_json_with_field_id(document, None)
    }

    /// Read a field, assigning `default_field_id` only for the v1 array form.
    fn from_json_with_field_id(document: &Scalar, default_field_id: Option<i32>) -> Result<Self> {
        let name = document
            .get_key_str("name")
            .and_then(Scalar::as_str)
            .ok_or_else(|| invalid(SmolStr::new_static("expected a partition field \"name\"")))?;
        let source_id = narrow(document.get_key_str("source-id"), "source-id", name)?;
        let field_id = match document.get_key_str("field-id") {
            Some(value) => narrow(Some(value), "field-id", name)?,
            None => default_field_id.ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected a 32-bit integer \"field-id\" on partition field {name:?}"
                ))
            })?,
        };
        let transform = Transform::from_str(
            document
                .get_key_str("transform")
                .and_then(Scalar::as_str)
                .ok_or_else(|| {
                    invalid(format_smolstr!(
                        "expected a partition field \"transform\" on {name:?}"
                    ))
                })?,
        )?;
        Ok(Self {
            source_id,
            field_id,
            name: SmolStr::new(name),
            transform,
        })
    }

    /// Write one partition field object.
    ///
    /// # Errors
    ///
    /// Returns an error only when the mapping cannot be built.
    pub fn into_json(self) -> Result<Scalar> {
        Scalar::from_mapping([
            (Scalar::from("name"), Scalar::from(self.name.clone())),
            (
                Scalar::from("transform"),
                Scalar::from(self.transform.to_string()),
            ),
            (
                Scalar::from("source-id"),
                Scalar::from(i64::from(self.source_id)),
            ),
            (
                Scalar::from("field-id"),
                Scalar::from(i64::from(self.field_id)),
            ),
        ])
    }
}

/// An ordered set of partition fields, identified by a spec id.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PartitionSpec {
    /// Identifier of this spec within the table.
    pub spec_id: i32,
    /// The partition columns, in the order they nest as directories.
    pub fields: Vec<PartitionField>,
}

impl PartitionSpec {
    /// Return a deterministic hash of this complete partition specification.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }

    /// The unpartitioned spec, which every table has as spec zero.
    pub const fn unpartitioned() -> Self {
        Self {
            spec_id: 0,
            fields: Vec::new(),
        }
    }

    /// Build a spec that partitions on the named columns' values unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error when a named column is not in the schema or carries no
    /// field identifier.
    pub fn identity(spec_id: i32, schema: &Field, columns: &[&str]) -> Result<Self> {
        let mut fields = Vec::with_capacity(columns.len());
        for (offset, column) in columns.iter().enumerate() {
            let source = schema.get_field_by_name(column).ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected a schema column to partition on, got {column:?}"
                ))
            })?;
            let source_id = source.parquet_field_id()?.ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected a PARQUET:field_id on the partition source {column:?}; call \
                     assign_field_ids first"
                ))
            })?;
            fields.push(PartitionField::identity(
                source_id,
                FIRST_PARTITION_ID + i32::try_from(offset).unwrap_or_default(),
                *column,
            ));
        }
        Ok(Self { spec_id, fields })
    }

    /// Build a spec from the columns a schema already marks as partitions.
    ///
    /// A [`Field`] says which of its children a path spells out, so a caller
    /// who declared that on the schema does not declare it again here. The
    /// marked columns become identity partition fields in declaration order,
    /// and a schema that marks none produces the unpartitioned spec.
    ///
    /// # Errors
    ///
    /// Returns an error when the field is not a struct root, or when a marked
    /// column carries no field identifier.
    pub fn from_schema(spec_id: i32, schema: &Field) -> Result<Self> {
        schema.require_struct()?;
        let columns: Vec<&str> = schema.partition_field_names().collect();
        Self::identity(spec_id, schema, &columns)
    }

    /// Read a spec back off the partition tuple it describes.
    ///
    /// This is the inverse of [`Self::partition_field`]: the tuple carries the
    /// transform, the source column, and the spec identifier as Iceberg
    /// properties, so a caller holding the tuple holds the spec and does not
    /// need the table metadata beside it.
    ///
    /// # Errors
    ///
    /// Returns an error when the field is not a struct, or when a child is
    /// missing its field identifier, source identifier, or transform.
    pub fn from_field(partition: &Field) -> Result<Self> {
        partition.require_struct()?;
        let spec_id = partition.iceberg().get(SPEC_ID).map_or(Ok(0), |id| {
            id.parse::<i32>().map_err(|_| {
                invalid(format_smolstr!(
                    "expected an integer iceberg:{SPEC_ID} on a partition tuple, got {id:?}"
                ))
            })
        })?;
        let mut fields = Vec::with_capacity(partition.field_len());
        for child in partition.fields() {
            let name = child.name();
            let field_id = child.parquet_field_id()?.ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected a PARQUET:field_id on the partition field {name:?}, got none"
                ))
            })?;
            let source_id = child
                .iceberg()
                .get(SOURCE_ID)
                .and_then(|id| id.parse::<i32>().ok())
                .ok_or_else(|| {
                    invalid(format_smolstr!(
                        "expected an integer iceberg:{SOURCE_ID} on the partition field {name:?}"
                    ))
                })?;
            let transform = child
                .iceberg()
                .get(TRANSFORM)
                .map_or(Ok(Transform::Identity), Transform::from_str)?;
            fields.push(PartitionField {
                source_id,
                field_id,
                name: SmolStr::new(name),
                transform,
            });
        }
        Ok(Self { spec_id, fields })
    }

    /// Return `schema` with the columns this spec partitions on marked.
    ///
    /// Only an identity transform marks a column: it is the one transform whose
    /// partition value *is* the column's value, so it is the only one a path can
    /// spell and a reader can invert. A schema carrying the marks says how it is
    /// laid out without a spec beside it.
    ///
    /// # Errors
    ///
    /// Returns an error when the field is not a struct root, or when a source
    /// column is missing from it.
    pub fn mark_partitions(&self, schema: &Field) -> Result<Field> {
        let mut columns = Vec::with_capacity(self.fields.len());
        for field in &self.fields {
            if field.transform != Transform::Identity {
                continue;
            }
            // A spec can name a column of a nested struct, which no path spells
            // out and no top-level marker describes; such a field is left alone.
            let Some(source) = schema.field_by_parquet_field_id(field.source_id) else {
                continue;
            };
            if schema.get_field_by_name(source.name()).is_some() {
                columns.push(source.name());
            }
        }
        schema.with_partition_fields(&columns)
    }

    /// Return whether this spec places every file in one partition.
    pub fn is_unpartitioned(&self) -> bool {
        self.fields.is_empty()
    }

    /// Return the highest partition field identifier this spec uses.
    pub fn last_field_id(&self) -> i32 {
        self.fields
            .iter()
            .map(|field| field.field_id)
            .max()
            .unwrap_or(FIRST_PARTITION_ID - 1)
    }

    /// Return the source column names, in partition order.
    pub fn source_names(&self, schema: &Field) -> Result<Vec<SmolStr>> {
        let mut names = Vec::with_capacity(self.fields.len());
        for field in &self.fields {
            names.push(SmolStr::new(source_column(schema, field.source_id)?.name()));
        }
        Ok(names)
    }

    /// Reject a spec containing a transform this writer cannot evaluate.
    ///
    /// # Errors
    ///
    /// Returns an error naming an unknown transform or invalid parameter.
    pub fn require_writable(&self) -> Result<()> {
        for field in &self.fields {
            field.transform.validate_parameter()?;
            if field.transform == Transform::Unknown {
                return Err(invalid(format_smolstr!(
                    "expected a supported partition transform to place a row, got {} on {:?}",
                    field.transform,
                    field.name
                )));
            }
        }
        Ok(())
    }

    /// Build the typed scalar transform plan used by a data write.
    pub(super) fn write_transforms(
        &self,
        schema: &Field,
        partition: &Field,
    ) -> Result<Vec<PartitionTransform>> {
        self.require_writable()?;
        if partition.field_len() != self.fields.len() {
            return Err(invalid(format_smolstr!(
                "expected {} partition fields for spec {}, got {}",
                self.fields.len(),
                self.spec_id,
                partition.field_len()
            )));
        }

        let mut transforms = Vec::with_capacity(self.fields.len());
        for (field, result) in self.fields.iter().zip(partition.fields()) {
            let (path, source) = source_path(schema, field.source_id)?;
            let expected = field.transform.result_type(source.data_type())?;
            if result.data_type() != &expected {
                return Err(invalid(format_smolstr!(
                    "expected partition field {:?} to have type {expected}, got {}",
                    field.name,
                    result.data_type()
                )));
            }
            let function = match field.transform {
                Transform::Identity | Transform::Void => None,
                transform => Some(
                    create_transform_function(&official_transform(transform))
                        .map_err(Error::from_iceberg)?,
                ),
            };
            transforms.push(PartitionTransform {
                path,
                source: source.clone(),
                result: result.clone(),
                transform: field.transform,
                function,
            });
        }
        Ok(transforms)
    }

    /// Return the non-null struct Field the partition tuple has.
    ///
    /// This is the schema of a manifest's `partition` column, which is what
    /// makes a partition value readable without consulting the path. Each child
    /// also carries what produced it - the transform, the source column's
    /// identifier, and the partition marker every path-borne column carries - so
    /// the tuple describes itself and [`Self::from_field`] reads this spec back
    /// out of it.
    ///
    /// # Errors
    ///
    /// Returns an error when a source column is missing from `schema`, a
    /// transform cannot apply to it, or a property cannot be recorded.
    pub fn partition_field(&self, schema: &Field) -> Result<Field> {
        let mut children = Vec::with_capacity(self.fields.len());
        for field in &self.fields {
            let source = source_column(schema, field.source_id)?;
            let data_type = field.transform.result_type(source.data_type())?;
            // A partition value is nullable even when its source is not: a
            // spec can retire a field, and `void` produces nothing but null.
            let mut child = Field::new(field.name.as_str(), data_type, true);
            if child.data_type() == source.data_type() {
                if let Some(declared) = source.iceberg().get(super::schema::DECLARED_TYPE) {
                    child
                        .iceberg_mut()
                        .insert(super::schema::DECLARED_TYPE, declared)?;
                }
            }
            child.set_parquet_field_id(field.field_id);
            child.set_partition(true);
            child
                .iceberg_mut()
                .insert(SOURCE_ID, field.source_id.to_string())?;
            child
                .iceberg_mut()
                .insert(TRANSFORM, field.transform.to_string())?;
            children.push(child);
        }
        let mut partition = Field::new("partition", DataType::from_fields(children)?, false);
        partition
            .iceberg_mut()
            .insert(SPEC_ID, self.spec_id.to_string())?;
        Ok(partition)
    }

    /// Return the Hive-style directory chain one partition tuple names.
    ///
    /// `values` is one value per partition field, in spec order. A null value
    /// writes the literal `null`, which is what Iceberg's own writers spell and
    /// why the manifest, not the path, is the authority on a partition value.
    ///
    /// # Errors
    ///
    /// Returns an error when the tuple is not one value per partition field.
    pub fn partition_path(&self, values: &[Scalar]) -> Result<String> {
        if values.len() != self.fields.len() {
            return Err(invalid(format_smolstr!(
                "expected {} partition values for spec {}, got {}",
                self.fields.len(),
                self.spec_id,
                values.len()
            )));
        }
        let mut path = String::new();
        for (field, value) in self.fields.iter().zip(values) {
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(&field.name);
            path.push('=');
            path.push_str(&super::value::scalar_text(value));
        }
        Ok(path)
    }

    /// Read a partition spec object, in either the v1 or the v2 shape.
    ///
    /// # Errors
    ///
    /// Returns an error when the document is neither a spec object nor the
    /// bare field array a v1 table writes.
    pub fn from_json(document: &Scalar) -> Result<Self> {
        // v1 wrote `partition-spec` as a bare array of fields with no id.
        if let Some(entries) = document.as_sequence() {
            let mut fields = Vec::with_capacity(entries.len());
            for (offset, entry) in entries.iter().enumerate() {
                let offset = i32::try_from(offset).map_err(|_| {
                    invalid(SmolStr::new_static(
                        "expected fewer than 2147482648 fields in a v1 partition spec",
                    ))
                })?;
                let field_id = FIRST_PARTITION_ID.checked_add(offset).ok_or_else(|| {
                    invalid(SmolStr::new_static(
                        "expected a v1 partition field id within signed 32-bit range",
                    ))
                })?;
                fields.push(PartitionField::from_json_with_field_id(
                    entry,
                    Some(field_id),
                )?);
            }
            let spec = Self { spec_id: 0, fields };
            spec.validate_shape()?;
            return Ok(spec);
        }

        let spec_id = document
            .get_key_str("spec-id")
            .and_then(Scalar::as_i64)
            .and_then(|id| i32::try_from(id).ok())
            .ok_or_else(|| {
                invalid(SmolStr::new_static(
                    "expected a 32-bit integer partition spec \"spec-id\"",
                ))
            })?;
        let entries = document
            .get_key_str("fields")
            .and_then(Scalar::as_sequence)
            .ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected a \"fields\" array in partition spec {spec_id}"
                ))
            })?;
        let mut fields = Vec::with_capacity(entries.len());
        for entry in entries {
            fields.push(PartitionField::from_json(entry)?);
        }
        let spec = Self { spec_id, fields };
        spec.validate_shape()?;
        Ok(spec)
    }

    /// Validate identifiers and names without needing the source schema.
    pub(crate) fn validate_shape(&self) -> Result<()> {
        if self.spec_id < 0 {
            return Err(invalid(format_smolstr!(
                "expected a non-negative partition spec id, got {}",
                self.spec_id
            )));
        }
        let mut ids = HashSet::with_capacity(self.fields.len());
        let mut names = HashSet::with_capacity(self.fields.len());
        for field in &self.fields {
            if field.source_id <= 0 {
                return Err(invalid(format_smolstr!(
                    "expected a positive source-id on partition field {:?}, got {}",
                    crate::text::elide_to(&field.name, 64),
                    field.source_id
                )));
            }
            if field.field_id < FIRST_PARTITION_ID {
                return Err(invalid(format_smolstr!(
                    "expected a partition field-id of at least {FIRST_PARTITION_ID}, got {}",
                    field.field_id
                )));
            }
            if field.name.is_empty() {
                return Err(invalid(SmolStr::new_static(
                    "expected a non-empty partition field name",
                )));
            }
            if !ids.insert(field.field_id) {
                return Err(invalid(format_smolstr!(
                    "expected unique partition field ids in spec {}, got {} more than once",
                    self.spec_id,
                    field.field_id
                )));
            }
            if !names.insert(field.name.as_str()) {
                return Err(invalid(format_smolstr!(
                    "expected unique partition field names in spec {}, got {:?} more than once",
                    self.spec_id,
                    crate::text::elide_to(&field.name, 64)
                )));
            }
        }
        Ok(())
    }

    /// Write this spec as a v2 partition spec object.
    ///
    /// # Errors
    ///
    /// Returns an error only when the mapping cannot be built.
    pub fn into_json(self) -> Result<Scalar> {
        self.validate_shape()?;
        let mut fields = Vec::with_capacity(self.fields.len());
        for field in &self.fields {
            fields.push(field.clone().into_json()?);
        }
        Scalar::from_mapping([
            (
                Scalar::from("spec-id"),
                Scalar::from(i64::from(self.spec_id)),
            ),
            (Scalar::from("fields"), Scalar::from_sequence(fields)),
        ])
    }

    /// Write this spec as the bare field array a v1 table stores.
    ///
    /// # Errors
    ///
    /// Returns an error only when a field mapping cannot be built.
    pub fn into_v1_json(self) -> Result<Scalar> {
        self.validate_shape()?;
        let mut fields = Vec::with_capacity(self.fields.len());
        for field in &self.fields {
            fields.push(field.clone().into_json()?);
        }
        Ok(Scalar::from_sequence(fields))
    }
}

/// One validated scalar transform in a partitioned data-write plan.
pub(super) struct PartitionTransform {
    path: Vec<SmolStr>,
    source: Field,
    result: Field,
    transform: Transform,
    function: Option<BoxedTransformFunction>,
}

impl PartitionTransform {
    pub(super) fn path(&self) -> &[SmolStr] {
        &self.path
    }

    pub(super) fn source(&self) -> &Field {
        &self.source
    }

    /// Compute one partition value through Apache Iceberg's scalar transform.
    pub(super) fn partition_value(&self, value: Scalar) -> Result<Scalar> {
        if value.is_null() || self.transform == Transform::Void {
            return Ok(Scalar::Null);
        }
        if self.transform == Transform::Identity {
            return Ok(value);
        }

        // The official literal implementation unwraps Date32 calendar
        // conversion. Core calendar arithmetic is total over i32 day counts,
        // so keep caller-controlled extremes out of that panic path.
        if let Scalar::Date32(days, ..) = &value {
            let (year, month, _) = crate::generic::timezone::civil_from_days(i64::from(*days));
            let month = i32::try_from(month).map_err(|_| {
                invalid(format_smolstr!(
                    "expected a calendar month fitting i32, got {month}"
                ))
            })?;
            return match self.transform {
                Transform::Year => Ok(Scalar::I32(year - 1970)),
                Transform::Month => Ok(Scalar::I32((year - 1970) * 12 + month - 1)),
                Transform::Day => Ok(Scalar::date32(*days)),
                _ => self.official_value(Scalar::date32(*days)),
            };
        }

        self.official_value(value)
    }

    fn official_value(&self, value: Scalar) -> Result<Scalar> {
        // iceberg-rust 0.10 validates binary truncation but its literal path
        // implements only numeric and string values. Keep its exact byte-prefix
        // semantics here until the official literal implementation covers it.
        if let (Transform::Truncate(width), Some(bytes)) = (self.transform, value.as_bytes()) {
            let width = usize::try_from(width).unwrap_or(usize::MAX);
            return Ok(Scalar::from(&bytes[..bytes.len().min(width)]));
        }

        let datum = official_datum(&value, self.source.data_type())?;
        let transformed = self
            .function
            .as_ref()
            .ok_or_else(|| invalid(SmolStr::new_static("expected a partition transform")))?
            .transform_literal(&datum)
            .map_err(Error::from_iceberg)?
            .ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected {} to produce a partition value, got null",
                    self.transform
                ))
            })?;
        scalar_from_official(&transformed, self.result.data_type())
    }
}

fn official_transform(transform: Transform) -> OfficialTransform {
    match transform {
        Transform::Identity => OfficialTransform::Identity,
        Transform::Bucket(count) => OfficialTransform::Bucket(count),
        Transform::Truncate(width) => OfficialTransform::Truncate(width),
        Transform::Year => OfficialTransform::Year,
        Transform::Month => OfficialTransform::Month,
        Transform::Day => OfficialTransform::Day,
        Transform::Hour => OfficialTransform::Hour,
        Transform::Void => OfficialTransform::Void,
        Transform::Unknown => OfficialTransform::Unknown,
    }
}

fn official_primitive_type(data_type: &DataType) -> Result<OfficialPrimitiveType> {
    Ok(match super::PrimitiveType::from_data_type(data_type)? {
        super::PrimitiveType::Boolean => OfficialPrimitiveType::Boolean,
        super::PrimitiveType::Int => OfficialPrimitiveType::Int,
        super::PrimitiveType::Long => OfficialPrimitiveType::Long,
        super::PrimitiveType::Float => OfficialPrimitiveType::Float,
        super::PrimitiveType::Double => OfficialPrimitiveType::Double,
        super::PrimitiveType::Decimal { precision, scale } => OfficialPrimitiveType::Decimal {
            precision: u32::from(precision),
            scale: u32::try_from(scale).map_err(|_| {
                invalid(format_smolstr!(
                    "expected a non-negative Iceberg decimal scale, got {scale}"
                ))
            })?,
        },
        super::PrimitiveType::Date => OfficialPrimitiveType::Date,
        super::PrimitiveType::Time => OfficialPrimitiveType::Time,
        super::PrimitiveType::Timestamp => OfficialPrimitiveType::Timestamp,
        super::PrimitiveType::Timestamptz => OfficialPrimitiveType::Timestamptz,
        super::PrimitiveType::TimestampNs => OfficialPrimitiveType::TimestampNs,
        super::PrimitiveType::TimestamptzNs => OfficialPrimitiveType::TimestamptzNs,
        super::PrimitiveType::String => OfficialPrimitiveType::String,
        super::PrimitiveType::Uuid => OfficialPrimitiveType::Uuid,
        super::PrimitiveType::Fixed(width) => {
            OfficialPrimitiveType::Fixed(u64::try_from(width).map_err(|_| {
                invalid(format_smolstr!(
                    "expected a non-negative Iceberg fixed width, got {width}"
                ))
            })?)
        }
        super::PrimitiveType::Binary => OfficialPrimitiveType::Binary,
        super::PrimitiveType::Unknown => {
            return Err(invalid(SmolStr::new_static(
                "expected a concrete Iceberg partition source type, got unknown",
            )));
        }
    })
}

fn official_datum(value: &Scalar, data_type: &DataType) -> Result<OfficialDatum> {
    let primitive = official_primitive_type(data_type)?;
    let bytes = match (value, data_type) {
        (
            Scalar::D128(unscaled, actual_scale),
            DataType::Decimal32 { scale, .. }
            | DataType::Decimal64 { scale, .. }
            | DataType::Decimal128 { scale, .. },
        ) if actual_scale == scale => unscaled.to_be_bytes().to_vec(),
        (
            Scalar::D128(_, actual_scale),
            DataType::Decimal32 { scale, .. }
            | DataType::Decimal64 { scale, .. }
            | DataType::Decimal128 { scale, .. },
        ) => {
            return Err(invalid(format_smolstr!(
                "expected a decimal scalar at scale {scale}, got scale {actual_scale}"
            )));
        }
        _ => super::value::single_value(value, data_type).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a scalar compatible with Iceberg type {data_type}, got {}",
                value.kind()
            ))
        })?,
    };
    OfficialDatum::try_from_bytes(&bytes, primitive).map_err(Error::from_iceberg)
}

fn scalar_from_official(value: &OfficialDatum, data_type: &DataType) -> Result<Scalar> {
    if let DataType::Decimal32 { scale, .. }
    | DataType::Decimal64 { scale, .. }
    | DataType::Decimal128 { scale, .. } = data_type
    {
        return match value.literal() {
            OfficialLiteral::Int128(unscaled) => Ok(Scalar::D128(*unscaled, *scale)),
            literal => Err(invalid(format_smolstr!(
                "expected an Iceberg decimal partition value, got {literal:?}"
            ))),
        };
    }

    let bytes = value.to_bytes().map_err(Error::from_iceberg)?;
    super::value::single_to_value(bytes.as_ref(), data_type).ok_or_else(|| {
        invalid(format_smolstr!(
            "expected an Iceberg partition value of type {data_type}, got {value}"
        ))
    })
}

/// Find the schema column one partition field reads.
fn source_column(schema: &Field, source_id: i32) -> Result<&Field> {
    schema.field_by_parquet_field_id(source_id).ok_or_else(|| {
        invalid(format_smolstr!(
            "expected a schema column with field id {source_id} to partition on, got none"
        ))
    })
}

/// Return a primitive source and its top-level/struct child path.
fn source_path(schema: &Field, source_id: i32) -> Result<(Vec<SmolStr>, &Field)> {
    fn find<'a>(field: &'a Field, source_id: i32, path: &mut Vec<SmolStr>) -> Option<&'a Field> {
        for child in field.fields() {
            path.push(SmolStr::new(child.name()));
            if child.parquet_field_id().ok().flatten() == Some(source_id) {
                return Some(child);
            }
            if matches!(child.data_type(), DataType::Struct(_)) {
                if let Some(found) = find(child, source_id, path) {
                    return Some(found);
                }
            }
            path.pop();
        }
        None
    }

    let mut path = Vec::new();
    let source = find(schema, source_id, &mut path).ok_or_else(|| {
        invalid(format_smolstr!(
            "expected a schema primitive reachable through structs with field id {source_id}, \
             got none"
        ))
    })?;
    Ok((path, source))
}

/// Narrow one required integer key of a partition field.
fn narrow(value: Option<&Scalar>, key: &str, name: &str) -> Result<i32> {
    value
        .and_then(Scalar::as_i64)
        .and_then(|id| i32::try_from(id).ok())
        .ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a 32-bit integer {key:?} on partition field {name:?}"
            ))
        })
}

/// Report a malformed Iceberg partition document.
fn invalid(reason: SmolStr) -> Error {
    Error::Codec {
        format: "iceberg",
        position: 0,
        reason,
    }
}

#[cfg(test)]
mod strict_json_tests {
    use super::{PartitionField, PartitionSpec};

    #[test]
    fn modern_partition_json_requires_exact_identifiers() {
        for text in [
            r#"{"name":"day","source-id":1,"transform":"identity"}"#,
            r#"{"name":"day","source-id":1,"field-id":2147483648,"transform":"identity"}"#,
        ] {
            let document = crate::json::from_utf8(text).unwrap();
            assert!(PartitionField::from_json(&document).is_err(), "{text}");
        }
        for text in [
            r#"{"fields":[]}"#,
            r#"{"spec-id":2147483648,"fields":[]}"#,
            r#"{"spec-id":1,"fields":[{"name":"day","source-id":1,"transform":"identity"}]}"#,
        ] {
            let document = crate::json::from_utf8(text).unwrap();
            assert!(PartitionSpec::from_json(&document).is_err(), "{text}");
        }
    }

    #[test]
    fn only_the_v1_array_synthesizes_field_ids() {
        let document =
            crate::json::from_utf8(r#"[{"name":"day","source-id":1,"transform":"identity"}]"#)
                .unwrap();
        let spec = PartitionSpec::from_json(&document).unwrap();
        assert_eq!(spec.spec_id, 0);
        assert_eq!(spec.fields[0].field_id, 1000);
    }

    #[test]
    fn a_spec_rejects_duplicate_field_ids_and_names() {
        for fields in [
            r#"[{"name":"a","source-id":1,"field-id":1000,"transform":"identity"},{"name":"b","source-id":2,"field-id":1000,"transform":"identity"}]"#,
            r#"[{"name":"a","source-id":1,"field-id":1000,"transform":"identity"},{"name":"a","source-id":2,"field-id":1001,"transform":"identity"}]"#,
        ] {
            let document =
                crate::json::from_utf8(&format!(r#"{{"spec-id":1,"fields":{fields}}}"#)).unwrap();
            assert!(PartitionSpec::from_json(&document).is_err(), "{fields}");
        }
    }
}
