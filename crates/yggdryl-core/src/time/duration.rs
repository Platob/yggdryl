//! The [`Duration`] type — a signed span of time, stored as nanoseconds.

use std::fmt;

#[allow(unused_imports)]
use crate::log_event;
use crate::Mapping;

use super::{TimeError, TimeUnit};

const NS_PER_SEC: i128 = 1_000_000_000;
const NS_PER_MIN: i128 = 60 * NS_PER_SEC;
const NS_PER_HOUR: i128 = 60 * NS_PER_MIN;
const NS_PER_DAY: i128 = 24 * NS_PER_HOUR;

/// A signed span of time with nanosecond resolution. Ordered by length.
///
/// ```
/// use yggdryl_core::{Duration, TimeUnit};
///
/// let d = Duration::from_str("1h30m").unwrap();
/// assert_eq!(d.as_seconds(), 5_400);
/// assert_eq!(d.to_str(), "1h30m");
/// assert_eq!(Duration::from_unit(500, TimeUnit::Millisecond).to_str(), "500ms");
/// assert!(Duration::from_secs(-5).is_negative());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Duration {
    nanos: i128,
}

impl Duration {
    /// A span of `nanos` nanoseconds.
    pub fn from_nanos(nanos: i128) -> Duration {
        Duration { nanos }
    }

    /// A span of `seconds` seconds.
    pub fn from_secs(seconds: i64) -> Duration {
        Duration {
            nanos: seconds as i128 * NS_PER_SEC,
        }
    }

    /// A span of `millis` milliseconds.
    pub fn from_millis(millis: i64) -> Duration {
        Duration {
            nanos: millis as i128 * 1_000_000,
        }
    }

    /// A span of `micros` microseconds.
    pub fn from_micros(micros: i64) -> Duration {
        Duration {
            nanos: micros as i128 * 1_000,
        }
    }

    /// A span of `value` of the given [`TimeUnit`].
    pub fn from_unit(value: i64, unit: TimeUnit) -> Duration {
        Duration {
            nanos: value as i128 * unit.nanos() as i128,
        }
    }

    /// The whole seconds (truncated toward zero).
    pub fn as_seconds(&self) -> i64 {
        (self.nanos / NS_PER_SEC) as i64
    }

    /// The total milliseconds (truncated toward zero).
    pub fn as_millis(&self) -> i128 {
        self.nanos / 1_000_000
    }

    /// The total microseconds (truncated toward zero).
    pub fn as_micros(&self) -> i128 {
        self.nanos / 1_000
    }

    /// The total nanoseconds.
    pub fn as_nanos(&self) -> i128 {
        self.nanos
    }

    /// The span as fractional seconds.
    pub fn as_seconds_f64(&self) -> f64 {
        self.nanos as f64 / NS_PER_SEC as f64
    }

    /// Whether the span is zero.
    pub fn is_zero(&self) -> bool {
        self.nanos == 0
    }

    /// Whether the span is negative.
    pub fn is_negative(&self) -> bool {
        self.nanos < 0
    }

    /// The absolute (non-negative) span.
    pub fn abs(&self) -> Duration {
        Duration {
            nanos: self.nanos.abs(),
        }
    }

    /// The negated span.
    pub fn negate(&self) -> Duration {
        Duration { nanos: -self.nanos }
    }

    /// The sum of two spans.
    pub fn add(&self, other: &Duration) -> Duration {
        Duration {
            nanos: self.nanos + other.nanos,
        }
    }

    /// The difference of two spans.
    pub fn sub(&self, other: &Duration) -> Duration {
        Duration {
            nanos: self.nanos - other.nanos,
        }
    }

    /// The span scaled by an integer `factor`.
    pub fn mul(&self, factor: i64) -> Duration {
        Duration {
            nanos: self.nanos * factor as i128,
        }
    }

    /// The span divided by an integer `divisor` (truncating toward zero); dividing by
    /// `0` yields a zero span.
    pub fn div(&self, divisor: i64) -> Duration {
        Duration {
            nanos: self.nanos.checked_div(divisor as i128).unwrap_or(0),
        }
    }

    /// Parses a compact span like `"1h30m"`, `"1s500ms"`, `"-2d"`, a plain number of
    /// seconds (`"90"`, `"1.5"`), with units `d` / `h` / `m` / `s` / `ms` / `us` /
    /// `ns`. The inverse of [`to_str`](Duration::to_str).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<Duration, TimeError> {
        log_event!(trace, "Duration::from_str {input:?}");
        let value = input.trim();
        // An empty span parses to the zero default (no error), mirroring an empty
        // numeric field decoding to 0.
        if value.is_empty() {
            return Ok(Duration::from_nanos(0));
        }
        let negative = value.starts_with('-');
        // Strip at most one leading sign; a second sign (`--5`, `+-5`) is malformed.
        let body = value.strip_prefix(['+', '-']).unwrap_or(value);
        if body.starts_with(['+', '-']) {
            return Err(TimeError::Invalid(input.to_string()));
        }
        let sign: i128 = if negative { -1 } else { 1 };
        // ISO-8601 duration: P[nY][nM][nW][nD][T[nH][nM][nS]] (Y≈365d, M≈30d).
        if body.starts_with(['P', 'p']) {
            return parse_iso8601(body, sign, input);
        }
        // A bare number is seconds (fractional allowed).
        if let Ok(secs) = body.parse::<i64>() {
            return Ok(Duration::from_nanos(sign * secs as i128 * NS_PER_SEC));
        }
        if !body.bytes().any(|b| b.is_ascii_alphabetic()) {
            if let Ok(secs) = body.parse::<f64>() {
                return Ok(Duration::from_nanos(
                    (sign as f64 * secs * NS_PER_SEC as f64) as i128,
                ));
            }
            return Err(TimeError::Invalid(input.to_string()));
        }
        let bytes = body.as_bytes();
        let mut pos = 0;
        let mut total: i128 = 0;
        let mut matched = false;
        while pos < bytes.len() {
            let num_start = pos;
            while pos < bytes.len() && (bytes[pos].is_ascii_digit() || bytes[pos] == b'.') {
                pos += 1;
            }
            if pos == num_start {
                return Err(TimeError::Invalid(input.to_string()));
            }
            let number = &body[num_start..pos];
            let unit_start = pos;
            while pos < bytes.len() && bytes[pos].is_ascii_alphabetic() {
                pos += 1;
            }
            let unit = &body[unit_start..pos];
            let scale = match unit {
                "d" => NS_PER_DAY,
                "h" => NS_PER_HOUR,
                "m" => NS_PER_MIN,
                "s" => NS_PER_SEC,
                "ms" => 1_000_000,
                "us" => 1_000,
                "ns" => 1,
                _ => return Err(TimeError::UnknownUnit(unit.to_string())),
            };
            if number.contains('.') {
                let f = number
                    .parse::<f64>()
                    .map_err(|_| TimeError::Invalid(input.to_string()))?;
                total += (f * scale as f64) as i128;
            } else {
                let n = number
                    .parse::<i128>()
                    .map_err(|_| TimeError::Invalid(input.to_string()))?;
                total += n * scale;
            }
            matched = true;
        }
        if !matched {
            return Err(TimeError::Invalid(input.to_string()));
        }
        Ok(Duration::from_nanos(sign * total))
    }

    /// Builds a span from a [`Mapping`] (`nanoseconds`).
    pub fn from_mapping(fields: &Mapping) -> Result<Duration, TimeError> {
        let nanos = fields
            .get("nanoseconds")
            .ok_or_else(|| TimeError::Invalid("missing 'nanoseconds'".into()))?
            .parse::<i128>()
            .map_err(|_| TimeError::Invalid("'nanoseconds' is not an integer".into()))?;
        Ok(Duration::from_nanos(nanos))
    }

    /// Renders the canonical compact form, e.g. `"1h30m"`, `"1s500ms"` or `"0s"`.
    pub fn to_str(&self) -> String {
        if self.nanos == 0 {
            return "0s".to_string();
        }
        let mut out = String::new();
        if self.nanos < 0 {
            out.push('-');
        }
        let mut rem = self.nanos.abs();
        for (scale, unit) in [
            (NS_PER_DAY, "d"),
            (NS_PER_HOUR, "h"),
            (NS_PER_MIN, "m"),
            (NS_PER_SEC, "s"),
        ] {
            let value = rem / scale;
            if value != 0 {
                out.push_str(&format!("{value}{unit}"));
                rem %= scale;
            }
        }
        if rem != 0 {
            if rem % 1_000_000 == 0 {
                out.push_str(&format!("{}ms", rem / 1_000_000));
            } else if rem % 1_000 == 0 {
                out.push_str(&format!("{}us", rem / 1_000));
            } else {
                out.push_str(&format!("{rem}ns"));
            }
        }
        out
    }

    /// Renders to a component [`Mapping`] (`nanoseconds`).
    pub fn to_mapping(&self) -> Mapping {
        Mapping::from([("nanoseconds".to_string(), self.nanos.to_string())])
    }

    /// The canonical string as UTF-8 bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_str().into_bytes()
    }

    /// Parses a span from the UTF-8 bytes of its canonical string.
    pub fn from_bytes(bytes: &[u8]) -> Result<Duration, TimeError> {
        let value = std::str::from_utf8(bytes).map_err(|_| TimeError::Invalid("<bytes>".into()))?;
        Duration::from_str(value)
    }
}

/// Parses an ISO-8601 duration (`body` begins with `P`). Calendar fields are
/// approximated: a year is 365 days and a month is 30 days.
fn parse_iso8601(body: &str, sign: i128, input: &str) -> Result<Duration, TimeError> {
    let after_p = &body[1..];
    let split = after_p.find(['T', 't']);
    let date_part = split.map_or(after_p, |i| &after_p[..i]);
    let time_part = split.map_or("", |i| &after_p[i + 1..]);
    let mut total: i128 = 0;
    let mut matched = false;
    accumulate_iso(date_part, false, &mut total, &mut matched, input)?;
    accumulate_iso(time_part, true, &mut total, &mut matched, input)?;
    if !matched {
        return Err(TimeError::Invalid(input.to_string()));
    }
    Ok(Duration::from_nanos(sign * total))
}

/// Accumulates one ISO-8601 segment (`time` selects H/M/S vs Y/M/W/D semantics).
fn accumulate_iso(
    segment: &str,
    time: bool,
    total: &mut i128,
    matched: &mut bool,
    input: &str,
) -> Result<(), TimeError> {
    let invalid = || TimeError::Invalid(input.to_string());
    let bytes = segment.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() {
        let start = pos;
        while pos < bytes.len() && (bytes[pos].is_ascii_digit() || bytes[pos] == b'.') {
            pos += 1;
        }
        if pos == start || pos >= bytes.len() {
            return Err(invalid());
        }
        let number = &segment[start..pos];
        let scale: i128 = match (bytes[pos].to_ascii_uppercase(), time) {
            (b'Y', false) => 365 * NS_PER_DAY,
            (b'M', false) => 30 * NS_PER_DAY,
            (b'W', false) => 7 * NS_PER_DAY,
            (b'D', false) => NS_PER_DAY,
            (b'H', true) => NS_PER_HOUR,
            (b'M', true) => NS_PER_MIN,
            (b'S', true) => NS_PER_SEC,
            _ => return Err(invalid()),
        };
        pos += 1;
        if number.contains('.') {
            let value = number.parse::<f64>().map_err(|_| invalid())?;
            *total += (value * scale as f64) as i128;
        } else {
            let value = number.parse::<i128>().map_err(|_| invalid())?;
            *total += value * scale;
        }
        *matched = true;
    }
    Ok(())
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_str())
    }
}

impl std::ops::Add for Duration {
    type Output = Duration;
    fn add(self, rhs: Duration) -> Duration {
        Duration::add(&self, &rhs)
    }
}

impl std::ops::Sub for Duration {
    type Output = Duration;
    fn sub(self, rhs: Duration) -> Duration {
        Duration::sub(&self, &rhs)
    }
}

impl std::ops::Mul<i64> for Duration {
    type Output = Duration;
    fn mul(self, rhs: i64) -> Duration {
        Duration::mul(&self, rhs)
    }
}

impl std::ops::Div<i64> for Duration {
    type Output = Duration;
    fn div(self, rhs: i64) -> Duration {
        Duration::div(&self, rhs)
    }
}

impl std::ops::Neg for Duration {
    type Output = Duration;
    fn neg(self) -> Duration {
        self.negate()
    }
}
