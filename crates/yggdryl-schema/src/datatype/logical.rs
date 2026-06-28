//! The [`LogicalType`] — types whose logical meaning is richer than their physical
//! storage (decimal, the temporal family, JSON/BSON) — and the [`IntervalUnit`].

use super::DataTypeId;
use yggdryl_core::{TimeUnit, Timezone};

/// The resolution of an [`Interval`](LogicalType::Interval).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntervalUnit {
    /// Whole months.
    YearMonth,
    /// Days and milliseconds.
    DayTime,
    /// Months, days and nanoseconds.
    MonthDayNano,
}

impl IntervalUnit {
    /// Parses a canonical token (`"year_month"` / `"day_time"` / `"month_day_nano"`),
    /// returning `None` for anything else.
    pub fn from_name(value: &str) -> Option<IntervalUnit> {
        match value {
            "year_month" => Some(IntervalUnit::YearMonth),
            "day_time" => Some(IntervalUnit::DayTime),
            "month_day_nano" => Some(IntervalUnit::MonthDayNano),
            _ => None,
        }
    }

    /// The canonical token (`"year_month"` / `"day_time"` / `"month_day_nano"`).
    pub fn name(self) -> &'static str {
        match self {
            IntervalUnit::YearMonth => "year_month",
            IntervalUnit::DayTime => "day_time",
            IntervalUnit::MonthDayNano => "month_day_nano",
        }
    }
}

/// A logical type. Each carries the parameters that distinguish it (a decimal's
/// `precision`/`scale`, a temporal type's [`TimeUnit`] / [`Timezone`], …).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LogicalType {
    /// A decimal with `(precision, scale)`.
    Decimal {
        /// Total number of significant digits.
        precision: u8,
        /// Digits after the decimal point (may be negative).
        scale: i8,
    },
    /// A calendar date.
    Date,
    /// A time of day in the given resolution.
    Time {
        /// Resolution.
        unit: TimeUnit,
    },
    /// A timestamp in the given resolution, optionally zoned.
    Timestamp {
        /// Resolution.
        unit: TimeUnit,
        /// Display timezone, if zoned.
        timezone: Option<Timezone>,
    },
    /// An elapsed duration in the given resolution.
    Duration {
        /// Resolution.
        unit: TimeUnit,
    },
    /// A calendar interval in the given resolution.
    Interval {
        /// Resolution.
        unit: IntervalUnit,
    },
    /// JSON text (string-backed).
    Json,
    /// A BSON document (binary-backed).
    Bson,
}

impl LogicalType {
    /// The [`DataTypeId`] of this type.
    pub fn type_id(&self) -> DataTypeId {
        use LogicalType::*;
        match self {
            Decimal { .. } => DataTypeId::Decimal,
            Date => DataTypeId::Date,
            Time { .. } => DataTypeId::Time,
            Timestamp { .. } => DataTypeId::Timestamp,
            Duration { .. } => DataTypeId::Duration,
            Interval { .. } => DataTypeId::Interval,
            Json => DataTypeId::Json,
            Bson => DataTypeId::Bson,
        }
    }

    /// The canonical name (`"decimal"`, `"timestamp"`, …).
    pub fn name(&self) -> &'static str {
        self.type_id().name()
    }
}
