//! [`Timezone`] — the trait every timezone answers — and [`Tz`], the concrete, `Copy` timezone a
//! timestamp carries: **naive** (no zone), **UTC**, a **fixed UTC offset**, or a named **IANA**
//! zone (DST-aware, backed by the full IANA database via `chrono-tz`). A string parser accepts an
//! IANA name (`"Europe/Paris"`), an offset (`"+02:00"`, `"Z"`), or a Windows-ish `UTC±hh:mm`.
//!
//! `chrono` / `chrono-tz` are an implementation detail here — they never appear in a public
//! signature; the public surface speaks in offset **seconds**, names, and this crate's [`Tz`].

use chrono::{Offset, TimeZone as _};

/// A timezone: it maps a UTC instant to a local **offset** (DST-aware), and names itself.
///
/// The offset is what every calendar conversion needs — a timestamp is stored as a UTC-relative
/// count, and its local wall-clock is `utc + offset`. A **naive** zone has no offset (0) and no
/// name; it means "wall-clock, zone unspecified".
pub trait Timezone {
    /// The seconds east of UTC in effect at `utc_epoch_seconds` (accounting for DST). `0` for UTC
    /// and naive.
    fn offset_seconds_at(&self, utc_epoch_seconds: i64) -> i32;

    /// The zone's name — `"UTC"`, an IANA name like `"Europe/Paris"`, an offset like `"+02:00"`, or
    /// empty for a naive zone.
    fn name(&self) -> String;

    /// Whether this is the **naive** (zone-unspecified) timezone.
    fn is_naive(&self) -> bool;
}

/// The concrete timezone a timestamp carries — `Copy`, so a timestamp stays a plain value.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tz(TzKind);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum TzKind {
    /// No zone — a wall-clock time with unspecified zone.
    Naive,
    /// Coordinated Universal Time.
    Utc,
    /// A fixed offset, in seconds east of UTC.
    FixedOffset(i32),
    /// A named IANA zone (DST-aware).
    Iana(chrono_tz::Tz),
}

impl Tz {
    /// The **naive** timezone (no zone).
    pub const NAIVE: Tz = Tz(TzKind::Naive);
    /// The **UTC** timezone.
    pub const UTC: Tz = Tz(TzKind::Utc);

    /// The naive timezone.
    pub const fn naive() -> Tz {
        Tz::NAIVE
    }

    /// The UTC timezone.
    pub const fn utc() -> Tz {
        Tz::UTC
    }

    /// A fixed offset of `seconds` east of UTC (negative for west).
    pub const fn fixed_offset(seconds: i32) -> Tz {
        Tz(TzKind::FixedOffset(seconds))
    }

    /// A fixed offset from whole `hours` and `minutes` east of UTC (both negative for a western
    /// zone, e.g. `-5, 0` for `-05:00`).
    pub const fn fixed_hours_minutes(hours: i32, minutes: i32) -> Tz {
        Tz(TzKind::FixedOffset(hours * 3_600 + minutes * 60))
    }

    /// The named IANA zone `name` (`"Europe/Paris"`, `"America/New_York"`, …), or `None` if the
    /// name is not a known IANA zone.
    pub fn iana(name: &str) -> Option<Tz> {
        name.parse::<chrono_tz::Tz>()
            .ok()
            .map(|tz| Tz(TzKind::Iana(tz)))
    }

    /// `Europe/Paris` (CET / CEST).
    pub fn europe_paris() -> Tz {
        Tz(TzKind::Iana(chrono_tz::Europe::Paris))
    }
    /// `Europe/London` (GMT / BST).
    pub fn europe_london() -> Tz {
        Tz(TzKind::Iana(chrono_tz::Europe::London))
    }
    /// `America/New_York` (EST / EDT).
    pub fn america_new_york() -> Tz {
        Tz(TzKind::Iana(chrono_tz::America::New_York))
    }
    /// `Asia/Tokyo` (JST).
    pub fn asia_tokyo() -> Tz {
        Tz(TzKind::Iana(chrono_tz::Asia::Tokyo))
    }

    /// Parses a timezone: an empty string or `"naive"` → naive; `"UTC"` / `"Z"` → UTC; an offset
    /// (`"+02:00"`, `"-0530"`, `"+09"`, with an optional `UTC` / `GMT` prefix → Windows style
    /// `"UTC+01:00"`); otherwise a named IANA zone. `None` if unrecognized.
    pub fn parse(text: &str) -> Option<Tz> {
        let trimmed = text.trim().trim_matches(['(', ')']);
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("naive") {
            return Some(Tz::NAIVE);
        }
        if trimmed.eq_ignore_ascii_case("utc")
            || trimmed.eq_ignore_ascii_case("z")
            || trimmed.eq_ignore_ascii_case("gmt")
        {
            return Some(Tz::UTC);
        }
        if let Some(seconds) = parse_offset(trimmed) {
            return Some(Tz::fixed_offset(seconds));
        }
        Tz::iana(trimmed)
    }

    /// The offset in seconds east of UTC at `utc_epoch_seconds` (DST-aware for IANA zones).
    pub fn offset_seconds_at(&self, utc_epoch_seconds: i64) -> i32 {
        match self.0 {
            TzKind::Naive | TzKind::Utc => 0,
            TzKind::FixedOffset(seconds) => seconds,
            TzKind::Iana(tz) => match chrono::DateTime::from_timestamp(utc_epoch_seconds, 0) {
                Some(utc) => tz
                    .offset_from_utc_datetime(&utc.naive_utc())
                    .fix()
                    .local_minus_utc(),
                None => 0, // out of chrono's representable range — fall back to no offset
            },
        }
    }

    /// The zone's name (see [`Timezone::name`]).
    pub fn name(&self) -> String {
        match self.0 {
            TzKind::Naive => String::new(),
            TzKind::Utc => "UTC".to_string(),
            TzKind::FixedOffset(seconds) => format_offset(seconds),
            TzKind::Iana(tz) => tz.name().to_string(),
        }
    }

    /// Whether this is the naive zone.
    pub const fn is_naive(&self) -> bool {
        matches!(self.0, TzKind::Naive)
    }
    /// Whether this is UTC.
    pub const fn is_utc(&self) -> bool {
        matches!(self.0, TzKind::Utc)
    }
    /// Whether this is a fixed offset.
    pub const fn is_fixed_offset(&self) -> bool {
        matches!(self.0, TzKind::FixedOffset(_))
    }
    /// Whether this is a named IANA zone.
    pub const fn is_iana(&self) -> bool {
        matches!(self.0, TzKind::Iana(_))
    }

    /// A total-order key `(kind, offset, iana-name)` consistent with [`Eq`] — the tiebreak a zoned
    /// timestamp's ordering uses (allocation-free; the IANA name is `&'static`).
    pub(super) fn sort_key(&self) -> (u8, i64, &'static str) {
        match self.0 {
            TzKind::Naive => (0, 0, ""),
            TzKind::Utc => (1, 0, ""),
            TzKind::FixedOffset(seconds) => (2, seconds as i64, ""),
            TzKind::Iana(tz) => (3, 0, tz.name()),
        }
    }
}

impl Timezone for Tz {
    fn offset_seconds_at(&self, utc_epoch_seconds: i64) -> i32 {
        Tz::offset_seconds_at(self, utc_epoch_seconds)
    }
    fn name(&self) -> String {
        Tz::name(self)
    }
    fn is_naive(&self) -> bool {
        Tz::is_naive(self)
    }
}

impl Default for Tz {
    fn default() -> Self {
        Tz::NAIVE
    }
}

impl core::fmt::Debug for Tz {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            TzKind::Naive => f.write_str("Tz(naive)"),
            _ => write!(f, "Tz({})", self.name()),
        }
    }
}

impl core::fmt::Display for Tz {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            TzKind::Naive => f.write_str("naive"),
            _ => f.write_str(&self.name()),
        }
    }
}

/// Formats a fixed offset in seconds as `±HH:MM` (`0` → `"+00:00"`).
fn format_offset(seconds: i32) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    let abs = seconds.unsigned_abs();
    format!("{sign}{:02}:{:02}", abs / 3_600, (abs % 3_600) / 60)
}

/// Parses an offset string (`"+02:00"`, `"-0530"`, `"+09"`, or with a leading `UTC`/`GMT`) into
/// seconds east of UTC.
fn parse_offset(text: &str) -> Option<i32> {
    // Strip an optional Windows-style `UTC` / `GMT` prefix.
    let body = text
        .strip_prefix("UTC")
        .or_else(|| text.strip_prefix("utc"))
        .or_else(|| text.strip_prefix("GMT"))
        .or_else(|| text.strip_prefix("gmt"))
        .unwrap_or(text)
        .trim();
    let (sign, rest) = match body.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, body.strip_prefix('+')?), // an offset must carry an explicit sign
    };
    let digits: String = rest.chars().filter(|c| c.is_ascii_digit()).collect();
    let (hours, minutes) = match digits.len() {
        1 | 2 => (digits.parse::<i32>().ok()?, 0), // "+9", "+09"
        4 => (digits[..2].parse().ok()?, digits[2..].parse().ok()?), // "+0930"
        _ => return None,
    };
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some(sign * (hours * 3_600 + minutes * 60))
}
