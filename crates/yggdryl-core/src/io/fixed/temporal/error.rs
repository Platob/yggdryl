//! [`TemporalError`] — the guided failures of temporal construction and conversion. Every message
//! names the offending value and how to fix it.

use super::TimeUnit;

/// A temporal construction, conversion, or parse failure. Each variant's [`Display`] is guided.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TemporalError {
    /// A date component was out of range (`month` 13, `day` 32, an impossible `2023-02-29`, …).
    InvalidDate {
        /// The offending year.
        year: i32,
        /// The offending month.
        month: u32,
        /// The offending day.
        day: u32,
    },
    /// A time-of-day component was out of range (`hour` 24, `minute` 60, …).
    InvalidTime {
        /// The offending hour.
        hour: u32,
        /// The offending minute.
        minute: u32,
        /// The offending second.
        second: u32,
    },
    /// A time unit is not valid for this type (e.g. `Time32` only holds seconds / milliseconds, a
    /// timestamp only fixed units).
    UnsupportedUnit {
        /// The type name (`"time32"`, `"ts64"`, …).
        ty: &'static str,
        /// The rejected unit.
        unit: TimeUnit,
    },
    /// A value did not fit the width's integer range (e.g. a nanosecond count in a `ts32`).
    OutOfRange {
        /// The type name.
        ty: &'static str,
    },
    /// An arithmetic or unit/scale conversion overflowed.
    Overflow {
        /// The type name.
        ty: &'static str,
        /// The operation that overflowed (`"add"`, `"to_unit"`, …).
        op: &'static str,
    },
    /// A calendar unit (`Month` / `Year`) has no fixed length, so it cannot take part in a plain
    /// count conversion.
    CalendarUnit {
        /// The calendar unit involved.
        unit: TimeUnit,
    },
    /// A string was not a valid literal for this type.
    ParseError {
        /// The type name.
        ty: &'static str,
    },
}

impl core::fmt::Display for TemporalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidDate { year, month, day } => write!(
                f,
                "invalid date {year:04}-{month:02}-{day:02}: month must be 1..=12 and day a real \
                 day of that month (check for a non-leap February 29)"
            ),
            Self::InvalidTime {
                hour,
                minute,
                second,
            } => write!(
                f,
                "invalid time {hour:02}:{minute:02}:{second:02}: hour must be 0..=23, minute \
                 0..=59, second 0..=60 (a leap second)"
            ),
            Self::UnsupportedUnit { ty, unit } => write!(
                f,
                "{ty} does not support the {} unit; use a supported resolution for this type",
                unit.name()
            ),
            Self::OutOfRange { ty } => write!(
                f,
                "value is out of range for {ty}: use a wider temporal type (e.g. ts64 or ts96) or a \
                 coarser unit"
            ),
            Self::Overflow { ty, op } => write!(
                f,
                "{ty} {op} overflow: the result exceeds the value range — use a wider type or a \
                 coarser unit"
            ),
            Self::CalendarUnit { unit } => write!(
                f,
                "the {} unit has no fixed length, so it cannot be converted by a plain count — use \
                 calendar-aware date arithmetic instead",
                unit.name()
            ),
            Self::ParseError { ty } => write!(
                f,
                "invalid {ty} literal: expected an ISO-8601 form (e.g. a date \"2024-02-29\", a \
                 time \"13:45:30\", or a timestamp \"2024-02-29T13:45:30Z\")"
            ),
        }
    }
}

impl std::error::Error for TemporalError {}
