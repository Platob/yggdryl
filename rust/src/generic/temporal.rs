//! The four temporals, spelled the same way in every language.
//!
//! A temporal is a count plus a unit, and for a timestamp a zone as well. Those
//! parts used to live in a tagged payload - a name over a sequence - which meant
//! nothing checked that a value tagged `timestamp` actually held a unit, and a
//! reader had to parse the unit back out of a string on every access. They are
//! now fields on the value, so a timestamp cannot be built without its unit and
//! cannot be read without getting one.
//!
//! Each variant carries exactly the shape the matching Arrow datatype carries,
//! so a value converts to a column without a lookup table:
//!
//! | Variant | Payload | Arrow datatype |
//! | --- | --- | --- |
//! | [`Value::Timestamp`] | count of unit since the epoch, zone | `Timestamp(unit, zone)` |
//! | [`Value::Date`] | days since the epoch | `Date32` |
//! | [`Value::Time`] | count of unit since midnight | `Time32`/`Time64(unit)` |
//! | [`Value::Duration`] | elapsed count of unit | `Duration(unit)` |
//!
//! ```
//! use yggdryl::{TimeUnit, Value};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let at = Value::timestamp(1_700_000_000_000_000, TimeUnit::Microsecond, Some("UTC"))?;
//!
//! let (count, unit, zone) = at.as_timestamp().expect("a canonical timestamp");
//! assert_eq!(count, 1_700_000_000_000_000);
//! assert_eq!(unit, TimeUnit::Microsecond);
//! assert_eq!(zone, Some("UTC"));
//!
//! // One instant spelled two ways is one value.
//! assert_eq!(Value::duration(1, TimeUnit::Second), Value::duration(1_000, TimeUnit::Millisecond));
//! # Ok(())
//! # }
//! ```

use super::value::Value;
use crate::{DataType, Result, TimeUnit, Timezone};

impl Value {
    /// Build a timestamp: a count of `unit` since the Unix epoch.
    ///
    /// The count is always relative to UTC, as Arrow defines it; the zone says
    /// how to display it, and its absence means a naive wall-clock reading.
    ///
    /// # Errors
    ///
    /// Returns an error when `zone` is not a time zone name or a fixed offset.
    pub fn timestamp(count: i64, unit: TimeUnit, zone: Option<&str>) -> Result<Self> {
        let zone = zone.map(Timezone::from_str).transpose()?;
        Ok(Self::Timestamp(count, unit, zone))
    }

    /// Build a timestamp from a zone that is already validated.
    pub const fn timestamp_in(count: i64, unit: TimeUnit, zone: Option<Timezone>) -> Self {
        Self::Timestamp(count, unit, zone)
    }

    /// Build a date: a count of days since the Unix epoch.
    pub const fn date(days: i32) -> Self {
        Self::Date(days)
    }

    /// Build a time of day: a count of `unit` since midnight.
    pub const fn time(count: i64, unit: TimeUnit) -> Self {
        Self::Time(count, unit)
    }

    /// Build a duration: an elapsed count of `unit`.
    pub const fn duration(count: i64, unit: TimeUnit) -> Self {
        Self::Duration(count, unit)
    }

    /// Read a timestamp as its count, unit, and zone name.
    pub fn as_timestamp(&self) -> Option<(i64, TimeUnit, Option<&str>)> {
        match self {
            Self::Timestamp(count, unit, zone) => {
                Some((*count, *unit, zone.as_ref().map(Timezone::as_str)))
            }
            _ => None,
        }
    }

    /// Read a timestamp as its count, unit, and validated zone.
    pub const fn as_timestamp_in(&self) -> Option<(i64, TimeUnit, Option<&Timezone>)> {
        match self {
            Self::Timestamp(count, unit, zone) => Some((*count, *unit, zone.as_ref())),
            _ => None,
        }
    }

    /// Read a date as its day count since the Unix epoch.
    pub const fn as_date(&self) -> Option<i32> {
        match self {
            Self::Date(days) => Some(*days),
            _ => None,
        }
    }

    /// Read a time of day as its count and unit.
    pub const fn as_time(&self) -> Option<(i64, TimeUnit)> {
        match self {
            Self::Time(count, unit) => Some((*count, *unit)),
            _ => None,
        }
    }

    /// Read a duration as its count and unit.
    pub const fn as_duration(&self) -> Option<(i64, TimeUnit)> {
        match self {
            Self::Duration(count, unit) => Some((*count, *unit)),
            _ => None,
        }
    }

    /// Return this temporal's count restated in `unit`, when it is exact.
    ///
    /// A column declares its own resolution, so a temporal written into one has
    /// to be restated at that resolution. Restating a nanosecond count as
    /// seconds would drop digits, so it answers `None` rather than truncating.
    /// A date carries no unit and answers `None`.
    pub fn temporal_count_at(&self, unit: TimeUnit) -> Option<i64> {
        let (count, current) = match self {
            Self::Timestamp(count, current, _)
            | Self::Time(count, current)
            | Self::Duration(count, current) => (*count, *current),
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

    /// Return whether this value is any of the four temporals.
    pub const fn is_temporal(&self) -> bool {
        matches!(
            self,
            Self::Timestamp(..) | Self::Date(_) | Self::Time(..) | Self::Duration(..)
        )
    }

    /// Return the datatype a temporal materializes into.
    ///
    /// This is [`Value::data_type`] narrowed to the temporals: it answers
    /// `None` for every other kind rather than describing it.
    pub fn temporal_data_type(&self) -> Option<DataType> {
        if !self.is_temporal() {
            return None;
        }
        self.data_type().ok()
    }
}

/// The nanoseconds one resolution unit is worth.
///
/// A calendar interval layout answers `None`, because a month is not a fixed
/// number of nanoseconds and pretending otherwise is how a date silently moves.
const fn nanoseconds_per(unit: TimeUnit) -> Option<i128> {
    match unit {
        TimeUnit::Second => Some(1_000_000_000),
        TimeUnit::Millisecond => Some(1_000_000),
        TimeUnit::Microsecond => Some(1_000),
        TimeUnit::Nanosecond => Some(1),
        TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => None,
    }
}

/// The sort key one count and unit reduce to.
///
/// A resolution unit reduces to a nanosecond count, so a duration of one second
/// and a duration of a thousand milliseconds are one value rather than two
/// spellings that sort apart. A count of nanoseconds is at most `i64::MAX`
/// nanoseconds, so the widest product here is nine thousand times smaller than
/// `i128::MAX` and cannot overflow. An interval layout has no fixed nanosecond
/// width - a month is not a number of seconds - so it keeps its own bucket
/// instead of being given one it does not have.
pub(super) fn temporal_key(count: i64, unit: TimeUnit) -> (u8, i128) {
    let count = i128::from(count);
    match unit {
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
mod tests;
