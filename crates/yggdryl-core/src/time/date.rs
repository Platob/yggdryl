//! The [`Date`] type — a proleptic-Gregorian calendar date, stored as days since
//! the UNIX epoch (1970-01-01), matching Arrow `Date32`.

use std::fmt;

#[allow(unused_imports)]
use crate::log_event;
use crate::Mapping;

use super::{civil_from_days, days_from_civil, days_in_month, TimeError};

/// A calendar date with no time-of-day or timezone, stored as days since the UNIX
/// epoch. Ordered chronologically.
///
/// ```
/// use yggdryl_core::Date;
///
/// let d = Date::from_ymd(2024, 2, 29).unwrap();
/// assert_eq!((d.year(), d.month(), d.day()), (2024, 2, 29));
/// assert_eq!(d.to_str(), "2024-02-29");
/// assert_eq!(Date::from_str("2024-02-29").unwrap(), d);
/// assert!(Date::from_ymd(2023, 2, 29).is_err()); // not a leap year
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Date {
    /// Days since 1970-01-01 (negative before the epoch).
    epoch_days: i32,
}

impl Date {
    /// Builds a date from `(year, month, day)`, validating the calendar.
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Result<Date, TimeError> {
        if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
            return Err(TimeError::OutOfRange(format!(
                "{year:04}-{month:02}-{day:02}"
            )));
        }
        Ok(Date {
            epoch_days: days_from_civil(year, month, day) as i32,
        })
    }

    /// Builds a date from a count of days since the UNIX epoch.
    pub fn from_epoch_days(epoch_days: i32) -> Date {
        Date { epoch_days }
    }

    /// Days since the UNIX epoch.
    pub fn epoch_days(&self) -> i32 {
        self.epoch_days
    }

    /// The `(year, month, day)` components.
    pub fn ymd(&self) -> (i32, u32, u32) {
        civil_from_days(self.epoch_days as i64)
    }

    /// The year component.
    pub fn year(&self) -> i32 {
        self.ymd().0
    }

    /// The month component (1–12).
    pub fn month(&self) -> u32 {
        self.ymd().1
    }

    /// The day component (1–31).
    pub fn day(&self) -> u32 {
        self.ymd().2
    }

    /// The day of the week (0 = Sunday … 6 = Saturday).
    pub fn weekday(&self) -> u32 {
        (self.epoch_days as i64 + 4).rem_euclid(7) as u32
    }

    /// Returns a copy `days` days later (or earlier, if negative).
    pub fn add_days(&self, days: i32) -> Date {
        Date {
            epoch_days: self.epoch_days + days,
        }
    }

    /// Parses an ISO `YYYY-MM-DD` date (a leading `-` denotes a negative year).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<Date, TimeError> {
        log_event!(trace, "Date::from_str {input:?}");
        let value = input.trim();
        if value.is_empty() {
            return Err(TimeError::Empty);
        }
        let negative = value.starts_with('-');
        let body = value.strip_prefix('-').unwrap_or(value);
        let mut parts = body.split('-');
        let year = parts
            .next()
            .and_then(|p| p.parse::<i32>().ok())
            .ok_or_else(|| TimeError::Invalid(input.to_string()))?;
        let month = parts
            .next()
            .and_then(|p| p.parse::<u32>().ok())
            .ok_or_else(|| TimeError::Invalid(input.to_string()))?;
        let day = parts
            .next()
            .and_then(|p| p.parse::<u32>().ok())
            .ok_or_else(|| TimeError::Invalid(input.to_string()))?;
        if parts.next().is_some() {
            return Err(TimeError::Invalid(input.to_string()));
        }
        Date::from_ymd(if negative { -year } else { year }, month, day)
    }

    /// Builds a date from a [`Mapping`] (`year` / `month` / `day`).
    pub fn from_mapping(fields: &Mapping) -> Result<Date, TimeError> {
        let component = |key: &str| -> Result<i64, TimeError> {
            fields
                .get(key)
                .ok_or_else(|| TimeError::Invalid(format!("missing '{key}'")))?
                .parse::<i64>()
                .map_err(|_| TimeError::Invalid(format!("'{key}' is not an integer")))
        };
        Date::from_ymd(
            component("year")? as i32,
            component("month")? as u32,
            component("day")? as u32,
        )
    }

    /// Renders the canonical ISO `YYYY-MM-DD` string.
    pub fn to_str(&self) -> String {
        let (y, m, d) = self.ymd();
        if y < 0 {
            format!("-{:04}-{:02}-{:02}", -y, m, d)
        } else {
            format!("{y:04}-{m:02}-{d:02}")
        }
    }

    /// Renders to a component [`Mapping`] (`year` / `month` / `day`).
    pub fn to_mapping(&self) -> Mapping {
        let (y, m, d) = self.ymd();
        Mapping::from([
            ("year".to_string(), y.to_string()),
            ("month".to_string(), m.to_string()),
            ("day".to_string(), d.to_string()),
        ])
    }

    /// The canonical string as UTF-8 bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_str().into_bytes()
    }

    /// Parses a date from the UTF-8 bytes of its canonical string.
    pub fn from_bytes(bytes: &[u8]) -> Result<Date, TimeError> {
        let value = std::str::from_utf8(bytes).map_err(|_| TimeError::Invalid("<bytes>".into()))?;
        Date::from_str(value)
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_str())
    }
}
