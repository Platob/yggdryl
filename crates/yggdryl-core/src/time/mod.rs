//! A self-contained calendar/time foundation — no external timezone database.
//!
//! Rust's standard library has no civil date/time types (only `SystemTime` /
//! `Duration`), so this module supplies them:
//!
//! - [`Date`] — a proleptic-Gregorian calendar date (days since the UNIX epoch);
//! - [`Time`] — a time of day (nanoseconds since midnight);
//! - [`DateTime`] — an absolute instant (UTC nanoseconds) with an optional
//!   [`Timezone`] for display;
//! - [`Duration`] — a signed span of seconds + nanoseconds;
//! - [`Timezone`] — UTC, a fixed offset, or a named zone whose DST rules are
//!   computed from an **embedded POSIX-TZ rule** (so timezone/DST conversions work
//!   with no external tz database). See [`timezone`] for the supported zones and
//!   the (current-rules, no historical-transitions) caveat.
//! - [`TimeUnit`] — the shared temporal resolution enum.
//!
//! Every type is `from_str`/`to_str`, `from_mapping`/`to_mapping`,
//! `to_json`/`from_json` (under `json`) and `to_bytes`/`from_bytes` convertible,
//! `serde`-serializable (under `serde`) and [`Hash`].
//!
//! ```
//! use yggdryl_core::{Date, DateTime, Timezone};
//!
//! let d = Date::from_str("2024-02-29").unwrap(); // a leap day
//! assert_eq!((d.year(), d.month(), d.day()), (2024, 2, 29));
//!
//! // 2024-07-01 12:00:00 UTC seen from New York is 08:00 (EDT, UTC-4).
//! let utc = DateTime::from_str("2024-07-01T12:00:00Z").unwrap();
//! let ny = utc.to_timezone(Timezone::from_str("America/New_York").unwrap());
//! assert_eq!((ny.hour(), ny.minute()), (8, 0));
//! ```

use std::fmt;

#[allow(unused_imports)]
use crate::log_event;

mod date;
mod datetime;
mod duration;
#[allow(clippy::module_inception)]
mod time;
mod timezone;

pub use date::Date;
pub use datetime::DateTime;
pub use duration::Duration;
pub use time::Time;
pub use timezone::Timezone;

/// Error returned when a temporal value cannot be parsed or is out of range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeError {
    /// The input was empty.
    Empty,
    /// The input was not a well-formed value of the expected shape.
    Invalid(String),
    /// A [`TimeUnit`] token was not recognised.
    UnknownUnit(String),
    /// A timezone name / offset was not recognised.
    UnknownZone(String),
    /// A component was outside its valid range (e.g. month 13, hour 25).
    OutOfRange(String),
}

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeError::Empty => write!(f, "temporal value is empty"),
            TimeError::Invalid(value) => write!(f, "'{value}' is not a valid temporal value"),
            TimeError::UnknownUnit(value) => write!(
                f,
                "unknown time unit '{value}', expected 's', 'ms', 'us' or 'ns'"
            ),
            TimeError::UnknownZone(value) => write!(
                f,
                "unknown timezone '{value}', expected 'UTC', a '+HH:MM' offset, an IANA name or a POSIX TZ string"
            ),
            TimeError::OutOfRange(value) => write!(f, "temporal component out of range: {value}"),
        }
    }
}

impl std::error::Error for TimeError {}

/// The resolution of a temporal value — seconds down to nanoseconds. Shared by
/// [`Duration`], the schema temporal types and the unit-aware constructors.
///
/// ```
/// use yggdryl_core::TimeUnit;
/// assert_eq!(TimeUnit::from_str("ms").unwrap(), TimeUnit::Millisecond);
/// assert_eq!(TimeUnit::Microsecond.nanos(), 1_000);
/// assert!(TimeUnit::Second < TimeUnit::Nanosecond); // ordered by resolution
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TimeUnit {
    /// Seconds.
    Second,
    /// Milliseconds (10⁻³ s).
    Millisecond,
    /// Microseconds (10⁻⁶ s).
    Microsecond,
    /// Nanoseconds (10⁻⁹ s).
    Nanosecond,
}

impl TimeUnit {
    /// Parses a unit token, accepting the short (`s` / `ms` / `us` / `ns`) and long
    /// (`second` / `millisecond` / …) spellings, case-insensitively.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<TimeUnit, TimeError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "s" | "sec" | "second" | "seconds" => Ok(TimeUnit::Second),
            "ms" | "milli" | "millisecond" | "milliseconds" => Ok(TimeUnit::Millisecond),
            "us" | "µs" | "micro" | "microsecond" | "microseconds" => Ok(TimeUnit::Microsecond),
            "ns" | "nano" | "nanosecond" | "nanoseconds" => Ok(TimeUnit::Nanosecond),
            _ => Err(TimeError::UnknownUnit(value.to_string())),
        }
    }

    /// The canonical short token (`s` / `ms` / `us` / `ns`).
    pub fn as_str(&self) -> &'static str {
        match self {
            TimeUnit::Second => "s",
            TimeUnit::Millisecond => "ms",
            TimeUnit::Microsecond => "us",
            TimeUnit::Nanosecond => "ns",
        }
    }

    /// The number of nanoseconds in one of this unit (`1` … `1_000_000_000`).
    pub fn nanos(&self) -> i64 {
        match self {
            TimeUnit::Second => 1_000_000_000,
            TimeUnit::Millisecond => 1_000_000,
            TimeUnit::Microsecond => 1_000,
            TimeUnit::Nanosecond => 1,
        }
    }

    /// The number of these units in one second (`1` … `1_000_000_000`).
    pub fn per_second(&self) -> i64 {
        1_000_000_000 / self.nanos()
    }
}

impl fmt::Display for TimeUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---- civil-date algorithms (Howard Hinnant's `chrono`-style, exact) ----

/// Days from the UNIX epoch (1970-01-01) to the given proleptic-Gregorian date.
/// Valid for any `year`. (Howard Hinnant, *chrono-Compatible Low-Level Date
/// Algorithms*.)
pub(crate) fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let m = month as i64;
    let d = day as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// The proleptic-Gregorian `(year, month, day)` for a count of days since the UNIX
/// epoch — the inverse of [`days_from_civil`].
pub(crate) fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (year as i32, m as u32, d as u32)
}

/// Whether `year` is a leap year in the proleptic Gregorian calendar.
pub(crate) fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// The number of days in `month` of `year` (1-based month).
pub(crate) fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

#[cfg(test)]
mod tests;
