//! The [`Timezone`] type and its self-contained DST engine.
//!
//! A zone is one of: [`Utc`](Timezone::Utc), a [`Fixed`](Timezone::Fixed) offset,
//! or a [`Named`](Timezone::Named) IANA zone. Named zones carry a parsed **POSIX TZ
//! rule** (e.g. `EST5EDT,M3.2.0,M11.1.0`), so the standard-vs-DST offset for any
//! instant is computed from embedded rules — no external timezone database.
//!
//! Coverage: UTC, every fixed offset, raw POSIX TZ strings, and a curated table of
//! common IANA zone names (see [`ZONE_TABLE`]). DST is computed from each zone's
//! **current** rule; historical transitions (which need the full tz database) are
//! not modelled, so instants far in the past may use today's rule.

use std::fmt;

#[allow(unused_imports)]
use crate::log_event;

use super::{civil_from_days, days_from_civil, days_in_month, TimeError};

/// A timezone: UTC, a fixed offset, or a named IANA zone with DST rules.
///
/// ```
/// use yggdryl_core::Timezone;
///
/// assert_eq!(Timezone::from_str("UTC").unwrap(), Timezone::Utc);
/// assert_eq!(Timezone::from_str("+05:30").unwrap().offset_seconds(0), 19_800);
/// let ny = Timezone::from_str("America/New_York").unwrap();
/// // 2024-01-01T00:00Z is EST (UTC-5); 2024-07-01T00:00Z is EDT (UTC-4).
/// assert_eq!(ny.offset_seconds(1_704_067_200), -5 * 3600);
/// assert_eq!(ny.offset_seconds(1_719_792_000), -4 * 3600);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Timezone {
    /// Coordinated Universal Time (offset 0, no DST).
    Utc,
    /// A fixed offset east of UTC, in seconds (e.g. `+05:30` is `19_800`).
    Fixed(i32),
    /// A named zone with its parsed POSIX-TZ DST [`rule`](TzRule).
    Named {
        /// The canonical IANA name (or the raw POSIX string).
        name: String,
        /// The parsed DST rule used to compute offsets.
        rule: TzRule,
    },
}

impl Timezone {
    /// Parses `"UTC"` / `"Z"`, a `±HH[:MM[:SS]]` offset, an IANA name from
    /// [`ZONE_TABLE`] (case-insensitive) or a raw POSIX TZ string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<Timezone, TimeError> {
        log_event!(trace, "Timezone::from_str {input:?}");
        let value = input.trim();
        if value.is_empty() {
            return Err(TimeError::Empty);
        }
        if value.eq_ignore_ascii_case("utc")
            || value.eq_ignore_ascii_case("gmt")
            || value.eq_ignore_ascii_case("z")
            || value.eq_ignore_ascii_case("etc/utc")
        {
            return Ok(Timezone::Utc);
        }
        if let Some(offset) = parse_fixed_offset(value) {
            return Ok(if offset == 0 {
                Timezone::Utc
            } else {
                Timezone::Fixed(offset)
            });
        }
        // A known IANA name resolves to its embedded POSIX rule.
        if let Some((canonical, posix)) = lookup_zone(value) {
            if let Some(rule) = TzRule::from_posix(posix) {
                return Ok(Timezone::Named {
                    name: canonical.to_string(),
                    rule,
                });
            }
        }
        // Otherwise accept a raw POSIX TZ string directly.
        if let Some(rule) = TzRule::from_posix(value) {
            return Ok(Timezone::Named {
                name: value.to_string(),
                rule,
            });
        }
        Err(TimeError::UnknownZone(input.to_string()))
    }

    /// The offset east of UTC, in seconds, that applies at the given UTC instant
    /// (`utc_epoch_seconds`). DST-aware for [`Named`](Timezone::Named) zones.
    pub fn offset_seconds(&self, utc_epoch_seconds: i64) -> i32 {
        match self {
            Timezone::Utc => 0,
            Timezone::Fixed(offset) => *offset,
            Timezone::Named { rule, .. } => rule.offset_seconds(utc_epoch_seconds),
        }
    }

    /// The canonical name / offset string (`"UTC"`, `"+05:30"`, `"America/New_York"`).
    pub fn name(&self) -> String {
        match self {
            Timezone::Utc => "UTC".to_string(),
            Timezone::Fixed(offset) => format_offset(*offset),
            Timezone::Named { name, .. } => name.clone(),
        }
    }

    /// Whether this is [`Utc`](Timezone::Utc).
    pub fn is_utc(&self) -> bool {
        matches!(self, Timezone::Utc)
    }

    /// Whether this is a [`Fixed`](Timezone::Fixed) offset.
    pub fn is_fixed(&self) -> bool {
        matches!(self, Timezone::Fixed(_))
    }

    /// Renders to its canonical string — the inverse of [`from_str`](Timezone::from_str).
    pub fn to_str(&self) -> String {
        self.name()
    }
}

impl fmt::Display for Timezone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name())
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Timezone {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Timezone {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Timezone, D::Error> {
        let raw = <String as serde::Deserialize>::deserialize(deserializer)?;
        Timezone::from_str(&raw).map_err(serde::de::Error::custom)
    }
}

/// A parsed POSIX-TZ rule: a standard offset and an optional DST schedule.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TzRule {
    /// Standard-time offset east of UTC, in seconds.
    std_offset: i32,
    /// The DST schedule, if the zone observes daylight saving.
    dst: Option<DstRule>,
}

/// The DST half of a [`TzRule`]: the summer offset and the two transitions.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DstRule {
    offset: i32,
    start: TzTransition,
    end: TzTransition,
}

/// One POSIX-TZ transition rule (when, in local wall-clock terms, a change occurs).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum TzTransition {
    /// `Mm.w.d`: the `week`-th `dow` of `month` (week 5 = last), at `time` seconds.
    Month {
        month: u32,
        week: u32,
        dow: u32,
        time: i32,
    },
    /// `Jn`: day `1..=365`, never counting 29 February, at `time` seconds.
    Julian { day: u32, time: i32 },
    /// `n`: zero-based day `0..=365`, counting 29 February, at `time` seconds.
    ZeroJulian { day: u32, time: i32 },
}

impl TzRule {
    /// Parses a POSIX TZ string such as `EST5EDT,M3.2.0,M11.1.0`. Returns `None`
    /// for anything that is not a valid POSIX rule.
    pub fn from_posix(input: &str) -> Option<TzRule> {
        let bytes = input.trim().as_bytes();
        let mut pos = 0;
        skip_abbrev(bytes, &mut pos);
        let std_posix = parse_offset(bytes, &mut pos)?;
        let std_offset = -std_posix; // POSIX is west-positive; store east-positive.
        if pos >= bytes.len() {
            return Some(TzRule {
                std_offset,
                dst: None,
            });
        }
        skip_abbrev(bytes, &mut pos);
        // An explicit DST offset, else the POSIX default of one hour ahead.
        let dst_posix = if pos < bytes.len() && bytes[pos] != b',' {
            parse_offset(bytes, &mut pos)?
        } else {
            std_posix - 3600
        };
        let dst_offset = -dst_posix;
        if pos < bytes.len() && bytes[pos] == b',' {
            pos += 1;
            let start = parse_transition(bytes, &mut pos)?;
            if pos >= bytes.len() || bytes[pos] != b',' {
                return None;
            }
            pos += 1;
            let end = parse_transition(bytes, &mut pos)?;
            Some(TzRule {
                std_offset,
                dst: Some(DstRule {
                    offset: dst_offset,
                    start,
                    end,
                }),
            })
        } else {
            // A DST abbreviation with no transition rules: treat as standard-only.
            Some(TzRule {
                std_offset,
                dst: None,
            })
        }
    }

    /// The offset east of UTC, in seconds, at the given UTC instant.
    fn offset_seconds(&self, utc: i64) -> i32 {
        let Some(dst) = &self.dst else {
            return self.std_offset;
        };
        let approx_local = utc + self.std_offset as i64;
        let (year, _, _) = civil_from_days(approx_local.div_euclid(86_400));
        let start = transition_utc(&dst.start, year, self.std_offset);
        let end = transition_utc(&dst.end, year, dst.offset);
        let in_dst = if start <= end {
            utc >= start && utc < end // northern hemisphere
        } else {
            utc >= start || utc < end // southern hemisphere (wraps the year)
        };
        if in_dst {
            dst.offset
        } else {
            self.std_offset
        }
    }
}

/// The UTC instant (epoch seconds) of a transition in `year`, given the offset in
/// effect immediately before it.
fn transition_utc(transition: &TzTransition, year: i32, offset_before: i32) -> i64 {
    let (y, m, d, time) = match transition {
        TzTransition::Month {
            month,
            week,
            dow,
            time,
        } => {
            let (y, m, d) = nth_weekday(year, *month, *week, *dow);
            (y, m, d, *time)
        }
        TzTransition::Julian { day, time } => {
            let (m, d) = julian_to_md(*day);
            (year, m, d, *time)
        }
        TzTransition::ZeroJulian { day, time } => {
            let (y, m, d) = civil_from_days(days_from_civil(year, 1, 1) + *day as i64);
            (y, m, d, *time)
        }
    };
    let local = days_from_civil(y, m, d) * 86_400 + time as i64;
    local - offset_before as i64
}

/// The date of the `week`-th `dow` (0 = Sunday) of `month` in `year` (week 5 = last).
fn nth_weekday(year: i32, month: u32, week: u32, dow: u32) -> (i32, u32, u32) {
    let first = days_from_civil(year, month, 1);
    let first_dow = (first + 4).rem_euclid(7) as u32; // 1970-01-01 was a Thursday
    let shift = (7 + dow - first_dow) % 7;
    let mut day = 1 + shift + (week.saturating_sub(1)) * 7;
    let dim = days_in_month(year, month);
    if day > dim {
        day -= 7; // week 5 overshoot -> the last occurrence
    }
    (year, month, day)
}

/// `(month, day)` of the POSIX Julian day `1..=365` (29 February never counted).
fn julian_to_md(day: u32) -> (u32, u32) {
    let months = [31u32, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut n = day.clamp(1, 365);
    let mut m = 0usize;
    while m < 12 && n > months[m] {
        n -= months[m];
        m += 1;
    }
    ((m + 1) as u32, n)
}

/// Parses a `±HH[:MM[:SS]]` clock offset into seconds east of UTC, or `None`.
fn parse_fixed_offset(value: &str) -> Option<i32> {
    let bytes = value.as_bytes();
    if bytes.is_empty() || (bytes[0] != b'+' && bytes[0] != b'-') {
        return None;
    }
    let sign = if bytes[0] == b'-' { -1 } else { 1 };
    let rest = &value[1..];
    // Accept "HH:MM[:SS]" or compact "HHMM" / "HH".
    let (h, m, s) = if rest.contains(':') {
        let mut it = rest.split(':');
        let h = it.next()?.parse::<i32>().ok()?;
        let m = it.next().map(|p| p.parse::<i32>()).transpose().ok()??;
        let s = match it.next() {
            Some(p) => p.parse::<i32>().ok()?,
            None => 0,
        };
        if it.next().is_some() {
            return None;
        }
        (h, m, s)
    } else if rest.len() == 4 && rest.bytes().all(|b| b.is_ascii_digit()) {
        (rest[..2].parse().ok()?, rest[2..].parse().ok()?, 0)
    } else if (rest.len() == 1 || rest.len() == 2) && rest.bytes().all(|b| b.is_ascii_digit()) {
        (rest.parse().ok()?, 0, 0)
    } else {
        return None;
    };
    if !(0..=14).contains(&h) || !(0..60).contains(&m) || !(0..60).contains(&s) {
        return None;
    }
    Some(sign * (h * 3600 + m * 60 + s))
}

/// Renders an offset (seconds east of UTC) as `"+HH:MM"` (or `"+HH:MM:SS"`).
fn format_offset(seconds: i32) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    let abs = seconds.unsigned_abs();
    let (h, m, s) = (abs / 3600, (abs % 3600) / 60, abs % 60);
    if s == 0 {
        format!("{sign}{h:02}:{m:02}")
    } else {
        format!("{sign}{h:02}:{m:02}:{s:02}")
    }
}

// ---- POSIX TZ byte scanner ----

/// Skips a zone abbreviation: either `<...>` or a run of letters / `+` / `-` that
/// are *not* part of an offset (a leading sign before digits belongs to the offset).
fn skip_abbrev(bytes: &[u8], pos: &mut usize) {
    if *pos < bytes.len() && bytes[*pos] == b'<' {
        while *pos < bytes.len() && bytes[*pos] != b'>' {
            *pos += 1;
        }
        if *pos < bytes.len() {
            *pos += 1; // consume '>'
        }
        return;
    }
    while *pos < bytes.len() && bytes[*pos].is_ascii_alphabetic() {
        *pos += 1;
    }
}

/// Parses a POSIX offset `[+|-]hh[:mm[:ss]]` at `pos`, returning seconds (as
/// written, west-positive). Advances `pos`. `None` if no digits are present.
fn parse_offset(bytes: &[u8], pos: &mut usize) -> Option<i32> {
    let mut sign = 1;
    if *pos < bytes.len() && (bytes[*pos] == b'+' || bytes[*pos] == b'-') {
        if bytes[*pos] == b'-' {
            sign = -1;
        }
        *pos += 1;
    }
    let h = parse_uint(bytes, pos)?;
    let mut m = 0;
    let mut s = 0;
    if *pos < bytes.len() && bytes[*pos] == b':' {
        *pos += 1;
        m = parse_uint(bytes, pos).unwrap_or(0);
        if *pos < bytes.len() && bytes[*pos] == b':' {
            *pos += 1;
            s = parse_uint(bytes, pos).unwrap_or(0);
        }
    }
    Some(sign * (h * 3600 + m * 60 + s))
}

/// Parses a transition rule `Mm.w.d` / `Jn` / `n`, with an optional `/time`.
fn parse_transition(bytes: &[u8], pos: &mut usize) -> Option<TzTransition> {
    if *pos >= bytes.len() {
        return None;
    }
    let kind = bytes[*pos];
    let transition = if kind == b'M' {
        *pos += 1;
        let month = parse_uint(bytes, pos)? as u32;
        expect(bytes, pos, b'.')?;
        let week = parse_uint(bytes, pos)? as u32;
        expect(bytes, pos, b'.')?;
        let dow = parse_uint(bytes, pos)? as u32;
        if !(1..=12).contains(&month) || !(1..=5).contains(&week) || dow > 6 {
            return None;
        }
        TzTransition::Month {
            month,
            week,
            dow,
            time: parse_rule_time(bytes, pos),
        }
    } else if kind == b'J' {
        *pos += 1;
        let day = parse_uint(bytes, pos)? as u32;
        TzTransition::Julian {
            day,
            time: parse_rule_time(bytes, pos),
        }
    } else {
        let day = parse_uint(bytes, pos)? as u32;
        TzTransition::ZeroJulian {
            day,
            time: parse_rule_time(bytes, pos),
        }
    };
    Some(transition)
}

/// Parses an optional `/time` suffix of a transition, defaulting to 02:00:00.
fn parse_rule_time(bytes: &[u8], pos: &mut usize) -> i32 {
    if *pos < bytes.len() && bytes[*pos] == b'/' {
        *pos += 1;
        parse_offset(bytes, pos).unwrap_or(7200)
    } else {
        7200
    }
}

/// Parses a run of ASCII digits as an `i32`, or `None` if none are present.
fn parse_uint(bytes: &[u8], pos: &mut usize) -> Option<i32> {
    let start = *pos;
    while *pos < bytes.len() && bytes[*pos].is_ascii_digit() {
        *pos += 1;
    }
    if *pos == start {
        return None;
    }
    std::str::from_utf8(&bytes[start..*pos])
        .ok()?
        .parse::<i32>()
        .ok()
}

/// Consumes the expected byte at `pos`, or `None` if it is not there.
fn expect(bytes: &[u8], pos: &mut usize, byte: u8) -> Option<()> {
    if *pos < bytes.len() && bytes[*pos] == byte {
        *pos += 1;
        Some(())
    } else {
        None
    }
}

/// Looks up an IANA name (case-insensitive) in [`ZONE_TABLE`], returning the
/// canonical name and its POSIX rule string.
fn lookup_zone(name: &str) -> Option<(&'static str, &'static str)> {
    ZONE_TABLE
        .iter()
        .find(|(zone, _)| zone.eq_ignore_ascii_case(name))
        .map(|(zone, posix)| (*zone, *posix))
}

/// The embedded IANA name → POSIX-TZ table (current DST rules). A curated but broad
/// set covering the common zones across every continent; pass a raw POSIX TZ string
/// for anything not listed.
pub static ZONE_TABLE: &[(&str, &str)] = &[
    // ---- North America ----
    ("America/New_York", "EST5EDT,M3.2.0,M11.1.0"),
    ("America/Detroit", "EST5EDT,M3.2.0,M11.1.0"),
    ("America/Toronto", "EST5EDT,M3.2.0,M11.1.0"),
    ("America/Chicago", "CST6CDT,M3.2.0,M11.1.0"),
    ("America/Winnipeg", "CST6CDT,M3.2.0,M11.1.0"),
    ("America/Denver", "MST7MDT,M3.2.0,M11.1.0"),
    ("America/Edmonton", "MST7MDT,M3.2.0,M11.1.0"),
    ("America/Phoenix", "MST7"),
    ("America/Los_Angeles", "PST8PDT,M3.2.0,M11.1.0"),
    ("America/Vancouver", "PST8PDT,M3.2.0,M11.1.0"),
    ("America/Anchorage", "AKST9AKDT,M3.2.0,M11.1.0"),
    ("America/Halifax", "AST4ADT,M3.2.0,M11.1.0"),
    ("America/St_Johns", "NST3:30NDT,M3.2.0,M11.1.0"),
    ("America/Mexico_City", "CST6"),
    ("America/Sao_Paulo", "<-03>3"),
    ("America/Argentina/Buenos_Aires", "<-03>3"),
    ("America/Bogota", "<-05>5"),
    ("America/Lima", "<-05>5"),
    ("America/Santiago", "<-04>4<-03>,M9.1.6/24,M4.1.6/24"),
    ("Pacific/Honolulu", "HST10"),
    // ---- Europe ----
    ("Europe/London", "GMT0BST,M3.5.0/1,M10.5.0"),
    ("Europe/Dublin", "GMT0IST,M3.5.0/1,M10.5.0"),
    ("Europe/Lisbon", "WET0WEST,M3.5.0/1,M10.5.0"),
    ("Europe/Paris", "CET-1CEST,M3.5.0,M10.5.0/3"),
    ("Europe/Berlin", "CET-1CEST,M3.5.0,M10.5.0/3"),
    ("Europe/Madrid", "CET-1CEST,M3.5.0,M10.5.0/3"),
    ("Europe/Rome", "CET-1CEST,M3.5.0,M10.5.0/3"),
    ("Europe/Amsterdam", "CET-1CEST,M3.5.0,M10.5.0/3"),
    ("Europe/Brussels", "CET-1CEST,M3.5.0,M10.5.0/3"),
    ("Europe/Vienna", "CET-1CEST,M3.5.0,M10.5.0/3"),
    ("Europe/Zurich", "CET-1CEST,M3.5.0,M10.5.0/3"),
    ("Europe/Warsaw", "CET-1CEST,M3.5.0,M10.5.0/3"),
    ("Europe/Stockholm", "CET-1CEST,M3.5.0,M10.5.0/3"),
    ("Europe/Prague", "CET-1CEST,M3.5.0,M10.5.0/3"),
    ("Europe/Budapest", "CET-1CEST,M3.5.0,M10.5.0/3"),
    ("Europe/Athens", "EET-2EEST,M3.5.0/3,M10.5.0/4"),
    ("Europe/Helsinki", "EET-2EEST,M3.5.0/3,M10.5.0/4"),
    ("Europe/Bucharest", "EET-2EEST,M3.5.0/3,M10.5.0/4"),
    ("Europe/Kyiv", "EET-2EEST,M3.5.0/3,M10.5.0/4"),
    ("Europe/Kiev", "EET-2EEST,M3.5.0/3,M10.5.0/4"),
    ("Europe/Moscow", "MSK-3"),
    ("Europe/Istanbul", "<+03>-3"),
    // ---- Asia ----
    ("Asia/Tokyo", "JST-9"),
    ("Asia/Shanghai", "CST-8"),
    ("Asia/Hong_Kong", "HKT-8"),
    ("Asia/Taipei", "CST-8"),
    ("Asia/Singapore", "<+08>-8"),
    ("Asia/Seoul", "KST-9"),
    ("Asia/Kolkata", "IST-5:30"),
    ("Asia/Dubai", "<+04>-4"),
    ("Asia/Bangkok", "<+07>-7"),
    ("Asia/Jakarta", "WIB-7"),
    ("Asia/Manila", "PST-8"),
    ("Asia/Karachi", "PKT-5"),
    ("Asia/Tehran", "<+0330>-3:30"),
    ("Asia/Jerusalem", "IST-2IDT,M3.4.4/26,M10.5.0"),
    ("Asia/Kathmandu", "<+0545>-5:45"),
    ("Asia/Yangon", "<+0630>-6:30"),
    // ---- Australia / Pacific ----
    ("Australia/Sydney", "AEST-10AEDT,M10.1.0,M4.1.0/3"),
    ("Australia/Melbourne", "AEST-10AEDT,M10.1.0,M4.1.0/3"),
    ("Australia/Brisbane", "AEST-10"),
    ("Australia/Adelaide", "ACST-9:30ACDT,M10.1.0,M4.1.0/3"),
    ("Australia/Perth", "AWST-8"),
    ("Australia/Darwin", "ACST-9:30"),
    ("Pacific/Auckland", "NZST-12NZDT,M9.5.0,M4.1.0/3"),
    ("Pacific/Guam", "ChST-10"),
    // ---- Africa ----
    ("Africa/Cairo", "EET-2EEST,M4.4.5,M10.5.4/24"),
    ("Africa/Johannesburg", "SAST-2"),
    ("Africa/Lagos", "WAT-1"),
    ("Africa/Nairobi", "EAT-3"),
    ("Africa/Casablanca", "<+01>-1"),
    ("Africa/Accra", "GMT0"),
    // ---- UTC aliases ----
    ("UTC", "UTC0"),
    ("Etc/UTC", "UTC0"),
];
