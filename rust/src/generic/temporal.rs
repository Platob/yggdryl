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

use super::scalar::Scalar;
use crate::{DataType, Error, Result, TimeUnit, Timezone};

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
}

impl TemporalFamily {
    /// Return the canonical family name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Date => "date",
            Self::Time => "time",
            Self::DateTime => "datetime",
            Self::Duration => "duration",
        }
    }
}

/// A borrowed, width-aware view over any temporal [`Scalar`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TemporalRef<'a> {
    family: TemporalFamily,
    count: i64,
    unit: TimeUnit,
    timezone: &'a Timezone,
    bit_width: u8,
}

impl<'a> TemporalRef<'a> {
    const fn new(
        family: TemporalFamily,
        count: i64,
        unit: TimeUnit,
        timezone: &'a Timezone,
        bit_width: u8,
    ) -> Self {
        Self {
            family,
            count,
            unit,
            timezone,
            bit_width,
        }
    }

    /// Return the logical temporal family.
    pub const fn family(self) -> TemporalFamily {
        self.family
    }

    /// Return the stored physical count.
    pub const fn count(self) -> i64 {
        self.count
    }

    /// Return the stored resolution.
    pub const fn unit(self) -> TimeUnit {
        self.unit
    }

    /// Return the explicit timezone marker.
    pub const fn timezone(self) -> &'a Timezone {
        self.timezone
    }

    /// Return the physical count width.
    pub const fn bit_width(self) -> u8 {
        self.bit_width
    }

    /// Return the deterministic hash of the complete exact view.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }

    /// Rebuild and validate the exact scalar variant this view came from.
    pub fn into_scalar(self) -> Result<Scalar> {
        match (self.family, self.bit_width) {
            (TemporalFamily::Date, 32) => Scalar::date32_in(
                narrow_i32(self.count, "date32")?,
                self.unit,
                self.timezone.clone(),
            ),
            (TemporalFamily::Date, 64) => {
                Scalar::date64_in(self.count, self.unit, self.timezone.clone())
            }
            (TemporalFamily::Time, 32) => Scalar::time32(
                narrow_i32(self.count, "time32")?,
                self.unit,
                self.timezone.clone(),
            ),
            (TemporalFamily::Time, 64) => {
                Scalar::time64(self.count, self.unit, self.timezone.clone())
            }
            (TemporalFamily::DateTime, 64) => {
                Scalar::datetime64(self.count, self.unit, self.timezone.clone())
            }
            (TemporalFamily::Duration, 32) => Scalar::duration32_in(
                narrow_i32(self.count, "duration32")?,
                self.unit,
                self.timezone.clone(),
            ),
            (TemporalFamily::Duration, 64) => {
                Scalar::duration64_in(self.count, self.unit, self.timezone.clone())
            }
            _ => Err(invalid("invalid temporal family width")),
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

    /// Borrow the shared family view of any temporal value.
    pub const fn as_temporal(&self) -> Option<TemporalRef<'_>> {
        match self {
            Self::Date32(count, unit, zone) => Some(TemporalRef::new(
                TemporalFamily::Date,
                *count as i64,
                *unit,
                zone,
                32,
            )),
            Self::Date64(count, unit, zone) => Some(TemporalRef::new(
                TemporalFamily::Date,
                *count,
                *unit,
                zone,
                64,
            )),
            Self::Time32(count, unit, zone) => Some(TemporalRef::new(
                TemporalFamily::Time,
                *count as i64,
                *unit,
                zone,
                32,
            )),
            Self::Time64(count, unit, zone) => Some(TemporalRef::new(
                TemporalFamily::Time,
                *count,
                *unit,
                zone,
                64,
            )),
            Self::DateTime64(count, unit, zone) => Some(TemporalRef::new(
                TemporalFamily::DateTime,
                *count,
                *unit,
                zone,
                64,
            )),
            Self::Duration32(count, unit, zone) => Some(TemporalRef::new(
                TemporalFamily::Duration,
                *count as i64,
                *unit,
                zone,
                32,
            )),
            Self::Duration64(count, unit, zone) => Some(TemporalRef::new(
                TemporalFamily::Duration,
                *count,
                *unit,
                zone,
                64,
            )),
            _ => None,
        }
    }

    /// Borrow this value as either exact date width.
    pub const fn as_date(&self) -> Option<TemporalRef<'_>> {
        match self.as_temporal() {
            Some(value) if matches!(value.family(), TemporalFamily::Date) => Some(value),
            _ => None,
        }
    }

    /// Borrow this value as either exact time-of-day width.
    pub const fn as_time(&self) -> Option<TemporalRef<'_>> {
        match self.as_temporal() {
            Some(value) if matches!(value.family(), TemporalFamily::Time) => Some(value),
            _ => None,
        }
    }

    /// Borrow this value as a datetime without exposing its physical suffix.
    pub const fn as_datetime(&self) -> Option<TemporalRef<'_>> {
        match self.as_temporal() {
            Some(value) if matches!(value.family(), TemporalFamily::DateTime) => Some(value),
            _ => None,
        }
    }

    /// Borrow this value as either exact duration width.
    pub const fn as_duration(&self) -> Option<TemporalRef<'_>> {
        match self.as_temporal() {
            Some(value) if matches!(value.family(), TemporalFamily::Duration) => Some(value),
            _ => None,
        }
    }

    /// Return the non-optional timezone carried by any temporal.
    pub const fn temporal_timezone(&self) -> Option<&Timezone> {
        match self.as_temporal() {
            Some(value) => Some(value.timezone()),
            None => None,
        }
    }

    /// Return this temporal's count restated in `unit`, when exact.
    pub fn temporal_count_at(&self, unit: TimeUnit) -> Option<i64> {
        let temporal = self.as_temporal()?;
        let (count, current) = (temporal.count(), temporal.unit());
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
    pub fn temporal_data_type(&self) -> Option<DataType> {
        self.is_temporal().then(|| self.data_type().ok()).flatten()
    }
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
        assert!(matches!(values[0], Scalar::Date32(..)));
        assert!(matches!(values[1], Scalar::Date64(..)));
        assert!(matches!(values[2], Scalar::Time32(..)));
        assert!(matches!(values[3], Scalar::Time64(..)));
        assert!(matches!(values[4], Scalar::DateTime64(..)));
        assert!(matches!(values[5], Scalar::Duration32(..)));
        assert!(matches!(values[6], Scalar::Duration64(..)));

        for count in [i64::from(i32::MIN), i64::from(i32::MAX)] {
            assert!(matches!(
                Scalar::from_duration(count, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
                Scalar::Duration32(..)
            ));
        }
        for count in [i64::from(i32::MIN) - 1, i64::from(i32::MAX) + 1] {
            assert!(matches!(
                Scalar::from_duration(count, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
                Scalar::Duration64(..)
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
            assert_eq!(&temporal.into_scalar().unwrap(), value);
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
