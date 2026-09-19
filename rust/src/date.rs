//! The date family: one calendar day, held as a day count or as the
//! milliseconds of its midnight.
//!
//! Arrow lays a date out two ways - `Date32`, days since the epoch, and
//! `Date64`, milliseconds since the epoch that always name a midnight - and
//! both are one thing to a reader: a day. [`DateType`] names the two leaves,
//! [`crate::DataType::Date`] is the one date datatype, and [`Date32`] and
//! [`Date64`] are the values, each carrying the unit its width means so the
//! shared temporal readers never ask the width.
//!
//! ```
//! use yggdryl::DateType;
//! use yggdryl::{DataType, Scalar, TimeUnit, Timezone};
//!
//! # fn main() -> yggdryl::Result<()> {
//! // Every spelling is one leaf, and it reads back as itself.
//! assert_eq!(DataType::from_str("date")?, DataType::date32());
//! assert_eq!(DataType::date64().to_string(), "date64");
//!
//! // The leaf is what a date column declares; the unit follows from it.
//! let leaf = DataType::date64().date_type().expect("a date datatype");
//! assert_eq!(leaf, DateType::Date64);
//! assert_eq!(leaf.unit(), TimeUnit::Millisecond);
//!
//! // A value knows its unit, and the family constructor picks the width.
//! let day = Scalar::from_date(20_000, TimeUnit::Day, Timezone::NAIVE)?;
//! assert_eq!(day.as_date32().map(|(count, ..)| count), Some(20_000));
//! # Ok(())
//! # }
//! ```

use std::fmt;

pub(crate) use arrow::{arrow_storage, from_arrow_storage};
use smol_str::format_smolstr;

use crate::family::DataTypeValue;
use crate::temporal::scalars::{narrow_i32, require};
use crate::temporal::{TemporalFamily, invalid_record, temporal_leaf};
use crate::{DataType, DataTypeId, DataTypeKind, Error, Result, Scalar, TimeUnit, Timezone};

// ------------------------------------------------------------------------
// The date payload: two widths of one calendar day.
// ------------------------------------------------------------------------

/// The date family's datatype payload: one leaf per width a day is held at.
///
/// A date has no parameter: the unit is what the width means, so the leaf
/// says it and [`Self::unit`] reads it back.
///
/// ```
/// use yggdryl::DateType;
/// use yggdryl::{DataTypeId, TimeUnit};
///
/// assert_eq!(DateType::Date32.id(), DataTypeId::Date32);
/// assert_eq!(DateType::Date32.unit(), TimeUnit::Day);
/// assert_eq!(DateType::Date64.bit_width(), 64);
/// assert_eq!(DateType::from_id(DataTypeId::Date64), Some(DateType::Date64));
/// assert_eq!(DateType::from_id(DataTypeId::Time32), None);
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DateType {
    /// Days since the Unix epoch - Arrow's `Date32`.
    #[default]
    Date32,
    /// Milliseconds since the Unix epoch representing whole days - Arrow's
    /// `Date64`.
    Date64,
}

impl DateType {
    /// Every leaf in identifier order.
    pub const ALL: [Self; 2] = [Self::Date32, Self::Date64];

    /// Return the exact datatype identifier.
    #[must_use]
    pub const fn id(self) -> DataTypeId {
        match self {
            Self::Date32 => DataTypeId::Date32,
            Self::Date64 => DataTypeId::Date64,
        }
    }

    /// The leaf one identifier names, or `None` for an identifier of another
    /// family.
    #[must_use]
    pub const fn from_id(id: DataTypeId) -> Option<Self> {
        match id {
            DataTypeId::Date32 => Some(Self::Date32),
            DataTypeId::Date64 => Some(Self::Date64),
            _ => None,
        }
    }

    /// The canonical name of this leaf.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.id().as_str()
    }

    /// The unit the width counts in: a day, or the milliseconds of one.
    #[must_use]
    pub const fn unit(self) -> TimeUnit {
        match self {
            Self::Date32 => TimeUnit::Day,
            Self::Date64 => TimeUnit::Millisecond,
        }
    }

    /// The physical count width in bits.
    #[must_use]
    pub const fn bit_width(self) -> u8 {
        match self {
            Self::Date32 => 32,
            Self::Date64 => 64,
        }
    }

    /// The temporal family every leaf belongs to.
    #[must_use]
    pub const fn family(self) -> TemporalFamily {
        TemporalFamily::Date
    }

    /// A date has no parameter to refuse.
    ///
    /// # Errors
    ///
    /// Never; the signature is the family contract's.
    pub const fn validate(self) -> Result<()> {
        Ok(())
    }
}

impl DataTypeValue for DateType {
    const FAMILY: &'static str = "date";

    type Sidecar = ();

    fn id(&self) -> DataTypeId {
        Self::id(*self)
    }

    fn kind(&self) -> DataTypeKind {
        DataTypeKind::Temporal
    }

    fn validate(&self) -> Result<()> {
        Self::validate(*self)
    }

    fn into_dtype(self) -> DataType {
        DataType::Date(self)
    }

    fn from_dtype(dtype: &DataType) -> Option<Self> {
        match dtype {
            DataType::Date(leaf) => Some(*leaf),
            _ => None,
        }
    }
}

impl fmt::Display for DateType {
    /// The canonical spelling, which [`crate::DataType`]'s grammar reads back.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<DateType> for DataType {
    fn from(value: DateType) -> Self {
        Self::Date(value)
    }
}

impl TryFrom<&DataType> for DateType {
    type Error = Error;

    fn try_from(value: &DataType) -> Result<Self> {
        match value {
            DataType::Date(leaf) => Ok(*leaf),
            other => Err(Error::InvalidDataType {
                kind: "date",
                reason: format_smolstr!("expected a date datatype, got {other}"),
            }),
        }
    }
}

// ------------------------------------------------------------------------
// The doors into the date family.
// ------------------------------------------------------------------------

impl DataType {
    /// Days since the Unix epoch - Arrow's `Date32`.
    #[must_use]
    pub const fn date32() -> Self {
        Self::Date(DateType::Date32)
    }

    /// Milliseconds since the Unix epoch representing whole days - Arrow's
    /// `Date64`.
    #[must_use]
    pub const fn date64() -> Self {
        Self::Date(DateType::Date64)
    }

    /// The leaf a date datatype declares, `None` for every other.
    #[must_use]
    pub const fn date_type(&self) -> Option<DateType> {
        match self {
            Self::Date(leaf) => Some(*leaf),
            _ => None,
        }
    }
}

// ------------------------------------------------------------------------
// Arrow projection: the two date storages are Arrow's own.
// ------------------------------------------------------------------------

mod arrow {
    use arrow_schema::DataType as ArrowDataType;
    use smol_str::format_smolstr;

    use super::DateType;
    use crate::invalid;
    use crate::{DataType, Result};

    /// The Arrow storage one date datatype lays out.
    ///
    /// # Errors
    ///
    /// Returns an error when the datatype belongs to another family.
    pub(crate) fn arrow_storage(dtype: &DataType) -> Result<ArrowDataType> {
        match dtype {
            DataType::Date(DateType::Date32) => Ok(ArrowDataType::Date32),
            DataType::Date(DateType::Date64) => Ok(ArrowDataType::Date64),
            other => Err(invalid(
                "date",
                format_smolstr!("expected a date datatype, got {other}"),
            )),
        }
    }

    /// The date datatype one Arrow storage imports as.
    ///
    /// # Errors
    ///
    /// Returns an error when the storage belongs to another family.
    pub(crate) fn from_arrow_storage(value: &ArrowDataType) -> Result<DataType> {
        match value {
            ArrowDataType::Date32 => Ok(DataType::date32()),
            ArrowDataType::Date64 => Ok(DataType::date64()),
            other => Err(invalid(
                "date",
                format_smolstr!("expected a date storage, got {other}"),
            )),
        }
    }
}

// ------------------------------------------------------------------------
// The date values: a day count, and the milliseconds of a midnight.
// ------------------------------------------------------------------------

temporal_leaf!(
    Date32,
    i32,
    Date,
    32,
    valid = |unit: TimeUnit, timezone: Timezone| unit == TimeUnit::Day && timezone.is_naive(),
    dtype = |_: &Date32| Ok(DataType::date32()),
    "Date32 requires day units and the NAIVE timezone",
);
temporal_leaf!(
    Date64,
    i64,
    Date,
    64,
    valid =
        |unit: TimeUnit, timezone: Timezone| unit == TimeUnit::Millisecond && timezone.is_naive(),
    dtype = |_: &Date64| Ok(DataType::date64()),
    "Date64 requires millisecond units and the NAIVE timezone",
);

impl Scalar {
    /// Build the exact date width selected by its unit.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a unit no date width counts in,
    /// for a day count outside 32 bits, and for a zone other than
    /// [`Timezone::NAIVE`].
    pub fn from_date(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        match unit {
            TimeUnit::Day => Self::date32_in(narrow_i32(count, "date32")?, unit, zone),
            TimeUnit::Millisecond => Self::date64_in(count, unit, zone),
            _ => Err(invalid_record("date unit must be day or millisecond")),
        }
    }

    /// Build a Date32 day count.
    #[must_use]
    pub const fn date32(days: i32) -> Self {
        Self::Date32(Date32 {
            count: days,
            unit: TimeUnit::Day,
            timezone: Timezone::NAIVE,
        })
    }

    /// Build a Date32 after validating its unit and zone.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a unit other than
    /// [`TimeUnit::Day`] or a zone other than [`Timezone::NAIVE`].
    pub fn date32_in(days: i32, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(unit == TimeUnit::Day, "date32 unit must be day")?;
        require(zone.is_naive(), "date32 timezone must be NAIVE")?;
        Date32::new(days, unit, zone).map(Self::Date32)
    }

    /// Build a Date64 millisecond count.
    #[must_use]
    pub const fn date64(milliseconds: i64) -> Self {
        Self::Date64(Date64 {
            count: milliseconds,
            unit: TimeUnit::Millisecond,
            timezone: Timezone::NAIVE,
        })
    }

    /// Build a Date64 after validating its unit and zone.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a unit other than
    /// [`TimeUnit::Millisecond`] or a zone other than [`Timezone::NAIVE`].
    pub fn date64_in(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            unit == TimeUnit::Millisecond,
            "date64 unit must be millisecond",
        )?;
        require(zone.is_naive(), "date64 timezone must be NAIVE")?;
        Date64::new(count, unit, zone).map(Self::Date64)
    }

    /// Return Date32's count, unit, and zone.
    #[must_use]
    pub const fn as_date32(&self) -> Option<(i32, TimeUnit, &Timezone)> {
        match self {
            Self::Date32(value) => Some((value.count(), value.unit(), &value.timezone)),
            _ => None,
        }
    }

    /// Return Date64's count, unit, and zone.
    #[must_use]
    pub const fn as_date64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
        match self {
            Self::Date64(value) => Some((value.count(), value.unit(), &value.timezone)),
            _ => None,
        }
    }
}
