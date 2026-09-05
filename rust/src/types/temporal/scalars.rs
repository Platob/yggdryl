//! Width-accurate temporal values with explicit units and zones.
//!
//! ```
//! use yggdryl::{Scalar, TemporalFamily, TimeUnit, Timezone};
//!
//! let day = Scalar::from_date(20_000, TimeUnit::Day, Timezone::NAIVE)?;
//! let at = Scalar::from_datetime(1, TimeUnit::Microsecond, Timezone::UTC)?;
//! assert_eq!(day.as_date().map(|value| value.bit_width()), Some(32));
//! assert_eq!(at.as_datetime().map(|value| value.family()), Some(TemporalFamily::DateTime));
//! # Ok::<(), yggdryl::Error>(())
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::{SmolStr, format_smolstr};

use crate::types::arithmetic::{Arithmetic, invalid_binary};
use crate::types::ascii::iso;
use crate::types::decimal::scalars::exact_value_parts;
use crate::types::typed::define_scalar_type;
use crate::types::value::{ValidationFailure, expected};
use crate::{
    DataType, DataTypeId, DataTypeKind, Error, I256, Result, Scalar, ScalarFamily, ScalarValue,
    TimeUnit, Timezone,
};

/// Operations shared by every temporal representation.
pub trait TemporalValue: crate::ScalarValue {
    /// The semantic temporal family.
    const FAMILY: TemporalFamily;
    /// The physical count width in bits.
    const BIT_WIDTH: u8;

    /// Return the stored count widened to 64 bits.
    fn count(&self) -> i64;
    /// Return the count's unit.
    fn unit(&self) -> TimeUnit;
    /// Return the explicit timezone marker.
    fn timezone(&self) -> Timezone;
    /// Convert this value to another valid unit.
    fn with_unit(self, unit: TimeUnit) -> Result<Self>;
    /// Restate this value with another valid timezone marker.
    fn with_timezone(self, timezone: Timezone) -> Result<Self>;
}

fn invalid_temporal_leaf(reason: &'static str) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: SmolStr::new_static(reason),
    }
}

macro_rules! temporal_leaf {
    ($name:ident, $count:ty, $valid:expr, $reason:literal) => {
        #[doc = concat!("One exact `", stringify!($name), "` count, unit, and timezone.")]
        #[derive(
            Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
        )]
        pub struct $name {
            count: $count,
            unit: TimeUnit,
            timezone: Timezone,
        }

        impl $name {
            /// Validate and construct this exact temporal representation.
            pub fn new(count: $count, unit: TimeUnit, timezone: Timezone) -> Result<Self> {
                if !$valid(unit, timezone) {
                    return Err(invalid_temporal_leaf($reason));
                }
                Ok(Self {
                    count,
                    unit,
                    timezone,
                })
            }

            /// Return the stored count.
            pub const fn count(&self) -> $count {
                self.count
            }

            /// Return the count's unit.
            pub const fn unit(&self) -> TimeUnit {
                self.unit
            }

            /// Return the explicit timezone marker.
            pub const fn timezone(&self) -> Timezone {
                self.timezone
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}@{}[{}]", self.count, self.unit, self.timezone)
            }
        }
    };
}

temporal_leaf!(
    Date32,
    i32,
    |unit: TimeUnit, timezone: Timezone| unit == TimeUnit::Day && timezone.is_naive(),
    "Date32 requires day units and the NAIVE timezone"
);
temporal_leaf!(
    Date64,
    i64,
    |unit: TimeUnit, timezone: Timezone| unit == TimeUnit::Millisecond && timezone.is_naive(),
    "Date64 requires millisecond units and the NAIVE timezone"
);
temporal_leaf!(
    Time32,
    i32,
    |unit: TimeUnit, timezone: Timezone| matches!(unit, TimeUnit::Second | TimeUnit::Millisecond)
        && timezone.is_naive(),
    "Time32 requires second or millisecond units and the NAIVE timezone"
);
temporal_leaf!(
    Time64,
    i64,
    |unit: TimeUnit, timezone: Timezone| matches!(
        unit,
        TimeUnit::Microsecond | TimeUnit::Nanosecond
    ) && timezone.is_naive(),
    "Time64 requires microsecond or nanosecond units and the NAIVE timezone"
);
temporal_leaf!(
    DateTime64,
    i64,
    |unit: TimeUnit, _timezone: Timezone| unit.is_arrow_time(),
    "DateTime64 requires an Arrow clock resolution"
);
temporal_leaf!(
    Duration32,
    i32,
    |unit: TimeUnit, timezone: Timezone| unit.is_temporal() && timezone.is_naive(),
    "Duration32 requires a fixed temporal unit and the NAIVE timezone"
);
temporal_leaf!(
    Duration64,
    i64,
    |unit: TimeUnit, timezone: Timezone| unit.is_temporal() && timezone.is_naive(),
    "Duration64 requires a fixed temporal unit and the NAIVE timezone"
);

const _: () = assert!(std::mem::size_of::<DateTime64>() == 16);

/// One Arrow interval represented without losing any of its three layouts.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Interval {
    months: i32,
    days: i32,
    nanoseconds: i64,
    unit: TimeUnit,
}

impl<'de> Deserialize<'de> for Interval {
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
    /// Construct an interval, rejecting fields the selected layout cannot hold.
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
            return Err(invalid_temporal_leaf(
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
    pub const fn months(&self) -> i32 {
        self.months
    }

    /// Return the day component.
    pub const fn days(&self) -> i32 {
        self.days
    }

    /// Return the nanosecond component.
    pub const fn nanoseconds(&self) -> i64 {
        self.nanoseconds
    }

    /// Return the physical interval layout.
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

/// One exact temporal or interval representation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[non_exhaustive]
pub enum Temporal {
    /// Day-count date.
    Date32(Date32),
    /// Millisecond-count date.
    Date64(Date64),
    /// 32-bit time of day.
    Time32(Time32),
    /// 64-bit time of day.
    Time64(Time64),
    /// 64-bit datetime.
    DateTime64(DateTime64),
    /// 32-bit duration.
    Duration32(Duration32),
    /// 64-bit duration.
    Duration64(Duration64),
    /// Calendar interval.
    Interval(Interval),
}

const _: () = assert!(std::mem::size_of::<Temporal>() == 24);

impl Temporal {
    /// Return the logical family within the temporal group.
    pub const fn family(self) -> TemporalFamily {
        match self {
            Self::Date32(_) | Self::Date64(_) => TemporalFamily::Date,
            Self::Time32(_) | Self::Time64(_) => TemporalFamily::Time,
            Self::DateTime64(_) => TemporalFamily::DateTime,
            Self::Duration32(_) | Self::Duration64(_) => TemporalFamily::Duration,
            Self::Interval(_) => TemporalFamily::Interval,
        }
    }

    /// Return the physical unit or interval layout.
    pub const fn unit(self) -> TimeUnit {
        match self {
            Self::Date32(value) => value.unit(),
            Self::Date64(value) => value.unit(),
            Self::Time32(value) => value.unit(),
            Self::Time64(value) => value.unit(),
            Self::DateTime64(value) => value.unit(),
            Self::Duration32(value) => value.unit(),
            Self::Duration64(value) => value.unit(),
            Self::Interval(value) => value.unit(),
        }
    }

    /// Return the temporal timezone, with intervals explicitly zone-free.
    pub const fn timezone(self) -> Timezone {
        match self {
            Self::Date32(value) => value.timezone(),
            Self::Date64(value) => value.timezone(),
            Self::Time32(value) => value.timezone(),
            Self::Time64(value) => value.timezone(),
            Self::DateTime64(value) => value.timezone(),
            Self::Duration32(value) => value.timezone(),
            Self::Duration64(value) => value.timezone(),
            Self::Interval(_) => Timezone::NAIVE,
        }
    }

    /// Return the physical count width, or 128 bits for an interval payload.
    pub const fn bit_width(self) -> u8 {
        match self {
            Self::Date32(_) | Self::Time32(_) | Self::Duration32(_) => 32,
            Self::Date64(_) | Self::Time64(_) | Self::DateTime64(_) | Self::Duration64(_) => 64,
            Self::Interval(_) => 128,
        }
    }

    /// Return the stored count widened to 64 bits.
    ///
    /// For an interval this is its nanosecond component; callers that need all
    /// three interval components match [`Temporal::Interval`] directly.
    pub const fn count(self) -> i64 {
        match self {
            Self::Date32(value) => value.count() as i64,
            Self::Date64(value) => value.count(),
            Self::Time32(value) => value.count() as i64,
            Self::Time64(value) => value.count(),
            Self::DateTime64(value) => value.count(),
            Self::Duration32(value) => value.count() as i64,
            Self::Duration64(value) => value.count(),
            Self::Interval(value) => value.nanoseconds(),
        }
    }
}

impl fmt::Display for Temporal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Date32(value) => value.fmt(formatter),
            Self::Date64(value) => value.fmt(formatter),
            Self::Time32(value) => value.fmt(formatter),
            Self::Time64(value) => value.fmt(formatter),
            Self::DateTime64(value) => value.fmt(formatter),
            Self::Duration32(value) => value.fmt(formatter),
            Self::Duration64(value) => value.fmt(formatter),
            Self::Interval(value) => value.fmt(formatter),
        }
    }
}

macro_rules! temporal_value {
    ($leaf:ident, $marker:ty, $variant:ident, $id:ident, $family:ident, $bits:literal, $count:ty) => {
        impl ScalarValue for $leaf {
            type Family = Temporal;
            type Type = $marker;

            const ID: DataTypeId = DataTypeId::$id;
            const KIND: DataTypeKind = DataTypeKind::Temporal;

            fn dtype(&self) -> Result<DataType> {
                Scalar::Temporal(Temporal::$variant(*self)).dtype()
            }

            fn into_family(self) -> Self::Family {
                Temporal::$variant(self)
            }

            fn from_family(family: &Self::Family) -> Option<&Self> {
                match family {
                    Temporal::$variant(value) => Some(value),
                    _ => None,
                }
            }

            fn into_scalar(self) -> Scalar {
                Scalar::Temporal(Temporal::$variant(self))
            }

            fn from_scalar(value: &Scalar) -> Option<&Self> {
                match value {
                    Scalar::Temporal(Temporal::$variant(value)) => Some(value),
                    _ => None,
                }
            }
        }

        impl TemporalValue for $leaf {
            const FAMILY: TemporalFamily = TemporalFamily::$family;
            const BIT_WIDTH: u8 = $bits;

            fn count(&self) -> i64 {
                i64::from(self.count())
            }

            fn unit(&self) -> TimeUnit {
                self.unit()
            }

            fn timezone(&self) -> Timezone {
                self.timezone()
            }

            fn with_unit(self, unit: TimeUnit) -> Result<Self> {
                let value = Scalar::Temporal(Temporal::$variant(self));
                let count = value.temporal_count_at(unit).ok_or_else(|| {
                    invalid_temporal_leaf("temporal unit conversion is not exact")
                })?;
                let count = <$count>::try_from(count).map_err(|_| {
                    invalid_temporal_leaf("converted temporal count exceeds its physical width")
                })?;
                Self::new(count, unit, self.timezone())
            }

            fn with_timezone(self, timezone: Timezone) -> Result<Self> {
                Self::new(self.count(), self.unit(), timezone)
            }
        }
    };
}

temporal_value!(Date32, super::Date32Type, Date32, Date32, Date, 32, i32);
temporal_value!(Date64, super::Date64Type, Date64, Date64, Date, 64, i64);
temporal_value!(Time32, super::Time32Type, Time32, Time32, Time, 32, i32);
temporal_value!(Time64, super::Time64Type, Time64, Time64, Time, 64, i64);
temporal_value!(
    DateTime64,
    super::DateTime64Type,
    DateTime64,
    DateTime64,
    DateTime,
    64,
    i64
);
temporal_value!(
    Duration32,
    super::Duration32Type,
    Duration32,
    Duration32,
    Duration,
    32,
    i32
);
temporal_value!(
    Duration64,
    super::Duration64Type,
    Duration64,
    Duration64,
    Duration,
    64,
    i64
);

impl ScalarValue for Interval {
    type Family = Temporal;
    type Type = super::IntervalType;

    const ID: DataTypeId = DataTypeId::Interval;
    const KIND: DataTypeKind = DataTypeKind::Temporal;

    fn dtype(&self) -> Result<DataType> {
        Scalar::Temporal(Temporal::Interval(*self)).dtype()
    }

    fn into_family(self) -> Self::Family {
        Temporal::Interval(self)
    }

    fn from_family(family: &Self::Family) -> Option<&Self> {
        match family {
            Temporal::Interval(value) => Some(value),
            _ => None,
        }
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Temporal(Temporal::Interval(self))
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Temporal(Temporal::Interval(value)) => Some(value),
            _ => None,
        }
    }
}

impl TemporalValue for Interval {
    const FAMILY: TemporalFamily = TemporalFamily::Interval;
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
            Err(invalid_temporal_leaf(
                "Interval requires the NAIVE timezone",
            ))
        }
    }
}

impl ScalarFamily for Temporal {
    const KIND: DataTypeKind = DataTypeKind::Temporal;

    fn id(&self) -> DataTypeId {
        match self {
            Self::Date32(_) => DataTypeId::Date32,
            Self::Date64(_) => DataTypeId::Date64,
            Self::Time32(_) => DataTypeId::Time32,
            Self::Time64(_) => DataTypeId::Time64,
            Self::DateTime64(_) => DataTypeId::DateTime64,
            Self::Duration32(_) => DataTypeId::Duration32,
            Self::Duration64(_) => DataTypeId::Duration64,
            Self::Interval(_) => DataTypeId::Interval,
        }
    }

    fn dtype(&self) -> Result<DataType> {
        (*self).into_scalar().dtype()
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Temporal(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Temporal(value) => Some(value),
            _ => None,
        }
    }
}

define_scalar_type!(DateTime64Scalar, super::DateTime64Type, "datetime64");
define_scalar_type!(
    Date32Scalar,
    super::Date32Type,
    "date32",
    crate::DataType::Date32
);
define_scalar_type!(
    Date64Scalar,
    super::Date64Type,
    "date64",
    crate::DataType::Date64
);
define_scalar_type!(Time32Scalar, super::Time32Type, "time32");
define_scalar_type!(Time64Scalar, super::Time64Type, "time64");
define_scalar_type!(Duration32Scalar, super::Duration32Type, "duration32");
define_scalar_type!(Duration64Scalar, super::Duration64Type, "duration64");
define_scalar_type!(IntervalScalar, super::IntervalType, "interval");

/// One logical temporal family, independent of its physical width.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TemporalFamily {
    /// Calendar dates.
    Date,
    /// Times of day.
    Time,
    /// Epoch or wall-clock datetimes.
    DateTime,
    /// Elapsed durations.
    Duration,
    /// Calendar intervals.
    Interval,
}

impl TemporalFamily {
    /// Return the canonical family name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Date => "date",
            Self::Time => "time",
            Self::DateTime => "datetime",
            Self::Duration => "duration",
            Self::Interval => "interval",
        }
    }
}

impl Scalar {
    /// Build the exact date width selected by its unit.
    pub fn from_date(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        match unit {
            TimeUnit::Day => Self::date32_in(narrow_i32(count, "date32")?, unit, zone),
            TimeUnit::Millisecond => Self::date64_in(count, unit, zone),
            _ => Err(invalid("date unit must be day or millisecond")),
        }
    }

    /// Build the exact time-of-day width selected by its unit.
    pub fn from_time(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        match unit {
            TimeUnit::Second | TimeUnit::Millisecond => {
                Self::time32(narrow_i32(count, "time32")?, unit, zone)
            }
            TimeUnit::Microsecond | TimeUnit::Nanosecond => Self::time64(count, unit, zone),
            _ => Err(invalid(
                "time unit must be second, millisecond, microsecond, or nanosecond",
            )),
        }
    }

    /// Build a 64-bit epoch or wall-clock datetime.
    pub fn from_datetime(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        Self::datetime64(count, unit, zone)
    }

    /// Build the narrowest duration width that holds `count`.
    pub fn from_duration(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        match i32::try_from(count) {
            Ok(count) => Self::duration32_in(count, unit, zone),
            Err(_) => Self::duration64_in(count, unit, zone),
        }
    }

    /// Build a Date32 day count.
    pub const fn date32(days: i32) -> Self {
        Self::Temporal(Temporal::Date32(Date32 {
            count: days,
            unit: TimeUnit::Day,
            timezone: Timezone::NAIVE,
        }))
    }

    /// Build a Date32 after validating its unit and zone.
    pub fn date32_in(days: i32, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(unit == TimeUnit::Day, "date32 unit must be day")?;
        require(zone.is_naive(), "date32 timezone must be NAIVE")?;
        Date32::new(days, unit, zone).map(|value| Self::Temporal(Temporal::Date32(value)))
    }

    /// Build a Date64 millisecond count.
    pub const fn date64(milliseconds: i64) -> Self {
        Self::Temporal(Temporal::Date64(Date64 {
            count: milliseconds,
            unit: TimeUnit::Millisecond,
            timezone: Timezone::NAIVE,
        }))
    }

    /// Build a Date64 after validating its unit and zone.
    pub fn date64_in(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            unit == TimeUnit::Millisecond,
            "date64 unit must be millisecond",
        )?;
        require(zone.is_naive(), "date64 timezone must be NAIVE")?;
        Date64::new(count, unit, zone).map(|value| Self::Temporal(Temporal::Date64(value)))
    }

    /// Build a 32-bit time of day.
    pub fn time32(count: i32, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            matches!(unit, TimeUnit::Second | TimeUnit::Millisecond),
            "time32 unit must be second or millisecond",
        )?;
        require(
            zone.is_naive(),
            "time32 timezone must be NAIVE because its datatype has no timezone",
        )?;
        Time32::new(count, unit, zone).map(|value| Self::Temporal(Temporal::Time32(value)))
    }

    /// Build a 64-bit time of day.
    pub fn time64(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            matches!(unit, TimeUnit::Microsecond | TimeUnit::Nanosecond),
            "time64 unit must be microsecond or nanosecond",
        )?;
        require(
            zone.is_naive(),
            "time64 timezone must be NAIVE because its datatype has no timezone",
        )?;
        Time64::new(count, unit, zone).map(|value| Self::Temporal(Temporal::Time64(value)))
    }

    /// Build an instant or wall-clock datetime at 64-bit width.
    pub fn datetime64(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            unit.is_arrow_time(),
            "datetime64 requires an Arrow time unit",
        )?;
        DateTime64::new(count, unit, zone).map(|value| Self::Temporal(Temporal::DateTime64(value)))
    }

    /// Parse a timezone and build a 64-bit datetime.
    pub fn datetime64_in(count: i64, unit: TimeUnit, zone: &str) -> Result<Self> {
        Self::datetime64(count, unit, Timezone::from_str(zone)?)
    }

    /// Build a 32-bit duration.
    pub fn duration32(count: i32, unit: TimeUnit) -> Result<Self> {
        Self::duration32_in(count, unit, Timezone::NAIVE)
    }

    /// Build a 32-bit duration after validating its explicit timezone marker.
    pub fn duration32_in(count: i32, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            unit.is_temporal(),
            "duration32 requires a fixed temporal unit",
        )?;
        require(zone.is_naive(), "duration32 timezone must be NAIVE")?;
        Duration32::new(count, unit, zone).map(|value| Self::Temporal(Temporal::Duration32(value)))
    }

    /// Build a 64-bit duration.
    pub fn duration64(count: i64, unit: TimeUnit) -> Result<Self> {
        Self::duration64_in(count, unit, Timezone::NAIVE)
    }

    /// Build a 64-bit duration after validating its explicit timezone marker.
    pub fn duration64_in(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            unit.is_temporal(),
            "duration64 requires a fixed temporal unit",
        )?;
        require(zone.is_naive(), "duration64 timezone must be NAIVE")?;
        Duration64::new(count, unit, zone).map(|value| Self::Temporal(Temporal::Duration64(value)))
    }

    /// Return Date32's count, unit, and zone.
    pub const fn as_date32(&self) -> Option<(i32, TimeUnit, &Timezone)> {
        match self {
            Self::Temporal(Temporal::Date32(value)) => {
                Some((value.count(), value.unit(), &value.timezone))
            }
            _ => None,
        }
    }

    /// Return Date64's count, unit, and zone.
    pub const fn as_date64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
        match self {
            Self::Temporal(Temporal::Date64(value)) => {
                Some((value.count(), value.unit(), &value.timezone))
            }
            _ => None,
        }
    }

    /// Return Time32's count, unit, and zone.
    pub const fn as_time32(&self) -> Option<(i32, TimeUnit, &Timezone)> {
        match self {
            Self::Temporal(Temporal::Time32(value)) => {
                Some((value.count(), value.unit(), &value.timezone))
            }
            _ => None,
        }
    }

    /// Return Time64's count, unit, and zone.
    pub const fn as_time64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
        match self {
            Self::Temporal(Temporal::Time64(value)) => {
                Some((value.count(), value.unit(), &value.timezone))
            }
            _ => None,
        }
    }

    /// Return DateTime64's count, unit, and zone.
    pub const fn as_datetime64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
        match self {
            Self::Temporal(Temporal::DateTime64(value)) => {
                Some((value.count(), value.unit(), &value.timezone))
            }
            _ => None,
        }
    }

    /// Return Duration32's count, unit, and zone.
    pub const fn as_duration32(&self) -> Option<(i32, TimeUnit, &Timezone)> {
        match self {
            Self::Temporal(Temporal::Duration32(value)) => {
                Some((value.count(), value.unit(), &value.timezone))
            }
            _ => None,
        }
    }

    /// Return Duration64's count, unit, and zone.
    pub const fn as_duration64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
        match self {
            Self::Temporal(Temporal::Duration64(value)) => {
                Some((value.count(), value.unit(), &value.timezone))
            }
            _ => None,
        }
    }

    /// Borrow the shared family view of any temporal value.
    pub const fn as_temporal(&self) -> Option<&Temporal> {
        match self {
            Self::Temporal(value) => Some(value),
            _ => None,
        }
    }

    /// Borrow this value as either exact date width.
    pub const fn as_date(&self) -> Option<&Temporal> {
        match self.as_temporal() {
            Some(value) if matches!(value.family(), TemporalFamily::Date) => Some(value),
            _ => None,
        }
    }

    /// Borrow this value as either exact time-of-day width.
    pub const fn as_time(&self) -> Option<&Temporal> {
        match self.as_temporal() {
            Some(value) if matches!(value.family(), TemporalFamily::Time) => Some(value),
            _ => None,
        }
    }

    /// Borrow this value as a datetime without exposing its physical suffix.
    pub const fn as_datetime(&self) -> Option<&Temporal> {
        match self.as_temporal() {
            Some(value) if matches!(value.family(), TemporalFamily::DateTime) => Some(value),
            _ => None,
        }
    }

    /// Borrow this value as either exact duration width.
    pub const fn as_duration(&self) -> Option<&Temporal> {
        match self.as_temporal() {
            Some(value) if matches!(value.family(), TemporalFamily::Duration) => Some(value),
            _ => None,
        }
    }

    /// Return the non-optional timezone carried by any temporal.
    pub const fn temporal_timezone(&self) -> Option<Timezone> {
        match self.as_temporal() {
            Some(value) => Some((*value).timezone()),
            None => None,
        }
    }

    /// Read one temporal from its classic text spelling, at the exact width,
    /// unit and zone `dtype` declares.
    ///
    /// This is the crate's one text reading of a temporal: the row evaluator,
    /// the field-directed record parsers and the Arrow cast leaf all arrive
    /// here, so a spelling reads the same count wherever it is met. The unit
    /// the spelling names is restated in the declared one and has to land
    /// exactly - `10:00:00.500` is no `time32(second)` - and the zone is the
    /// datatype's: a zoned datetime wants an offset in the text, while a
    /// `NAIVE` datetime refuses one.
    ///
    /// # Errors
    ///
    /// Returns the parse error the spelling raised, or an invalid-value error
    /// when the count does not fit the declared unit and width.
    pub(crate) fn from_temporal_text(dtype: &DataType, text: &str) -> Result<Self> {
        match dtype {
            DataType::Date32 => Ok(Self::date32(iso::parse_date(text)?)),
            DataType::Date64 => i64::from(iso::parse_date(text)?)
                .checked_mul(86_400_000)
                .map(Self::date64)
                .ok_or_else(|| invalid("date64 count must fit signed 64 bits")),
            DataType::Time32(unit) => {
                let count = restated(clock_of_day(text)?, *unit, "time32")?;
                Self::time32(narrow_i32(count, "time32")?, *unit, Timezone::NAIVE)
            }
            DataType::Time64(unit) => {
                let count = restated(clock_of_day(text)?, *unit, "time64")?;
                Self::time64(count, *unit, Timezone::NAIVE)
            }
            DataType::DateTime64 { unit, timezone } if timezone.is_naive() => {
                let count = restated(iso::parse_datetime(text)?, *unit, "datetime64")?;
                Self::datetime64(count, *unit, Timezone::NAIVE)
            }
            DataType::DateTime64 { unit, timezone } => {
                let (count, source, _) = iso::parse_timestamp(text)?;
                let count = restated((count, source), *unit, "datetime64")?;
                Self::datetime64(count, *unit, *timezone)
            }
            DataType::Duration32(unit) => {
                let count = restated(iso::parse_duration(text)?, *unit, "duration32")?;
                Self::duration32(narrow_i32(count, "duration32")?, *unit)
            }
            DataType::Duration64(unit) => {
                let count = restated(iso::parse_duration(text)?, *unit, "duration64")?;
                Self::duration64(count, *unit)
            }
            other => Err(invalid(format!("{other} holds no temporal text"))),
        }
    }

    /// Spell this temporal the classic way, when it has a classic spelling.
    ///
    /// This is the crate's one text spelling of a temporal: an expression
    /// literal, a cast to text and the Arrow cast leaf all render here, so a
    /// value reads back as what it printed. A reading with no classic
    /// spelling - a date beyond four-digit years, an interval layout -
    /// answers `None`, as [`iso`] does.
    // The name states the conversion direction, as the text codecs' own
    // `into_*` readers do; the spelling is built, so it cannot borrow.
    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn into_temporal_text(&self) -> Option<smol_str::SmolStr> {
        match self {
            Self::Temporal(Temporal::Date32(value)) => iso::format_date(value.count()),
            Self::Temporal(Temporal::Date64(value)) => {
                i32::try_from(value.count().div_euclid(86_400_000))
                    .ok()
                    .and_then(iso::format_date)
            }
            Self::Temporal(Temporal::Time32(value)) => {
                iso::format_time(i64::from(value.count()), value.unit())
            }
            Self::Temporal(Temporal::Time64(value)) => {
                iso::format_time(value.count(), value.unit())
            }
            Self::Temporal(Temporal::DateTime64(value)) if value.timezone().is_naive() => {
                iso::format_datetime(value.count(), value.unit())
            }
            Self::Temporal(Temporal::DateTime64(value)) => {
                iso::format_timestamp(value.count(), value.unit(), &value.timezone())
            }
            Self::Temporal(Temporal::Duration32(value)) => {
                iso::format_duration(i64::from(value.count()), value.unit())
            }
            Self::Temporal(Temporal::Duration64(value)) => {
                iso::format_duration(value.count(), value.unit())
            }
            _ => None,
        }
    }

    /// Return this temporal's count restated in `unit`, when exact.
    pub fn temporal_count_at(&self, unit: TimeUnit) -> Option<i64> {
        let temporal = self.as_temporal()?;
        if matches!(temporal, Temporal::Interval(_)) {
            return None;
        }
        let (count, current) = ((*temporal).count(), (*temporal).unit());
        if current == unit {
            return Some(count);
        }
        let nanoseconds = i128::from(count) * nanoseconds_per(current)?;
        let divisor = nanoseconds_per(unit)?;
        (nanoseconds % divisor == 0)
            .then(|| i64::try_from(nanoseconds / divisor).ok())
            .flatten()
    }

    /// Return whether this is a temporal value.
    pub const fn is_temporal(&self) -> bool {
        self.as_temporal().is_some()
    }

    /// Return the datatype this temporal materializes into.
    pub fn temporal_dtype(&self) -> Option<DataType> {
        self.is_temporal().then(|| self.dtype().ok()).flatten()
    }
}

/// Read a time of day, naming the type that reads a zoned clock instead.
///
/// An offset makes a clock an instant, and the message says so rather than
/// reporting the offset as trailing text.
fn clock_of_day(text: &str) -> Result<(i64, TimeUnit)> {
    let zoned = text.ends_with(['Z', 'z'])
        || text
            .len()
            .checked_sub(6)
            .is_some_and(|start| matches!(text.as_bytes()[start], b'+' | b'-'));
    if zoned {
        return Err(invalid(
            "time-of-day cannot carry a timezone; use DateTime64 for a zoned instant",
        ));
    }
    iso::parse_time(text)
}

/// Restate a parsed count in the unit its datatype declares, when exact.
fn restated((count, source): (i64, TimeUnit), unit: TimeUnit, kind: &'static str) -> Result<i64> {
    if source == unit {
        return Ok(count);
    }
    let restate = |count: i128| -> Option<i64> {
        let nanoseconds = count.checked_mul(nanoseconds_per(source)?)?;
        let divisor = nanoseconds_per(unit)?;
        (nanoseconds % divisor == 0)
            .then(|| i64::try_from(nanoseconds / divisor).ok())
            .flatten()
    };
    restate(i128::from(count)).ok_or_else(|| invalid(format!("{kind} count is no exact {unit}")))
}

fn narrow_i32(count: i64, kind: &'static str) -> Result<i32> {
    i32::try_from(count).map_err(|_| invalid(format!("{kind} count must fit signed 32 bits")))
}

fn invalid(reason: impl Into<smol_str::SmolStr>) -> Error {
    Error::InvalidRecord {
        path: "$".into(),
        reason: reason.into(),
    }
}

fn require(valid: bool, reason: &'static str) -> Result<()> {
    if valid { Ok(()) } else { Err(invalid(reason)) }
}

const fn nanoseconds_per(unit: TimeUnit) -> Option<i128> {
    match unit {
        TimeUnit::Day => Some(86_400_000_000_000),
        TimeUnit::Second => Some(1_000_000_000),
        TimeUnit::Millisecond => Some(1_000_000),
        TimeUnit::Microsecond => Some(1_000),
        TimeUnit::Nanosecond => Some(1),
        TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => None,
    }
}

pub(crate) fn temporal_key(count: i64, unit: TimeUnit) -> (u8, i128) {
    let count = i128::from(count);
    match unit {
        TimeUnit::Day => (0, count * 86_400_000_000_000),
        TimeUnit::Second => (0, count * 1_000_000_000),
        TimeUnit::Millisecond => (0, count * 1_000_000),
        TimeUnit::Microsecond => (0, count * 1_000),
        TimeUnit::Nanosecond => (0, count),
        TimeUnit::YearMonth => (1, count),
        TimeUnit::DayTime => (2, count),
        TimeUnit::MonthDayNano => (3, count),
    }
}

pub(crate) fn validate_date64(value: &Scalar) -> std::result::Result<(), ValidationFailure> {
    const MILLIS_PER_DAY: i128 = 86_400_000;
    let Some(number) = value.as_i128() else {
        return Err(expected("date64 whole-day milliseconds", value));
    };
    if i64::try_from(number).is_err() || number % MILLIS_PER_DAY != 0 {
        return Err(ValidationFailure::new(
            "date64 must be signed 64-bit whole-day milliseconds",
        ));
    }
    Ok(())
}

pub(crate) fn validate_time(
    value: &Scalar,
    unit: TimeUnit,
) -> std::result::Result<(), ValidationFailure> {
    let maximum = match unit {
        TimeUnit::Second => 86_400_i128,
        TimeUnit::Millisecond => 86_400_000_i128,
        TimeUnit::Microsecond => 86_400_000_000_i128,
        TimeUnit::Nanosecond => 86_400_000_000_000_i128,
        _ => return Err(ValidationFailure::new("invalid time-of-day unit")),
    };
    let Some(number) = value.as_i128() else {
        return Err(expected("time-of-day integer", value));
    };
    if !(0..maximum).contains(&number) {
        return Err(ValidationFailure::new(format_smolstr!(
            "time-of-day value must be in 0..{maximum} for {unit}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_reject_illegal_width_unit_combinations() {
        assert!(Scalar::time32(1, TimeUnit::Microsecond, Timezone::NAIVE).is_err());
        assert!(Scalar::time64(1, TimeUnit::Millisecond, Timezone::NAIVE).is_err());
        assert!(Scalar::duration32(1, TimeUnit::DayTime).is_err());
        assert!(Scalar::duration64(1, TimeUnit::DayTime).is_err());
        assert!(Scalar::duration32_in(1, TimeUnit::Second, Timezone::UTC).is_err());
        assert!(Scalar::duration64_in(1, TimeUnit::Second, Timezone::UTC).is_err());
        assert!(Scalar::date32_in(1, TimeUnit::DayTime, Timezone::NAIVE).is_err());
        assert!(Scalar::date64_in(1, TimeUnit::Second, Timezone::NAIVE).is_err());
    }

    #[test]
    fn time_of_day_refuses_a_zone_its_datatype_would_lose() {
        for error in [
            Scalar::time32(1, TimeUnit::Second, Timezone::UTC).unwrap_err(),
            Scalar::time64(1, TimeUnit::Microsecond, Timezone::UTC).unwrap_err(),
        ] {
            let message = error.to_string();
            assert!(message.contains("timezone"), "{message}");
            assert!(message.contains("no timezone"), "{message}");
        }
    }

    #[test]
    fn every_temporal_carries_a_zone() {
        let values = [
            Scalar::date32(1),
            Scalar::date64(86_400_000),
            Scalar::time32(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::time64(1, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
            Scalar::datetime64(1, TimeUnit::Nanosecond, Timezone::UTC).unwrap(),
            Scalar::duration32(1, TimeUnit::Millisecond).unwrap(),
            Scalar::duration64(1, TimeUnit::Microsecond).unwrap(),
        ];
        assert!(values.iter().all(Scalar::is_temporal));
    }

    #[test]
    fn family_constructors_select_exact_widths() {
        let values = [
            Scalar::from_date(1, TimeUnit::Day, Timezone::NAIVE).unwrap(),
            Scalar::from_date(86_400_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
            Scalar::from_time(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::from_time(1, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
            Scalar::from_datetime(1, TimeUnit::Microsecond, Timezone::UTC).unwrap(),
            Scalar::from_duration(i64::from(i32::MAX), TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::from_duration(i64::from(i32::MAX) + 1, TimeUnit::Second, Timezone::NAIVE)
                .unwrap(),
        ];
        assert!(matches!(values[0], Scalar::Temporal(Temporal::Date32(_))));
        assert!(matches!(values[1], Scalar::Temporal(Temporal::Date64(_))));
        assert!(matches!(values[2], Scalar::Temporal(Temporal::Time32(_))));
        assert!(matches!(values[3], Scalar::Temporal(Temporal::Time64(_))));
        assert!(matches!(
            values[4],
            Scalar::Temporal(Temporal::DateTime64(_))
        ));
        assert!(matches!(
            values[5],
            Scalar::Temporal(Temporal::Duration32(_))
        ));
        assert!(matches!(
            values[6],
            Scalar::Temporal(Temporal::Duration64(_))
        ));

        for count in [i64::from(i32::MIN), i64::from(i32::MAX)] {
            assert!(matches!(
                Scalar::from_duration(count, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
                Scalar::Temporal(Temporal::Duration32(_))
            ));
        }
        for count in [i64::from(i32::MIN) - 1, i64::from(i32::MAX) + 1] {
            assert!(matches!(
                Scalar::from_duration(count, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
                Scalar::Temporal(Temporal::Duration64(_))
            ));
        }
    }

    #[test]
    fn temporal_family_views_are_exact_and_reversible() {
        let values = [
            Scalar::date32(1),
            Scalar::date64(86_400_000),
            Scalar::time32(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::time64(1, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
            Scalar::datetime64(1, TimeUnit::Nanosecond, Timezone::UTC).unwrap(),
            Scalar::duration32(1, TimeUnit::Millisecond).unwrap(),
            Scalar::duration64(1, TimeUnit::Microsecond).unwrap(),
        ];
        for value in &values {
            let temporal = value.as_temporal().unwrap();
            assert_eq!(&Scalar::Temporal(*temporal), value);
            assert_eq!(temporal.timezone(), value.temporal_timezone().unwrap());
        }
        assert_eq!(values[0].as_date().unwrap().bit_width(), 32);
        assert_eq!(values[1].as_date().unwrap().bit_width(), 64);
        assert!(values[2].as_time().is_some());
        assert!(values[4].as_datetime().is_some());
        assert!(values[5].as_duration().is_some());
        assert!(Scalar::from(1).as_temporal().is_none());
    }

    #[test]
    fn family_constructors_reject_invalid_parts() {
        assert!(Scalar::from_date(1, TimeUnit::Second, Timezone::NAIVE).is_err());
        assert!(Scalar::from_date(i64::MAX, TimeUnit::Day, Timezone::NAIVE).is_err());
        assert!(Scalar::from_time(1, TimeUnit::Day, Timezone::NAIVE).is_err());
        assert!(Scalar::from_time(1, TimeUnit::Second, Timezone::UTC).is_err());
        assert!(Scalar::from_datetime(1, TimeUnit::Day, Timezone::NAIVE).is_err());
        assert!(Scalar::from_duration(1, TimeUnit::DayTime, Timezone::NAIVE).is_err());
        assert!(Scalar::from_duration(1, TimeUnit::Second, Timezone::UTC).is_err());
    }
}

#[derive(Clone)]
pub(crate) struct TemporalParts {
    pub(crate) family: TemporalFamily,
    pub(crate) unit: TimeUnit,
    pub(crate) zone: Timezone,
    pub(crate) dtype: DataType,
}

pub(crate) fn temporal_value_parts(value: &Scalar) -> Option<TemporalParts> {
    let temporal = value.as_temporal()?;
    let unit = temporal.unit();
    let zone = temporal.timezone();
    let dtype = match (temporal.family(), temporal.bit_width()) {
        (TemporalFamily::Date, 32) => DataType::Date32,
        (TemporalFamily::Date, 64) => DataType::Date64,
        (TemporalFamily::Time, 32) => DataType::Time32(unit),
        (TemporalFamily::Time, 64) => DataType::Time64(unit),
        (TemporalFamily::DateTime, 64) => DataType::DateTime64 {
            unit,
            timezone: zone,
        },
        (TemporalFamily::Duration, 32) => DataType::Duration32(unit),
        (TemporalFamily::Duration, 64) => DataType::Duration64(unit),
        _ => return None,
    };
    Some(TemporalParts {
        family: temporal.family(),
        unit,
        zone,
        dtype,
    })
}

pub(crate) fn temporal_target(dtype: &DataType) -> Option<(TemporalFamily, TimeUnit)> {
    match dtype {
        DataType::Date32 => Some((TemporalFamily::Date, TimeUnit::Day)),
        DataType::Date64 => Some((TemporalFamily::Date, TimeUnit::Millisecond)),
        DataType::Time32(unit) | DataType::Time64(unit) => Some((TemporalFamily::Time, *unit)),
        DataType::DateTime64 { unit, .. } => Some((TemporalFamily::DateTime, *unit)),
        DataType::Duration32(unit) | DataType::Duration64(unit) => {
            Some((TemporalFamily::Duration, *unit))
        }
        _ => None,
    }
}

pub(crate) fn temporal_result_type(
    left: &Scalar,
    left_parts: TemporalParts,
    operation: Arithmetic,
    right: &Scalar,
    right_parts: TemporalParts,
) -> Result<DataType> {
    match (left_parts.family, right_parts.family, operation) {
        (family, TemporalFamily::Duration, Arithmetic::Add | Arithmetic::Sub)
            if family != TemporalFamily::Duration =>
        {
            Ok(left_parts.dtype)
        }
        (TemporalFamily::Duration, family, Arithmetic::Add)
            if family != TemporalFamily::Duration =>
        {
            Ok(right_parts.dtype)
        }
        (family, other, Arithmetic::Sub)
            if family == other && family != TemporalFamily::Duration =>
        {
            if left_parts.zone.is_naive() != right_parts.zone.is_naive() {
                return Err(invalid_binary(
                    operation,
                    left,
                    right,
                    "zoned and timezone-naive temporal values cannot be subtracted",
                ));
            }
            let unit = finer_unit(left_parts.unit, right_parts.unit);
            DataType::duration64(unit)
                .map_err(|error| invalid_binary(operation, left, right, error.to_string()))
        }
        (TemporalFamily::Duration, TemporalFamily::Duration, Arithmetic::Add | Arithmetic::Sub) => {
            let unit = finer_unit(left_parts.unit, right_parts.unit);
            let wide = matches!(left_parts.dtype, DataType::Duration64(_))
                || matches!(right_parts.dtype, DataType::Duration64(_));
            if wide {
                DataType::duration64(unit)
            } else {
                DataType::duration32(unit)
            }
            .map_err(|error| invalid_binary(operation, left, right, error.to_string()))
        }
        _ => Err(invalid_binary(
            operation,
            left,
            right,
            "temporal arithmetic supports temporal +/- duration, temporal subtraction, and duration +/- duration",
        )),
    }
}

fn finer_unit(left: TimeUnit, right: TimeUnit) -> TimeUnit {
    if unit_rank(left) >= unit_rank(right) {
        left
    } else {
        right
    }
}

const fn unit_rank(unit: TimeUnit) -> u8 {
    match unit {
        TimeUnit::Day => 0,
        TimeUnit::Second => 1,
        TimeUnit::Millisecond => 2,
        TimeUnit::Microsecond => 3,
        TimeUnit::Nanosecond => 4,
        TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => 5,
    }
}

pub(crate) fn temporal_arithmetic(
    left: &Scalar,
    operation: Arithmetic,
    right: &Scalar,
    target: &DataType,
) -> Result<Scalar> {
    if !matches!(operation, Arithmetic::Add | Arithmetic::Sub) {
        return Err(invalid_binary(
            operation,
            left,
            right,
            "temporal multiplication, division, and remainder are undefined",
        ));
    }
    let left_parts = temporal_value_parts(left)
        .ok_or_else(|| invalid_binary(operation, left, right, "left operand is not temporal"))?;
    let right_parts = temporal_value_parts(right)
        .ok_or_else(|| invalid_binary(operation, left, right, "right operand is not temporal"))?;
    let (target_family, unit) = temporal_target(target).ok_or_else(|| {
        invalid_binary(
            operation,
            left,
            right,
            "the promoted datatype is not temporal",
        )
    })?;
    let (left_count, right_count) = match (left_parts.family, right_parts.family, operation) {
        (family, TemporalFamily::Duration, Arithmetic::Add | Arithmetic::Sub)
            if family == target_family =>
        {
            (
                temporal_at(left, unit, operation, right)?,
                temporal_at(right, unit, operation, left)?,
            )
        }
        (TemporalFamily::Duration, family, Arithmetic::Add) if family == target_family => (
            temporal_at(right, unit, operation, left)?,
            temporal_at(left, unit, operation, right)?,
        ),
        (family, other, Arithmetic::Sub)
            if family == other
                && target_family == TemporalFamily::Duration
                && family != TemporalFamily::Duration =>
        {
            if left_parts.zone.is_naive() != right_parts.zone.is_naive() {
                return Err(invalid_binary(
                    operation,
                    left,
                    right,
                    "zoned and timezone-naive temporal values cannot be subtracted",
                ));
            }
            (
                temporal_at(left, unit, operation, right)?,
                temporal_at(right, unit, operation, left)?,
            )
        }
        (TemporalFamily::Duration, TemporalFamily::Duration, Arithmetic::Add | Arithmetic::Sub)
            if target_family == TemporalFamily::Duration =>
        {
            (
                temporal_at(left, unit, operation, right)?,
                temporal_at(right, unit, operation, left)?,
            )
        }
        _ => {
            return Err(invalid_binary(
                operation,
                left,
                right,
                "operands do not match the promoted temporal result",
            ));
        }
    };
    let held = match operation {
        Arithmetic::Add => left_count.checked_add(right_count),
        Arithmetic::Sub => left_count.checked_sub(right_count),
        _ => {
            return Err(invalid_binary(
                operation,
                left,
                right,
                "temporal multiplication, division, and remainder are undefined",
            ));
        }
    }
    .ok_or_else(|| Error::ArithmeticOverflow {
        operation: operation.name(),
        kind: temporal_kind_name(target),
    })?;
    temporal_value(target, held, unit).map_err(|_| Error::ArithmeticOverflow {
        operation: operation.name(),
        kind: temporal_kind_name(target),
    })
}

pub(crate) fn duration_integer_arithmetic(
    left: &Scalar,
    operation: Arithmetic,
    right: &Scalar,
    target: &DataType,
) -> Result<Scalar> {
    let (duration, integer, duration_first) = if temporal_value_parts(left)
        .is_some_and(|parts| parts.family == TemporalFamily::Duration)
        && right.as_integer().is_some()
    {
        (left, right, true)
    } else if left.as_integer().is_some()
        && temporal_value_parts(right).is_some_and(|parts| parts.family == TemporalFamily::Duration)
    {
        (right, left, false)
    } else {
        return Err(invalid_binary(
            operation,
            left,
            right,
            "expected one duration and one integer",
        ));
    };
    if !(matches!(operation, Arithmetic::Mul)
        || duration_first && matches!(operation, Arithmetic::Div))
    {
        return Err(invalid_binary(
            operation,
            left,
            right,
            "durations support multiplication by an integer and exact division by an integer",
        ));
    }

    let parts = temporal_value_parts(duration).ok_or_else(|| {
        invalid_binary(operation, left, right, "duration operand is not temporal")
    })?;
    let count = duration
        .temporal_count_at(parts.unit)
        .map(|value| I256::from_i128(i128::from(value)))
        .ok_or_else(|| invalid_binary(operation, left, right, "invalid duration count"))?;
    let scalar = exact_value_parts(integer)
        .map(|parts| parts.0)
        .ok_or_else(|| invalid_binary(operation, left, right, "integer is out of range"))?;
    if scalar.is_zero() && matches!(operation, Arithmetic::Div) {
        return Err(Error::DivisionByZero {
            operation: operation.name(),
        });
    }
    let held = match operation {
        Arithmetic::Mul => count.checked_mul(scalar),
        Arithmetic::Div => {
            if !count
                .checked_rem(scalar)
                .is_some_and(|remainder| remainder.is_zero())
            {
                return Err(Error::InexactArithmetic {
                    operation: operation.name(),
                    kind: temporal_kind_name(target),
                });
            }
            count.checked_div(scalar)
        }
        _ => {
            return Err(invalid_binary(
                operation,
                left,
                right,
                "durations support multiplication or exact division by an integer",
            ));
        }
    }
    .and_then(I256::as_i128)
    .and_then(|value| i64::try_from(value).ok())
    .ok_or_else(|| Error::ArithmeticOverflow {
        operation: operation.name(),
        kind: temporal_kind_name(target),
    })?;
    temporal_value(target, held, parts.unit).map_err(|_| Error::ArithmeticOverflow {
        operation: operation.name(),
        kind: temporal_kind_name(target),
    })
}

fn temporal_at(
    value: &Scalar,
    unit: TimeUnit,
    operation: Arithmetic,
    other: &Scalar,
) -> Result<i64> {
    value.temporal_count_at(unit).ok_or_else(|| {
        invalid_binary(
            operation,
            value,
            other,
            "temporal unit conversion is inexact or out of range",
        )
    })
}

fn temporal_value(dtype: &DataType, count: i64, unit: TimeUnit) -> Result<Scalar> {
    match dtype {
        DataType::Date32 => Scalar::date32_in(
            i32::try_from(count).map_err(|_| Error::ArithmeticOverflow {
                operation: "temporal arithmetic",
                kind: "date32",
            })?,
            unit,
            Timezone::NAIVE,
        ),
        DataType::Date64 => Scalar::date64_in(count, unit, Timezone::NAIVE),
        DataType::Time32(expected) if *expected == unit => Scalar::time32(
            i32::try_from(count).map_err(|_| Error::ArithmeticOverflow {
                operation: "temporal arithmetic",
                kind: "time32",
            })?,
            unit,
            Timezone::NAIVE,
        ),
        DataType::Time64(expected) if *expected == unit => {
            Scalar::time64(count, unit, Timezone::NAIVE)
        }
        DataType::DateTime64 {
            unit: expected,
            timezone,
        } if *expected == unit => Scalar::datetime64(count, unit, *timezone),
        DataType::Duration32(expected) if *expected == unit => Scalar::duration32(
            i32::try_from(count).map_err(|_| Error::ArithmeticOverflow {
                operation: "temporal arithmetic",
                kind: "duration32",
            })?,
            unit,
        ),
        DataType::Duration64(expected) if *expected == unit => Scalar::duration64(count, unit),
        _ => Err(Error::InvalidArithmetic {
            operation: "temporal arithmetic",
            left: temporal_kind_name(dtype),
            right: None,
            reason: "invalid result unit".into(),
        }),
    }
}

const fn temporal_kind_name(dtype: &DataType) -> &'static str {
    match dtype {
        DataType::Date32 => "date32",
        DataType::Date64 => "date64",
        DataType::Time32(_) => "time32",
        DataType::Time64(_) => "time64",
        DataType::DateTime64 { .. } => "datetime64",
        DataType::Duration32(_) => "duration32",
        DataType::Duration64(_) => "duration64",
        _ => "temporal",
    }
}
