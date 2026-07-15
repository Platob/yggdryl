//! [`Ts32`] / [`Ts64`] / [`Ts96`] — an **instant**: a count since the Unix
//! epoch in a [`TimeUnit`], carried with a [`Tz`] (naive, UTC, a fixed offset, or a DST-aware IANA
//! zone). The value is always UTC-relative; the timezone drives the wall-clock decomposition.
//! Backed by `i32` / `i64` / a 96-bit integer (stored in an `i128`), Arrow `Timestamp` and wider.

use super::civil::{self, Civil};
use super::date::FmtYear;
use super::time::{unit_from_tag, unit_tag};
use super::{Date32, Duration64, Temporal, TemporalError, Time64, TimeUnit, Tz};

const NANOS_PER_SEC: i128 = 1_000_000_000;

/// The wall-clock [`Civil`] components of `value` counts of `unit`, viewed in `tz` (DST-aware).
fn decompose(value: i128, unit: TimeUnit, tz: &Tz) -> Civil {
    let epoch_nanos = value.saturating_mul(unit.nanos().unwrap_or(1));
    let offset = tz.offset_seconds_at(epoch_seconds_of(epoch_nanos)) as i128;
    let (days, nanos_of_day) = civil::split_epoch_nanos(epoch_nanos + offset * NANOS_PER_SEC);
    let (year, month, day) = civil::civil_from_days(days);
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

/// The UTC count of `unit` for wall-clock `civil` viewed in `tz`, or `None` on overflow.
///
/// DESIGN: resolving a *local* wall-clock to a UTC instant needs the zone's offset, which itself
/// depends on the instant — a one-shot lookup at the local time is used, so it is exact for
/// naive / UTC / fixed offsets and off by at most the DST step for the ~1 hour/year around an IANA
/// zone's transitions (documented; use a fixed offset when an exact boundary matters).
fn compose(civil: &Civil, unit: TimeUnit, tz: &Tz) -> Option<i128> {
    let local = civil::join_epoch_nanos(
        civil::days_from_civil(civil.year, civil.month, civil.day),
        civil::nanos_of_day_from_hms(civil.hour, civil.minute, civil.second, civil.nanosecond),
    );
    let offset = tz.offset_seconds_at(epoch_seconds_of(local)) as i128;
    TimeUnit::convert(local - offset * NANOS_PER_SEC, TimeUnit::Nanosecond, unit)
}

/// The whole seconds of `epoch_nanos`, clamped to `i64` for the offset lookup.
fn epoch_seconds_of(epoch_nanos: i128) -> i64 {
    epoch_nanos
        .div_euclid(NANOS_PER_SEC)
        .clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

/// Formats an instant as ISO-8601, with `tz`'s offset suffix (`Z` for UTC, none for naive).
fn fmt_instant(
    f: &mut core::fmt::Formatter<'_>,
    value: i128,
    unit: TimeUnit,
    tz: &Tz,
) -> core::fmt::Result {
    let c = decompose(value, unit, tz);
    write!(
        f,
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}",
        FmtYear(c.year),
        c.month,
        c.day,
        c.hour,
        c.minute,
        c.second
    )?;
    let digits = match unit {
        TimeUnit::Millisecond => 3,
        TimeUnit::Microsecond => 6,
        TimeUnit::Nanosecond => 9,
        _ => 0,
    };
    if digits > 0 {
        write!(
            f,
            ".{:0width$}",
            c.nanosecond / 10u32.pow(9 - digits),
            width = digits as usize
        )?;
    }
    if !tz.is_naive() {
        let offset = tz.offset_seconds_at(epoch_seconds_of(
            value.saturating_mul(unit.nanos().unwrap_or(1)),
        ));
        if offset == 0 {
            f.write_str("Z")?;
        } else {
            let (sign, abs) = if offset < 0 {
                ('-', -offset)
            } else {
                ('+', offset)
            };
            write!(f, "{sign}{:02}:{:02}", abs / 3_600, (abs % 3_600) / 60)?;
        }
    }
    Ok(())
}

/// Writes `value`'s low `width` little-endian bytes (two's complement) onto `out`.
fn write_le_width(value: i128, width: usize, out: &mut Vec<u8>) {
    out.extend_from_slice(&value.to_le_bytes()[..width]);
}

/// Sign-extends `width` little-endian bytes into an `i128`.
fn read_le_width(bytes: &[u8], width: usize) -> i128 {
    let mut buf = [0u8; 16];
    buf[..width].copy_from_slice(&bytes[..width]);
    if buf[width - 1] & 0x80 != 0 {
        for byte in &mut buf[width..] {
            *byte = 0xff;
        }
    }
    i128::from_le_bytes(buf)
}

macro_rules! timestamp_type {
    ($Ty:ident, $width:literal, $name:literal, $min:expr, $max:expr) => {
        #[doc = concat!("An **instant** — a count since the epoch in a `TimeUnit`, plus a `Tz` (Arrow-style `", $name, "`).")]
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $Ty {
            value: i128,
            unit: TimeUnit,
            tz: Tz,
        }

        impl core::fmt::Debug for $Ty {
            /// The signature + value, e.g. `ts64[ns, UTC](2024-07-15T12:00:00Z)`.
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}[{}", $name, self.unit.abbreviation())?;
                if !self.tz.is_naive() {
                    write!(f, ", {}", self.tz.name())?;
                }
                write!(f, "]({self})")
            }
        }

        impl $Ty {
            /// The lowest / highest representable epoch count.
            pub const MIN_VALUE: i128 = $min;
            #[allow(missing_docs)]
            pub const MAX_VALUE: i128 = $max;

            /// An instant `value` counts of `unit` since the epoch, in zone `tz`. Errors
            /// [`UnsupportedUnit`](TemporalError::UnsupportedUnit) for a calendar unit or
            /// [`OutOfRange`](TemporalError::OutOfRange) beyond the width.
            pub fn from_epoch(value: i128, unit: TimeUnit, tz: Tz) -> Result<Self, TemporalError> {
                if unit.is_calendar() {
                    return Err(TemporalError::UnsupportedUnit { ty: $name, unit });
                }
                if !($min..=$max).contains(&value) {
                    return Err(TemporalError::OutOfRange { ty: $name });
                }
                Ok(Self { value, unit, tz })
            }

            /// An instant from `nanos` nanoseconds since the epoch, re-expressed in `unit` / `tz`.
            pub fn from_epoch_nanos(
                nanos: i128,
                unit: TimeUnit,
                tz: Tz,
            ) -> Result<Self, TemporalError> {
                let value = TimeUnit::convert(nanos, TimeUnit::Nanosecond, unit)
                    .ok_or(TemporalError::UnsupportedUnit { ty: $name, unit })?;
                Self::from_epoch(value, unit, tz)
            }

            /// An instant from wall-clock components interpreted in `tz` (see [`compose`] for the
            /// DST-boundary caveat). Errors for an invalid date/time or on overflow.
            #[allow(clippy::too_many_arguments)]
            pub fn from_datetime(
                year: i32,
                month: u32,
                day: u32,
                hour: u32,
                minute: u32,
                second: u32,
                nanosecond: u32,
                unit: TimeUnit,
                tz: Tz,
            ) -> Result<Self, TemporalError> {
                let civil = Civil {
                    year,
                    month,
                    day,
                    hour,
                    minute,
                    second,
                    nanosecond,
                };
                if !civil::is_valid(&civil) {
                    return Err(if day == 0 || month == 0 || month > 12 || day > 31 {
                        TemporalError::InvalidDate { year, month, day }
                    } else {
                        TemporalError::InvalidTime {
                            hour,
                            minute,
                            second,
                        }
                    });
                }
                let value = compose(&civil, unit, &tz).ok_or(TemporalError::Overflow {
                    ty: $name,
                    op: "from_datetime",
                })?;
                Self::from_epoch(value, unit, tz)
            }

            /// Parses an instant from a **flexible** set of common formats (ISO, `MM/DD/YYYY HH:MM`,
            /// a date only → midnight, a time only → the epoch date, a trailing `Z`/offset) — see
            /// [`parse`](super::parse). The result is expressed in `unit`; the zone comes from the
            /// string when present, otherwise from the `tz` default (both let a caller default and
            /// cast while parsing). Use [`FromStr`](core::str::FromStr) for strict ISO only.
            pub fn parse_str(text: &str, unit: TimeUnit, tz: Tz) -> Result<Self, TemporalError> {
                let (civil, parsed_tz, _frac) = super::parse::parse_datetime(text)
                    .ok_or(TemporalError::ParseError { ty: $name })?;
                Self::from_datetime(
                    civil.year, civil.month, civil.day, civil.hour, civil.minute, civil.second,
                    civil.nanosecond, unit, parsed_tz.unwrap_or(tz),
                )
            }

            /// The raw epoch count.
            pub const fn epoch_value(&self) -> i128 {
                self.value
            }
            /// The resolution unit.
            pub const fn unit(&self) -> TimeUnit {
                self.unit
            }
            /// The timezone.
            pub const fn tz(&self) -> Tz {
                self.tz
            }
            /// The instant in **nanoseconds** since the epoch (saturating past `i128`).
            pub fn epoch_nanos(&self) -> i128 {
                self.value.saturating_mul(self.unit.nanos().unwrap_or(1))
            }
            /// The whole seconds since the epoch.
            pub fn epoch_seconds(&self) -> i64 {
                epoch_seconds_of(self.epoch_nanos())
            }
            /// The zone's UTC offset (seconds) in effect at this instant.
            pub fn offset_seconds(&self) -> i32 {
                self.tz.offset_seconds_at(self.epoch_seconds())
            }

            /// This instant re-expressed in `unit` (truncating a finer→coarser step). Errors on
            /// overflow or a calendar unit.
            pub fn to_unit(&self, unit: TimeUnit) -> Result<Self, TemporalError> {
                let value = TimeUnit::convert(self.value, self.unit, unit)
                    .ok_or(TemporalError::UnsupportedUnit { ty: $name, unit })?;
                Self::from_epoch(value, unit, self.tz)
            }

            /// The **same instant** displayed in a different `tz` (the stored UTC count is
            /// unchanged; only the wall-clock view moves).
            pub fn with_timezone(&self, tz: Tz) -> Self {
                Self {
                    value: self.value,
                    unit: self.unit,
                    tz,
                }
            }

            /// The wall-clock `(year, month, day, hour, minute, second, nanosecond)` in this
            /// instant's zone.
            pub fn to_datetime(&self) -> (i32, u32, u32, u32, u32, u32, u32) {
                let c = decompose(self.value, self.unit, &self.tz);
                (
                    c.year,
                    c.month,
                    c.day,
                    c.hour,
                    c.minute,
                    c.second,
                    c.nanosecond,
                )
            }
            /// The calendar year in this instant's zone.
            pub fn year(&self) -> i32 {
                self.to_datetime().0
            }
            /// The month (`1..=12`).
            pub fn month(&self) -> u32 {
                self.to_datetime().1
            }
            /// The day of the month.
            pub fn day(&self) -> u32 {
                self.to_datetime().2
            }
            /// The hour (`0..=23`).
            pub fn hour(&self) -> u32 {
                self.to_datetime().3
            }
            /// The minute.
            pub fn minute(&self) -> u32 {
                self.to_datetime().4
            }
            /// The second.
            pub fn second(&self) -> u32 {
                self.to_datetime().5
            }
            /// The sub-second nanoseconds.
            pub fn nanosecond(&self) -> u32 {
                self.to_datetime().6
            }

            /// The calendar **date** in this instant's zone.
            pub fn to_date(&self) -> Result<Date32, TemporalError> {
                let (year, month, day, ..) = self.to_datetime();
                Date32::from_ymd(year, month, day)
            }
            /// The **time of day** (nanosecond resolution) in this instant's zone.
            pub fn to_time(&self) -> Result<Time64, TemporalError> {
                let (.., hour, minute, second, nanosecond) = self.to_datetime();
                Time64::from_hms_nano(hour, minute, second, nanosecond)
            }
            /// The elapsed **span** since the epoch, as a [`Duration64`] in this instant's unit.
            /// Errors [`OutOfRange`](TemporalError::OutOfRange) if the count exceeds the `i64` a
            /// `Duration64` holds (only reachable from a wide [`Ts96`]).
            pub fn to_duration(&self) -> Result<Duration64, TemporalError> {
                let value = i64::try_from(self.value)
                    .map_err(|_| TemporalError::OutOfRange { ty: "duration64" })?;
                Duration64::new(value, self.unit)
            }

            /// The value's bytes: the count (`$width` LE, two's complement), a unit tag, then a
            /// length-prefixed timezone name.
            pub fn serialize_bytes(&self) -> Vec<u8> {
                let mut bytes = Vec::with_capacity($width + 3);
                write_le_width(self.value, $width, &mut bytes);
                bytes.push(unit_tag(self.unit));
                let name = self.tz.name();
                bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
                bytes.extend_from_slice(name.as_bytes());
                bytes
            }
            /// Reconstructs an instant from [`serialize_bytes`](Self::serialize_bytes).
            pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, TemporalError> {
                let err = || TemporalError::ParseError { ty: $name };
                if bytes.len() < $width + 3 {
                    return Err(err());
                }
                let value = read_le_width(&bytes[..$width], $width);
                let unit = unit_from_tag(bytes[$width]).ok_or_else(err)?;
                let name_len = u16::from_le_bytes([bytes[$width + 1], bytes[$width + 2]]) as usize;
                let name = bytes
                    .get($width + 3..$width + 3 + name_len)
                    .and_then(|s| core::str::from_utf8(s).ok())
                    .ok_or_else(err)?;
                let tz = Tz::parse(name).ok_or_else(err)?;
                Self::from_epoch(value, unit, tz)
            }
        }

        impl Temporal for $Ty {
            fn time_unit(&self) -> TimeUnit {
                self.unit
            }
            fn timezone(&self) -> Tz {
                self.tz
            }
        }

        // Ordering is by the instant (then a stable structural tiebreak), consistent with the
        // structural equality — same value + unit + zone.
        impl Ord for $Ty {
            fn cmp(&self, other: &Self) -> core::cmp::Ordering {
                (
                    self.epoch_nanos(),
                    self.value,
                    self.unit,
                    self.tz.sort_key(),
                )
                    .cmp(&(
                        other.epoch_nanos(),
                        other.value,
                        other.unit,
                        other.tz.sort_key(),
                    ))
            }
        }
        impl PartialOrd for $Ty {
            fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }

        impl core::fmt::Display for $Ty {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                fmt_instant(f, self.value, self.unit, &self.tz)
            }
        }

        impl core::str::FromStr for $Ty {
            type Err = TemporalError;
            fn from_str(text: &str) -> Result<Self, Self::Err> {
                let (civil, unit, tz) =
                    parse_datetime(text.trim()).ok_or(TemporalError::ParseError { ty: $name })?;
                Self::from_datetime(
                    civil.year,
                    civil.month,
                    civil.day,
                    civil.hour,
                    civil.minute,
                    civil.second,
                    civil.nanosecond,
                    unit,
                    tz,
                )
            }
        }
    };
}

timestamp_type!(Ts32, 4, "ts32", i32::MIN as i128, i32::MAX as i128);
timestamp_type!(Ts64, 8, "ts64", i64::MIN as i128, i64::MAX as i128);
timestamp_type!(Ts96, 12, "ts96", -(1i128 << 95), (1i128 << 95) - 1);

/// Cross-width conversions (widening is always in range; narrowing may overflow).
impl Ts32 {
    /// Widen to a [`Ts64`] (always in range).
    pub fn to_ts64(&self) -> Ts64 {
        Ts64 {
            value: self.value,
            unit: self.unit,
            tz: self.tz,
        }
    }
    /// Widen to a [`Ts96`] (always in range).
    pub fn to_ts96(&self) -> Ts96 {
        Ts96 {
            value: self.value,
            unit: self.unit,
            tz: self.tz,
        }
    }
}
impl Ts64 {
    /// Narrow to a [`Ts32`], or [`OutOfRange`](TemporalError::OutOfRange) if it does not fit.
    pub fn to_ts32(&self) -> Result<Ts32, TemporalError> {
        Ts32::from_epoch(self.value, self.unit, self.tz)
    }
    /// Widen to a [`Ts96`].
    pub fn to_ts96(&self) -> Ts96 {
        Ts96 {
            value: self.value,
            unit: self.unit,
            tz: self.tz,
        }
    }
}
impl Ts96 {
    /// Narrow to a [`Ts32`], or [`OutOfRange`](TemporalError::OutOfRange) if it does not fit.
    pub fn to_ts32(&self) -> Result<Ts32, TemporalError> {
        Ts32::from_epoch(self.value, self.unit, self.tz)
    }
    /// Narrow to a [`Ts64`], or [`OutOfRange`](TemporalError::OutOfRange) if it does not fit.
    pub fn to_ts64(&self) -> Result<Ts64, TemporalError> {
        Ts64::from_epoch(self.value, self.unit, self.tz)
    }
}

/// Parses an ISO-8601 datetime `YYYY-MM-DDThh:mm:ss[.frac][Z|±hh:mm]` into its civil components,
/// the finest unit the fraction needs, and the timezone the offset implies.
fn parse_datetime(text: &str) -> Option<(Civil, TimeUnit, Tz)> {
    let (date_part, rest) = text.split_once(['T', 't', ' '])?;
    let (year, month, day) = super::date::parse_ymd(date_part)?;

    // Split off a trailing zone: `Z`, or a signed offset `±hh:mm` / `±hhmm` / `±hh`.
    let (time_part, tz) = if let Some(body) = rest.strip_suffix(['Z', 'z']) {
        (body, Tz::UTC)
    } else if let Some(sign_pos) = rest.rfind(['+', '-']).filter(|&p| p > 0) {
        let offset = Tz::parse(&rest[sign_pos..])?;
        (&rest[..sign_pos], offset)
    } else {
        (rest, Tz::NAIVE)
    };

    let (clock, frac) = time_part.split_once('.').unwrap_or((time_part, ""));
    let mut parts = clock.split(':');
    let hour: u32 = parts.next()?.parse().ok()?;
    let minute: u32 = parts.next()?.parse().ok()?;
    let second: u32 = parts.next().unwrap_or("0").parse().ok()?;
    if parts.next().is_some() || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut nanosecond = 0u32;
    for (index, byte) in frac.bytes().take(9).enumerate() {
        nanosecond += (byte - b'0') as u32 * 10u32.pow(8 - index as u32);
    }
    let unit = match frac.len() {
        0 => TimeUnit::Second,
        1..=3 => TimeUnit::Millisecond,
        4..=6 => TimeUnit::Microsecond,
        _ => TimeUnit::Nanosecond,
    };
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
        unit,
        tz,
    ))
}
