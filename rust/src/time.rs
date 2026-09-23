//! The time family: one time of day, held at one resolution in one of two
//! widths.
//!
//! Arrow lays a clock reading out as `Time32` at seconds or milliseconds and
//! `Time64` at microseconds or nanoseconds, so the width follows from the
//! resolution and a caller who states only the resolution gets the width it
//! fits in. [`TimeType`] names the two leaves, each carrying its unit as a
//! parameter; `DataType::Time32` and `DataType::Time64` are the time datatypes; [`Time32`]
//! and [`Time64`] are the values.
//!
//! ```
//! use yggdryl::TimeType;
//! use yggdryl::{DataType, Scalar, TimeUnit, Timezone};
//!
//! # fn main() -> yggdryl::Result<()> {
//! // The resolution picks the width, and the spelling reads back as itself.
//! assert_eq!(DataType::time(TimeUnit::Second)?, DataType::time32(TimeUnit::Second)?);
//! assert_eq!(DataType::from_str("time")?, DataType::time64(TimeUnit::Microsecond)?);
//! assert_eq!(DataType::time32(TimeUnit::Millisecond)?.to_string(), "time32(ms)");
//!
//! // A width refuses a resolution it does not carry.
//! assert!(DataType::time32(TimeUnit::Nanosecond).is_err());
//!
//! // The leaf is what a time column declares.
//! let leaf = DataType::time64(TimeUnit::Nanosecond)?.time_type().expect("a time datatype");
//! assert_eq!(leaf, TimeType::Time64(TimeUnit::Nanosecond));
//! assert_eq!(leaf.unit(), TimeUnit::Nanosecond);
//!
//! // A value follows the same rule.
//! let noon = Scalar::from_time(43_200, TimeUnit::Second, Timezone::NAIVE)?;
//! assert_eq!(noon.as_time32().map(|(count, ..)| count), Some(43_200));
//! # Ok(())
//! # }
//! ```

use std::fmt;

pub(crate) use arrow::{arrow_storage, from_arrow_storage};
use smol_str::format_smolstr;

use crate::invalid;
use crate::parser::{Parser, precision_to_unit};
use crate::temporal::scalars::{narrow_i32, require};
use crate::temporal::{
    TemporalKind, invalid_record, temporal_leaf, validate_time32_unit, validate_time64_unit,
};
use crate::value::DataTypeValue;
use crate::{DataType, DataTypeId, DataTypeKind, Error, Result, Scalar, TimeUnit, Timezone};

// ------------------------------------------------------------------------
// The time payload: two widths, each over the resolutions it carries.
// ------------------------------------------------------------------------

/// The time family's datatype payload: one leaf per width, each carrying
/// the resolution its column counts in.
///
/// The unit is a parameter and never a leaf: `time32(s)` and
/// `time32(ms)` are one storage with one parameter, exactly as the
/// two scales of a `decimal128` are. Which width a unit belongs to is
/// [`Self::for_unit`]'s one rule.
///
/// ```
/// use yggdryl::TimeType;
/// use yggdryl::{DataTypeId, TimeUnit};
///
/// # fn main() -> yggdryl::Result<()> {
/// assert_eq!(TimeType::for_unit(TimeUnit::Millisecond)?, TimeType::Time32(TimeUnit::Millisecond));
/// assert_eq!(TimeType::for_unit(TimeUnit::Nanosecond)?, TimeType::Time64(TimeUnit::Nanosecond));
/// assert!(TimeType::for_unit(TimeUnit::Day).is_err());
/// assert_eq!(TimeType::Time32(TimeUnit::Second).id(), DataTypeId::Time32);
/// assert_eq!(
///     TimeType::from_id(DataTypeId::Time64, TimeUnit::Microsecond),
///     Some(TimeType::Time64(TimeUnit::Microsecond))
/// );
/// // The leaf is public, so a width can carry a unit it does not hold; the
/// // check is what a constructor and a boundary both run.
/// assert!(TimeType::Time32(TimeUnit::Nanosecond).validate().is_err());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum TimeType {
    /// 32-bit time of day; seconds and milliseconds are valid.
    Time32(TimeUnit),
    /// 64-bit time of day; microseconds and nanoseconds are valid.
    Time64(TimeUnit),
}

impl TimeType {
    /// Every leaf in identifier order, each at the resolution the grammar
    /// and the bindings default it to.
    pub const ALL: [Self; 2] = [
        Self::Time32(TimeUnit::Millisecond),
        Self::Time64(TimeUnit::Microsecond),
    ];

    /// Return the exact datatype identifier.
    #[must_use]
    pub const fn id(self) -> DataTypeId {
        match self {
            Self::Time32(_) => DataTypeId::Time32,
            Self::Time64(_) => DataTypeId::Time64,
        }
    }

    /// The leaf one identifier names at `unit`, or `None` for an identifier
    /// of another family. The unit is carried, not checked.
    #[must_use]
    pub const fn from_id(id: DataTypeId, unit: TimeUnit) -> Option<Self> {
        match id {
            DataTypeId::Time32 => Some(Self::Time32(unit)),
            DataTypeId::Time64 => Some(Self::Time64(unit)),
            _ => None,
        }
    }

    /// The canonical name of this leaf.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.id().as_str()
    }

    /// The resolution this column counts in, whichever width holds it.
    #[must_use]
    pub const fn unit(self) -> TimeUnit {
        match self {
            Self::Time32(unit) | Self::Time64(unit) => unit,
        }
    }

    /// The physical count width in bits.
    #[must_use]
    pub const fn bit_width(self) -> u8 {
        match self {
            Self::Time32(_) => 32,
            Self::Time64(_) => 64,
        }
    }

    /// The family's name, `time`, as a datatype spells it.
    #[must_use]
    pub const fn family(self) -> &'static str {
        TemporalKind::Time.as_str()
    }

    /// The width a resolution fits in: seconds and milliseconds in 32 bits,
    /// microseconds and nanoseconds in 64.
    ///
    /// This is the one rule behind [`DataType::time`]: a caller who states
    /// only the resolution gets the width Arrow holds it at.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a day or an interval layout,
    /// which are not resolutions of a clock.
    pub fn for_unit(unit: TimeUnit) -> Result<Self> {
        match unit {
            TimeUnit::Second | TimeUnit::Millisecond => Ok(Self::Time32(unit)),
            TimeUnit::Microsecond | TimeUnit::Nanosecond => Ok(Self::Time64(unit)),
            TimeUnit::Day | TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => {
                Err(invalid("Time", "unit must be a temporal resolution"))
            }
        }
    }

    /// Reject a width carrying a resolution it does not hold.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] naming the width and the
    /// resolutions it does carry.
    pub fn validate(self) -> Result<()> {
        match self {
            Self::Time32(unit) => validate_time32_unit(unit),
            Self::Time64(unit) => validate_time64_unit(unit),
        }
    }
}

impl DataTypeValue for TimeType {
    const FAMILY: &'static str = "time";

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
        match self {
            Self::Time32(unit) => DataType::Time32(unit),
            Self::Time64(unit) => DataType::Time64(unit),
        }
    }

    fn from_dtype(dtype: &DataType) -> Option<Self> {
        dtype.time_type()
    }
}

impl fmt::Display for TimeType {
    /// The canonical spelling, `time32(ms)`, which
    /// [`crate::DataType`]'s grammar reads back.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}({})", self.as_str(), self.unit())
    }
}

impl From<TimeType> for DataType {
    fn from(value: TimeType) -> Self {
        DataTypeValue::into_dtype(value)
    }
}

impl TryFrom<&DataType> for TimeType {
    type Error = Error;

    fn try_from(value: &DataType) -> Result<Self> {
        match value.time_type() {
            Some(leaf) => Ok(leaf),
            None => Err(Error::InvalidDataType {
                kind: "time",
                reason: format_smolstr!("expected a time datatype, got {value}"),
            }),
        }
    }
}

// ------------------------------------------------------------------------
// The doors into the time family.
// ------------------------------------------------------------------------

impl DataType {
    /// The time-of-day datatype at the width `unit` fits in.
    ///
    /// Seconds and milliseconds land in [`TimeType::Time32`], microseconds
    /// and nanoseconds in [`TimeType::Time64`]; [`TimeType::for_unit`] is the
    /// rule.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a day or an interval layout.
    pub fn time(unit: TimeUnit) -> Result<Self> {
        Ok(TimeType::for_unit(unit)?.into())
    }

    /// A 32-bit time of day at `unit`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a unit other than seconds or
    /// milliseconds.
    pub fn time32(unit: TimeUnit) -> Result<Self> {
        Self::time_of(TimeType::Time32(unit))
    }

    /// A 64-bit time of day at `unit`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a unit other than microseconds
    /// or nanoseconds.
    pub fn time64(unit: TimeUnit) -> Result<Self> {
        Self::time_of(TimeType::Time64(unit))
    }

    /// One checked time leaf, whichever width names it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] when the width does not carry the
    /// unit.
    pub fn time_of(leaf: TimeType) -> Result<Self> {
        leaf.validate()?;
        Ok(DataTypeValue::into_dtype(leaf))
    }

    /// The leaf a time datatype declares, `None` for every other.
    #[must_use]
    pub const fn time_type(&self) -> Option<TimeType> {
        match self {
            Self::Time32(unit) => Some(TimeType::Time32(*unit)),
            Self::Time64(unit) => Some(TimeType::Time64(*unit)),
            _ => None,
        }
    }
}

// ------------------------------------------------------------------------
// The grammar of the resolution-only spelling.
// ------------------------------------------------------------------------

impl Parser<'_> {
    /// Parse SQL's `time`, `time(p)` or `time(unit)` into the width the
    /// resolution fits in; bare `time` is microseconds.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Parse`] for a precision past nine, a unit no
    /// name spells, or one that is not a clock resolution.
    pub(crate) fn parse_sql_time(&mut self, depth: usize) -> Result<DataType> {
        self.check_depth(depth)?;
        if self.peek_opening().is_none() {
            return DataType::time(TimeUnit::Microsecond);
        }
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected time unit"))?;
        let (unit, unit_start) = if let Some(precision) = self.peek_integer() {
            let start = self.current_position();
            self.index += 1;
            (precision_to_unit(precision, start)?, start)
        } else {
            self.parse_time_unit_span(Some(close), "time unit")?
        };
        self.expect_symbol(close)?;
        DataType::time(unit).map_err(|error| self.error_at(unit_start, format_smolstr!("{error}")))
    }
}

// ------------------------------------------------------------------------
// Arrow projection: the two clock storages.
// ------------------------------------------------------------------------

mod arrow {
    use arrow_schema::DataType as ArrowDataType;
    use smol_str::format_smolstr;

    use super::TimeType;
    use crate::invalid;
    use crate::{DataType, Result, TimeUnit};

    /// The Arrow storage one time datatype lays out.
    ///
    /// A unit Arrow cannot state is refused here rather than silently
    /// widened: the leaf is public, so `time32(nanosecond)` can reach this
    /// boundary and this is where it stops.
    ///
    /// # Errors
    ///
    /// Returns an error when the unit is not one the width carries, or when
    /// the datatype belongs to another family.
    pub(crate) fn arrow_storage(dtype: &DataType) -> Result<ArrowDataType> {
        let Some(leaf) = dtype.time_type() else {
            return Err(invalid(
                "time",
                format_smolstr!("expected a time datatype, got {dtype}"),
            ));
        };
        leaf.validate()?;
        Ok(match leaf {
            TimeType::Time32(unit) => ArrowDataType::Time32(unit.into_arrow_time()?),
            TimeType::Time64(unit) => ArrowDataType::Time64(unit.into_arrow_time()?),
        })
    }

    /// The time datatype one Arrow storage imports as.
    ///
    /// # Errors
    ///
    /// Returns an error when the unit is not one the width carries, or when
    /// the storage belongs to another family.
    pub(crate) fn from_arrow_storage(value: &ArrowDataType) -> Result<DataType> {
        match value {
            ArrowDataType::Time32(unit) => DataType::time32(TimeUnit::from_arrow_time(*unit)),
            ArrowDataType::Time64(unit) => DataType::time64(TimeUnit::from_arrow_time(*unit)),
            other => Err(invalid(
                "time",
                format_smolstr!("expected a time storage, got {other}"),
            )),
        }
    }
}

// ------------------------------------------------------------------------
// The time values: a count since midnight at one resolution.
// ------------------------------------------------------------------------

temporal_leaf!(
    Time32,
    i32,
    Time,
    32,
    valid = |unit: TimeUnit, timezone: Timezone| matches!(
        unit,
        TimeUnit::Second | TimeUnit::Millisecond
    ) && timezone.is_naive(),
    dtype = |value: &Time32| DataType::time32(value.unit()),
    "Time32 requires second or millisecond units and the NAIVE timezone",
);
temporal_leaf!(
    Time64,
    i64,
    Time,
    64,
    valid = |unit: TimeUnit, timezone: Timezone| matches!(
        unit,
        TimeUnit::Microsecond | TimeUnit::Nanosecond
    ) && timezone.is_naive(),
    dtype = |value: &Time64| DataType::time64(value.unit()),
    "Time64 requires microsecond or nanosecond units and the NAIVE timezone",
);

impl Scalar {
    /// Build the exact time-of-day width selected by its unit.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a unit no clock counts in, for a
    /// 32-bit count that does not fit, and for a zone other than
    /// [`Timezone::NAIVE`].
    pub fn from_time(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        match unit {
            TimeUnit::Second | TimeUnit::Millisecond => {
                Self::time32(narrow_i32(count, "time32")?, unit, zone)
            }
            TimeUnit::Microsecond | TimeUnit::Nanosecond => Self::time64(count, unit, zone),
            _ => Err(invalid_record(
                "time unit must be second, millisecond, microsecond, or nanosecond",
            )),
        }
    }

    /// Build a 32-bit time of day.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a unit other than seconds or
    /// milliseconds, or a zone other than [`Timezone::NAIVE`].
    pub fn time32(count: i32, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            matches!(unit, TimeUnit::Second | TimeUnit::Millisecond),
            "time32 unit must be second or millisecond",
        )?;
        require(
            zone.is_naive(),
            "time32 timezone must be NAIVE because its datatype has no timezone",
        )?;
        Time32::new(count, unit, zone).map(Self::Time32)
    }

    /// Build a 64-bit time of day.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a unit other than microseconds
    /// or nanoseconds, or a zone other than [`Timezone::NAIVE`].
    pub fn time64(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            matches!(unit, TimeUnit::Microsecond | TimeUnit::Nanosecond),
            "time64 unit must be microsecond or nanosecond",
        )?;
        require(
            zone.is_naive(),
            "time64 timezone must be NAIVE because its datatype has no timezone",
        )?;
        Time64::new(count, unit, zone).map(Self::Time64)
    }

    /// Return Time32's count, unit, and zone.
    #[must_use]
    pub const fn as_time32(&self) -> Option<(i32, TimeUnit, &Timezone)> {
        match self {
            Self::Time32(value) => Some((value.count(), value.unit(), &value.timezone)),
            _ => None,
        }
    }

    /// Return Time64's count, unit, and zone.
    #[must_use]
    pub const fn as_time64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
        match self {
            Self::Time64(value) => Some((value.count(), value.unit(), &value.timezone)),
            _ => None,
        }
    }
}
