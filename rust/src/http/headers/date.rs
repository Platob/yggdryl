//! HTTP-date: the three spellings RFC 9110 section 5.6.7 receives and the one
//! it sends.
//!
//! A sender writes IMF-fixdate (`Sun, 06 Nov 1994 08:49:37 GMT`); a receiver
//! also reads the obsolete RFC 850 form (`Sunday, 06-Nov-94 08:49:37 GMT`)
//! and ANSI C's `asctime()` form (`Sun Nov  6 08:49:37 1994`). Every form is
//! UTC, and an instant crosses as nanoseconds since the Unix epoch, the unit
//! every temporal value of the crate speaks.

use smol_str::format_smolstr;

use crate::timezone::{civil_from_days, days_from_civil};
use crate::{Error, Result};

const NANOS_PER_SECOND: i64 = 1_000_000_000;
const SECONDS_PER_DAY: i64 = 86_400;

const DAY_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const LONG_DAY_NAMES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Parse an HTTP-date in any of the three forms RFC 9110 receives.
///
/// The answer is UTC nanoseconds since the Unix epoch. The day name is read
/// and must be one of the seven, but its agreement with the date is not
/// checked: RFC 9110 makes it a display convenience, and a clock that
/// disagrees with its own calendar still names one instant. An RFC 850
/// two-digit year reads `70`-`99` as 1970-1999 and `00`-`69` as 2000-2069,
/// so the parser needs no clock.
///
/// ```
/// use yggdryl::http::parse_http_date;
///
/// # fn main() -> yggdryl::Result<()> {
/// let imf = parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT")?;
/// assert_eq!(imf, 784_111_777 * 1_000_000_000);
/// assert_eq!(parse_http_date("Sunday, 06-Nov-94 08:49:37 GMT")?, imf);
/// assert_eq!(parse_http_date("Sun Nov  6 08:49:37 1994")?, imf);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns [`Error::Parse`] with `target` `http date` and the byte position
/// of the first token that fits none of the three forms, or names a
/// calendar value out of range - a 31st of April, a 25th hour.
pub fn parse_http_date(text: &str) -> Result<i64> {
    let mut cursor = Cursor { text, position: 0 };
    let day_name = cursor.token(|byte| byte.is_ascii_alphabetic())?;
    if cursor.peek() == Some(b',') {
        cursor.position += 1;
        cursor.expect(b' ')?;
        if DAY_NAMES.contains(&day_name) {
            return parse_imf_fixdate(&mut cursor);
        }
        if LONG_DAY_NAMES.contains(&day_name) {
            return parse_rfc850(&mut cursor);
        }
        return Err(cursor.refuse_at(0, "expected a day name"));
    }
    if !DAY_NAMES.contains(&day_name) {
        return Err(cursor.refuse_at(0, "expected a day name"));
    }
    cursor.expect(b' ')?;
    parse_asctime(&mut cursor)
}

/// Render an instant as IMF-fixdate, the one form RFC 9110 sends.
///
/// `ns` is UTC nanoseconds since the Unix epoch; a fraction of a second is
/// dropped, and an instant before the epoch rounds toward the past.
///
/// ```
/// use yggdryl::http::render_http_date;
///
/// assert_eq!(
///     render_http_date(784_111_777 * 1_000_000_000),
///     "Sun, 06 Nov 1994 08:49:37 GMT"
/// );
/// ```
pub fn render_http_date(ns: i64) -> String {
    let seconds = ns.div_euclid(NANOS_PER_SECOND);
    let days = seconds.div_euclid(SECONDS_PER_DAY);
    let second_of_day = seconds.rem_euclid(SECONDS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    // 1970-01-01 was a Thursday, which is index 4.
    let weekday = DAY_NAMES[usize::try_from((days + 4).rem_euclid(7)).unwrap_or(0)];
    let month_name = MONTH_NAMES[usize::try_from(month.saturating_sub(1))
        .unwrap_or(0)
        .min(11)];
    let (hour, minute, second) = (
        second_of_day / 3600,
        second_of_day / 60 % 60,
        second_of_day % 60,
    );
    format!("{weekday}, {day:02} {month_name} {year:04} {hour:02}:{minute:02}:{second:02} GMT")
}

/// `day SP month SP year SP time-of-day SP GMT`, after `day-name ", "`.
fn parse_imf_fixdate(cursor: &mut Cursor<'_>) -> Result<i64> {
    let day = cursor.digits(2)?;
    cursor.expect(b' ')?;
    let month = cursor.month()?;
    cursor.expect(b' ')?;
    let year = cursor.digits(4)?;
    cursor.expect(b' ')?;
    let seconds = cursor.time_of_day()?;
    cursor.expect(b' ')?;
    cursor.gmt()?;
    cursor.finish()?;
    cursor.instant(year, month, day, seconds)
}

/// `day "-" month "-" 2DIGIT SP time-of-day SP GMT`, after `day-name ", "`.
fn parse_rfc850(cursor: &mut Cursor<'_>) -> Result<i64> {
    let day = cursor.digits(2)?;
    cursor.expect(b'-')?;
    let month = cursor.month()?;
    cursor.expect(b'-')?;
    let year = cursor.digits(2)?;
    cursor.expect(b' ')?;
    let seconds = cursor.time_of_day()?;
    cursor.expect(b' ')?;
    cursor.gmt()?;
    cursor.finish()?;
    let year = if year >= 70 { 1900 + year } else { 2000 + year };
    cursor.instant(year, month, day, seconds)
}

/// `month SP ( 2DIGIT / ( SP 1DIGIT )) SP time-of-day SP year`, after
/// `day-name SP`.
fn parse_asctime(cursor: &mut Cursor<'_>) -> Result<i64> {
    let month = cursor.month()?;
    cursor.expect(b' ')?;
    let day = if cursor.peek() == Some(b' ') {
        cursor.position += 1;
        cursor.digits(1)?
    } else {
        cursor.digits(2)?
    };
    cursor.expect(b' ')?;
    let seconds = cursor.time_of_day()?;
    cursor.expect(b' ')?;
    let year = cursor.digits(4)?;
    cursor.finish()?;
    cursor.instant(year, month, day, seconds)
}

struct Cursor<'a> {
    text: &'a str,
    position: usize,
}

impl<'a> Cursor<'a> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.position).copied()
    }

    fn refuse(&self, reason: &str) -> Error {
        self.refuse_at(self.position, reason)
    }

    fn refuse_at(&self, position: usize, reason: &str) -> Error {
        Error::Parse {
            target: "http date",
            position,
            reason: format_smolstr!("{reason} in {:?}", crate::text::elide_to(self.text, 64)),
        }
    }

    fn token(&mut self, accept: impl Fn(u8) -> bool) -> Result<&'a str> {
        let start = self.position;
        while self.peek().is_some_and(&accept) {
            self.position += 1;
        }
        if self.position == start {
            return Err(self.refuse("expected a name"));
        }
        Ok(&self.text[start..self.position])
    }

    fn expect(&mut self, byte: u8) -> Result<()> {
        if self.peek() == Some(byte) {
            self.position += 1;
            return Ok(());
        }
        Err(self.refuse(&format!("expected {:?}", char::from(byte))))
    }

    fn digits(&mut self, count: usize) -> Result<i64> {
        let start = self.position;
        let mut value = 0_i64;
        for _ in 0..count {
            match self.peek() {
                Some(byte @ b'0'..=b'9') => {
                    value = value * 10 + i64::from(byte - b'0');
                    self.position += 1;
                }
                _ => return Err(self.refuse_at(start, &format!("expected {count} digit(s)"))),
            }
        }
        Ok(value)
    }

    fn month(&mut self) -> Result<u32> {
        let start = self.position;
        let name = self.token(|byte| byte.is_ascii_alphabetic())?;
        MONTH_NAMES
            .iter()
            .position(|month| *month == name)
            .map(|index| index as u32 + 1)
            .ok_or_else(|| self.refuse_at(start, "expected a month name"))
    }

    fn time_of_day(&mut self) -> Result<i64> {
        let start = self.position;
        let hour = self.digits(2)?;
        self.expect(b':')?;
        let minute = self.digits(2)?;
        self.expect(b':')?;
        let second = self.digits(2)?;
        // A leap second is spelled `60` and is one second past `59`.
        if hour > 23 || minute > 59 || second > 60 {
            return Err(self.refuse_at(start, "expected a time of day below 24:00:00"));
        }
        Ok(hour * 3600 + minute * 60 + second)
    }

    fn gmt(&mut self) -> Result<()> {
        if self.text[self.position..].starts_with("GMT") {
            self.position += 3;
            return Ok(());
        }
        Err(self.refuse("expected GMT"))
    }

    fn finish(&self) -> Result<()> {
        if self.position == self.text.len() {
            return Ok(());
        }
        Err(self.refuse("expected the end of the date"))
    }

    fn instant(&self, year: i64, month: u32, day: i64, seconds: i64) -> Result<i64> {
        let Ok(day) = u32::try_from(day) else {
            return Err(self.refuse_at(0, "expected a day of the month"));
        };
        let Ok(year) = i32::try_from(year) else {
            return Err(self.refuse_at(0, "expected a four-digit year"));
        };
        let days = days_from_civil(year, month, day);
        if day == 0 || civil_from_days(days) != (year, month, day) {
            return Err(self.refuse_at(0, "expected a day the month has"));
        }
        days.checked_mul(SECONDS_PER_DAY)
            .and_then(|day_seconds| day_seconds.checked_add(seconds))
            .and_then(|seconds| seconds.checked_mul(NANOS_PER_SECOND))
            .ok_or_else(|| self.refuse_at(0, "expected an instant nanoseconds can hold"))
    }
}
