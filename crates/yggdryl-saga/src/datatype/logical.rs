//! The [`LogicalType`] family: semantic Arrow types layered over a physical
//! representation — temporal types, intervals and decimals — plus the
//! [`TimeUnit`] and [`IntervalUnit`] enumerations they carry.

use std::fmt;

#[allow(unused_imports)]
use crate::log_event;
use crate::parse::{split_head, split_top_level, Head};

use super::DataTypeError;

/// The resolution of a temporal [`LogicalType`] (`Timestamp` / `Time*` /
/// `Duration`). Rendered as the compact `s` / `ms` / `us` / `ns`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    /// Parses a unit, accepting the compact form (`s` / `ms` / `us` / `ns`) and
    /// the long name (`second` / `millisecond` / `microsecond` / `nanosecond`).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<TimeUnit, DataTypeError> {
        match input.trim().to_ascii_lowercase().as_str() {
            "s" | "sec" | "second" => Ok(TimeUnit::Second),
            "ms" | "milli" | "millisecond" => Ok(TimeUnit::Millisecond),
            "us" | "micro" | "microsecond" => Ok(TimeUnit::Microsecond),
            "ns" | "nano" | "nanosecond" => Ok(TimeUnit::Nanosecond),
            _ => Err(DataTypeError::Invalid(format!(
                "unknown time unit '{input}', expected 's', 'ms', 'us' or 'ns'"
            ))),
        }
    }

    /// The compact name (`s` / `ms` / `us` / `ns`).
    pub fn as_str(&self) -> &'static str {
        match self {
            TimeUnit::Second => "s",
            TimeUnit::Millisecond => "ms",
            TimeUnit::Microsecond => "us",
            TimeUnit::Nanosecond => "ns",
        }
    }
}

impl fmt::Display for TimeUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The layout of an [`Interval`](LogicalType::Interval) — which calendar fields it
/// spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum IntervalUnit {
    /// A number of months (`year_month`).
    YearMonth,
    /// A number of days plus milliseconds (`day_time`).
    DayTime,
    /// Months, days and nanoseconds (`month_day_nano`).
    MonthDayNano,
}

impl IntervalUnit {
    /// Parses an interval unit name (case-insensitive).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<IntervalUnit, DataTypeError> {
        match input.trim().to_ascii_lowercase().as_str() {
            "year_month" => Ok(IntervalUnit::YearMonth),
            "day_time" => Ok(IntervalUnit::DayTime),
            "month_day_nano" => Ok(IntervalUnit::MonthDayNano),
            _ => Err(DataTypeError::Invalid(format!(
                "unknown interval unit '{input}', expected 'year_month', 'day_time' or 'month_day_nano'"
            ))),
        }
    }

    /// The lowercase name.
    pub fn as_str(&self) -> &'static str {
        match self {
            IntervalUnit::YearMonth => "year_month",
            IntervalUnit::DayTime => "day_time",
            IntervalUnit::MonthDayNano => "month_day_nano",
        }
    }
}

impl fmt::Display for IntervalUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A semantic Arrow type built over a physical layout: a calendar/clock value, an
/// interval, or a fixed-scale decimal. Each is child-less but parameterised (a
/// [`TimeUnit`], a timezone, an [`IntervalUnit`], or a precision/scale pair).
///
/// ```
/// use yggdryl_saga::{LogicalType, TimeUnit};
///
/// let ts = LogicalType::from_str("timestamp(us, UTC)").unwrap();
/// assert_eq!(ts, LogicalType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())));
/// assert_eq!(ts.to_str(), "timestamp(us, UTC)");
///
/// assert_eq!(LogicalType::from_str("decimal128(38, 10)").unwrap().to_str(), "decimal128(38, 10)");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LogicalType {
    /// Days since the UNIX epoch, as an `int32` (`date32`).
    Date32,
    /// Milliseconds since the UNIX epoch, as an `int64` (`date64`).
    Date64,
    /// Time of day as an `int32` at the given resolution (`time32(unit)`).
    Time32(TimeUnit),
    /// Time of day as an `int64` at the given resolution (`time64(unit)`).
    Time64(TimeUnit),
    /// An instant at the given resolution, with an optional timezone
    /// (`timestamp(unit)` / `timestamp(unit, tz)`).
    Timestamp(TimeUnit, Option<String>),
    /// An elapsed span at the given resolution (`duration(unit)`).
    Duration(TimeUnit),
    /// A calendar interval with the given layout (`interval(unit)`).
    Interval(IntervalUnit),
    /// A 32-bit fixed-scale decimal (`decimal32(precision, scale)`).
    Decimal32(u8, i8),
    /// A 64-bit fixed-scale decimal (`decimal64(precision, scale)`).
    Decimal64(u8, i8),
    /// A 128-bit fixed-scale decimal (`decimal128(precision, scale)`).
    Decimal128(u8, i8),
    /// A 256-bit fixed-scale decimal (`decimal256(precision, scale)`).
    Decimal256(u8, i8),
}

impl LogicalType {
    /// `true` for the calendar/clock types (`date32` / `date64` / `time32` /
    /// `time64` / `timestamp` / `duration`) — the ones a string ISO value can be
    /// cast into. Intervals and decimals are excluded.
    pub fn is_temporal(&self) -> bool {
        matches!(
            self,
            LogicalType::Date32
                | LogicalType::Date64
                | LogicalType::Time32(_)
                | LogicalType::Time64(_)
                | LogicalType::Timestamp(_, _)
                | LogicalType::Duration(_)
        )
    }

    /// Parses a canonical logical name (e.g. `date32`, `timestamp(us, UTC)`,
    /// `decimal128(38, 10)`). Returns [`DataTypeError::Unknown`] for a name that is
    /// not a logical type, so [`DataType::from_str`](crate::DataType::from_str) can
    /// try the other families.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<LogicalType, DataTypeError> {
        log_event!(trace, "LogicalType::from_str {input:?}");
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(DataTypeError::Empty);
        }
        let head =
            split_head(trimmed).ok_or_else(|| DataTypeError::Invalid(trimmed.to_string()))?;
        LogicalType::from_head(&head)
    }

    /// Builds a logical type from a parsed [`Head`]. Unowned names return
    /// [`DataTypeError::Unknown`]; an owned name with bad params returns
    /// [`DataTypeError::Invalid`].
    pub(crate) fn from_head(head: &Head) -> Result<LogicalType, DataTypeError> {
        // A logical type never carries a `<body>`; `needs_no_body` rejects one on
        // an otherwise-owned name.
        let needs_no_body = |t: LogicalType| -> Result<LogicalType, DataTypeError> {
            match head.body {
                Some(_) => Err(DataTypeError::Invalid(format!(
                    "'{}' takes no <body>",
                    head.name
                ))),
                None => Ok(t),
            }
        };
        match head.name {
            "date32" => {
                Self::require_no_params(head)?;
                needs_no_body(LogicalType::Date32)
            }
            "date64" => {
                Self::require_no_params(head)?;
                needs_no_body(LogicalType::Date64)
            }
            "time32" => needs_no_body(LogicalType::Time32(Self::unit(head)?)),
            "time64" => needs_no_body(LogicalType::Time64(Self::unit(head)?)),
            "duration" => needs_no_body(LogicalType::Duration(Self::unit(head)?)),
            "interval" => {
                let raw = head.params.ok_or_else(|| {
                    DataTypeError::Invalid(
                        "'interval' needs a unit, e.g. interval(month_day_nano)".to_string(),
                    )
                })?;
                needs_no_body(LogicalType::Interval(IntervalUnit::from_str(raw)?))
            }
            "timestamp" => {
                let raw = head.params.ok_or_else(|| {
                    DataTypeError::Invalid(
                        "'timestamp' needs a unit, e.g. timestamp(us)".to_string(),
                    )
                })?;
                let parts = split_top_level(raw, ',');
                let unit = TimeUnit::from_str(parts[0])?;
                let tz = match parts.get(1) {
                    Some(tz) if !tz.is_empty() => Some(tz.to_string()),
                    _ => None,
                };
                needs_no_body(LogicalType::Timestamp(unit, tz))
            }
            "decimal32" => needs_no_body(Self::decimal(head, LogicalType::Decimal32)?),
            "decimal64" => needs_no_body(Self::decimal(head, LogicalType::Decimal64)?),
            "decimal128" => needs_no_body(Self::decimal(head, LogicalType::Decimal128)?),
            "decimal256" => needs_no_body(Self::decimal(head, LogicalType::Decimal256)?),
            _ => Err(DataTypeError::Unknown(head.name.to_string())),
        }
    }

    /// Reads the single [`TimeUnit`] parameter of a temporal type.
    fn unit(head: &Head) -> Result<TimeUnit, DataTypeError> {
        let raw = head.params.ok_or_else(|| {
            DataTypeError::Invalid(format!(
                "'{}' needs a unit, e.g. {}(us)",
                head.name, head.name
            ))
        })?;
        TimeUnit::from_str(raw)
    }

    /// Reads the `(precision, scale)` parameters of a decimal type.
    fn decimal(
        head: &Head,
        build: fn(u8, i8) -> LogicalType,
    ) -> Result<LogicalType, DataTypeError> {
        let raw = head.params.ok_or_else(|| {
            DataTypeError::Invalid(format!(
                "'{}' needs (precision, scale), e.g. {}(38, 10)",
                head.name, head.name
            ))
        })?;
        let parts = split_top_level(raw, ',');
        if parts.len() != 2 {
            return Err(DataTypeError::Invalid(format!(
                "'{}' needs exactly (precision, scale)",
                head.name
            )));
        }
        let precision = parts[0]
            .parse::<u8>()
            .map_err(|_| DataTypeError::Invalid("decimal precision must be 0..=255".to_string()))?;
        let scale = parts[1]
            .parse::<i8>()
            .map_err(|_| DataTypeError::Invalid("decimal scale must be -128..=127".to_string()))?;
        Ok(build(precision, scale))
    }

    /// Errors if a name expecting no parameters was given a `(…)` group.
    fn require_no_params(head: &Head) -> Result<(), DataTypeError> {
        if head.params.is_some() {
            return Err(DataTypeError::Invalid(format!(
                "'{}' takes no parameters",
                head.name
            )));
        }
        Ok(())
    }

    /// Renders the canonical name — the inverse of [`from_str`](LogicalType::from_str).
    pub fn to_str(&self) -> String {
        use LogicalType::*;
        match self {
            Date32 => "date32".to_string(),
            Date64 => "date64".to_string(),
            Time32(u) => format!("time32({u})"),
            Time64(u) => format!("time64({u})"),
            Timestamp(u, None) => format!("timestamp({u})"),
            Timestamp(u, Some(tz)) => format!("timestamp({u}, {tz})"),
            Duration(u) => format!("duration({u})"),
            Interval(u) => format!("interval({u})"),
            Decimal32(p, s) => format!("decimal32({p}, {s})"),
            Decimal64(p, s) => format!("decimal64({p}, {s})"),
            Decimal128(p, s) => format!("decimal128({p}, {s})"),
            Decimal256(p, s) => format!("decimal256({p}, {s})"),
        }
    }
}

impl fmt::Display for LogicalType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_str())
    }
}

#[cfg(feature = "arrow")]
impl From<TimeUnit> for arrow_schema::TimeUnit {
    fn from(u: TimeUnit) -> arrow_schema::TimeUnit {
        match u {
            TimeUnit::Second => arrow_schema::TimeUnit::Second,
            TimeUnit::Millisecond => arrow_schema::TimeUnit::Millisecond,
            TimeUnit::Microsecond => arrow_schema::TimeUnit::Microsecond,
            TimeUnit::Nanosecond => arrow_schema::TimeUnit::Nanosecond,
        }
    }
}

#[cfg(feature = "arrow")]
impl From<arrow_schema::TimeUnit> for TimeUnit {
    fn from(u: arrow_schema::TimeUnit) -> TimeUnit {
        match u {
            arrow_schema::TimeUnit::Second => TimeUnit::Second,
            arrow_schema::TimeUnit::Millisecond => TimeUnit::Millisecond,
            arrow_schema::TimeUnit::Microsecond => TimeUnit::Microsecond,
            arrow_schema::TimeUnit::Nanosecond => TimeUnit::Nanosecond,
        }
    }
}

#[cfg(feature = "arrow")]
impl From<IntervalUnit> for arrow_schema::IntervalUnit {
    fn from(u: IntervalUnit) -> arrow_schema::IntervalUnit {
        match u {
            IntervalUnit::YearMonth => arrow_schema::IntervalUnit::YearMonth,
            IntervalUnit::DayTime => arrow_schema::IntervalUnit::DayTime,
            IntervalUnit::MonthDayNano => arrow_schema::IntervalUnit::MonthDayNano,
        }
    }
}

#[cfg(feature = "arrow")]
impl From<arrow_schema::IntervalUnit> for IntervalUnit {
    fn from(u: arrow_schema::IntervalUnit) -> IntervalUnit {
        match u {
            arrow_schema::IntervalUnit::YearMonth => IntervalUnit::YearMonth,
            arrow_schema::IntervalUnit::DayTime => IntervalUnit::DayTime,
            arrow_schema::IntervalUnit::MonthDayNano => IntervalUnit::MonthDayNano,
        }
    }
}

/// Conversion to the matching `arrow_schema::DataType` (infallible).
#[cfg(feature = "arrow")]
impl From<&LogicalType> for arrow_schema::DataType {
    fn from(l: &LogicalType) -> arrow_schema::DataType {
        use arrow_schema::DataType as A;
        use LogicalType::*;
        match l {
            Date32 => A::Date32,
            Date64 => A::Date64,
            Time32(u) => A::Time32((*u).into()),
            Time64(u) => A::Time64((*u).into()),
            Timestamp(u, tz) => A::Timestamp((*u).into(), tz.as_ref().map(|s| s.as_str().into())),
            Duration(u) => A::Duration((*u).into()),
            Interval(u) => A::Interval((*u).into()),
            Decimal32(p, s) => A::Decimal32(*p, *s),
            Decimal64(p, s) => A::Decimal64(*p, *s),
            Decimal128(p, s) => A::Decimal128(*p, *s),
            Decimal256(p, s) => A::Decimal256(*p, *s),
        }
    }
}
