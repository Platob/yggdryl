//! [`Date32`] (days since epoch, `i32`) and [`Date64`] (milliseconds since epoch, `i64`) — a
//! calendar day with no time-of-day and no timezone (naive). Arrow `Date32` / `Date64`.

use super::{civil, Duration64, Temporal, TemporalError, Time64, TimeUnit, Ts64, Tz};

/// A calendar date as **days since the Unix epoch** (`1970-01-01`), in an `i32` — a range of about
/// ±5.8 million years. Naive (no timezone), resolution [`Day`](TimeUnit::Day). Arrow `Date32`.
///
/// ```
/// use yggdryl_core::io::fixed::temporal::Date32;
///
/// let d = Date32::from_ymd(2024, 2, 29).unwrap(); // a leap day
/// assert_eq!(d.to_ymd(), (2024, 2, 29));
/// assert_eq!(d.to_string(), "2024-02-29");
/// assert_eq!(Date32::from_days(0).to_ymd(), (1970, 1, 1));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date32 {
    days: i32,
}

impl core::fmt::Debug for Date32 {
    /// The signature + value, e.g. `date32(2024-02-29)`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "date32({self})")
    }
}

impl Date32 {
    /// The Unix epoch, `1970-01-01`.
    pub const UNIX_EPOCH: Date32 = Date32 { days: 0 };

    /// A date `days` days after the epoch (negative for before).
    pub const fn from_days(days: i32) -> Self {
        Self { days }
    }

    /// The date `year-month-day`, or [`InvalidDate`](TemporalError::InvalidDate) for an impossible
    /// date / [`OutOfRange`](TemporalError::OutOfRange) beyond the `i32` day range.
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Result<Self, TemporalError> {
        if !(1..=12).contains(&month) || day < 1 || day > civil::days_in_month(year, month) {
            return Err(TemporalError::InvalidDate { year, month, day });
        }
        i32::try_from(civil::days_from_civil(year, month, day))
            .map(Self::from_days)
            .map_err(|_| TemporalError::OutOfRange { ty: "date32" })
    }

    /// The day count since the epoch.
    pub const fn days(&self) -> i32 {
        self.days
    }

    /// The `(year, month, day)`.
    pub fn to_ymd(&self) -> (i32, u32, u32) {
        civil::civil_from_days(self.days as i64)
    }

    /// The year.
    pub fn year(&self) -> i32 {
        self.to_ymd().0
    }
    /// The month (`1..=12`).
    pub fn month(&self) -> u32 {
        self.to_ymd().1
    }
    /// The day of the month (`1..=31`).
    pub fn day(&self) -> u32 {
        self.to_ymd().2
    }
    /// The day of the week — `0` = Sunday … `6` = Saturday.
    pub fn weekday(&self) -> u32 {
        civil::weekday_from_days(self.days as i64)
    }
    /// The 1-based day of the year (`1..=366`).
    pub fn day_of_year(&self) -> u32 {
        let (year, month, day) = self.to_ymd();
        civil::day_of_year(year, month, day)
    }
    /// Whether this date's year is a leap year.
    pub fn is_leap_year(&self) -> bool {
        civil::is_leap(self.year())
    }

    /// This date as a [`Date64`] (milliseconds since epoch).
    pub fn to_date64(&self) -> Date64 {
        Date64::from_days(self.days as i64)
    }

    /// Parses a date from a **flexible** set of common formats (ISO `2024-02-29`, US `02/29/2024`,
    /// European `29.02.2024`, or a month name `Feb 29, 2024`) — see [`parse`](super::parse). Use
    /// [`FromStr`](core::str::FromStr) for strict ISO only.
    pub fn parse_str(text: &str) -> Result<Self, TemporalError> {
        let (year, month, day) =
            super::parse::parse_date(text).ok_or(TemporalError::ParseError { ty: "date32" })?;
        Self::from_ymd(year, month, day)
    }

    /// This date at midnight, as a [`Ts64`] in `unit` and zone `tz` — the instant
    /// `00:00:00` on this day. Errors [`OutOfRange`](TemporalError::OutOfRange) on overflow.
    pub fn at_midnight(&self, unit: TimeUnit, tz: Tz) -> Result<Ts64, TemporalError> {
        let nanos = civil::join_epoch_nanos(self.days as i64, 0);
        Ts64::from_epoch_nanos(nanos, unit, tz)
    }

    /// This date at the given wall-clock `time`, as a [`Ts64`] in `unit` and zone `tz` — the
    /// instant `date`T`time` (interpreted like [`at_midnight`](Date32::at_midnight)).
    pub fn at_time(&self, time: &Time64, unit: TimeUnit, tz: Tz) -> Result<Ts64, TemporalError> {
        let nanos = civil::join_epoch_nanos(self.days as i64, time.nanos_of_day());
        Ts64::from_epoch_nanos(nanos, unit, tz)
    }

    /// The elapsed span from the epoch to this date, as a [`Duration64`] of whole days.
    pub fn to_duration(&self) -> Duration64 {
        Duration64::new(self.days as i64, TimeUnit::Day).expect("Day is a supported duration unit")
    }

    /// The value's little-endian bytes (`i32` days).
    pub fn serialize_bytes(&self) -> [u8; 4] {
        self.days.to_le_bytes()
    }
    /// Reconstructs a date from [`serialize_bytes`](Date32::serialize_bytes).
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, TemporalError> {
        let array: [u8; 4] = bytes
            .get(..4)
            .and_then(|s| s.try_into().ok())
            .ok_or(TemporalError::ParseError { ty: "date32" })?;
        Ok(Self::from_days(i32::from_le_bytes(array)))
    }
}

impl Temporal for Date32 {
    fn time_unit(&self) -> TimeUnit {
        TimeUnit::Day
    }
    fn timezone(&self) -> Tz {
        Tz::NAIVE
    }
}

impl core::fmt::Display for Date32 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let (year, month, day) = self.to_ymd();
        write!(f, "{}-{month:02}-{day:02}", FmtYear(year))
    }
}

impl core::str::FromStr for Date32 {
    type Err = TemporalError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (year, month, day) =
            parse_ymd(text.trim()).ok_or(TemporalError::ParseError { ty: "date32" })?;
        Self::from_ymd(year, month, day)
    }
}

/// A calendar date as **milliseconds since the Unix epoch**, in an `i64` (Arrow `Date64`; the value
/// is a whole number of days' worth of milliseconds). Naive, resolution
/// [`Millisecond`](TimeUnit::Millisecond).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date64 {
    millis: i64,
}

impl core::fmt::Debug for Date64 {
    /// The signature + value, e.g. `date64(2024-02-29)`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "date64({self})")
    }
}

/// Milliseconds in a day.
const DAY_MILLIS: i64 = 86_400_000;

impl Date64 {
    /// The Unix epoch, `1970-01-01`.
    pub const UNIX_EPOCH: Date64 = Date64 { millis: 0 };

    /// A date `days` days after the epoch.
    pub const fn from_days(days: i64) -> Self {
        Self {
            millis: days * DAY_MILLIS,
        }
    }
    /// A date from a raw milliseconds-since-epoch value (floored to the containing day on read).
    pub const fn from_millis(millis: i64) -> Self {
        Self { millis }
    }
    /// The date `year-month-day`, or an error for an impossible / out-of-range date.
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Result<Self, TemporalError> {
        if !(1..=12).contains(&month) || day < 1 || day > civil::days_in_month(year, month) {
            return Err(TemporalError::InvalidDate { year, month, day });
        }
        civil::days_from_civil(year, month, day)
            .checked_mul(DAY_MILLIS)
            .map(|millis| Self { millis })
            .ok_or(TemporalError::OutOfRange { ty: "date64" })
    }

    /// The raw milliseconds-since-epoch value.
    pub const fn millis(&self) -> i64 {
        self.millis
    }
    /// The day count since the epoch (floored).
    pub const fn days(&self) -> i64 {
        self.millis.div_euclid(DAY_MILLIS)
    }
    /// The `(year, month, day)`.
    pub fn to_ymd(&self) -> (i32, u32, u32) {
        civil::civil_from_days(self.days())
    }
    /// The year.
    pub fn year(&self) -> i32 {
        self.to_ymd().0
    }
    /// The month (`1..=12`).
    pub fn month(&self) -> u32 {
        self.to_ymd().1
    }
    /// The day of the month.
    pub fn day(&self) -> u32 {
        self.to_ymd().2
    }
    /// The day of the week — `0` = Sunday … `6` = Saturday.
    pub fn weekday(&self) -> u32 {
        civil::weekday_from_days(self.days())
    }
    /// Whether this date's year is a leap year.
    pub fn is_leap_year(&self) -> bool {
        civil::is_leap(self.year())
    }
    /// Parses a date from a **flexible** set of common formats (see [`Date32::parse_str`]).
    pub fn parse_str(text: &str) -> Result<Self, TemporalError> {
        let (year, month, day) =
            super::parse::parse_date(text).ok_or(TemporalError::ParseError { ty: "date64" })?;
        Self::from_ymd(year, month, day)
    }

    /// This date as a [`Date32`], or [`OutOfRange`](TemporalError::OutOfRange) beyond the `i32` day
    /// range.
    pub fn to_date32(&self) -> Result<Date32, TemporalError> {
        i32::try_from(self.days())
            .map(Date32::from_days)
            .map_err(|_| TemporalError::OutOfRange { ty: "date32" })
    }

    /// This date at midnight, as a [`Ts64`] in `unit` and zone `tz`.
    pub fn at_midnight(&self, unit: TimeUnit, tz: Tz) -> Result<Ts64, TemporalError> {
        let nanos = civil::join_epoch_nanos(self.days(), 0);
        Ts64::from_epoch_nanos(nanos, unit, tz)
    }

    /// This date at the given wall-clock `time`, as a [`Ts64`] in `unit` and zone `tz`.
    pub fn at_time(&self, time: &Time64, unit: TimeUnit, tz: Tz) -> Result<Ts64, TemporalError> {
        let nanos = civil::join_epoch_nanos(self.days(), time.nanos_of_day());
        Ts64::from_epoch_nanos(nanos, unit, tz)
    }

    /// The elapsed span from the epoch to this date, as a [`Duration64`] of whole days.
    pub fn to_duration(&self) -> Duration64 {
        Duration64::new(self.days(), TimeUnit::Day).expect("Day is a supported duration unit")
    }

    /// The value's little-endian bytes (`i64` millis).
    pub fn serialize_bytes(&self) -> [u8; 8] {
        self.millis.to_le_bytes()
    }
    /// Reconstructs a date from [`serialize_bytes`](Date64::serialize_bytes).
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, TemporalError> {
        let array: [u8; 8] = bytes
            .get(..8)
            .and_then(|s| s.try_into().ok())
            .ok_or(TemporalError::ParseError { ty: "date64" })?;
        Ok(Self::from_millis(i64::from_le_bytes(array)))
    }
}

impl Temporal for Date64 {
    fn time_unit(&self) -> TimeUnit {
        TimeUnit::Millisecond
    }
    fn timezone(&self) -> Tz {
        Tz::NAIVE
    }
}

impl core::fmt::Display for Date64 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let (year, month, day) = self.to_ymd();
        write!(f, "{}-{month:02}-{day:02}", FmtYear(year))
    }
}

impl core::str::FromStr for Date64 {
    type Err = TemporalError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (year, month, day) =
            parse_ymd(text.trim()).ok_or(TemporalError::ParseError { ty: "date64" })?;
        Self::from_ymd(year, month, day)
    }
}

/// Formats a year as ISO-8601 (`0000`-padded in `0..=9999`, else signed with an explicit sign).
pub(super) struct FmtYear(pub(super) i32);
impl core::fmt::Display for FmtYear {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if (0..=9999).contains(&self.0) {
            write!(f, "{:04}", self.0)
        } else {
            write!(f, "{:+}", self.0)
        }
    }
}

/// Parses `[+-]?YYYY-MM-DD` into `(year, month, day)`.
pub(super) fn parse_ymd(text: &str) -> Option<(i32, u32, u32)> {
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let mut parts = rest.splitn(3, '-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((if negative { -year } else { year }, month, day))
}
