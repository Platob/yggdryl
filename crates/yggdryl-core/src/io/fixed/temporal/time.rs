//! [`Time32`] (seconds / milliseconds of day, `i32`) and [`Time64`] (microseconds / nanoseconds of
//! day, `i64`) — a wall-clock time of day, naive (no date, no timezone). Arrow `Time32` / `Time64`.

use super::{civil, Duration64, Temporal, TemporalError, TimeUnit, Ts64, Tz};

/// The nanoseconds-of-day just past the end of a day (`24:00:00`), the exclusive upper bound.
const DAY_NANOS: i64 = 86_400 * 1_000_000_000;

/// Formats a time of day from its nanosecond-of-day at `unit`'s sub-second precision.
fn fmt_time(
    f: &mut core::fmt::Formatter<'_>,
    nanos_of_day: i64,
    unit: TimeUnit,
) -> core::fmt::Result {
    let (hour, minute, second, nanosecond) = civil::hms_from_nanos_of_day(nanos_of_day);
    write!(f, "{hour:02}:{minute:02}:{second:02}")?;
    let digits = match unit {
        TimeUnit::Millisecond => 3,
        TimeUnit::Microsecond => 6,
        TimeUnit::Nanosecond => 9,
        _ => 0,
    };
    if digits > 0 {
        let frac = nanosecond / 10u32.pow(9 - digits);
        write!(f, ".{frac:0width$}", width = digits as usize)?;
    }
    Ok(())
}

macro_rules! time_type {
    ($Ty:ident, $int:ty, $width:literal, $name:literal, $($unit:ident),+) => {
        #[doc = concat!("A wall-clock **time of day** as a count of its unit since midnight, in an `",
            stringify!($int), "` (Arrow `", $name, "`). Naive; supported units: ", $(stringify!($unit), " "),+, ".")]
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $Ty {
            value: $int,
            unit: TimeUnit,
        }

        impl core::fmt::Debug for $Ty {
            /// The signature + value, e.g. `time64[ns](01:02:03.456000000)`.
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}[{}]({self})", $name, self.unit.abbreviation())
            }
        }

        impl $Ty {
            /// A time from a raw `value` count in `unit`, or an error if `unit` is unsupported or
            /// `value` is outside a single day.
            pub fn new(value: $int, unit: TimeUnit) -> Result<Self, TemporalError> {
                if !matches!(unit, $(TimeUnit::$unit)|+) {
                    return Err(TemporalError::UnsupportedUnit { ty: $name, unit });
                }
                let per_day = DAY_NANOS / unit.nanos().expect("supported units are fixed") as i64;
                if value < 0 || value as i64 >= per_day {
                    return Err(TemporalError::OutOfRange { ty: $name });
                }
                Ok(Self { value, unit })
            }

            /// The raw count of [`unit`](Self::unit) since midnight.
            pub const fn value(&self) -> $int {
                self.value
            }
            /// The resolution unit.
            pub const fn unit(&self) -> TimeUnit {
                self.unit
            }
            /// The nanosecond-of-day (`0..86_400×10⁹`).
            pub fn nanos_of_day(&self) -> i64 {
                self.value as i64 * self.unit.nanos().expect("supported units are fixed") as i64
            }
            /// The `(hour, minute, second, nanosecond)`.
            pub fn to_hms(&self) -> (u32, u32, u32, u32) {
                civil::hms_from_nanos_of_day(self.nanos_of_day())
            }
            /// The hour (`0..=23`).
            pub fn hour(&self) -> u32 {
                self.to_hms().0
            }
            /// The minute (`0..=59`).
            pub fn minute(&self) -> u32 {
                self.to_hms().1
            }
            /// The second (`0..=59`).
            pub fn second(&self) -> u32 {
                self.to_hms().2
            }
            /// The sub-second nanoseconds (`0..10⁹`).
            pub fn nanosecond(&self) -> u32 {
                self.to_hms().3
            }

            /// This time-of-day as an elapsed **span** since midnight, as a [`Duration64`] in this
            /// time's unit.
            pub fn to_duration(&self) -> Duration64 {
                Duration64::new(self.value as i64, self.unit)
                    .expect("time units are supported duration spans")
            }

            /// This time-of-day on the **epoch date** (`1970-01-01`), as a [`Ts64`] in `unit` and
            /// zone `tz`. Errors on overflow.
            pub fn to_timestamp(&self, unit: TimeUnit, tz: Tz) -> Result<Ts64, TemporalError> {
                Ts64::from_epoch_nanos(self.nanos_of_day() as i128, unit, tz)
            }

            /// This time re-expressed in `unit`, or an error if `unit` is unsupported for this type
            /// or the conversion is lossy-truncating past its range.
            pub fn to_unit(&self, unit: TimeUnit) -> Result<Self, TemporalError> {
                let converted = TimeUnit::convert(self.value as i128, self.unit, unit)
                    .ok_or(TemporalError::Overflow { ty: $name, op: "to_unit" })?;
                let value = <$int>::try_from(converted).map_err(|_| TemporalError::OutOfRange { ty: $name })?;
                Self::new(value, unit)
            }

            /// The value's little-endian bytes: the count then a one-byte unit tag.
            pub fn serialize_bytes(&self) -> Vec<u8> {
                let mut bytes = self.value.to_le_bytes().to_vec();
                bytes.push(unit_tag(self.unit));
                bytes
            }
            /// Reconstructs a time from [`serialize_bytes`](Self::serialize_bytes).
            pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, TemporalError> {
                let array: [u8; $width] = bytes
                    .get(..$width)
                    .and_then(|s| s.try_into().ok())
                    .ok_or(TemporalError::ParseError { ty: $name })?;
                let unit = bytes
                    .get($width)
                    .and_then(|&tag| unit_from_tag(tag))
                    .ok_or(TemporalError::ParseError { ty: $name })?;
                Self::new(<$int>::from_le_bytes(array), unit)
            }
        }

        impl Temporal for $Ty {
            fn time_unit(&self) -> TimeUnit {
                self.unit
            }
            fn timezone(&self) -> Tz {
                Tz::NAIVE
            }
        }

        // Ordering is by the instant-of-day (then unit), so `12:00:00` and `12:00:00.000` sort
        // together while staying distinct values.
        impl Ord for $Ty {
            fn cmp(&self, other: &Self) -> core::cmp::Ordering {
                (self.nanos_of_day(), self.unit).cmp(&(other.nanos_of_day(), other.unit))
            }
        }
        impl PartialOrd for $Ty {
            fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }

        impl core::fmt::Display for $Ty {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                fmt_time(f, self.nanos_of_day(), self.unit)
            }
        }

        impl core::str::FromStr for $Ty {
            type Err = TemporalError;
            fn from_str(text: &str) -> Result<Self, Self::Err> {
                let (nanos_of_day, frac_digits) =
                    parse_time(text.trim()).ok_or(TemporalError::ParseError { ty: $name })?;
                let unit = pick_time_unit::<$int>(frac_digits);
                let value = (nanos_of_day / unit.nanos().unwrap() as i64) as $int;
                Self::new(value, unit)
            }
        }
    };
}

time_type!(Time32, i32, 4, "time32", Second, Millisecond);
time_type!(Time64, i64, 8, "time64", Microsecond, Nanosecond);

impl Time32 {
    /// A time from `hour:minute:second` (second resolution).
    pub fn from_hms(hour: u32, minute: u32, second: u32) -> Result<Self, TemporalError> {
        validate_hms(hour, minute, second)?;
        Self::new(
            (hour * 3_600 + minute * 60 + second) as i32,
            TimeUnit::Second,
        )
    }
    /// This time as a [`Time64`] in `unit` (`Microsecond` / `Nanosecond`).
    pub fn to_time64(&self, unit: TimeUnit) -> Result<Time64, TemporalError> {
        Time64::new(self.nanos_of_day() / unit.nanos().unwrap_or(1) as i64, unit)
    }

    /// Parses a time from a **flexible** set of common formats (24-hour `13:45:30`, 12-hour
    /// `1:45 PM`, optional fraction) — see [`parse`](super::parse). The unit is milliseconds when
    /// a fraction is present, else seconds.
    pub fn parse_str(text: &str) -> Result<Self, TemporalError> {
        let (nanos_of_day, frac) =
            super::parse::parse_time(text).ok_or(TemporalError::ParseError { ty: "time32" })?;
        let unit = if frac > 0 {
            TimeUnit::Millisecond
        } else {
            TimeUnit::Second
        };
        Self::new((nanos_of_day / unit.nanos().unwrap() as i64) as i32, unit)
    }
}

impl Time64 {
    /// A time from `hour:minute:second` and a sub-second `nanosecond`, at nanosecond resolution.
    pub fn from_hms_nano(
        hour: u32,
        minute: u32,
        second: u32,
        nanosecond: u32,
    ) -> Result<Self, TemporalError> {
        validate_hms(hour, minute, second)?;
        Self::new(
            civil::nanos_of_day_from_hms(hour, minute, second, nanosecond),
            TimeUnit::Nanosecond,
        )
    }
    /// This time as a [`Time32`] in `unit` (`Second` / `Millisecond`), truncating.
    pub fn to_time32(&self, unit: TimeUnit) -> Result<Time32, TemporalError> {
        Time32::new(
            (self.nanos_of_day() / unit.nanos().unwrap_or(1) as i64) as i32,
            unit,
        )
    }

    /// Parses a time from a **flexible** set of common formats (see [`Time32::parse_str`]). The unit is
    /// nanoseconds when the fraction exceeds 6 digits, else microseconds.
    pub fn parse_str(text: &str) -> Result<Self, TemporalError> {
        let (nanos_of_day, frac) =
            super::parse::parse_time(text).ok_or(TemporalError::ParseError { ty: "time64" })?;
        let unit = if frac > 6 {
            TimeUnit::Nanosecond
        } else {
            TimeUnit::Microsecond
        };
        Self::new(nanos_of_day / unit.nanos().unwrap() as i64, unit)
    }
}

fn validate_hms(hour: u32, minute: u32, second: u32) -> Result<(), TemporalError> {
    if hour < 24 && minute < 60 && second < 60 {
        Ok(())
    } else {
        Err(TemporalError::InvalidTime {
            hour,
            minute,
            second,
        })
    }
}

/// The one-byte tag for a time unit in the codec.
pub(super) fn unit_tag(unit: TimeUnit) -> u8 {
    unit as u8
}
/// The time unit for a codec tag.
pub(super) fn unit_from_tag(tag: u8) -> Option<TimeUnit> {
    TimeUnit::ALL.get(tag as usize).copied()
}

/// Picks the finest supported unit for `Int` given a fraction of `frac_digits` decimal places.
fn pick_time_unit<Int>(frac_digits: usize) -> TimeUnit {
    // The macro instantiates this per int type; `Int` is only a witness for which family we're in.
    if core::mem::size_of::<Int>() <= 4 {
        if frac_digits == 0 {
            TimeUnit::Second
        } else {
            TimeUnit::Millisecond
        }
    } else if frac_digits <= 6 {
        TimeUnit::Microsecond
    } else {
        TimeUnit::Nanosecond
    }
}

/// Parses `HH:MM:SS[.frac]` into `(nanos_of_day, frac_digit_count)`.
fn parse_time(text: &str) -> Option<(i64, usize)> {
    let (clock, frac) = match text.split_once('.') {
        Some((clock, frac)) => (clock, frac),
        None => (text, ""),
    };
    let mut parts = clock.split(':');
    let hour: u32 = parts.next()?.parse().ok()?;
    let minute: u32 = parts.next()?.parse().ok()?;
    let second: u32 = parts.next().unwrap_or("0").parse().ok()?;
    if parts.next().is_some() || hour >= 24 || minute >= 60 || second >= 60 {
        return None;
    }
    if !frac.is_empty() && !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Pad / truncate the fraction to 9 digits of nanoseconds.
    let mut nanos_frac = 0i64;
    for (index, byte) in frac.bytes().take(9).enumerate() {
        nanos_frac += (byte - b'0') as i64 * 10i64.pow(8 - index as u32);
    }
    let nanos_of_day = civil::nanos_of_day_from_hms(hour, minute, second, 0) + nanos_frac;
    Some((nanos_of_day, frac.len()))
}
