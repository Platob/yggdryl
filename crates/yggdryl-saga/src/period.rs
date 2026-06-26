//! The [`Period`] — a fixed-width time span (in nanoseconds) used as a
//! [`resample`](crate::DataFrame::resample) bucket size, e.g. `1h` / `5m` / `100ms`.
//! It is calendar-agnostic: `1d` is exactly 86 400 s (no DST / month length).

use std::fmt;

#[allow(unused_imports)]
use crate::log_event;

/// Error returned when a [`Period`] cannot be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeriodError {
    /// The input was empty.
    Empty,
    /// The input was not `<integer><unit>` with a known unit.
    Invalid(String),
}

impl fmt::Display for PeriodError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PeriodError::Empty => write!(f, "period is empty"),
            PeriodError::Invalid(value) => write!(
                f,
                "invalid period '{value}', expected '<n><unit>' with unit ns/us/ms/s/m/h/d (e.g. 1h, 5m, 100ms)"
            ),
        }
    }
}

impl std::error::Error for PeriodError {}

/// The recognised units, largest first, as `(suffix, nanoseconds)`.
const UNITS: &[(&str, i64)] = &[
    ("d", 86_400_000_000_000),
    ("h", 3_600_000_000_000),
    ("m", 60_000_000_000),
    ("s", 1_000_000_000),
    ("ms", 1_000_000),
    ("us", 1_000),
    ("ns", 1),
];

/// A fixed-width span of time, stored as a positive number of nanoseconds.
///
/// ```
/// use yggdryl_saga::Period;
///
/// assert_eq!(Period::from_str("1h").unwrap().nanos(), 3_600_000_000_000);
/// assert_eq!(Period::from_str("100ms").unwrap().to_str(), "100ms");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Period {
    nanos: i64,
}

impl Period {
    /// Builds a period from a positive nanosecond count.
    pub fn from_nanos(nanos: i64) -> Period {
        Period { nanos }
    }

    /// The span in nanoseconds.
    pub fn nanos(&self) -> i64 {
        self.nanos
    }

    /// Parses `<integer><unit>` (unit one of `ns` / `us` / `ms` / `s` / `m` / `h` /
    /// `d`), e.g. `5m`, `1h`, `100ms`. The count must be a positive integer.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<Period, PeriodError> {
        log_event!(trace, "Period::from_str {input:?}");
        let s = input.trim();
        if s.is_empty() {
            return Err(PeriodError::Empty);
        }
        // Split the leading digits from the trailing unit.
        let split = s
            .find(|c: char| !c.is_ascii_digit())
            .ok_or_else(|| PeriodError::Invalid(s.to_string()))?;
        let (number, unit) = s.split_at(split);
        let count: i64 = number
            .parse()
            .map_err(|_| PeriodError::Invalid(s.to_string()))?;
        if count <= 0 {
            return Err(PeriodError::Invalid(s.to_string()));
        }
        let unit_nanos = UNITS
            .iter()
            .find(|(suffix, _)| *suffix == unit)
            .map(|(_, n)| *n)
            .ok_or_else(|| PeriodError::Invalid(s.to_string()))?;
        Ok(Period {
            nanos: count * unit_nanos,
        })
    }

    /// Renders the period using the largest unit that divides it evenly — the
    /// inverse of [`from_str`](Period::from_str).
    pub fn to_str(&self) -> String {
        for (suffix, unit_nanos) in UNITS {
            if self.nanos % unit_nanos == 0 {
                return format!("{}{suffix}", self.nanos / unit_nanos);
            }
        }
        format!("{}ns", self.nanos)
    }
}

impl fmt::Display for Period {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_units() {
        assert_eq!(Period::from_str("1ns").unwrap().nanos(), 1);
        assert_eq!(Period::from_str("5us").unwrap().nanos(), 5_000);
        assert_eq!(Period::from_str("100ms").unwrap().nanos(), 100_000_000);
        assert_eq!(Period::from_str("30s").unwrap().nanos(), 30_000_000_000);
        assert_eq!(Period::from_str("15m").unwrap().nanos(), 900_000_000_000);
        assert_eq!(Period::from_str("1h").unwrap().nanos(), 3_600_000_000_000);
        assert_eq!(Period::from_str("1d").unwrap().nanos(), 86_400_000_000_000);
    }

    #[test]
    fn round_trips_via_largest_unit() {
        assert_eq!(Period::from_str("60s").unwrap().to_str(), "1m");
        assert_eq!(Period::from_str("100ms").unwrap().to_str(), "100ms");
        assert_eq!(Period::from_str("1h").unwrap().to_str(), "1h");
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(Period::from_str(""), Err(PeriodError::Empty));
        assert!(Period::from_str("h").is_err());
        assert!(Period::from_str("1y").is_err());
        assert!(Period::from_str("0s").is_err());
        assert!(Period::from_str("-1s").is_err());
    }
}
