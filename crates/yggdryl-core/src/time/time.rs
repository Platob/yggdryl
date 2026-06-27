//! The [`Time`] type — a time of day, stored as nanoseconds since midnight.

use std::fmt;

#[allow(unused_imports)]
use crate::log_event;
use crate::Mapping;

use super::{Date, DateTime, Temporal, TimeError};

const NANOS_PER_DAY: u64 = 86_400 * 1_000_000_000;

/// A time of day with nanosecond resolution and no date or timezone. Ordered from
/// midnight.
///
/// ```
/// use yggdryl_core::Time;
///
/// let t = Time::from_hms(13, 45, 30).unwrap();
/// assert_eq!((t.hour(), t.minute(), t.second()), (13, 45, 30));
/// assert_eq!(t.to_str(), "13:45:30");
/// assert_eq!(Time::from_str("13:45:30.250").unwrap().nanosecond(), 250_000_000);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Time {
    /// Nanoseconds since midnight, in `0 .. 86_400_000_000_000`.
    nanos_of_day: u64,
}

impl Time {
    /// Builds a time from `hour:minute:second`, validating each component.
    pub fn from_hms(hour: u32, minute: u32, second: u32) -> Result<Time, TimeError> {
        Time::from_hms_nano(hour, minute, second, 0)
    }

    /// Builds a time from `hour:minute:second` plus sub-second nanoseconds.
    pub fn from_hms_nano(
        hour: u32,
        minute: u32,
        second: u32,
        nano: u32,
    ) -> Result<Time, TimeError> {
        if hour > 23 || minute > 59 || second > 59 || nano > 999_999_999 {
            return Err(TimeError::OutOfRange(format!(
                "{hour:02}:{minute:02}:{second:02}.{nano:09}"
            )));
        }
        Ok(Time {
            nanos_of_day: (hour as u64 * 3600 + minute as u64 * 60 + second as u64) * 1_000_000_000
                + nano as u64,
        })
    }

    /// Builds a time from nanoseconds since midnight (must be within one day).
    pub fn from_nanos_of_day(nanos: u64) -> Result<Time, TimeError> {
        if nanos >= NANOS_PER_DAY {
            return Err(TimeError::OutOfRange(format!("{nanos} ns of day")));
        }
        Ok(Time {
            nanos_of_day: nanos,
        })
    }

    /// Nanoseconds since midnight.
    pub fn nanos_of_day(&self) -> u64 {
        self.nanos_of_day
    }

    /// The hour (0–23).
    pub fn hour(&self) -> u32 {
        (self.nanos_of_day / 1_000_000_000 / 3600) as u32
    }

    /// The minute (0–59).
    pub fn minute(&self) -> u32 {
        (self.nanos_of_day / 1_000_000_000 / 60 % 60) as u32
    }

    /// The second (0–59).
    pub fn second(&self) -> u32 {
        (self.nanos_of_day / 1_000_000_000 % 60) as u32
    }

    /// The sub-second nanoseconds (0–999_999_999).
    pub fn nanosecond(&self) -> u32 {
        (self.nanos_of_day % 1_000_000_000) as u32
    }

    /// Parses `HH:MM[:SS[.fraction]]` (fraction up to 9 digits), or a compact
    /// colon-less `HHMM` / `HHMMSS`.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<Time, TimeError> {
        log_event!(trace, "Time::from_str {input:?}");
        let value = input.trim();
        if value.is_empty() {
            return Err(TimeError::Empty);
        }
        // Compact colon-less form: HHMM or HHMMSS.
        if !value.contains([':', '.']) && value.bytes().all(|b| b.is_ascii_digit()) {
            let (h, m, s) = match value.len() {
                4 => (&value[..2], &value[2..4], "0"),
                6 => (&value[..2], &value[2..4], &value[4..6]),
                _ => return Err(TimeError::Invalid(input.to_string())),
            };
            return Time::from_hms(
                h.parse()
                    .map_err(|_| TimeError::Invalid(input.to_string()))?,
                m.parse()
                    .map_err(|_| TimeError::Invalid(input.to_string()))?,
                s.parse()
                    .map_err(|_| TimeError::Invalid(input.to_string()))?,
            );
        }
        let (clock, frac) = match value.split_once('.') {
            Some((clock, frac)) => (clock, Some(frac)),
            None => (value, None),
        };
        let mut parts = clock.split(':');
        let hour = parts
            .next()
            .and_then(|p| p.parse::<u32>().ok())
            .ok_or_else(|| TimeError::Invalid(input.to_string()))?;
        let minute = parts
            .next()
            .and_then(|p| p.parse::<u32>().ok())
            .ok_or_else(|| TimeError::Invalid(input.to_string()))?;
        let second = match parts.next() {
            Some(p) => p
                .parse::<u32>()
                .map_err(|_| TimeError::Invalid(input.to_string()))?,
            None => 0,
        };
        if parts.next().is_some() {
            return Err(TimeError::Invalid(input.to_string()));
        }
        let nano = match frac {
            Some(frac) if frac.bytes().all(|b| b.is_ascii_digit()) && !frac.is_empty() => {
                let mut digits = frac.to_string();
                digits.truncate(9);
                while digits.len() < 9 {
                    digits.push('0');
                }
                digits
                    .parse::<u32>()
                    .map_err(|_| TimeError::Invalid(input.to_string()))?
            }
            Some(_) => return Err(TimeError::Invalid(input.to_string())),
            None => 0,
        };
        Time::from_hms_nano(hour, minute, second, nano)
    }

    /// Builds a time from a [`Mapping`] (`hour` / `minute` / `second` / `nanosecond`).
    pub fn from_mapping(fields: &Mapping) -> Result<Time, TimeError> {
        let component = |key: &str, default: u32| -> Result<u32, TimeError> {
            match fields.get(key) {
                Some(v) => v
                    .parse::<u32>()
                    .map_err(|_| TimeError::Invalid(format!("'{key}'"))),
                None => Ok(default),
            }
        };
        Time::from_hms_nano(
            component("hour", 0)?,
            component("minute", 0)?,
            component("second", 0)?,
            component("nanosecond", 0)?,
        )
    }

    /// Renders `HH:MM:SS`, adding a fractional part only when non-zero (trimmed to
    /// milli / micro / nano precision as needed).
    pub fn to_str(&self) -> String {
        let base = format!(
            "{:02}:{:02}:{:02}",
            self.hour(),
            self.minute(),
            self.second()
        );
        let nano = self.nanosecond();
        if nano == 0 {
            base
        } else if nano.is_multiple_of(1_000_000) {
            format!("{base}.{:03}", nano / 1_000_000)
        } else if nano.is_multiple_of(1_000) {
            format!("{base}.{:06}", nano / 1_000)
        } else {
            format!("{base}.{nano:09}")
        }
    }

    /// Renders to a component [`Mapping`] (`hour` / `minute` / `second` / `nanosecond`).
    pub fn to_mapping(&self) -> Mapping {
        Mapping::from([
            ("hour".to_string(), self.hour().to_string()),
            ("minute".to_string(), self.minute().to_string()),
            ("second".to_string(), self.second().to_string()),
            ("nanosecond".to_string(), self.nanosecond().to_string()),
        ])
    }

    /// The canonical string as UTF-8 bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_str().into_bytes()
    }

    /// Parses a time from the UTF-8 bytes of its canonical string.
    pub fn from_bytes(bytes: &[u8]) -> Result<Time, TimeError> {
        let value = std::str::from_utf8(bytes).map_err(|_| TimeError::Invalid("<bytes>".into()))?;
        Time::from_str(value)
    }
}

impl Temporal for Time {
    /// This time of day on the UNIX-epoch day (1970-01-01), naive.
    fn to_datetime(&self) -> DateTime {
        DateTime::from_local(Date::from_epoch_days(0), *self, None)
    }

    fn to_time(&self) -> Time {
        *self
    }
}

impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_str())
    }
}
