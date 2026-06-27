//! The [`DateTime`] type — an absolute instant (UTC seconds + nanoseconds) with an
//! optional [`Timezone`] for display. Timezone conversions are DST-aware via the
//! zone's embedded rule.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

#[allow(unused_imports)]
use crate::log_event;
use crate::Mapping;

use super::{Date, Temporal, Time, TimeError, Timezone};

/// An instant in time, stored as UTC epoch seconds + sub-second nanoseconds, with
/// an optional [`Timezone`] used only for display / civil-field extraction. A naive
/// `DateTime` (no zone) is interpreted as UTC for its instant.
///
/// ```
/// use yggdryl_core::{DateTime, Timezone};
///
/// let utc = DateTime::from_str("2024-07-01T12:00:00Z").unwrap();
/// // The same instant displayed in Tokyo (UTC+9, no DST) and New York (EDT, UTC-4).
/// assert_eq!(utc.to_timezone(Timezone::from_str("Asia/Tokyo").unwrap()).hour(), 21);
/// assert_eq!(utc.to_timezone(Timezone::from_str("America/New_York").unwrap()).hour(), 8);
/// assert_eq!(utc.epoch_seconds(), 1_719_835_200);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DateTime {
    /// UTC epoch seconds.
    seconds: i64,
    /// Sub-second nanoseconds (0–999_999_999).
    nanos: u32,
    /// Optional display timezone (`None` = naive, interpreted as UTC).
    timezone: Option<Timezone>,
}

impl DateTime {
    /// Builds an instant from civil components in `timezone` (`None` = naive/UTC).
    /// The local time is resolved to UTC using the zone's offset (DST-aware).
    #[allow(clippy::too_many_arguments)]
    pub fn from_ymd_hms(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
        nano: u32,
        timezone: Option<Timezone>,
    ) -> Result<DateTime, TimeError> {
        let date = Date::from_ymd(year, month, day)?;
        let time = Time::from_hms_nano(hour, minute, second, nano)?;
        Ok(DateTime::from_local(date, time, timezone))
    }

    /// Builds an instant from a local [`Date`] + [`Time`] in `timezone`.
    pub fn from_local(date: Date, time: Time, timezone: Option<Timezone>) -> DateTime {
        let local_seconds =
            date.epoch_days() as i64 * 86_400 + (time.nanos_of_day() / 1_000_000_000) as i64;
        let nanos = time.nanosecond();
        let seconds = match &timezone {
            None => local_seconds,
            Some(tz) => {
                // Resolve local -> UTC: the offset itself depends on the instant, so
                // guess from the wall time then refine. Near a DST transition the wall
                // time is ambiguous (fold) or non-existent (gap); only accept the
                // refined offset when it is self-consistent, otherwise keep the first
                // (pre-transition) offset — a deterministic resolution matching
                // Python's `fold=0` for both the spring gap and the autumn fold.
                let guess = tz.offset_seconds(local_seconds);
                let refined = tz.offset_seconds(local_seconds - guess as i64);
                let chosen = if tz.offset_seconds(local_seconds - refined as i64) == refined {
                    refined
                } else {
                    guess
                };
                local_seconds - chosen as i64
            }
        };
        DateTime {
            seconds,
            nanos,
            timezone,
        }
    }

    /// The current instant in UTC.
    pub fn now() -> DateTime {
        let since = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        DateTime {
            seconds: since.as_secs() as i64,
            nanos: since.subsec_nanos(),
            timezone: Some(Timezone::Utc),
        }
    }

    /// Builds an instant from UTC epoch seconds, with an optional display timezone.
    pub fn from_epoch_seconds(seconds: i64, timezone: Option<Timezone>) -> DateTime {
        DateTime {
            seconds,
            nanos: 0,
            timezone,
        }
    }

    /// Builds an instant from UTC epoch nanoseconds, with an optional display timezone.
    pub fn from_epoch_nanos(nanos: i128, timezone: Option<Timezone>) -> DateTime {
        DateTime {
            seconds: (nanos.div_euclid(1_000_000_000)) as i64,
            nanos: (nanos.rem_euclid(1_000_000_000)) as u32,
            timezone,
        }
    }

    /// UTC epoch seconds.
    pub fn epoch_seconds(&self) -> i64 {
        self.seconds
    }

    /// UTC epoch milliseconds (saturates rather than overflowing for extreme years).
    pub fn epoch_millis(&self) -> i64 {
        self.seconds
            .saturating_mul(1_000)
            .saturating_add((self.nanos / 1_000_000) as i64)
    }

    /// UTC epoch microseconds (saturates rather than overflowing for extreme years).
    pub fn epoch_micros(&self) -> i64 {
        self.seconds
            .saturating_mul(1_000_000)
            .saturating_add((self.nanos / 1_000) as i64)
    }

    /// UTC epoch nanoseconds (128-bit to avoid overflow).
    pub fn epoch_nanos(&self) -> i128 {
        self.seconds as i128 * 1_000_000_000 + self.nanos as i128
    }

    /// The display timezone, or `None` if naive.
    pub fn timezone(&self) -> Option<&Timezone> {
        self.timezone.as_ref()
    }

    /// The offset east of UTC, in seconds, at this instant in its display zone
    /// (`0` if naive).
    pub fn offset_seconds(&self) -> i32 {
        self.timezone
            .as_ref()
            .map(|tz| tz.offset_seconds(self.seconds))
            .unwrap_or(0)
    }

    /// Local seconds (UTC instant shifted by the display offset).
    fn local_seconds(&self) -> i64 {
        self.seconds + self.offset_seconds() as i64
    }

    /// The local calendar [`Date`].
    pub fn date(&self) -> Date {
        Date::from_epoch_days(self.local_seconds().div_euclid(86_400) as i32)
    }

    /// The local [`Time`] of day.
    pub fn time(&self) -> Time {
        let nanos =
            self.local_seconds().rem_euclid(86_400) as u64 * 1_000_000_000 + self.nanos as u64;
        Time::from_nanos_of_day(nanos % (86_400 * 1_000_000_000))
            .unwrap_or_else(|_| Time::from_hms(0, 0, 0).expect("midnight is valid"))
    }

    /// The local year.
    pub fn year(&self) -> i32 {
        self.date().year()
    }

    /// The local month (1–12).
    pub fn month(&self) -> u32 {
        self.date().month()
    }

    /// The local day (1–31).
    pub fn day(&self) -> u32 {
        self.date().day()
    }

    /// The local hour (0–23).
    pub fn hour(&self) -> u32 {
        self.time().hour()
    }

    /// The local minute (0–59).
    pub fn minute(&self) -> u32 {
        self.time().minute()
    }

    /// The local second (0–59).
    pub fn second(&self) -> u32 {
        self.time().second()
    }

    /// The sub-second nanoseconds.
    pub fn nanosecond(&self) -> u32 {
        self.nanos
    }

    /// The same instant displayed in `timezone` (a pure display change — the
    /// underlying UTC instant is unchanged).
    pub fn to_timezone(&self, timezone: Timezone) -> DateTime {
        DateTime {
            seconds: self.seconds,
            nanos: self.nanos,
            timezone: Some(timezone),
        }
    }

    /// The same instant displayed in UTC.
    pub fn to_utc(&self) -> DateTime {
        self.to_timezone(Timezone::Utc)
    }

    /// The same instant displayed in the named zone — a string-keyed convenience
    /// over [`to_timezone`](DateTime::to_timezone) (e.g. `convert("Asia/Tokyo")`).
    pub fn convert(&self, timezone: &str) -> Result<DateTime, TimeError> {
        Ok(self.to_timezone(Timezone::from_str(timezone)?))
    }

    /// Parses a datetime, flexibly: an ISO-8601 `YYYY-MM-DD` then `T`/space then
    /// `HH:MM[:SS[.frac]]` then an optional `Z` / `±HH:MM` offset (no offset =
    /// naive); a **date-only** string (→ midnight); or a **bare integer** of epoch
    /// seconds (→ UTC).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<DateTime, TimeError> {
        log_event!(trace, "DateTime::from_str {input:?}");
        let value = input.trim();
        if value.is_empty() {
            return Err(TimeError::Empty);
        }
        // A bare integer is epoch seconds (a compact 8-digit YYYYMMDD is a date).
        if value.bytes().all(|b| b.is_ascii_digit()) && value.len() != 8 {
            let seconds = value
                .parse::<i64>()
                .map_err(|_| TimeError::Invalid(input.to_string()))?;
            return Ok(DateTime::from_epoch_seconds(seconds, Some(Timezone::Utc)));
        }
        // No time component -> a date-only string at midnight (keeping its zone).
        let Some(split) = value.find(['T', 't']).or_else(|| value.find(' ')) else {
            let date = Date::from_str(value)?;
            let timezone = date.timezone().cloned();
            return Ok(DateTime::from_local(
                date,
                Time::from_hms(0, 0, 0)?,
                timezone,
            ));
        };
        let date = Date::from_str(&value[..split])?;
        let mut rest = value[split + 1..].trim();
        let timezone =
            if let Some(stripped) = rest.strip_suffix('Z').or_else(|| rest.strip_suffix('z')) {
                rest = stripped;
                Some(Timezone::Utc)
            } else if let Some(sign) = rest.rfind(['+', '-']).filter(|&i| i > 0) {
                let tz = Timezone::from_str(&rest[sign..])?;
                rest = &rest[..sign];
                Some(tz)
            } else {
                None
            };
        let time = Time::from_str(rest)?;
        Ok(DateTime::from_local(date, time, timezone))
    }

    /// Builds an instant from a [`Mapping`] (`year`/`month`/`day`/`hour`/`minute`/
    /// `second`/`nanosecond`/`timezone`).
    pub fn from_mapping(fields: &Mapping) -> Result<DateTime, TimeError> {
        let date = Date::from_mapping(fields)?;
        let time = Time::from_mapping(fields)?;
        let timezone = match fields.get("timezone") {
            Some(tz) => Some(Timezone::from_str(tz)?),
            None => None,
        };
        Ok(DateTime::from_local(date, time, timezone))
    }

    /// Renders the canonical ISO-8601 string, with `Z` for UTC, `±HH:MM` for a
    /// zoned instant, or no suffix when naive.
    pub fn to_str(&self) -> String {
        let body = format!("{}T{}", self.date().to_str(), self.time().to_str());
        match &self.timezone {
            None => body,
            Some(Timezone::Utc) => format!("{body}Z"),
            Some(_) => {
                let offset = self.offset_seconds();
                let sign = if offset < 0 { '-' } else { '+' };
                let abs = offset.unsigned_abs();
                format!("{body}{sign}{:02}:{:02}", abs / 3600, (abs % 3600) / 60)
            }
        }
    }

    /// Renders to a component [`Mapping`] (the [`Date`] / [`Time`] components plus
    /// `timezone` when zoned).
    pub fn to_mapping(&self) -> Mapping {
        let mut map = self.date().to_mapping();
        map.extend(self.time().to_mapping());
        if let Some(tz) = &self.timezone {
            map.insert("timezone".to_string(), tz.name());
        }
        map
    }

    /// The canonical string as UTF-8 bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_str().into_bytes()
    }

    /// Parses an instant from the UTF-8 bytes of its canonical string.
    pub fn from_bytes(bytes: &[u8]) -> Result<DateTime, TimeError> {
        let value = std::str::from_utf8(bytes).map_err(|_| TimeError::Invalid("<bytes>".into()))?;
        DateTime::from_str(value)
    }
}

impl Temporal for DateTime {
    fn to_datetime(&self) -> DateTime {
        self.clone()
    }

    fn to_date(&self) -> Date {
        self.date()
    }

    fn to_time(&self) -> Time {
        self.time()
    }
}

impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_str())
    }
}

impl PartialOrd for DateTime {
    fn partial_cmp(&self, other: &DateTime) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DateTime {
    /// Orders by absolute instant first (so two zones of the same moment compare
    /// equal-instant), then by zone name for a total order.
    fn cmp(&self, other: &DateTime) -> std::cmp::Ordering {
        (self.seconds, self.nanos)
            .cmp(&(other.seconds, other.nanos))
            .then_with(|| {
                self.timezone
                    .as_ref()
                    .map(Timezone::name)
                    .cmp(&other.timezone.as_ref().map(Timezone::name))
            })
    }
}
