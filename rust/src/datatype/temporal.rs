//! Temporal units and validated time-of-day construction.

use crate::Result;
use crate::enums::TimeUnit;

use super::DataType;
use super::scalar::invalid;

impl DataType {
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
            TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => {
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
}

pub(super) fn validate_time32_unit(unit: TimeUnit) -> Result<()> {
    if matches!(unit, TimeUnit::Second | TimeUnit::Millisecond) {
        Ok(())
    } else {
        Err(invalid("Time32", "unit must be second or millisecond"))
    }
}

pub(super) fn validate_time64_unit(unit: TimeUnit) -> Result<()> {
    if matches!(unit, TimeUnit::Microsecond | TimeUnit::Nanosecond) {
        Ok(())
    } else {
        Err(invalid("Time64", "unit must be microsecond or nanosecond"))
    }
}
