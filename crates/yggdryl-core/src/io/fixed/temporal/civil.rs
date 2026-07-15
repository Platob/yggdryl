//! Internal proleptic-Gregorian calendar math — the exact `days ↔ (year, month, day)` conversions
//! (Howard Hinnant's algorithms, valid for the whole `i64` day range) plus leap-year, month-length,
//! and weekday helpers. Crate-private: the temporal value types are the public surface.

/// The civil `(year, month, day)` at `days` days since the Unix epoch (`1970-01-01`), and the
/// components of a time of day, packaged for the date/timestamp decomposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Civil {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub nanosecond: u32,
}

/// `days` since `1970-01-01` for a proleptic-Gregorian `(year, month, day)` — exact for the whole
/// range (Hinnant `days_from_civil`).
pub(crate) const fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = year as i64 - (month <= 2) as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// The proleptic-Gregorian `(year, month, day)` at `days` days since `1970-01-01` — the inverse of
/// [`days_from_civil`] (Hinnant `civil_from_days`).
pub(crate) const fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = (if month <= 2 { y + 1 } else { y }) as i32;
    (year, month, day)
}

/// Whether `year` is a leap year in the proleptic Gregorian calendar.
pub(crate) const fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// The number of days in `month` (`1..=12`) of `year` (`28`/`29`/`30`/`31`).
pub(crate) const fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// The day of the week at `days` since the epoch — `0` = Sunday … `6` = Saturday.
pub(crate) const fn weekday_from_days(days: i64) -> u32 {
    (if days >= -4 {
        (days + 4) % 7
    } else {
        (days + 5) % 7 + 6
    }) as u32
}

/// The 1-based day of the year (`1..=366`) for a civil date.
pub(crate) const fn day_of_year(year: i32, month: u32, day: u32) -> u32 {
    (days_from_civil(year, month, day) - days_from_civil(year, 1, 1) + 1) as u32
}

/// Nanoseconds in a day (`86_400 × 10⁹`).
pub(crate) const DAY_NANOS: i128 = 86_400 * 1_000_000_000;

/// Splits `epoch_nanos` (nanoseconds since the epoch) into whole days and the non-negative
/// nanosecond-of-day — floor division, so a negative instant lands on the right calendar day.
pub(crate) const fn split_epoch_nanos(epoch_nanos: i128) -> (i64, i64) {
    let days = epoch_nanos.div_euclid(DAY_NANOS) as i64;
    let nanos_of_day = epoch_nanos.rem_euclid(DAY_NANOS) as i64;
    (days, nanos_of_day)
}

/// Recombines whole `days` and a `nanos_of_day` into nanoseconds since the epoch.
pub(crate) const fn join_epoch_nanos(days: i64, nanos_of_day: i64) -> i128 {
    days as i128 * DAY_NANOS + nanos_of_day as i128
}

/// The `(hour, minute, second, nanosecond)` of a `nanos_of_day` in `[0, DAY_NANOS)`.
pub(crate) const fn hms_from_nanos_of_day(nanos_of_day: i64) -> (u32, u32, u32, u32) {
    let secs = nanos_of_day / 1_000_000_000;
    let nanosecond = (nanos_of_day % 1_000_000_000) as u32;
    (
        (secs / 3_600) as u32,
        ((secs % 3_600) / 60) as u32,
        (secs % 60) as u32,
        nanosecond,
    )
}

/// The nanosecond-of-day for `(hour, minute, second, nanosecond)`.
pub(crate) const fn nanos_of_day_from_hms(
    hour: u32,
    minute: u32,
    second: u32,
    nanosecond: u32,
) -> i64 {
    ((hour as i64 * 3_600 + minute as i64 * 60 + second as i64) * 1_000_000_000) + nanosecond as i64
}

/// Whether `(year, month, day, hour, minute, second, nanosecond)` is a real civil instant (a valid
/// date, a `[0,23]` hour, `[0,59]` minute, `[0,60]` second to allow a leap second, and a `[0,1e9)`
/// nanosecond).
pub(crate) const fn is_valid(civil: &Civil) -> bool {
    civil.month >= 1
        && civil.month <= 12
        && civil.day >= 1
        && civil.day <= days_in_month(civil.year, civil.month)
        && civil.hour < 24
        && civil.minute < 60
        && civil.second <= 60
        && civil.nanosecond < 1_000_000_000
}
