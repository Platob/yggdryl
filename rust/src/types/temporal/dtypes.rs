//! Temporal units and validated time-of-day construction.

use crate::types::invalid;
use crate::{DataType, DataTypeId, Error, Result, TimeUnit, Timezone};
use smol_str::format_smolstr;

/// One temporal or interval datatype.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum TemporalType {
    /// Day-count date.
    Date32,
    /// Millisecond-count date.
    Date64,
    /// 32-bit time of day.
    Time32(TimeUnit),
    /// 64-bit time of day.
    Time64(TimeUnit),
    /// 64-bit datetime with an explicit timezone.
    DateTime64 { unit: TimeUnit, timezone: Timezone },
    /// 32-bit duration.
    Duration32(TimeUnit),
    /// 64-bit duration.
    Duration64(TimeUnit),
    /// Calendar interval layout.
    Interval(TimeUnit),
}

impl TemporalType {
    /// Return the exact datatype identifier.
    pub const fn id(self) -> DataTypeId {
        match self {
            Self::Date32 => DataTypeId::Date32,
            Self::Date64 => DataTypeId::Date64,
            Self::Time32(_) => DataTypeId::Time32,
            Self::Time64(_) => DataTypeId::Time64,
            Self::DateTime64 { .. } => DataTypeId::DateTime64,
            Self::Duration32(_) => DataTypeId::Duration32,
            Self::Duration64(_) => DataTypeId::Duration64,
            Self::Interval(_) => DataTypeId::Interval,
        }
    }

    /// Validate and convert this family member into the root datatype.
    pub fn into_dtype(self) -> Result<DataType> {
        match self {
            Self::Date32 => Ok(DataType::Date32),
            Self::Date64 => Ok(DataType::Date64),
            Self::Time32(unit) => DataType::time32(unit),
            Self::Time64(unit) => DataType::time64(unit),
            Self::DateTime64 { unit, timezone } => DataType::datetime64(unit, timezone),
            Self::Duration32(unit) => DataType::duration32(unit),
            Self::Duration64(unit) => DataType::duration64(unit),
            Self::Interval(unit) if unit.is_interval() => Ok(DataType::Interval(unit)),
            Self::Interval(_) => Err(invalid(
                "Interval",
                "unit must be year_month, day_time, or month_day_nano",
            )),
        }
    }
}

impl From<TemporalType> for DataType {
    fn from(value: TemporalType) -> Self {
        match value {
            TemporalType::Date32 => Self::Date32,
            TemporalType::Date64 => Self::Date64,
            TemporalType::Time32(unit) => Self::Time32(unit),
            TemporalType::Time64(unit) => Self::Time64(unit),
            TemporalType::DateTime64 { unit, timezone } => Self::DateTime64 { unit, timezone },
            TemporalType::Duration32(unit) => Self::Duration32(unit),
            TemporalType::Duration64(unit) => Self::Duration64(unit),
            TemporalType::Interval(unit) => Self::Interval(unit),
        }
    }
}

impl TryFrom<&DataType> for TemporalType {
    type Error = Error;

    fn try_from(value: &DataType) -> Result<Self> {
        match value {
            DataType::Date32 => Ok(Self::Date32),
            DataType::Date64 => Ok(Self::Date64),
            DataType::Time32(unit) => Ok(Self::Time32(*unit)),
            DataType::Time64(unit) => Ok(Self::Time64(*unit)),
            DataType::DateTime64 { unit, timezone } => Ok(Self::DateTime64 {
                unit: *unit,
                timezone: *timezone,
            }),
            DataType::Duration32(unit) => Ok(Self::Duration32(*unit)),
            DataType::Duration64(unit) => Ok(Self::Duration64(*unit)),
            DataType::Interval(unit) => Ok(Self::Interval(*unit)),
            other => Err(Error::InvalidDataType {
                kind: "temporal",
                reason: format_smolstr!("expected a temporal datatype, got {other}"),
            }),
        }
    }
}

impl DataType {
    /// Creates a 64-bit datetime with an explicit timezone marker.
    ///
    /// Use [`Timezone::NAIVE`] for a wall-clock column without timezone
    /// interpretation.
    pub fn datetime64(unit: TimeUnit, timezone: Timezone) -> Result<Self> {
        if !unit.is_arrow_time() {
            return Err(invalid(
                "DateTime64",
                "unit must be second, millisecond, microsecond, or nanosecond",
            ));
        }
        Ok(Self::DateTime64 { unit, timezone })
    }

    /// Creates the Arrow time-of-day type selected by the requested unit.
    ///
    /// Seconds and milliseconds use [`Self::Time32`], while microseconds and
    /// nanoseconds use [`Self::Time64`]. Validation is delegated to
    /// [`Self::time32`] or [`Self::time64`]. Calendar interval layouts are not
    /// time-of-day resolutions and return an error.
    pub fn time(unit: TimeUnit) -> Result<Self> {
        match unit {
            TimeUnit::Second | TimeUnit::Millisecond => Self::time32(unit),
            TimeUnit::Microsecond | TimeUnit::Nanosecond => Self::time64(unit),
            TimeUnit::Day | TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => {
                Err(invalid("Time", "unit must be a temporal resolution"))
            }
        }
    }

    /// Creates a 32-bit time-of-day type with a valid physical unit.
    pub fn time32(unit: TimeUnit) -> Result<Self> {
        validate_time32_unit(unit)?;
        Ok(Self::Time32(unit))
    }

    /// Creates a 64-bit time-of-day type with a valid physical unit.
    pub fn time64(unit: TimeUnit) -> Result<Self> {
        validate_time64_unit(unit)?;
        Ok(Self::Time64(unit))
    }

    /// Creates a 32-bit elapsed-time type with a fixed-length resolution.
    pub fn duration32(unit: TimeUnit) -> Result<Self> {
        validate_duration_unit("Duration32", unit)?;
        Ok(Self::Duration32(unit))
    }

    /// Creates a 64-bit elapsed-time type with a fixed-length resolution.
    ///
    /// Arrow durations are physically 64-bit at every resolution, so Arrow
    /// import always selects this variant.
    pub fn duration64(unit: TimeUnit) -> Result<Self> {
        validate_duration_unit("Duration64", unit)?;
        Ok(Self::Duration64(unit))
    }
}

pub(crate) fn validate_time32_unit(unit: TimeUnit) -> Result<()> {
    if matches!(unit, TimeUnit::Second | TimeUnit::Millisecond) {
        Ok(())
    } else {
        Err(invalid("Time32", "unit must be second or millisecond"))
    }
}

pub(crate) fn validate_time64_unit(unit: TimeUnit) -> Result<()> {
    if matches!(unit, TimeUnit::Microsecond | TimeUnit::Nanosecond) {
        Ok(())
    } else {
        Err(invalid("Time64", "unit must be microsecond or nanosecond"))
    }
}

pub(crate) fn validate_duration_unit(kind: &'static str, unit: TimeUnit) -> Result<()> {
    if unit.is_temporal() {
        Ok(())
    } else {
        Err(invalid(
            kind,
            "unit must be day, second, millisecond, microsecond, or nanosecond",
        ))
    }
}
