//! Width-accurate temporal values with explicit units and zones.
//!
//! ```
//! use yggdryl::{TimeUnit, Timezone, Scalar};
//!
//! let day = Scalar::date32(20_000);
//! let at = Scalar::datetime64(1, TimeUnit::Microsecond, Timezone::UTC)?;
//! assert_eq!(day.as_date32().map(|parts| parts.1), Some(TimeUnit::Day));
//! assert_eq!(at.temporal_timezone(), Some(&Timezone::UTC));
//! # Ok::<(), yggdryl::Error>(())
//! ```

use super::scalar::Scalar;
use crate::{DataType, Error, Result, TimeUnit, Timezone};

impl Scalar {
    /// Build a Date32 day count.
    pub const fn date32(days: i32) -> Self {
        Self::Date32(days, TimeUnit::Day, Timezone::NAIVE)
    }

    /// Build a Date32 after validating its unit and zone.
    pub fn date32_in(days: i32, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(unit == TimeUnit::Day, "date32 unit must be day")?;
        require(zone.is_naive(), "date32 timezone must be NAIVE")?;
        Ok(Self::Date32(days, unit, zone))
    }

    /// Build a Date64 millisecond count.
    pub const fn date64(milliseconds: i64) -> Self {
        Self::Date64(milliseconds, TimeUnit::Millisecond, Timezone::NAIVE)
    }

    /// Build a Date64 after validating its unit and zone.
    pub fn date64_in(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            unit == TimeUnit::Millisecond,
            "date64 unit must be millisecond",
        )?;
        require(zone.is_naive(), "date64 timezone must be NAIVE")?;
        Ok(Self::Date64(count, unit, zone))
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
        Ok(Self::Time32(count, unit, zone))
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
        Ok(Self::Time64(count, unit, zone))
    }

    /// Build a timestamp or wall-clock datetime at 64-bit width.
    pub fn datetime64(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            unit.is_arrow_time(),
            "datetime64 requires an Arrow time unit",
        )?;
        Ok(Self::DateTime64(count, unit, zone))
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
        Ok(Self::Duration32(count, unit, zone))
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
        Ok(Self::Duration64(count, unit, zone))
    }

    /// Return Date32's count, unit, and zone.
    pub const fn as_date32(&self) -> Option<(i32, TimeUnit, &Timezone)> {
        match self {
            Self::Date32(count, unit, zone) => Some((*count, *unit, zone)),
            _ => None,
        }
    }

    /// Return Date64's count, unit, and zone.
    pub const fn as_date64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
        match self {
            Self::Date64(count, unit, zone) => Some((*count, *unit, zone)),
            _ => None,
        }
    }

    /// Return Time32's count, unit, and zone.
    pub const fn as_time32(&self) -> Option<(i32, TimeUnit, &Timezone)> {
        match self {
            Self::Time32(count, unit, zone) => Some((*count, *unit, zone)),
            _ => None,
        }
    }

    /// Return Time64's count, unit, and zone.
    pub const fn as_time64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
        match self {
            Self::Time64(count, unit, zone) => Some((*count, *unit, zone)),
            _ => None,
        }
    }

    /// Return DateTime64's count, unit, and zone.
    pub const fn as_datetime64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
        match self {
            Self::DateTime64(count, unit, zone) => Some((*count, *unit, zone)),
            _ => None,
        }
    }

    /// Return Duration32's count, unit, and zone.
    pub const fn as_duration32(&self) -> Option<(i32, TimeUnit, &Timezone)> {
        match self {
            Self::Duration32(count, unit, zone) => Some((*count, *unit, zone)),
            _ => None,
        }
    }

    /// Return Duration64's count, unit, and zone.
    pub const fn as_duration64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
        match self {
            Self::Duration64(count, unit, zone) => Some((*count, *unit, zone)),
            _ => None,
        }
    }

    /// Return the non-optional timezone carried by any temporal.
    pub const fn temporal_timezone(&self) -> Option<&Timezone> {
        match self {
            Self::Date32(_, _, zone)
            | Self::Date64(_, _, zone)
            | Self::Time32(_, _, zone)
            | Self::Time64(_, _, zone)
            | Self::DateTime64(_, _, zone)
            | Self::Duration32(_, _, zone)
            | Self::Duration64(_, _, zone) => Some(zone),
            _ => None,
        }
    }

    /// Return this temporal's count restated in `unit`, when exact.
    pub fn temporal_count_at(&self, unit: TimeUnit) -> Option<i64> {
        let (count, current) = match self {
            Self::Date32(count, current, _)
            | Self::Time32(count, current, _)
            | Self::Duration32(count, current, _) => (i64::from(*count), *current),
            Self::Date64(count, current, _)
            | Self::Time64(count, current, _)
            | Self::DateTime64(count, current, _)
            | Self::Duration64(count, current, _) => (*count, *current),
            _ => return None,
        };
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
        matches!(
            self,
            Self::Date32(..)
                | Self::Date64(..)
                | Self::Time32(..)
                | Self::Time64(..)
                | Self::DateTime64(..)
                | Self::Duration32(..)
                | Self::Duration64(..)
        )
    }

    /// Return the datatype this temporal materializes into.
    pub fn temporal_data_type(&self) -> Option<DataType> {
        self.is_temporal().then(|| self.data_type().ok()).flatten()
    }
}

fn require(valid: bool, reason: &'static str) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidRecord {
            path: "$".into(),
            reason: reason.into(),
        })
    }
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

pub(super) fn temporal_key(count: i64, unit: TimeUnit) -> (u8, i128) {
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
}
