//! Type casting: the [`CastError`], the [`DataType::can_cast_to`] compatibility
//! rule, and the dependency-free ISO-8601 → epoch helpers that let a string
//! literal be cast to a temporal type (the optimisation that types a filter value
//! for pushdown). **All casting rules live here.**

use std::fmt;

#[allow(unused_imports)]
use crate::log_event;
use crate::{DataType, LogicalType, TimeUnit};

/// Error returned when a value or type cannot be cast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CastError {
    /// No cast is defined between the two types.
    Unsupported {
        /// The source type.
        from: DataType,
        /// The requested target type.
        to: DataType,
    },
    /// The value did not parse into the target type (e.g. a non-ISO string cast to
    /// a timestamp).
    InvalidValue {
        /// The offending value, rendered.
        value: String,
        /// The type it failed to become.
        target: DataType,
    },
}

impl fmt::Display for CastError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CastError::Unsupported { from, to } => {
                write!(f, "cannot cast from {from} to {to}")
            }
            CastError::InvalidValue { value, target } => {
                write!(f, "value '{value}' is not a valid {target}")
            }
        }
    }
}

impl std::error::Error for CastError {}

impl DataType {
    /// Whether a value of this type can be cast to `target`.
    ///
    /// The dynamic [`Any`](DataType::Any) type casts to and from everything;
    /// otherwise numbers, booleans and strings interconvert, strings and integers
    /// convert to and from the temporal types (the ISO-date → timestamp path), and
    /// the temporal types interconvert. Nested types only cast to themselves.
    ///
    /// ```
    /// use yggdryl_saga::DataType;
    ///
    /// let utf8 = DataType::from_str("utf8").unwrap();
    /// let ts = DataType::from_str("timestamp(ns, UTC)").unwrap();
    /// assert!(utf8.can_cast_to(&ts));            // "2024-01-01" -> timestamp
    /// assert!(DataType::Any.can_cast_to(&ts));   // dynamic literal -> timestamp
    /// assert!(!DataType::from_str("struct<a: int64>").unwrap().can_cast_to(&utf8));
    /// ```
    pub fn can_cast_to(&self, target: &DataType) -> bool {
        if self == target || self.is_any() || target.is_any() {
            return true;
        }
        // The null type widens to anything.
        if matches!(self, DataType::Primitive(crate::PrimitiveType::Null)) {
            return true;
        }
        let scalarish = |dt: &DataType| {
            dt.is_numeric()
                || dt.is_string()
                || dt.is_temporal()
                || dt.is_boolean()
                || dt.is_decimal()
        };
        scalarish(self) && scalarish(target)
    }

    /// `true` for the boolean primitive.
    pub(crate) fn is_boolean(&self) -> bool {
        matches!(self, DataType::Primitive(crate::PrimitiveType::Boolean))
    }

    /// `true` for the decimal logical types.
    pub(crate) fn is_decimal(&self) -> bool {
        matches!(
            self,
            DataType::Logical(
                LogicalType::Decimal32(..)
                    | LogicalType::Decimal64(..)
                    | LogicalType::Decimal128(..)
                    | LogicalType::Decimal256(..)
            )
        )
    }
}

/// The number of days from the civil date `y-m-d` to the UNIX epoch
/// (1970-01-01), by Howard Hinnant's `days_from_civil` algorithm. Valid for any
/// proleptic-Gregorian date.
pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// A parsed ISO-8601 instant, decomposed into a day count and an intra-day offset.
pub(crate) struct IsoInstant {
    /// Days since the UNIX epoch.
    pub days: i64,
    /// Seconds since midnight (`0..86_400`).
    pub seconds_of_day: i64,
    /// Sub-second nanoseconds (`0..1_000_000_000`).
    pub nanos: i64,
}

impl IsoInstant {
    /// The epoch offset in `unit` ticks (e.g. nanoseconds for
    /// [`TimeUnit::Nanosecond`]).
    pub fn epoch(&self, unit: TimeUnit) -> i64 {
        let total_nanos =
            (self.days * 86_400 + self.seconds_of_day) as i128 * 1_000_000_000 + self.nanos as i128;
        let scaled = match unit {
            TimeUnit::Second => total_nanos / 1_000_000_000,
            TimeUnit::Millisecond => total_nanos / 1_000_000,
            TimeUnit::Microsecond => total_nanos / 1_000,
            TimeUnit::Nanosecond => total_nanos,
        };
        scaled as i64
    }

    /// The intra-day offset in `unit` ticks (for the `time32` / `time64` types).
    pub fn time_of_day(&self, unit: TimeUnit) -> i64 {
        let nanos = self.seconds_of_day as i128 * 1_000_000_000 + self.nanos as i128;
        let scaled = match unit {
            TimeUnit::Second => nanos / 1_000_000_000,
            TimeUnit::Millisecond => nanos / 1_000_000,
            TimeUnit::Microsecond => nanos / 1_000,
            TimeUnit::Nanosecond => nanos,
        };
        scaled as i64
    }
}

/// Parses an ISO-8601 date (`YYYY-MM-DD`) or date-time
/// (`YYYY-MM-DD[T| ]HH:MM:SS[.fffffffff][Z]`) into an [`IsoInstant`]. A trailing
/// `Z` is accepted and ignored (the value is treated as UTC). Returns `None` on
/// any malformed component.
pub(crate) fn parse_iso(input: &str) -> Option<IsoInstant> {
    let s = input.trim();
    // Split the date and (optional) time on the first 'T' or ' '.
    let (date, time) = match s.find(['T', ' ']) {
        Some(i) => (&s[..i], Some(s[i + 1..].trim_end_matches('Z'))),
        None => (s, None),
    };

    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month, day);

    let (seconds_of_day, nanos) = match time {
        None | Some("") => (0, 0),
        Some(t) => {
            let mut hms = t.split(':');
            let hour: i64 = hms.next()?.parse().ok()?;
            let minute: i64 = hms.next()?.parse().ok()?;
            let (sec, nanos) = match hms.next() {
                Some(sec_part) => {
                    let mut sf = sec_part.split('.');
                    let sec: i64 = sf.next()?.parse().ok()?;
                    let nanos = match sf.next() {
                        Some(frac) => {
                            // Pad/truncate the fraction to 9 digits (nanoseconds).
                            let mut digits: String = frac.chars().take(9).collect();
                            while digits.len() < 9 {
                                digits.push('0');
                            }
                            digits.parse::<i64>().ok()?
                        }
                        None => 0,
                    };
                    (sec, nanos)
                }
                None => (0, 0),
            };
            if hms.next().is_some()
                || !(0..=23).contains(&hour)
                || !(0..=59).contains(&minute)
                || !(0..=60).contains(&sec)
            {
                return None;
            }
            (hour * 3600 + minute * 60 + sec, nanos)
        }
    };

    Some(IsoInstant {
        days,
        seconds_of_day,
        nanos,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PrimitiveType;

    #[test]
    fn epoch_day_zero_is_1970() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        // 2024-01-01 is 19723 days after the epoch.
        assert_eq!(days_from_civil(2024, 1, 1), 19723);
    }

    #[test]
    fn parses_iso_date_and_datetime() {
        let date = parse_iso("2024-01-01").unwrap();
        assert_eq!(date.days, 19723);
        assert_eq!(
            date.epoch(TimeUnit::Nanosecond),
            19723 * 86_400 * 1_000_000_000
        );

        let dt = parse_iso("2024-01-01T00:00:01.5Z").unwrap();
        assert_eq!(dt.nanos, 500_000_000);
        assert_eq!(
            dt.epoch(TimeUnit::Millisecond),
            (19723 * 86_400 + 1) * 1_000 + 500
        );
        assert_eq!(dt.time_of_day(TimeUnit::Second), 1);

        assert!(parse_iso("not-a-date").is_none());
        assert!(parse_iso("2024-13-01").is_none());
    }

    #[test]
    fn cast_compatibility() {
        let utf8 = DataType::from(PrimitiveType::Utf8);
        let ts = DataType::from_str("timestamp(ns, UTC)").unwrap();
        let int = DataType::from(PrimitiveType::Int64);
        let struct_ty = DataType::from_str("struct<a: int64>").unwrap();

        assert!(utf8.can_cast_to(&ts));
        assert!(ts.can_cast_to(&utf8));
        assert!(int.can_cast_to(&ts));
        assert!(utf8.can_cast_to(&int));
        assert!(DataType::Any.can_cast_to(&ts));
        assert!(ts.can_cast_to(&DataType::Any));
        // Nested types only cast to themselves.
        assert!(!struct_ty.can_cast_to(&utf8));
        assert!(struct_ty.can_cast_to(&struct_ty));
        // The null type widens to anything.
        assert!(DataType::from(PrimitiveType::Null).can_cast_to(&ts));
    }
}
