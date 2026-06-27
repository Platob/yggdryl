//! The [`Date`] type — a proleptic-Gregorian calendar date, optionally anchored to
//! a [`Timezone`], stored as days since the UNIX epoch (1970-01-01).

use std::cmp::Ordering;
use std::fmt;

#[allow(unused_imports)]
use crate::log_event;
use crate::Mapping;

use super::{
    civil_from_days, days_from_civil, days_in_month, DateTime, Duration, Temporal, Time, TimeError,
    Timezone,
};

/// Nanoseconds in a calendar day — used by the [`Duration`] arithmetic helpers.
const NANOS_PER_DAY: i128 = 86_400 * 1_000_000_000;

/// A calendar date with no time-of-day, optionally tagged with a [`Timezone`]
/// (so it can name "this day in this zone"). Ordered chronologically.
///
/// ```
/// use yggdryl_core::Date;
///
/// let d = Date::from_ymd(2024, 2, 29).unwrap();
/// assert_eq!((d.year(), d.month(), d.day()), (2024, 2, 29));
/// assert_eq!(d.to_str(), "2024-02-29");
/// // Flexible parsing: a full datetime keeps just the date (and its zone).
/// let zoned = Date::from_str("2024-02-29T10:00:00Z").unwrap();
/// assert_eq!((zoned.year(), zoned.month(), zoned.day()), (2024, 2, 29));
/// assert!(Date::from_ymd(2023, 2, 29).is_err()); // not a leap year
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Date {
    /// Days since 1970-01-01 (negative before the epoch).
    epoch_days: i32,
    /// Optional timezone this date is anchored to.
    timezone: Option<Timezone>,
}

impl Date {
    /// Builds a date from `(year, month, day)`, validating the calendar.
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Result<Date, TimeError> {
        if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
            return Err(TimeError::OutOfRange(format!(
                "{year:04}-{month:02}-{day:02}"
            )));
        }
        // Guard the i32 epoch-day storage against extreme years overflowing silently.
        let epoch_days = i32::try_from(days_from_civil(year, month, day)).map_err(|_| {
            TimeError::OutOfRange(format!("{year:04}-{month:02}-{day:02} (year out of range)"))
        })?;
        Ok(Date {
            epoch_days,
            timezone: None,
        })
    }

    /// Builds a date from a count of days since the UNIX epoch.
    pub fn from_epoch_days(epoch_days: i32) -> Date {
        Date {
            epoch_days,
            timezone: None,
        }
    }

    /// Days since the UNIX epoch.
    pub fn epoch_days(&self) -> i32 {
        self.epoch_days
    }

    /// The `(year, month, day)` components.
    pub fn ymd(&self) -> (i32, u32, u32) {
        civil_from_days(self.epoch_days as i64)
    }

    /// The year component.
    pub fn year(&self) -> i32 {
        self.ymd().0
    }

    /// The month component (1–12).
    pub fn month(&self) -> u32 {
        self.ymd().1
    }

    /// The day component (1–31).
    pub fn day(&self) -> u32 {
        self.ymd().2
    }

    /// The day of the week (0 = Sunday … 6 = Saturday).
    pub fn weekday(&self) -> u32 {
        (self.epoch_days as i64 + 4).rem_euclid(7) as u32
    }

    /// The timezone this date is anchored to, if any.
    pub fn timezone(&self) -> Option<&Timezone> {
        self.timezone.as_ref()
    }

    /// Returns a copy anchored to `timezone`.
    pub fn with_timezone(mut self, timezone: Timezone) -> Date {
        self.timezone = Some(timezone);
        self
    }

    /// Returns a copy with no timezone.
    pub fn without_timezone(mut self) -> Date {
        self.timezone = None;
        self
    }

    /// Returns a copy `days` days later (keeping the timezone; saturates at the
    /// i32 epoch-day bounds rather than overflowing).
    pub fn add_days(&self, days: i32) -> Date {
        Date {
            epoch_days: self.epoch_days.saturating_add(days),
            timezone: self.timezone.clone(),
        }
    }

    /// Combines this date with a [`Time`] into a [`DateTime`] in the date's zone.
    pub fn at(&self, time: Time) -> DateTime {
        DateTime::from_local(self.clone(), time, self.timezone.clone())
    }

    /// This date advanced by a [`Duration`]'s **whole days** (sub-day components are
    /// truncated), keeping the timezone.
    pub fn add(&self, span: &Duration) -> Date {
        let days = span.as_nanos().div_euclid(NANOS_PER_DAY);
        self.add_days(days.clamp(i32::MIN as i128, i32::MAX as i128) as i32)
    }

    /// This date moved back by a [`Duration`]'s whole days.
    pub fn sub(&self, span: &Duration) -> Date {
        self.add(&span.negate())
    }

    /// The signed whole-day [`Duration`] from `other` to `self` (`self - other`).
    pub fn duration_since(&self, other: &Date) -> Duration {
        Duration::from_nanos((self.epoch_days as i128 - other.epoch_days as i128) * NANOS_PER_DAY)
    }

    /// This date floored to a multiple of `unit` (rounded down to whole days) since
    /// the epoch. A `unit` under one day returns the date unchanged.
    pub fn truncate(&self, unit: &Duration) -> Date {
        let days = unit.as_nanos().div_euclid(NANOS_PER_DAY);
        if days <= 0 {
            return self.clone();
        }
        let floored = (self.epoch_days as i128).div_euclid(days) * days;
        Date {
            epoch_days: floored.clamp(i32::MIN as i128, i32::MAX as i128) as i32,
            timezone: self.timezone.clone(),
        }
    }

    /// Parses a date, flexibly: an ISO `YYYY-MM-DD`, a `YYYY/MM/DD`, a compact
    /// `YYYYMMDD`, or a full datetime string (the date part is kept, along with its
    /// timezone). The inverse of [`to_str`](Date::to_str).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<Date, TimeError> {
        log_event!(trace, "Date::from_str {input:?}");
        let value = input.trim();
        // An empty string decodes to the epoch default (1970-01-01).
        if value.is_empty() {
            return Ok(Date::from_epoch_days(0));
        }
        // A full datetime keeps the date (and any timezone).
        if value.contains(['T', 't']) || value.contains(' ') {
            let dt = DateTime::from_str(value)?;
            let mut date = dt.date();
            date.timezone = dt.timezone().cloned();
            return Ok(date);
        }
        let (year, month, day) =
            parse_ymd(value).ok_or_else(|| TimeError::Invalid(input.to_string()))?;
        Date::from_ymd(year, month, day)
    }

    /// Builds a date from a [`Mapping`] (`year` / `month` / `day`, optional `timezone`).
    pub fn from_mapping(fields: &Mapping) -> Result<Date, TimeError> {
        let component = |key: &str| -> Result<i64, TimeError> {
            fields
                .get(key)
                .ok_or_else(|| TimeError::Invalid(format!("missing '{key}'")))?
                .parse::<i64>()
                .map_err(|_| TimeError::Invalid(format!("'{key}' is not an integer")))
        };
        // Convert without wrapping: an out-of-range component is an error, not a wrap.
        let year = i32::try_from(component("year")?)
            .map_err(|_| TimeError::OutOfRange("'year' out of range".into()))?;
        let month = u32::try_from(component("month")?)
            .map_err(|_| TimeError::Invalid("'month' out of range".into()))?;
        let day = u32::try_from(component("day")?)
            .map_err(|_| TimeError::Invalid("'day' out of range".into()))?;
        let mut date = Date::from_ymd(year, month, day)?;
        if let Some(tz) = fields.get("timezone") {
            date.timezone = Some(Timezone::from_str(tz)?);
        }
        Ok(date)
    }

    /// Renders the canonical ISO `YYYY-MM-DD` string.
    pub fn to_str(&self) -> String {
        let (y, m, d) = self.ymd();
        if y < 0 {
            format!("-{:04}-{:02}-{:02}", -y, m, d)
        } else {
            format!("{y:04}-{m:02}-{d:02}")
        }
    }

    /// Renders to a component [`Mapping`] (`year` / `month` / `day`, plus `timezone`
    /// when anchored).
    pub fn to_mapping(&self) -> Mapping {
        let (y, m, d) = self.ymd();
        let mut map = Mapping::from([
            ("year".to_string(), y.to_string()),
            ("month".to_string(), m.to_string()),
            ("day".to_string(), d.to_string()),
        ]);
        if let Some(tz) = &self.timezone {
            map.insert("timezone".to_string(), tz.name());
        }
        map
    }

    /// The canonical string as UTF-8 bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_str().into_bytes()
    }

    /// Parses a date from the UTF-8 bytes of its canonical string.
    pub fn from_bytes(bytes: &[u8]) -> Result<Date, TimeError> {
        let value = std::str::from_utf8(bytes).map_err(|_| TimeError::Invalid("<bytes>".into()))?;
        Date::from_str(value)
    }
}

impl Temporal for Date {
    /// Midnight on this date, in the date's timezone (naive if none).
    fn to_datetime(&self) -> DateTime {
        DateTime::from_local(
            self.clone(),
            Time::from_hms(0, 0, 0).expect("midnight is valid"),
            self.timezone.clone(),
        )
    }

    /// The local calendar date of `value`.
    fn from_datetime(value: &DateTime) -> Date {
        value.date()
    }

    fn to_date(&self) -> Date {
        self.clone()
    }
}

/// Parses `YYYY-MM-DD`, `YYYY/MM/DD` or compact `YYYYMMDD` into `(year, month, day)`.
/// Every component must be a plain run of digits (no embedded sign or junk).
fn parse_ymd(value: &str) -> Option<(i32, u32, u32)> {
    /// Parses a run of ASCII digits, rejecting an empty part or any non-digit byte
    /// (so a stray sign like `+3` does not slip through `str::parse`).
    fn digits<T: std::str::FromStr>(part: &str) -> Option<T> {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse::<T>().ok()
    }
    let negative = value.starts_with('-');
    let body = value.strip_prefix('-').unwrap_or(value);
    let sep = if body.contains('-') {
        '-'
    } else if body.contains('/') {
        '/'
    } else if body.len() == 8 && body.bytes().all(|b| b.is_ascii_digit()) {
        // Compact YYYYMMDD.
        let year = digits::<i32>(&body[..4])?;
        let month = digits::<u32>(&body[4..6])?;
        let day = digits::<u32>(&body[6..8])?;
        return Some((if negative { -year } else { year }, month, day));
    } else {
        return None;
    };
    let mut parts = body.split(sep);
    let year = digits::<i32>(parts.next()?)?;
    let month = digits::<u32>(parts.next()?)?;
    let day = digits::<u32>(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some((if negative { -year } else { year }, month, day))
}

impl std::ops::Add<Duration> for Date {
    type Output = Date;
    fn add(self, rhs: Duration) -> Date {
        Date::add(&self, &rhs)
    }
}

impl std::ops::Sub<Duration> for Date {
    type Output = Date;
    fn sub(self, rhs: Duration) -> Date {
        Date::sub(&self, &rhs)
    }
}

impl std::ops::Sub<Date> for Date {
    type Output = Duration;
    fn sub(self, rhs: Date) -> Duration {
        self.duration_since(&rhs)
    }
}

impl PartialOrd for Date {
    fn partial_cmp(&self, other: &Date) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Date {
    /// Orders by day first, then by timezone name (for a total order).
    fn cmp(&self, other: &Date) -> Ordering {
        self.epoch_days.cmp(&other.epoch_days).then_with(|| {
            self.timezone
                .as_ref()
                .map(Timezone::name)
                .cmp(&other.timezone.as_ref().map(Timezone::name))
        })
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_str())
    }
}
