//! The interval family: a calendar span in one of Arrow's three layouts.
//!
//! An interval is months, days and nanoseconds, which no fixed-length unit
//! restates: a month is not a count of days, so an interval has no classic
//! spelling and takes no arithmetic with the other temporals. Arrow lays it
//! out three ways - `YearMonth`, `DayTime`, `MonthDayNano` - and the layout
//! is the one parameter of the one leaf [`IntervalType`] has;
//! [`crate::DataType::Interval`] is the one interval datatype and
//! [`Interval`] is the value, holding all three components so no layout
//! loses one.
//!
//! ```
//! use yggdryl::{Interval, IntervalType};
//! use yggdryl::{DataType, Scalar, TimeUnit};
//!
//! # fn main() -> yggdryl::Result<()> {
//! // The layout is the parameter, and a resolution is refused.
//! assert_eq!(DataType::interval(TimeUnit::DayTime)?.to_string(), "interval(day_time)");
//! assert_eq!(DataType::from_str("interval")?, DataType::interval(TimeUnit::MonthDayNano)?);
//! assert!(DataType::interval(TimeUnit::Second).is_err());
//!
//! let leaf = DataType::interval(TimeUnit::YearMonth)?.interval_type().expect("an interval datatype");
//! assert_eq!(leaf, IntervalType::Interval(TimeUnit::YearMonth));
//!
//! // A value refuses a component its layout has nowhere to put.
//! let span = Scalar::interval(1, 2, 3_000_000, TimeUnit::MonthDayNano)?;
//! assert_eq!(span.temporal_unit(), Some(TimeUnit::MonthDayNano));
//! assert!(Interval::new(1, 2, 0, TimeUnit::YearMonth).is_err());
//! # Ok(())
//! # }
//! ```

use std::fmt;

pub(crate) use arrow::{arrow_storage, from_arrow_storage};
use serde::{Deserialize, Serialize};
use smol_str::format_smolstr;

use crate::invalid;
use crate::parser::Parser;
use crate::temporal::invalid_record;
use crate::value::DataTypeValue;
use crate::value::TemporalValue;
use crate::{DataType, DataTypeId, Error, Result, Scalar, TimeUnit, Timezone, Value};

// ------------------------------------------------------------------------
// The interval payload: one leaf, three layouts.
// ------------------------------------------------------------------------

/// The interval family's datatype payload: one leaf, carrying the layout
/// its column stores.
///
/// The three layouts stay the [`TimeUnit`] parameter they are today rather
/// than leaves of their own; one leaf is the shape every family has.
///
/// ```
/// use yggdryl::IntervalType;
/// use yggdryl::{DataTypeId, TimeUnit};
///
/// let leaf = IntervalType::Interval(TimeUnit::DayTime);
/// assert_eq!(leaf.id(), DataTypeId::Interval);
/// assert_eq!(leaf.unit(), TimeUnit::DayTime);
/// assert_eq!(IntervalType::from_id(DataTypeId::Interval, TimeUnit::YearMonth), Some(IntervalType::Interval(TimeUnit::YearMonth)));
/// assert!(IntervalType::Interval(TimeUnit::Second).validate().is_err());
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum IntervalType {
    /// A calendar interval in one of the three layouts: `YearMonth`,
    /// `DayTime` or `MonthDayNano`.
    Interval(TimeUnit),
}

impl IntervalType {
    /// Every leaf in identifier order, at the layout the grammar and the
    /// bindings default to.
    pub const ALL: [Self; 1] = [Self::Interval(TimeUnit::MonthDayNano)];

    /// Return the exact datatype identifier.
    #[must_use]
    pub const fn id(self) -> DataTypeId {
        match self {
            Self::Interval(_) => DataTypeId::Interval,
        }
    }

    /// The leaf one identifier names at `unit`, or `None` for an identifier
    /// of another family. The layout is carried, not checked.
    #[must_use]
    pub const fn from_id(id: DataTypeId, unit: TimeUnit) -> Option<Self> {
        match id {
            DataTypeId::Interval => Some(Self::Interval(unit)),
            _ => None,
        }
    }

    /// The canonical name of this leaf.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.id().as_str()
    }

    /// The layout this column stores: `YearMonth`, `DayTime` or
    /// `MonthDayNano`.
    #[must_use]
    pub const fn unit(self) -> TimeUnit {
        match self {
            Self::Interval(unit) => unit,
        }
    }

    /// Reject a unit that is not an interval layout.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a resolution.
    pub fn validate(self) -> Result<()> {
        if self.unit().is_interval() {
            Ok(())
        } else {
            Err(invalid("Interval", "unit must be an interval layout"))
        }
    }
}

impl DataTypeValue for IntervalType {
    const FAMILY: &'static str = "interval";

    type Sidecar = ();

    fn id(&self) -> DataTypeId {
        Self::id(*self)
    }

    fn validate(&self) -> Result<()> {
        Self::validate(*self)
    }

    fn into_dtype(self) -> DataType {
        match self {
            Self::Interval(unit) => DataType::Interval(unit),
        }
    }

    fn from_dtype(dtype: &DataType) -> Option<Self> {
        dtype.interval_type()
    }
}

impl fmt::Display for IntervalType {
    /// The canonical spelling, `interval(month_day_nano)`, which
    /// [`crate::DataType`]'s grammar reads back.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}({})", self.as_str(), self.unit())
    }
}

impl From<IntervalType> for DataType {
    fn from(value: IntervalType) -> Self {
        DataTypeValue::into_dtype(value)
    }
}

impl TryFrom<&DataType> for IntervalType {
    type Error = Error;

    fn try_from(value: &DataType) -> Result<Self> {
        match value.interval_type() {
            Some(leaf) => Ok(leaf),
            None => Err(Error::InvalidDataType {
                kind: "interval",
                reason: format_smolstr!("expected an interval datatype, got {value}"),
            }),
        }
    }
}

// ------------------------------------------------------------------------
// The door into the interval family.
// ------------------------------------------------------------------------

impl DataType {
    /// A calendar interval in the layout `unit` names.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a resolution rather than a
    /// layout.
    pub fn interval(unit: TimeUnit) -> Result<Self> {
        let leaf = IntervalType::Interval(unit);
        leaf.validate()?;
        Ok(DataTypeValue::into_dtype(leaf))
    }

    /// The leaf an interval datatype declares, `None` for every other.
    #[must_use]
    pub const fn interval_type(&self) -> Option<IntervalType> {
        match self {
            Self::Interval(unit) => Some(IntervalType::Interval(*unit)),
            _ => None,
        }
    }
}

// ------------------------------------------------------------------------
// The grammar of the layout parameter.
// ------------------------------------------------------------------------

impl Parser<'_> {
    /// Parse the layout after `interval`: bare, in parentheses, or SQL's
    /// bare word; nothing at all is `month_day_nano`.
    ///
    /// In SQL, bare `INTERVAL DAY` names the day-time layout, while the
    /// parenthesized `interval(day)` still names the scalar day unit and is
    /// refused rather than reinterpreted.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Parse`] for a unit no name spells or one
    /// that is not an interval layout.
    pub(crate) fn parse_interval_unit(&mut self, depth: usize) -> Result<TimeUnit> {
        self.check_depth(depth)?;
        let (unit, unit_start, sql_style) = if self
            .tokens
            .get(self.index)
            .is_none_or(|_| self.is_time_unit_boundary(self.index, None))
        {
            (TimeUnit::MonthDayNano, self.current_position(), false)
        } else if let Some(close) = self.consume_opening() {
            let (unit, unit_start) = self.parse_time_unit_span(Some(close), "interval unit")?;
            self.expect_symbol(close)?;
            (unit, unit_start, false)
        } else {
            let (unit, unit_start) = self.parse_time_unit_span(None, "interval unit")?;
            (unit, unit_start, true)
        };
        let unit = if sql_style && unit == TimeUnit::Day {
            TimeUnit::DayTime
        } else {
            unit
        };
        if unit.is_interval() {
            Ok(unit)
        } else {
            Err(self.error_at(unit_start, "interval requires an interval layout"))
        }
    }
}

// ------------------------------------------------------------------------
// Arrow projection: the three interval layouts are Arrow's own.
// ------------------------------------------------------------------------

mod arrow {
    use arrow_schema::DataType as ArrowDataType;
    use smol_str::format_smolstr;

    use crate::invalid;
    use crate::{DataType, Result, TimeUnit};

    /// The Arrow storage one interval datatype lays out.
    ///
    /// # Errors
    ///
    /// Returns an error when the unit is not a layout, or when the datatype
    /// belongs to another family.
    pub(crate) fn arrow_storage(dtype: &DataType) -> Result<ArrowDataType> {
        match dtype {
            DataType::Interval(unit) => Ok(ArrowDataType::Interval(unit.into_arrow_interval()?)),
            other => Err(invalid(
                "interval",
                format_smolstr!("expected an interval datatype, got {other}"),
            )),
        }
    }

    /// The interval datatype one Arrow storage imports as.
    ///
    /// # Errors
    ///
    /// Returns an error when the storage belongs to another family.
    pub(crate) fn from_arrow_storage(value: &ArrowDataType) -> Result<DataType> {
        match value {
            ArrowDataType::Interval(unit) => {
                DataType::interval(TimeUnit::from_arrow_interval(*unit))
            }
            other => Err(invalid(
                "interval",
                format_smolstr!("expected an interval storage, got {other}"),
            )),
        }
    }
}

// ------------------------------------------------------------------------
// The interval value: every component of every layout.
// ------------------------------------------------------------------------

/// One Arrow interval represented without losing any of its three layouts.
///
/// Every layout is months, days and nanoseconds; a layout with fewer
/// components holds the missing ones at zero, which [`Self::new`] enforces
/// so a value never claims a layout it does not fit.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Interval {
    months: i32,
    days: i32,
    nanoseconds: i64,
    unit: TimeUnit,
}

impl<'de> Deserialize<'de> for Interval {
    /// The components as written, then the layout rule: a document never
    /// builds a value the constructor would refuse.
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            months: i32,
            days: i32,
            nanoseconds: i64,
            unit: TimeUnit,
        }

        let value = Wire::deserialize(deserializer)?;
        Self::new(value.months, value.days, value.nanoseconds, value.unit)
            .map_err(serde::de::Error::custom)
    }
}

impl Interval {
    /// Construct an interval, rejecting fields the selected layout cannot
    /// hold.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a component the layout has
    /// nowhere to store, or a unit that is not a layout.
    pub fn new(months: i32, days: i32, nanoseconds: i64, unit: TimeUnit) -> Result<Self> {
        let valid = match unit {
            TimeUnit::YearMonth => days == 0 && nanoseconds == 0,
            TimeUnit::DayTime => {
                months == 0
                    && nanoseconds % 1_000_000 == 0
                    && i32::try_from(nanoseconds / 1_000_000).is_ok()
            }
            TimeUnit::MonthDayNano => true,
            _ => false,
        };
        if !valid {
            return Err(invalid_record(
                "Interval components do not fit the selected layout",
            ));
        }
        Ok(Self {
            months,
            days,
            nanoseconds,
            unit,
        })
    }

    /// Return the month component.
    #[must_use]
    pub const fn months(&self) -> i32 {
        self.months
    }

    /// Return the day component.
    #[must_use]
    pub const fn days(&self) -> i32 {
        self.days
    }

    /// Return the nanosecond component.
    #[must_use]
    pub const fn nanoseconds(&self) -> i64 {
        self.nanoseconds
    }

    /// Return the physical interval layout.
    #[must_use]
    pub const fn unit(&self) -> TimeUnit {
        self.unit
    }
}

impl fmt::Display for Interval {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}mo:{}d:{}ns@{}",
            self.months, self.days, self.nanoseconds, self.unit
        )
    }
}

impl Value for Interval {
    fn dtype(&self) -> Result<DataType> {
        DataType::interval(self.unit())
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Interval(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Interval(value) => Some(value),
            _ => None,
        }
    }
}

/// An interval answers the temporal contract with what it has: its
/// nanosecond component for a count, its layout for a unit, and no zone.
impl TemporalValue for Interval {
    const BIT_WIDTH: u8 = 128;

    fn count(&self) -> i64 {
        self.nanoseconds()
    }

    fn unit(&self) -> TimeUnit {
        self.unit()
    }

    fn timezone(&self) -> Timezone {
        Timezone::NAIVE
    }

    fn with_unit(self, unit: TimeUnit) -> Result<Self> {
        Self::new(self.months(), self.days(), self.nanoseconds(), unit)
    }

    fn with_timezone(self, timezone: Timezone) -> Result<Self> {
        if timezone.is_naive() {
            Ok(self)
        } else {
            Err(invalid_record("Interval requires the NAIVE timezone"))
        }
    }
}

impl Scalar {
    /// Build a calendar interval in the layout `unit` names.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a component the layout has
    /// nowhere to store, or a unit that is not a layout.
    pub fn interval(months: i32, days: i32, nanoseconds: i64, unit: TimeUnit) -> Result<Self> {
        Interval::new(months, days, nanoseconds, unit).map(Self::Interval)
    }
}
