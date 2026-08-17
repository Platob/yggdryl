//! The classic ISO spellings of the temporals.
//!
//! A temporal used to travel through the text formats as a tagged tuple - a
//! unit name beside a count - which no other tool reads. Every other tool
//! reads `2026-08-17`, `10:00:00.123`, `2026-08-17T10:00:00Z`, and `PT90S`,
//! so those are what the emitters write now. The spelling stays exact both
//! ways: the fraction is printed at the unit's full width, so the number of
//! digits *is* the unit, and a zoned instant carries its offset plus the zone
//! name in brackets when the name says more than the offset does.
//!
//! Formatting answers `None` for a reading with no classic spelling - a date
//! beyond four-digit years, a time of day outside its day, an interval-layout
//! duration - and the caller keeps its structural spelling instead. Parsing is
//! strict about shape and total about meaning: `2026-02-30` is an error, not a
//! guess.

use smol_str::{SmolStr, ToSmolStr, format_smolstr};

use crate::enums::timezone::{civil_from_days, days_from_civil};
use crate::{Error, Result, TimeUnit, Timezone};

/// Seconds in one day, the modulus a datetime splits on.
const DAY: i64 = 86_400;

/// The subdivisions of one second a resolution unit holds.
///
/// An interval layout has no fixed width, so it has no classic spelling.
const fn per_second(unit: TimeUnit) -> Option<i64> {
    match unit {
        TimeUnit::Second => Some(1),
        TimeUnit::Millisecond => Some(1_000),
        TimeUnit::Microsecond => Some(1_000_000),
        TimeUnit::Nanosecond => Some(1_000_000_000),
        TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => None,
    }
}

/// The fraction digits one resolution unit prints, which is how the unit
/// round-trips: three digits are milliseconds, six are microseconds.
const fn fraction_digits(unit: TimeUnit) -> usize {
    match unit {
        TimeUnit::Millisecond => 3,
        TimeUnit::Microsecond => 6,
        TimeUnit::Nanosecond => 9,
        _ => 0,
    }
}

/// The resolution unit one fraction width means.
const fn unit_of_fraction(digits: usize) -> TimeUnit {
    match digits {
        0 => TimeUnit::Second,
        1..=3 => TimeUnit::Millisecond,
        4..=6 => TimeUnit::Microsecond,
        _ => TimeUnit::Nanosecond,
    }
}

/// Write `seconds.fraction` for one in-day count, at the unit's full width.
fn push_clock(text: &mut String, count: i64, unit: TimeUnit) {
    let per = per_second(unit).expect("a resolution unit reaches the clock");
    let seconds = count.div_euclid(per);
    let fraction = count.rem_euclid(per);
    let (hours, minutes, seconds) = (seconds / 3_600, (seconds / 60) % 60, seconds % 60);
    text.push_str(&format!("{hours:02}:{minutes:02}:{seconds:02}"));
    let digits = fraction_digits(unit);
    if digits > 0 {
        text.push_str(&format!(".{fraction:0digits$}"));
    }
}

/// Spell a day count as `YYYY-MM-DD`, when it has four-digit years.
pub(crate) fn format_date(days: i32) -> Option<SmolStr> {
    let (year, month, day) = civil_from_days(i64::from(days));
    if !(0..=9_999).contains(&year) {
        return None;
    }
    Some(format_smolstr!("{year:04}-{month:02}-{day:02}"))
}

/// Spell a time of day as `HH:MM:SS[.fraction]`, when it is inside its day.
pub(crate) fn format_time(count: i64, unit: TimeUnit) -> Option<SmolStr> {
    let per = per_second(unit)?;
    if count < 0 || count >= DAY.checked_mul(per)? {
        return None;
    }
    let mut text = String::with_capacity(18);
    push_clock(&mut text, count, unit);
    Some(SmolStr::from(text))
}

/// Spell a naive reading as `YYYY-MM-DDTHH:MM:SS[.fraction]`.
pub(crate) fn format_datetime(count: i64, unit: TimeUnit) -> Option<SmolStr> {
    let per = per_second(unit)?;
    let days = count.div_euclid(DAY * per);
    let in_day = count.rem_euclid(DAY * per);
    let (year, month, day) = civil_from_days(days);
    if !(0..=9_999).contains(&year) {
        return None;
    }
    let mut text = String::with_capacity(30);
    text.push_str(&format!("{year:04}-{month:02}-{day:02}T"));
    push_clock(&mut text, in_day, unit);
    Some(SmolStr::from(text))
}

/// Spell a zoned instant as its local reading plus its offset.
///
/// The count is UTC, as Arrow defines it; the spelling is the local wall
/// clock with the offset that recovers the instant - `Z` for UTC itself - and
/// the zone's name in brackets when the name is a place rather than an
/// offset, because `+02:00` cannot say `Europe/Paris`. A zone this build has
/// no rules for spells the UTC reading with `Z` and keeps its bracketed name.
pub(crate) fn format_timestamp(count: i64, unit: TimeUnit, zone: &Timezone) -> Option<SmolStr> {
    let per = per_second(unit)?;
    let offset = zone.offset_at(count.div_euclid(per));
    let local = match offset {
        Some(offset) => count.checked_add(i64::from(offset).checked_mul(per)?)?,
        None => count,
    };
    let mut text = String::from(format_datetime(local, unit)?.as_str());
    match offset {
        Some(0) if zone.is_utc() => text.push('Z'),
        None => text.push('Z'),
        Some(offset) => {
            let sign = if offset < 0 { '-' } else { '+' };
            let total = offset.unsigned_abs();
            text.push_str(&format!(
                "{sign}{:02}:{:02}",
                total / 3_600,
                (total % 3_600) / 60
            ));
        }
    }
    if !zone.is_utc() && !zone.is_fixed() {
        text.push('[');
        text.push_str(zone.as_str());
        text.push(']');
    }
    Some(SmolStr::from(text))
}

/// Spell an elapsed count as the ISO duration `PT<seconds>[.fraction]S`.
///
/// Seconds are the one component every unit restates exactly, so the spelling
/// never decomposes into hours a reader would have to multiply back. The sign
/// leads, as ISO 8601 puts it. An interval layout has no fixed width and
/// answers `None`.
pub(crate) fn format_duration(count: i64, unit: TimeUnit) -> Option<SmolStr> {
    let per = per_second(unit)?;
    let magnitude = count.unsigned_abs();
    let seconds = magnitude / per.unsigned_abs();
    let fraction = magnitude % per.unsigned_abs();
    let sign = if count < 0 { "-" } else { "" };
    let digits = fraction_digits(unit);
    Some(if digits == 0 {
        format_smolstr!("{sign}PT{seconds}S")
    } else {
        format_smolstr!("{sign}PT{seconds}.{fraction:0digits$}S")
    })
}

/// The error one malformed ISO spelling reports.
fn iso_error(target: &'static str, position: usize, reason: &'static str) -> Error {
    Error::Parse {
        target,
        position,
        reason: reason.to_smolstr(),
    }
}

/// Read exactly `width` ASCII digits at `position`.
fn digits(text: &str, position: usize, width: usize, target: &'static str) -> Result<i64> {
    let slice = text
        .get(position..position + width)
        .filter(|slice| slice.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or_else(|| iso_error(target, position, "expected digits"))?;
    slice
        .parse::<i64>()
        .map_err(|_| iso_error(target, position, "expected digits"))
}

/// Expect one literal byte at `position`.
fn literal(text: &str, position: usize, byte: u8, target: &'static str) -> Result<()> {
    if text.as_bytes().get(position) == Some(&byte) {
        return Ok(());
    }
    Err(iso_error(target, position, "unexpected separator"))
}

/// Parse `YYYY-MM-DD` into a day count since the Unix epoch.
pub(crate) fn parse_date(text: &str) -> Result<i32> {
    let days = parse_date_at(text, 0)?;
    if text.len() != 10 {
        return Err(iso_error("date", 10, "trailing text after the date"));
    }
    Ok(days)
}

/// Parse the ten date characters starting at `position`.
fn parse_date_at(text: &str, position: usize) -> Result<i32> {
    let year = digits(text, position, 4, "date")?;
    literal(text, position + 4, b'-', "date")?;
    let month = digits(text, position + 5, 2, "date")?;
    literal(text, position + 7, b'-', "date")?;
    let day = digits(text, position + 8, 2, "date")?;
    if !(1..=12).contains(&month) {
        return Err(iso_error("date", position + 5, "month must be 01 to 12"));
    }
    let days = days_from_civil(year as i32, month as u32, day as u32);
    // Round-tripping the civil date rejects a day the month does not have,
    // such as February 30th, without a table of month lengths.
    if day < 1 || civil_from_days(days) != (year as i32, month as u32, day as u32) {
        return Err(iso_error("date", position + 8, "no such day in this month"));
    }
    i32::try_from(days).map_err(|_| iso_error("date", position, "date is out of range"))
}

/// Parse `HH:MM:SS[.fraction]`, returning the count, unit, and end position.
fn parse_clock_at(
    text: &str,
    position: usize,
    target: &'static str,
) -> Result<(i64, TimeUnit, usize)> {
    let hours = digits(text, position, 2, target)?;
    literal(text, position + 2, b':', target)?;
    let minutes = digits(text, position + 3, 2, target)?;
    literal(text, position + 5, b':', target)?;
    let seconds = digits(text, position + 6, 2, target)?;
    if hours >= 24 || minutes >= 60 || seconds >= 60 {
        return Err(iso_error(target, position, "clock reading out of range"));
    }
    let whole = hours * 3_600 + minutes * 60 + seconds;
    let mut end = position + 8;
    if text.as_bytes().get(end) != Some(&b'.') {
        return Ok((whole, TimeUnit::Second, end));
    }
    end += 1;
    let start = end;
    while text.as_bytes().get(end).is_some_and(u8::is_ascii_digit) {
        end += 1;
    }
    let width = end - start;
    if width == 0 || width > 9 {
        return Err(iso_error(target, start, "fraction must hold 1 to 9 digits"));
    }
    let unit = unit_of_fraction(width);
    let full = fraction_digits(unit);
    let mut fraction = digits(text, start, width, target)?;
    // A short fraction is right-padded to the unit's width: `.5` is `500`
    // milliseconds.
    for _ in width..full {
        fraction *= 10;
    }
    let per = per_second(unit).expect("a fraction width names a resolution unit");
    Ok((whole * per + fraction, unit, end))
}

/// Parse `HH:MM:SS[.fraction]` into a count of `unit` since midnight.
pub(crate) fn parse_time(text: &str) -> Result<(i64, TimeUnit)> {
    let (count, unit, end) = parse_clock_at(text, 0, "time")?;
    if end != text.len() {
        return Err(iso_error("time", end, "trailing text after the time"));
    }
    Ok((count, unit))
}

/// Parse a naive `YYYY-MM-DDTHH:MM:SS[.fraction]`, returning the end position.
fn parse_datetime_at(text: &str, target: &'static str) -> Result<(i64, TimeUnit, usize)> {
    let days = parse_date_at(text, 0)?;
    match text.as_bytes().get(10) {
        Some(b'T' | b't' | b' ') => {}
        _ => return Err(iso_error(target, 10, "expected T between date and time")),
    }
    let (in_day, unit, end) = parse_clock_at(text, 11, target)?;
    let per = per_second(unit).expect("the clock parsed at a resolution unit");
    let count = i64::from(days)
        .checked_mul(DAY * per)
        .and_then(|days| days.checked_add(in_day))
        .ok_or_else(|| iso_error(target, 0, "datetime is out of range"))?;
    Ok((count, unit, end))
}

/// Parse a naive datetime into a count of `unit` since the Unix epoch.
pub(crate) fn parse_datetime(text: &str) -> Result<(i64, TimeUnit)> {
    let (count, unit, end) = parse_datetime_at(text, "datetime")?;
    if end != text.len() {
        return Err(iso_error(
            "datetime",
            end,
            "a naive datetime carries no zone",
        ));
    }
    Ok((count, unit))
}

/// Parse a zoned instant: a local reading, an offset or `Z`, and optionally
/// the zone's bracketed name, which wins over the offset when both appear.
pub(crate) fn parse_timestamp(text: &str) -> Result<(i64, TimeUnit, Timezone)> {
    let (local, unit, mut end) = parse_datetime_at(text, "timestamp")?;
    let per = per_second(unit).expect("the clock parsed at a resolution unit");

    let offset_start = end;
    let offset = match text.as_bytes().get(end) {
        Some(b'Z' | b'z') => {
            end += 1;
            0
        }
        Some(b'+' | b'-') => {
            let rest = &text[end..];
            let bracket = rest.find('[').unwrap_or(rest.len());
            let spelled = &rest[..bracket];
            let zone = Timezone::from_str(spelled)?;
            end += spelled.len();
            zone.offset_at(0)
                .ok_or_else(|| iso_error("timestamp", offset_start, "expected a fixed offset"))?
        }
        _ => {
            return Err(iso_error(
                "timestamp",
                end,
                "a timestamp needs Z or a zone offset",
            ));
        }
    };

    let count = local
        .checked_sub(
            i64::from(offset)
                .checked_mul(per)
                .ok_or_else(|| iso_error("timestamp", offset_start, "timestamp is out of range"))?,
        )
        .ok_or_else(|| iso_error("timestamp", offset_start, "timestamp is out of range"))?;

    let zone = match text[end..].strip_prefix('[') {
        Some(rest) => {
            let name = rest
                .strip_suffix(']')
                .ok_or_else(|| iso_error("timestamp", end, "unclosed zone bracket"))?;
            Timezone::from_str(name)?
        }
        None if end == text.len() => match offset {
            0 => Timezone::UTC,
            offset => Timezone::from_offset(offset)?,
        },
        None => {
            return Err(iso_error("timestamp", end, "trailing text after the zone"));
        }
    };
    Ok((count, unit, zone))
}

/// Parse an ISO duration into a count of `unit`.
///
/// The general form is accepted - `-P1DT2H3M4.5S` - and every component is
/// restated in seconds, so the writer's seconds-only spelling and a reader's
/// decomposed one meet at the same count. The unit is the fraction's width.
pub(crate) fn parse_duration(text: &str) -> Result<(i64, TimeUnit)> {
    let (sign, rest) = match text.strip_prefix('-') {
        Some(rest) => (-1_i64, rest),
        None => (1, text.strip_prefix('+').unwrap_or(text)),
    };
    let rest = rest
        .strip_prefix(['P', 'p'])
        .ok_or_else(|| iso_error("duration", 0, "a duration starts with P"))?;

    let mut seconds: i64 = 0;
    let mut fraction: i64 = 0;
    let mut unit = TimeUnit::Second;
    let mut in_time = false;
    let mut saw_component = false;
    let mut chars = rest.char_indices().peekable();

    while let Some(&(position, character)) = chars.peek() {
        if character == 'T' || character == 't' {
            in_time = true;
            chars.next();
            continue;
        }
        let start = position;
        let mut end = position;
        while chars
            .peek()
            .is_some_and(|(_, digit)| digit.is_ascii_digit())
        {
            let (index, digit) = chars.next().expect("peeked");
            end = index + digit.len_utf8();
        }
        let number: i64 = rest[start..end]
            .parse()
            .map_err(|_| iso_error("duration", start, "expected a component count"))?;

        // A fraction is only classic on the final seconds component.
        let mut with_fraction = None;
        if chars.peek().is_some_and(|&(_, next)| next == '.') {
            chars.next();
            let fraction_start = end + 1;
            let mut fraction_end = fraction_start;
            while chars
                .peek()
                .is_some_and(|(_, digit)| digit.is_ascii_digit())
            {
                let (index, digit) = chars.next().expect("peeked");
                fraction_end = index + digit.len_utf8();
            }
            let width = fraction_end - fraction_start;
            if width == 0 || width > 9 {
                return Err(iso_error(
                    "duration",
                    fraction_start,
                    "fraction must hold 1 to 9 digits",
                ));
            }
            let mut value: i64 = rest[fraction_start..fraction_end]
                .parse()
                .map_err(|_| iso_error("duration", fraction_start, "expected digits"))?;
            let parsed_unit = unit_of_fraction(width);
            for _ in width..fraction_digits(parsed_unit) {
                value *= 10;
            }
            with_fraction = Some((value, parsed_unit));
        }

        let (label_position, label) = chars
            .next()
            .ok_or_else(|| iso_error("duration", end, "component is missing its label"))?;
        let weight = match (label.to_ascii_uppercase(), in_time) {
            ('D', false) => DAY,
            ('H', true) => 3_600,
            ('M', true) => 60,
            ('S', true) => 1,
            _ => {
                return Err(iso_error(
                    "duration",
                    label_position,
                    "expected D, or T then H, M, S",
                ));
            }
        };
        if let Some((value, parsed_unit)) = with_fraction {
            if !label.eq_ignore_ascii_case(&'S') {
                return Err(iso_error(
                    "duration",
                    label_position,
                    "only the seconds component takes a fraction",
                ));
            }
            fraction = value;
            unit = parsed_unit;
        }
        seconds = number
            .checked_mul(weight)
            .and_then(|component| seconds.checked_add(component))
            .ok_or_else(|| iso_error("duration", start, "duration is out of range"))?;
        saw_component = true;
    }

    if !saw_component {
        return Err(iso_error("duration", 0, "a duration names a component"));
    }
    let per = per_second(unit).expect("the fraction width names a resolution unit");
    let count = seconds
        .checked_mul(per)
        .and_then(|whole| whole.checked_add(fraction))
        .and_then(|count| count.checked_mul(sign))
        .ok_or_else(|| iso_error("duration", 0, "duration is out of range"))?;
    Ok((count, unit))
}

#[cfg(test)]
mod tests;
