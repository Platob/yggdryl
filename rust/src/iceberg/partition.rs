//! Partition specs, their transforms, and the Hive layout they write.
//!
//! A partition spec says how a row's column values become the directory a data
//! file lands in, and which values a manifest records for that file. Iceberg
//! writes those directories in exactly the `column=value` shape
//! [`Url::hive_partitions`](crate::Url::hive_partitions) already reads, so a
//! table this module writes is also a lake the rest of the crate can walk.
//!
//! Apache Iceberg validates the standard transform/source pairs and computes
//! a bucket or a truncation. Every time transform - the specification's
//! `year`, `month`, `day` and `hour`, and this crate's own `minute`, `qhour`,
//! `hhour`, `week` and `quarter` - is computed here through the one
//! [`EpochPeriod`] the expression grammar's `years(x)` through `minutes(x)`
//! read, so a partition value and a filter cannot disagree about a period.
//! Yggdryl keeps its Arrow 59 arrays at the I/O boundary and passes one typed
//! scalar at a time through that implementation while grouping rows for its
//! own data-file writer.
//!
//! The five transforms of this crate's own have no spelling in the official
//! model, which refuses a name it does not know. They cross it as a bucket
//! whose count no table can state - `bucket[2147483649]` through
//! `bucket[2147483653]`, above the `i32::MAX` [`Transform::result_type`]
//! refuses - because a bucket is accepted on every date and timestamp source,
//! answers the same `int32` a period does, and each count is its own
//! transform to the official duplicate check. [`Transform::into_official`]
//! and [`Transform::from_official`] are the two halves, and every document
//! handed to the official model is rewritten through them in `official.rs`.

use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use iceberg_official::spec::{
    Datum as OfficialDatum, PrimitiveLiteral as OfficialLiteral,
    PrimitiveType as OfficialPrimitiveType, Transform as OfficialTransform, Type as OfficialType,
};
use iceberg_official::transform::{BoxedTransformFunction, create_transform_function};
use smol_str::{SmolStr, format_smolstr};

use crate::expression::Function;
use crate::expression::eval::{EpochPeriod, epoch_value};
use crate::{DataType, Error, Field, Result, Scalar, StructType};

/// The identifier Iceberg assigns to the first partition field of a table.
pub const FIRST_PARTITION_ID: i32 = 1000;

/// The Iceberg property naming how a partition value is derived.
pub(super) const TRANSFORM: &str = "transform";

/// The Iceberg property naming the schema column a partition field reads.
pub(super) const SOURCE_ID: &str = "partition-source-id";

/// The Iceberg property naming the spec a partition tuple belongs to.
pub(super) const SPEC_ID: &str = "spec-id";

/// How a source column value becomes a partition value.
///
/// The specification's transforms and five of this crate's own, each
/// spelled by its name - `qhour` for a quarter hour - with the plural the
/// Spark DDL writes read as an alias; a reader that does not know the five
/// reads them as `unknown` and prunes nothing by them, which is what the
/// specification says of an unknown transform.
///
/// ```
/// use yggdryl::iceberg::Transform;
///
/// # fn main() -> yggdryl::Result<()> {
/// assert_eq!(Transform::from_str("bucket[16]")?, Transform::Bucket(16));
/// assert_eq!(Transform::Bucket(16).to_string(), "bucket[16]");
/// assert_eq!(Transform::from_str("qhours")?, Transform::QuarterHour);
/// assert_eq!(Transform::QuarterHour.to_string(), "qhour");
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
    /// Minutes since 1970-01-01T00:00, from a timestamp; `minute`.
    Minute,
    /// Quarter hours since the epoch, from a timestamp; `qhour`.
    QuarterHour,
    /// Half hours since the epoch, from a timestamp; `hhour`.
    HalfHour,
    /// Monday-start weeks since Monday 1969-12-29, from a date or
    /// timestamp; `week`.
    Week,
    /// Quarters since 1970-Q1, from a date or timestamp; `quarter`.
    Quarter,
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

    /// The grammar function spelling this transform, for the nine that one
    /// spells: `years(x)` for `year` through `minutes(x)` for `minute`.
    ///
    /// `Identity`, `Bucket`, `Truncate`, `Void` and `Unknown` have none.
    ///
    /// ```
    /// use yggdryl::expression::Function;
    /// use yggdryl::iceberg::Transform;
    ///
    /// assert_eq!(Transform::QuarterHour.function(), Some(Function::QuarterHours));
    /// assert_eq!(Transform::from_function(&Function::Weeks), Some(Transform::Week));
    /// assert_eq!(Transform::Bucket(4).function(), None);
    /// assert_eq!(Transform::from_function(&Function::Year), None);
    /// ```
    pub const fn function(self) -> Option<Function> {
        Some(match self {
            Self::Year => Function::Years,
            Self::Month => Function::Months,
            Self::Day => Function::Days,
            Self::Hour => Function::Hours,
            Self::Minute => Function::Minutes,
            Self::QuarterHour => Function::QuarterHours,
            Self::HalfHour => Function::HalfHours,
            Self::Week => Function::Weeks,
            Self::Quarter => Function::Quarters,
            Self::Identity | Self::Bucket(_) | Self::Truncate(_) | Self::Void | Self::Unknown => {
                return None;
            }
        })
    }

    /// The transform a grammar function spells, the inverse of
    /// [`Self::function`]: the nine epoch functions and no other.
    pub const fn from_function(function: &Function) -> Option<Self> {
        Some(match function {
            Function::Years => Self::Year,
            Function::Months => Self::Month,
            Function::Days => Self::Day,
            Function::Hours => Self::Hour,
            Function::Minutes => Self::Minute,
            Function::QuarterHours => Self::QuarterHour,
            Function::HalfHours => Self::HalfHour,
            Function::Weeks => Self::Week,
            Function::Quarters => Self::Quarter,
            _ => return None,
        })
    }

    /// The period a time transform floors its source to, for the nine.
    pub(super) const fn epoch_period(self) -> Option<EpochPeriod> {
        Some(match self {
            Self::Year => EpochPeriod::Year,
            Self::Month => EpochPeriod::Month,
            Self::Day => EpochPeriod::Day,
            Self::Hour => EpochPeriod::Hour,
            Self::Minute => EpochPeriod::Minute,
            Self::QuarterHour => EpochPeriod::QuarterHour,
            Self::HalfHour => EpochPeriod::HalfHour,
            Self::Week => EpochPeriod::Week,
            Self::Quarter => EpochPeriod::Quarter,
            Self::Identity | Self::Bucket(_) | Self::Truncate(_) | Self::Void | Self::Unknown => {
                return None;
            }
        })
    }

    /// Whether this is one of the five transforms the official model has no
    /// spelling for, which cross it as a reserved bucket count.
    pub(super) const fn is_bridged(self) -> bool {
        matches!(
            self,
            Self::Minute | Self::QuarterHour | Self::HalfHour | Self::Week | Self::Quarter
        )
    }

    /// The official transform this one crosses the Apache model as.
    ///
    /// The standard transforms are themselves; the five of this crate's own
    /// are each a bucket of a reserved count (see the module documentation).
    pub(super) const fn into_official(self) -> OfficialTransform {
        match self {
            Self::Identity => OfficialTransform::Identity,
            Self::Bucket(count) => OfficialTransform::Bucket(count),
            Self::Truncate(width) => OfficialTransform::Truncate(width),
            Self::Year => OfficialTransform::Year,
            Self::Month => OfficialTransform::Month,
            Self::Day => OfficialTransform::Day,
            Self::Hour => OfficialTransform::Hour,
            Self::Minute => OfficialTransform::Bucket(BRIDGE_BUCKET_BASE + 1),
            Self::QuarterHour => OfficialTransform::Bucket(BRIDGE_BUCKET_BASE + 2),
            Self::HalfHour => OfficialTransform::Bucket(BRIDGE_BUCKET_BASE + 3),
            Self::Week => OfficialTransform::Bucket(BRIDGE_BUCKET_BASE + 4),
            Self::Quarter => OfficialTransform::Bucket(BRIDGE_BUCKET_BASE + 5),
            Self::Void => OfficialTransform::Void,
            Self::Unknown => OfficialTransform::Unknown,
        }
    }

    /// The transform an official one spells, the inverse of
    /// [`Self::into_official`]: a reserved bucket count reads as the
    /// transform it carried.
    pub(super) const fn from_official(transform: OfficialTransform) -> Self {
        match transform {
            OfficialTransform::Identity => Self::Identity,
            OfficialTransform::Bucket(count) if count == BRIDGE_BUCKET_BASE + 1 => Self::Minute,
            OfficialTransform::Bucket(count) if count == BRIDGE_BUCKET_BASE + 2 => {
                Self::QuarterHour
            }
            OfficialTransform::Bucket(count) if count == BRIDGE_BUCKET_BASE + 3 => Self::HalfHour,
            OfficialTransform::Bucket(count) if count == BRIDGE_BUCKET_BASE + 4 => Self::Week,
            OfficialTransform::Bucket(count) if count == BRIDGE_BUCKET_BASE + 5 => Self::Quarter,
            OfficialTransform::Bucket(count) => Self::Bucket(count),
            OfficialTransform::Truncate(width) => Self::Truncate(width),
            OfficialTransform::Year => Self::Year,
            OfficialTransform::Month => Self::Month,
            OfficialTransform::Day => Self::Day,
            OfficialTransform::Hour => Self::Hour,
            OfficialTransform::Void => Self::Void,
            OfficialTransform::Unknown => Self::Unknown,
        }
    }

    /// Return the datatype a partition value has, given its source column.
    ///
    /// # Errors
    ///
    /// Returns an error when the transform cannot apply to the source type:
    /// a sub-day period needs a timestamp, a week or a quarter a date or a
    /// timestamp, and the standard transforms what Apache Iceberg accepts.
    pub fn result_type(self, source: &DataType) -> Result<DataType> {
        self.validate_parameter()?;
        if self == Self::Void {
            return Ok(source.clone());
        }
        if self == Self::Unknown {
            return Ok(DataType::utf8());
        }
        if self.is_bridged() {
            let period = self
                .epoch_period()
                .expect("a bridged transform is a period");
            let accepted = match source {
                DataType::DateTime64 { .. } => true,
                DataType::Date32 => period.takes_date(),
                _ => false,
            };
            if !accepted {
                return Err(invalid(format_smolstr!(
                    "expected {} as the source of the {self} transform, got {source}",
                    if period.takes_date() {
                        "a date or a timestamp"
                    } else {
                        "a timestamp"
                    }
                )));
            }
            return Ok(DataType::Int32);
        }

        let transform = self.into_official();
        let input = OfficialType::Primitive(official_primitive_type(source)?);
        transform.result_type(&input).map_err(Error::from_iceberg)?;

        Ok(match self {
            Self::Identity | Self::Truncate(_) => source.clone(),
            Self::Day => DataType::date32(),
            Self::Bucket(_) | Self::Year | Self::Month | Self::Hour => DataType::Int32,
            Self::Minute
            | Self::QuarterHour
            | Self::HalfHour
            | Self::Week
            | Self::Quarter
            | Self::Void
            | Self::Unknown => unreachable!("returned above"),
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
            "year" | "years" => return Ok(Self::Year),
            "month" | "months" => return Ok(Self::Month),
            "day" | "days" => return Ok(Self::Day),
            "hour" | "hours" => return Ok(Self::Hour),
            "minute" | "minutes" => return Ok(Self::Minute),
            "qhour" | "qhours" | "quarter_hour" => return Ok(Self::QuarterHour),
            "hhour" | "hhours" | "half_hour" => return Ok(Self::HalfHour),
            "week" | "weeks" => return Ok(Self::Week),
            "quarter" | "quarters" => return Ok(Self::Quarter),
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
                 day, hour, minute, qhour, hhour, week, quarter, void, unknown), got {trimmed:?}"
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
            Self::Minute => formatter.write_str("minute"),
            Self::QuarterHour => formatter.write_str("qhour"),
            Self::HalfHour => formatter.write_str("hhour"),
            Self::Week => formatter.write_str("week"),
            Self::Quarter => formatter.write_str("quarter"),
            Self::Void => formatter.write_str("void"),
            Self::Unknown => formatter.write_str("unknown"),
        }
    }
}

/// The bucket count above which the official model carries the five
/// transforms it has no spelling for: `i32::MAX + 1`, so the count after it
/// stands for `minute`, then `qhour`, `hhour`, `week` and `quarter`. A real
/// bucket count is at most `i32::MAX` ([`Transform::result_type`] refuses a
/// larger one), so no table can state a reserved one.
const BRIDGE_BUCKET_BASE: u32 = 1 << 31;

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
        crate::hashing::stable_hash_of(self)
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
        crate::hashing::stable_hash_of(self)
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
            let source = schema.dtype().get_field_by_name(column).ok_or_else(|| {
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
    pub fn from_partition_field(partition: &Field) -> Result<Self> {
        partition.require_struct()?;
        let spec_id = partition.as_iceberg().spec_id()?.unwrap_or_default();
        let mut fields = Vec::with_capacity(partition.field_len());
        for child in partition.fields() {
            let name = child.name();
            let field_id = child.parquet_field_id()?.ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected a PARQUET:field_id on the partition field {name:?}, got none"
                ))
            })?;
            let source_id = child.as_iceberg().partition_source_id()?.ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected an iceberg:{SOURCE_ID} on the partition field {name:?}"
                ))
            })?;
            let transform = child
                .as_iceberg()
                .transform()?
                .unwrap_or(Transform::Identity);
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
            if schema.dtype().get_field_by_name(source.name()).is_some() {
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
            let expected = field.transform.result_type(source.dtype())?;
            if result.dtype() != &expected {
                return Err(invalid(format_smolstr!(
                    "expected partition field {:?} to have type {expected}, got {}",
                    field.name,
                    result.dtype()
                )));
            }
            // A bucket and a truncation are the official scalar functions;
            // every time transform floors its count here.
            let function = match field.transform {
                transform @ (Transform::Bucket(_) | Transform::Truncate(_)) => Some(
                    create_transform_function(&transform.into_official())
                        .map_err(Error::from_iceberg)?,
                ),
                _ => None,
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
    /// the tuple describes itself and [`Self::from_partition_field`] reads this spec back
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
            let dtype = field.transform.result_type(source.dtype())?;
            // A partition value is nullable even when its source is not: a
            // spec can retire a field, and `void` produces nothing but null.
            let mut child = Field::new(field.name.as_str(), dtype, true);
            child.set_parquet_field_id(field.field_id);
            child.set_partition(true);
            child
                .as_iceberg_mut()
                .set_partition_source_id(field.source_id)?;
            child.as_iceberg_mut().set_transform(&field.transform)?;
            children.push(child);
        }
        let mut partition = Field::new(
            "partition",
            DataType::from(StructType::from_fields(children)?),
            false,
        );
        partition.as_iceberg_mut().set_spec_id(self.spec_id)?;
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
        if let Some(entries) = document.as_serie() {
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
                    &entry,
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
            .and_then(Scalar::as_serie)
            .ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected a \"fields\" array in partition spec {spec_id}"
                ))
            })?;
        let mut fields = Vec::with_capacity(entries.len());
        for entry in entries.iter() {
            fields.push(PartitionField::from_json(&entry)?);
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

    /// A column whose equal values always compute one partition value.
    ///
    /// A write groups a batch's rows by these keys and evaluates
    /// [`Self::partition_value`] once per distinct key rather than once per
    /// row, which is only the same partition when equal keys can never compute
    /// two values. The source column itself has that property for every
    /// transform. A timestamp under a time transform has a coarser one - the
    /// period itself for a fixed-length one (`minute`, `qhour`, `hhour`,
    /// `hour`, `day`), the UTC day for the calendar ones (`week`, `month`,
    /// `quarter`, `year`), which are not aligned to the epoch - because every
    /// instant of one day computes one week, month, quarter and year, so a
    /// column of distinct instants still keys a handful of groups. `void`
    /// computes null whatever the row holds, so it keys nothing.
    ///
    /// # Errors
    ///
    /// Returns an error when a timestamp column cannot be read as its count.
    pub(super) fn grouping_key(
        &self,
        source: &arrow_array::ArrayRef,
    ) -> Result<Option<arrow_array::ArrayRef>> {
        use arrow_array::cast::AsArray;
        use arrow_array::types::{
            Int64Type, TimestampMicrosecondType, TimestampMillisecondType, TimestampNanosecondType,
            TimestampSecondType,
        };
        use arrow_schema::TimeUnit as ArrowTimeUnit;

        let (seconds, unit) = match (
            self.transform.epoch_period(),
            self.transform,
            source.data_type(),
        ) {
            (_, Transform::Void, _) => return Ok(None),
            (Some(period), _, arrow_schema::DataType::Timestamp(unit, _)) => {
                (period.seconds().unwrap_or(86_400), *unit)
            }
            _ => return Ok(Some(std::sync::Arc::clone(source))),
        };
        let step = crate::temporal::per_second(crate::TimeUnit::from_arrow_time(unit)).unwrap_or(1)
            * seconds;
        // A timestamp is its count, read straight off its values buffer;
        // flooring keeps an instant before the epoch in its own period.
        macro_rules! floored {
            ($unit:ty) => {
                source
                    .as_primitive::<$unit>()
                    .unary::<_, Int64Type>(|count| count.div_euclid(step))
            };
        }
        let keys: arrow_array::Int64Array = match unit {
            ArrowTimeUnit::Second => floored!(TimestampSecondType),
            ArrowTimeUnit::Millisecond => floored!(TimestampMillisecondType),
            ArrowTimeUnit::Microsecond => floored!(TimestampMicrosecondType),
            ArrowTimeUnit::Nanosecond => floored!(TimestampNanosecondType),
        };
        Ok(Some(std::sync::Arc::new(keys)))
    }

    /// Compute one partition value: a time transform through the crate's own
    /// period arithmetic, a bucket or a truncation through Apache Iceberg's
    /// scalar transform.
    pub(super) fn partition_value(&self, value: Scalar) -> Result<Scalar> {
        if value.is_null() || self.transform == Transform::Void {
            return Ok(Scalar::Null);
        }
        if self.transform == Transform::Identity {
            return Ok(value);
        }

        // Every time transform floors its source's count to the UTC period,
        // as the specification and the Java implementation do. iceberg-rust
        // 0.10 truncates a count before the epoch toward zero first, which
        // moves the last second of a day before 1970 into the next day - and
        // a write keys every instant of one UTC day together, so one row
        // would label the whole day. Flooring here is also total over the
        // i32 day range the official literal path unwraps a calendar on.
        if let Some(period) = self.transform.epoch_period() {
            let value = match &value {
                Scalar::Date32(_) | Scalar::DateTime64(_) => epoch_value(period, &value),
                _ => {
                    let count = super::value::single_value(&value, self.source.dtype())
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                        .map(i64::from_le_bytes);
                    match (count, self.source.dtype()) {
                        (Some(count), DataType::DateTime64 { unit, timezone }) => {
                            epoch_value(period, &Scalar::datetime64(count, *unit, *timezone)?)
                        }
                        _ => {
                            return Err(invalid(format_smolstr!(
                                "expected a date or timestamp scalar for the {} transform, got {}",
                                self.transform,
                                value.kind()
                            )));
                        }
                    }
                }
            };
            if value.is_null() {
                return Err(invalid(format_smolstr!(
                    "expected the {} transform of a {} value to fit int32",
                    self.transform,
                    self.source.dtype()
                )));
            }
            return Ok(value);
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

        let datum = official_datum(&value, self.source.dtype())?;
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
        scalar_from_official(&transformed, self.result.dtype())
    }
}

fn official_primitive_type(dtype: &DataType) -> Result<OfficialPrimitiveType> {
    Ok(match super::PrimitiveType::from_dtype(dtype)? {
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
        super::PrimitiveType::Fixed(width) => OfficialPrimitiveType::Fixed(u64::from(width)),
        super::PrimitiveType::Binary => OfficialPrimitiveType::Binary,
        primitive @ (super::PrimitiveType::Unknown | super::PrimitiveType::Variant) => {
            return Err(invalid(format_smolstr!(
                "expected a concrete Iceberg partition source type, got {primitive}"
            )));
        }
    })
}

fn official_datum(value: &Scalar, dtype: &DataType) -> Result<OfficialDatum> {
    let primitive = official_primitive_type(dtype)?;
    let bytes = if let DataType::Decimal32 { scale, .. }
    | DataType::Decimal64 { scale, .. }
    | DataType::Decimal128 { scale, .. } = dtype
    {
        let (unscaled, actual_scale) = value.as_decimal().ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a decimal scalar at scale {scale}, got {}",
                value.kind()
            ))
        })?;
        if actual_scale != *scale {
            return Err(invalid(format_smolstr!(
                "expected a decimal scalar at scale {scale}, got scale {actual_scale}"
            )));
        }
        unscaled
            .as_i128()
            .ok_or_else(|| invalid(SmolStr::new_static("decimal coefficient exceeds i128")))?
            .to_be_bytes()
            .to_vec()
    } else {
        super::value::single_value(value, dtype).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a scalar compatible with Iceberg type {dtype}, got {}",
                value.kind()
            ))
        })?
    };
    OfficialDatum::try_from_bytes(&bytes, primitive).map_err(Error::from_iceberg)
}

fn scalar_from_official(value: &OfficialDatum, dtype: &DataType) -> Result<Scalar> {
    if let DataType::Decimal32 { scale, .. }
    | DataType::Decimal64 { scale, .. }
    | DataType::Decimal128 { scale, .. } = dtype
    {
        return match value.literal() {
            OfficialLiteral::Int128(unscaled) => {
                dtype.scalar(Scalar::decimal128(*unscaled, *scale))
            }
            literal => Err(invalid(format_smolstr!(
                "expected an Iceberg decimal partition value, got {literal:?}"
            ))),
        };
    }

    let bytes = value.to_bytes().map_err(Error::from_iceberg)?;
    super::value::single_to_value(bytes.as_ref(), dtype).ok_or_else(|| {
        invalid(format_smolstr!(
            "expected an Iceberg partition value of type {dtype}, got {value}"
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
pub(super) fn source_path(schema: &Field, source_id: i32) -> Result<(Vec<SmolStr>, &Field)> {
    fn find<'a>(field: &'a Field, source_id: i32, path: &mut Vec<SmolStr>) -> Option<&'a Field> {
        for child in field.fields() {
            path.push(SmolStr::new(child.name()));
            if child.parquet_field_id().ok().flatten() == Some(source_id) {
                return Some(child);
            }
            if matches!(child.dtype(), DataType::Struct(_))
                && let Some(found) = find(child, source_id, path)
            {
                return Some(found);
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

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/iceberg/mod_.rs` pins and a caller cannot reach.
    //!
    //! A caller hands rows to a commit and reads directories back; the plan
    //! that turns one column value into one partition value is resolved
    //! inside. [`PartitionTransform`] here is a forwarding wrapper, so nothing
    //! in `iceberg::partition` changes visibility.

    use super::PartitionSpec;
    use crate::{Field, Result, Scalar};

    /// One validated scalar transform of a write plan, forwarding to the real
    /// one.
    pub struct PartitionTransform(super::PartitionTransform);

    impl PartitionTransform {
        /// Compute one partition value through the official transform.
        pub fn partition_value(&self, value: Scalar) -> Result<Scalar> {
            self.0.partition_value(value)
        }
    }

    /// The typed scalar transform plan one data write runs with.
    pub fn write_transforms(
        spec: &PartitionSpec,
        schema: &Field,
        partition: &Field,
    ) -> Result<Vec<PartitionTransform>> {
        Ok(spec
            .write_transforms(schema, partition)?
            .into_iter()
            .map(PartitionTransform)
            .collect())
    }
}
