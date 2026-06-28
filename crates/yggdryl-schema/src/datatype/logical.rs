//! Logical-category types: the [`IntervalUnit`], the temporal / decimal /
//! dictionary checks and their constructors. Temporal types reuse the core
//! [`TimeUnit`](yggdryl_core::TimeUnit) and [`Timezone`](yggdryl_core::Timezone).

use std::fmt;

use super::fixed::{Decimal128, Decimal256, Decimal32, Decimal64, FixedKind};
use super::{DataType, SchemaError};
use yggdryl_core::{TimeUnit, Timezone};

/// The resolution of an [`Interval`](DataType::Interval) calendar type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum IntervalUnit {
    /// Whole months, as a 32-bit integer.
    YearMonth,
    /// Days and milliseconds, as two 32-bit integers.
    DayTime,
    /// Months, days and nanoseconds.
    MonthDayNano,
}

impl IntervalUnit {
    /// Parses an interval-unit token (case-insensitive).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<IntervalUnit, SchemaError> {
        match value
            .trim()
            .to_ascii_lowercase()
            .replace(['-', ' '], "_")
            .as_str()
        {
            "year_month" | "yearmonth" => Ok(IntervalUnit::YearMonth),
            "day_time" | "daytime" => Ok(IntervalUnit::DayTime),
            "month_day_nano" | "monthdaynano" => Ok(IntervalUnit::MonthDayNano),
            _ => Err(SchemaError::UnknownUnit(value.to_string())),
        }
    }

    /// The canonical token (`year_month` / `day_time` / `month_day_nano`).
    pub fn as_str(&self) -> &'static str {
        match self {
            IntervalUnit::YearMonth => "year_month",
            IntervalUnit::DayTime => "day_time",
            IntervalUnit::MonthDayNano => "month_day_nano",
        }
    }

    /// The physical width of this interval in bits.
    pub fn bit_size(&self) -> u16 {
        match self {
            IntervalUnit::YearMonth => 32,
            IntervalUnit::DayTime => 64,
            IntervalUnit::MonthDayNano => 128,
        }
    }
}

impl fmt::Display for IntervalUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl DataType {
    // ---- constructors ----

    /// A day-resolution [`Date`](DataType::Date) (32-bit).
    pub fn date() -> DataType {
        DataType::Date { large: false }
    }

    /// A decimal with `(precision, scale)`, stored in 128 bits
    /// ([`decimal128`](DataType::Decimal128)).
    pub fn decimal(precision: u8, scale: i8) -> DataType {
        DataType::decimal_with(precision, scale, 128)
    }

    /// The fixed-width decimal for an explicit storage width — the convenience builder
    /// over the concrete [`Decimal32`](DataType::Decimal32) /
    /// [`Decimal64`](DataType::Decimal64) / [`Decimal128`](DataType::Decimal128) /
    /// [`Decimal256`](DataType::Decimal256) variants. A width that is not 32/64/256
    /// defaults to the 128-bit decimal (the common case).
    pub fn decimal_with(precision: u8, scale: i8, bits: u16) -> DataType {
        match bits {
            32 => Decimal32::new(precision, scale).into(),
            64 => Decimal64::new(precision, scale).into(),
            256 => Decimal256::new(precision, scale).into(),
            _ => Decimal128::new(precision, scale).into(),
        }
    }

    /// A 32-bit decimal with `(precision, scale)` ([`decimal32`](DataType::Decimal32)).
    pub fn decimal32(precision: u8, scale: i8) -> DataType {
        Decimal32::new(precision, scale).into()
    }

    /// A 64-bit decimal with `(precision, scale)` ([`decimal64`](DataType::Decimal64)).
    pub fn decimal64(precision: u8, scale: i8) -> DataType {
        Decimal64::new(precision, scale).into()
    }

    /// A 128-bit decimal with `(precision, scale)` ([`decimal128`](DataType::Decimal128)).
    pub fn decimal128(precision: u8, scale: i8) -> DataType {
        Decimal128::new(precision, scale).into()
    }

    /// A 256-bit decimal with `(precision, scale)` ([`decimal256`](DataType::Decimal256)).
    pub fn decimal256(precision: u8, scale: i8) -> DataType {
        Decimal256::new(precision, scale).into()
    }

    /// A [`Timestamp`](DataType::Timestamp); pass `timezone` for a zoned timestamp.
    pub fn timestamp(unit: TimeUnit, timezone: Option<Timezone>) -> DataType {
        DataType::Timestamp { unit, timezone }
    }

    /// A [`Dictionary`](DataType::Dictionary) of `key` indices into `value`s.
    pub fn dictionary(key: DataType, value: DataType) -> DataType {
        DataType::Dictionary {
            key: Box::new(key),
            value: Box::new(value),
        }
    }

    /// JSON text (a string-backed logical type).
    pub fn json() -> DataType {
        DataType::Json
    }

    /// A BSON document (a binary-backed logical type).
    pub fn bson() -> DataType {
        DataType::Bson
    }

    // ---- checks ----

    /// Whether this is a [logical](super::TypeCategory::Logical) type.
    pub fn is_logical(&self) -> bool {
        self.is_temporal()
            || self.is_decimal()
            || self.is_dictionary()
            || self.is_json()
            || self.is_bson()
            || self.is_timezone()
    }

    /// Whether this is a temporal type (date / time / timestamp / duration / interval).
    pub fn is_temporal(&self) -> bool {
        matches!(
            self,
            DataType::Date { .. }
                | DataType::Time { .. }
                | DataType::Timestamp { .. }
                | DataType::Duration { .. }
                | DataType::Interval { .. }
        )
    }

    /// Whether this is a decimal type (any storage width).
    pub fn is_decimal(&self) -> bool {
        self.fixed().map(|t| t.kind) == Some(FixedKind::Decimal)
    }

    /// Whether this is a [`Dictionary`](DataType::Dictionary) encoding.
    pub fn is_dictionary(&self) -> bool {
        matches!(self, DataType::Dictionary { .. })
    }

    /// Whether this is the [`Json`](DataType::Json) logical type.
    pub fn is_json(&self) -> bool {
        matches!(self, DataType::Json)
    }

    /// Whether this is the [`Bson`](DataType::Bson) logical type.
    pub fn is_bson(&self) -> bool {
        matches!(self, DataType::Bson)
    }

    /// Whether this is the [`Timezone`](DataType::Timezone) logical type (a column of
    /// zone names — not the optional display timezone a
    /// [`Timestamp`](DataType::Timestamp) carries; read that with
    /// [`timezone`](DataType::timezone)).
    pub fn is_timezone(&self) -> bool {
        matches!(self, DataType::Timezone)
    }

    /// The [`TimeUnit`] of a temporal type that carries one, or `None`.
    pub fn time_unit(&self) -> Option<TimeUnit> {
        match self {
            DataType::Time { unit }
            | DataType::Timestamp { unit, .. }
            | DataType::Duration { unit } => Some(*unit),
            _ => None,
        }
    }

    /// The display [`Timezone`] of a [`Timestamp`](DataType::Timestamp), or `None`.
    pub fn timezone(&self) -> Option<&Timezone> {
        match self {
            DataType::Timestamp { timezone, .. } => timezone.as_ref(),
            _ => None,
        }
    }

    /// The `(precision, scale)` of a decimal type, or `None`.
    pub fn decimal_parts(&self) -> Option<(u8, i8)> {
        self.fixed().and_then(|t| t.decimal)
    }
}
