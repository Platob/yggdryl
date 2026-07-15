//! Flexible temporal parsing — common date, time, and datetime formats **in addition to** the
//! strict ISO-8601 `FromStr` of each value type. It handles numeric dates in ISO (`YYYY-MM-DD`),
//! US (`MM/DD/YYYY`), and European (`DD.MM.YYYY`) order, month-name dates (`Jan 2, 2024`), 24-hour
//! and 12-hour (`3:45 PM`) times with an optional fraction, and a trailing timezone (`Z`, `+02:00`,
//! `UTC`) — plus date-only and time-only inputs. The `parse` methods on the value types take
//! default `unit` / `tz` for defaulting and casting while parsing.

use super::civil::{self, Civil};
use super::{TimeUnit, Tz};

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// The 1-based month number for a name/abbreviation prefix (`"Jan"`, `"january"`), or `None`.
fn month_number(token: &str) -> Option<u32> {
    let lower = token.trim().to_ascii_lowercase();
    (lower.len() >= 3)
        .then(|| MONTHS.iter().position(|m| lower.starts_with(m)))
        .flatten()
        .map(|index| index as u32 + 1)
}

/// A flexible **date** → `(year, month, day)`: month-name forms, or numeric with `-`/`/`/`.`
/// separators in ISO / US / European order (disambiguated by which field is the 4-digit year and,
/// for a year-last date, by the separator — `/` is US `MM/DD/YYYY`, `.`/`-` is European `DD.MM.YYYY`).
pub(super) fn parse_date(text: &str) -> Option<(i32, u32, u32)> {
    let text = text.trim();
    if let Some(ymd) = parse_month_name_date(text) {
        return Some(ymd);
    }
    let separator = text.chars().find(|c| matches!(c, '-' | '/' | '.'))?;
    let parts: Vec<&str> = text.split(separator).collect();
    if parts.len() != 3 {
        return None;
    }
    let nums: Vec<u32> = parts
        .iter()
        .map(|p| p.trim().parse::<u32>().ok())
        .collect::<Option<_>>()?;
    let (year, month, day) = if parts[0].trim().len() == 4 {
        (nums[0], nums[1], nums[2]) // year-first (ISO)
    } else if parts[2].trim().len() == 4 {
        if separator == '/' {
            (nums[2], nums[0], nums[1]) // US MM/DD/YYYY
        } else {
            (nums[2], nums[1], nums[0]) // European DD.MM.YYYY / DD-MM-YYYY
        }
    } else {
        return None; // ambiguous — no 4-digit year
    };
    Some((year as i32, month, day))
}

/// A month-name date: `"Jan 2, 2024"`, `"January 2 2024"`, `"2 Jan 2024"`.
fn parse_month_name_date(text: &str) -> Option<(i32, u32, u32)> {
    let cleaned = text.replace(',', " ");
    let tokens: Vec<&str> = cleaned.split_whitespace().collect();
    if tokens.len() != 3 {
        return None;
    }
    let month_index = tokens.iter().position(|t| month_number(t).is_some())?;
    let month = month_number(tokens[month_index])?;
    let others: Vec<&str> = tokens
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != month_index)
        .map(|(_, t)| *t)
        .collect();
    let a: i64 = others[0].parse().ok()?;
    let b: i64 = others[1].parse().ok()?;
    // The 4-digit (or > 31) field is the year; the other is the day.
    let (year, day) = if others[0].len() == 4 || a > 31 {
        (a, b)
    } else {
        (b, a)
    };
    Some((year as i32, month, day as u32))
}

/// A flexible **time of day** → `(nanos_of_day, fraction_digit_count)`: 24-hour or 12-hour with an
/// `AM`/`PM` suffix, `HH:MM[:SS]`, and an optional `.frac`.
pub(super) fn parse_time(text: &str) -> Option<(i64, usize)> {
    let text = text.trim();
    let upper = text.to_ascii_uppercase();
    let (body, pm) = if let Some(rest) = upper.strip_suffix("AM") {
        (rest.trim(), Some(false))
    } else if let Some(rest) = upper.strip_suffix("PM") {
        (rest.trim(), Some(true))
    } else {
        (text, None)
    };
    let (clock, frac) = body.split_once('.').unwrap_or((body, ""));
    let mut parts = clock.split(':');
    let mut hour: u32 = parts.next()?.trim().parse().ok()?;
    let minute: u32 = parts.next()?.trim().parse().ok()?;
    let second: u32 = parts.next().unwrap_or("0").trim().parse().ok()?;
    if parts.next().is_some() || minute >= 60 || second >= 60 {
        return None;
    }
    match pm {
        Some(pm) => {
            if !(1..=12).contains(&hour) {
                return None;
            }
            hour = match (hour, pm) {
                (12, false) => 0,
                (12, true) => 12,
                (h, false) => h,
                (h, true) => h + 12,
            };
        }
        None if hour >= 24 => return None,
        None => {}
    }
    if !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut nanos_frac = 0i64;
    for (index, byte) in frac.bytes().take(9).enumerate() {
        nanos_frac += (byte - b'0') as i64 * 10i64.pow(8 - index as u32);
    }
    Some((
        civil::nanos_of_day_from_hms(hour, minute, second, 0) + nanos_frac,
        frac.len(),
    ))
}

/// Strips a trailing timezone (`Z`, `±hh:mm`, `±hhmm`, `±hh`, or ` UTC`/` GMT`) from a datetime
/// string, returning the body and the zone. A `-` is only taken as an offset when a time (a `:`)
/// precedes it, so a date's `-` separators are never mistaken for one.
pub(super) fn split_tz_suffix(text: &str) -> (&str, Option<Tz>) {
    let text = text.trim_end();
    if let Some(body) = text.strip_suffix(['Z', 'z']) {
        return (body.trim_end(), Some(Tz::UTC));
    }
    for suffix in [" UTC", " utc", " GMT", " gmt"] {
        if let Some(body) = text.strip_suffix(suffix) {
            return (body.trim_end(), Some(Tz::UTC));
        }
    }
    if let Some(pos) = text.rfind('+') {
        if let Some(tz) = Tz::parse(&text[pos..]).filter(Tz::is_fixed_offset) {
            return (text[..pos].trim_end(), Some(tz));
        }
    }
    if let Some(pos) = text.rfind('-') {
        if text[..pos].contains(':') {
            if let Some(tz) = Tz::parse(&text[pos..]).filter(Tz::is_fixed_offset) {
                return (text[..pos].trim_end(), Some(tz));
            }
        }
    }
    (text, None)
}

/// Parses a flexible datetime → `(civil components, zone from the string, fraction digit count)`.
/// Accepts a full datetime (`date` + `T`/space + `time`), a **date only** (time → midnight), or a
/// **time only** (date → the epoch).
pub(super) fn parse_datetime(text: &str) -> Option<(Civil, Option<Tz>, usize)> {
    let (body, tz) = split_tz_suffix(text.trim());
    let body = body.trim();

    // A month-name date is date-only (it contains spaces that are not a date/time separator).
    if body.split_whitespace().any(|t| month_number(t).is_some()) {
        let (year, month, day) = parse_month_name_date(body)?;
        return Some((civil_at(year, month, day, 0), tz, 0));
    }

    let (date_part, time_part) = if let Some(pos) = body.find(['T', 't']) {
        (Some(&body[..pos]), Some(body[pos + 1..].trim()))
    } else if let Some(pos) = body.find(' ') {
        (Some(body[..pos].trim()), Some(body[pos + 1..].trim()))
    } else if body.contains(':') {
        (None, Some(body)) // time only
    } else {
        (Some(body), None) // date only
    };

    let (year, month, day) = match date_part {
        Some(part) => parse_date(part)?,
        None => (1970, 1, 1),
    };
    let (nanos_of_day, frac) = match time_part {
        Some(part) => parse_time(part)?,
        None => (0, 0),
    };
    let (hour, minute, second, nanosecond) = civil::hms_from_nanos_of_day(nanos_of_day);
    Some((
        Civil {
            year,
            month,
            day,
            hour,
            minute,
            second,
            nanosecond,
        },
        tz,
        frac,
    ))
}

/// A [`Civil`] at midnight (or a given `nanos_of_day`).
fn civil_at(year: i32, month: u32, day: u32, nanos_of_day: i64) -> Civil {
    let (hour, minute, second, nanosecond) = civil::hms_from_nanos_of_day(nanos_of_day);
    Civil {
        year,
        month,
        day,
        hour,
        minute,
        second,
        nanosecond,
    }
}

// ---- flexible duration parsing --------------------------------------------------------------

/// A flexible **duration** → `(total_nanoseconds, result_unit)`, where `result_unit` is the
/// coarsest fixed [`TimeUnit`] (never coarser than the input's own granularity) that represents
/// the total exactly, so `total / result_unit.nanos()` is a whole count. Handles ISO-8601
/// (`PT1H30M`, `P1DT2H`, `P2W`), clock (`1:30:00`, `30:00.5`), compound unit runs (`1h30m15s`,
/// `2d 3h`, `1 hour 30 minutes`), and a single `<n><unit>` (`90s`, `-1500ms`, `5 min`). A leading
/// `-`/`+` negates the whole span. A **calendar** unit (`mo`/`y`) is rejected — a duration has a
/// fixed length.
pub(super) fn parse_duration(text: &str) -> Option<(i128, TimeUnit)> {
    let text = text.trim();
    let first = text.as_bytes().first()?;
    let (sign, body): (i128, &str) = match first {
        b'-' => (-1, text[1..].trim_start()),
        b'+' => (1, text[1..].trim_start()),
        _ => (1, text),
    };
    let (nanos, unit) = if let Some(rest) = body.strip_prefix(['P', 'p']) {
        parse_iso_duration(rest)?
    } else if body.contains(':') {
        parse_clock_duration(body)?
    } else {
        parse_component_duration(body)?
    };
    Some((sign.checked_mul(nanos)?, unit))
}

/// A non-negative integer, trimmed.
fn parse_u128(text: &str) -> Option<i128> {
    text.trim().parse::<i128>().ok().filter(|&n| n >= 0)
}

/// The nanoseconds for `<number><unit>` where `number` may carry a fraction (`"1.5"`), truncating
/// the sub-nanosecond remainder. `None` for a calendar unit or on overflow.
fn component_nanos(number: &str, unit: TimeUnit) -> Option<i128> {
    let unit_nanos = unit.nanos()?; // calendar unit → None → rejected
    let (int_part, frac_part) = number.split_once('.').unwrap_or((number, ""));
    let int_val: i128 = if int_part.is_empty() {
        0
    } else {
        parse_u128(int_part)?
    };
    let mut nanos = int_val.checked_mul(unit_nanos)?;
    if !frac_part.is_empty() {
        if !frac_part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let frac_val: i128 = frac_part.parse().ok()?;
        let scale = 10i128.checked_pow(frac_part.len() as u32)?;
        nanos = nanos.checked_add(frac_val.checked_mul(unit_nanos)? / scale)?;
    }
    Some(nanos)
}

/// The coarsest fixed unit `≤ coarsest_seen` whose nanosecond size divides `total` exactly — the
/// natural granularity of the parsed span (never promoting past the input's own coarsest unit).
fn refine_unit(total: i128, coarsest_seen: TimeUnit) -> TimeUnit {
    const FIXED: [TimeUnit; 8] = [
        TimeUnit::Nanosecond,
        TimeUnit::Microsecond,
        TimeUnit::Millisecond,
        TimeUnit::Second,
        TimeUnit::Minute,
        TimeUnit::Hour,
        TimeUnit::Day,
        TimeUnit::Week,
    ];
    for &unit in FIXED.iter().rev() {
        // `unit.nanos()` is `Some` for every fixed unit; `Nanosecond` divides any integer, so the
        // loop always returns before falling through.
        if unit <= coarsest_seen && total % unit.nanos().unwrap() == 0 {
            return unit;
        }
    }
    TimeUnit::Nanosecond
}

/// A duration unit token, with the duration-local convention that a bare `"m"` is **minutes**
/// (`TimeUnit::parse` leaves it ambiguous). Calendar units (`mo`/`y`) are rejected.
fn component_unit(token: &str) -> Option<TimeUnit> {
    let token = token.trim();
    if token.eq_ignore_ascii_case("m") {
        // A bare `m` means minutes in a duration (avoid the extra lowercase allocation).
        Some(TimeUnit::Minute)
    } else {
        TimeUnit::parse(token).filter(|u| u.is_fixed())
    }
}

/// A run of `<number><unit>` components (`1h30m15s`, `2d 3h`, `5 min`, `1 hour 30 minutes`).
fn parse_component_duration(text: &str) -> Option<(i128, TimeUnit)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut total: i128 = 0;
    let mut coarsest: Option<TimeUnit> = None;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let num_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
            i += 1;
        }
        if i == num_start {
            return None; // expected a number
        }
        let number = &text[num_start..i];
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        let unit_start = i;
        while i < bytes.len() && bytes[i] != b' ' && !bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == unit_start {
            return None; // expected a unit
        }
        let unit = component_unit(&text[unit_start..i])?;
        total = total.checked_add(component_nanos(number, unit)?)?;
        coarsest = Some(coarsest.map_or(unit, |c| c.max(unit)));
    }
    coarsest.map(|c| (total, refine_unit(total, c)))
}

/// A clock duration `H:M:S(.frac)` or `M:S(.frac)`.
fn parse_clock_duration(text: &str) -> Option<(i128, TimeUnit)> {
    let parts: Vec<&str> = text.split(':').collect();
    let ((hours, minutes, secs), coarsest) = match parts.as_slice() {
        [h, m, s] => ((parse_u128(h)?, parse_u128(m)?, *s), TimeUnit::Hour),
        [m, s] => ((0, parse_u128(m)?, *s), TimeUnit::Minute),
        _ => return None,
    };
    if minutes >= 60 {
        return None;
    }
    let (sec_int, frac) = secs.split_once('.').unwrap_or((secs, ""));
    let seconds = parse_u128(sec_int)?;
    if seconds >= 60 {
        return None;
    }
    let whole_secs = hours.checked_mul(3600)?.checked_add(minutes * 60)? + seconds;
    let mut nanos = whole_secs.checked_mul(1_000_000_000)?;
    if !frac.is_empty() {
        nanos = nanos.checked_add(component_nanos(&format!("0.{frac}"), TimeUnit::Second)?)?;
    }
    Some((nanos, refine_unit(nanos, coarsest)))
}

/// An ISO-8601 duration body (after the leading `P`): a date section (`W`/`D`; the calendar `Y`/`M`
/// are rejected) and, after `T`, a time section (`H`/`M`/`S`, `S` may carry a fraction).
fn parse_iso_duration(rest: &str) -> Option<(i128, TimeUnit)> {
    let upper = rest.to_ascii_uppercase();
    let (date_part, time_part) = upper.split_once('T').unwrap_or((upper.as_str(), ""));
    let mut total: i128 = 0;
    let mut coarsest: Option<TimeUnit> = None;
    parse_iso_section(date_part, true, &mut total, &mut coarsest)?;
    parse_iso_section(time_part, false, &mut total, &mut coarsest)?;
    coarsest.map(|c| (total, refine_unit(total, c)))
}

/// Accumulates one ISO-8601 section (`<number><letter>` runs) into `total` / `coarsest`.
fn parse_iso_section(
    section: &str,
    is_date: bool,
    total: &mut i128,
    coarsest: &mut Option<TimeUnit>,
) -> Option<()> {
    let bytes = section.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
            i += 1;
        }
        if i == start {
            return None;
        }
        let number = &section[start..i];
        let letter = *bytes.get(i)?;
        i += 1;
        let unit = match (is_date, letter) {
            (true, b'W') => TimeUnit::Week,
            (true, b'D') => TimeUnit::Day,
            (false, b'H') => TimeUnit::Hour,
            (false, b'M') => TimeUnit::Minute,
            (false, b'S') => TimeUnit::Second,
            _ => return None, // a calendar `Y`/`M` in the date section, or an unknown letter
        };
        *total = total.checked_add(component_nanos(number, unit)?)?;
        *coarsest = Some(coarsest.map_or(unit, |c| c.max(unit)));
    }
    Some(())
}
