//! [`Duration32`] (`i32`) and [`Duration64`] (`i64`) — an **elapsed span**: a signed count in a
//! [`TimeUnit`], with no date, time-of-day, or timezone. Arrow `Duration` (and a narrow `i32` form).

use super::{Date32, Temporal, TemporalError, Time64, TimeUnit, Ts64, Tz};

macro_rules! duration_type {
    ($Ty:ident, $int:ty, $width:literal, $name:literal) => {
        #[doc = concat!("A signed **elapsed span** as a count of a `TimeUnit`, in an `", stringify!($int), "` (Arrow-style `", $name, "`).")]
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $Ty {
            value: $int,
            unit: TimeUnit,
        }

        impl core::fmt::Debug for $Ty {
            /// The signature + value, e.g. `duration64[ms](1500ms)`.
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}[{}]({self})", $name, self.unit.abbreviation())
            }
        }

        impl $Ty {
            /// A span of `value` counts of `unit`, or [`UnsupportedUnit`](TemporalError::UnsupportedUnit)
            /// for a calendar unit (`Month` / `Year`, which have no fixed length).
            pub fn new(value: $int, unit: TimeUnit) -> Result<Self, TemporalError> {
                if unit.is_calendar() {
                    return Err(TemporalError::UnsupportedUnit { ty: $name, unit });
                }
                Ok(Self { value, unit })
            }

            /// A span of `value` **seconds**.
            pub fn seconds(value: $int) -> Self {
                Self {
                    value,
                    unit: TimeUnit::Second,
                }
            }
            /// A span of `value` **milliseconds**.
            pub fn milliseconds(value: $int) -> Self {
                Self {
                    value,
                    unit: TimeUnit::Millisecond,
                }
            }
            /// A span of `value` **nanoseconds**.
            pub fn nanoseconds(value: $int) -> Self {
                Self {
                    value,
                    unit: TimeUnit::Nanosecond,
                }
            }

            /// The raw count.
            pub const fn value(&self) -> $int {
                self.value
            }
            /// The resolution unit.
            pub const fn unit(&self) -> TimeUnit {
                self.unit
            }
            /// The span in **nanoseconds** (saturating), or `None` for a calendar unit.
            pub fn to_nanos(&self) -> Option<i128> {
                self.unit.to_nanos(self.value as i128)
            }
            /// Whether the span is negative.
            pub fn is_negative(&self) -> bool {
                self.value < 0
            }
            /// Whether the span is zero.
            pub fn is_zero(&self) -> bool {
                self.value == 0
            }

            /// The instant at `epoch + self`, as a [`Ts64`] in this span's unit and zone `tz`.
            pub fn to_timestamp(&self, tz: Tz) -> Result<Ts64, TemporalError> {
                Ts64::from_epoch(self.value as i128, self.unit, tz)
            }

            /// This span reduced to a **time of day** (nanoseconds modulo 24h), as a [`Time64`].
            /// Errors on overflow computing the nanosecond span.
            pub fn to_time(&self) -> Result<Time64, TemporalError> {
                let day_nanos = TimeUnit::Day.nanos().expect("Day is fixed");
                let nanos = self.to_nanos().ok_or(TemporalError::UnsupportedUnit {
                    ty: $name,
                    unit: self.unit,
                })?;
                Time64::new(nanos.rem_euclid(day_nanos) as i64, TimeUnit::Nanosecond)
            }

            /// This span as **days since the epoch** (truncating to whole days), as a [`Date32`].
            /// Errors [`OutOfRange`](TemporalError::OutOfRange) beyond the `i32` day range.
            pub fn to_date(&self) -> Result<Date32, TemporalError> {
                let day_nanos = TimeUnit::Day.nanos().expect("Day is fixed");
                let nanos = self.to_nanos().ok_or(TemporalError::UnsupportedUnit {
                    ty: $name,
                    unit: self.unit,
                })?;
                i32::try_from(nanos.div_euclid(day_nanos))
                    .map(Date32::from_days)
                    .map_err(|_| TemporalError::OutOfRange { ty: "date32" })
            }

            /// This span re-expressed in `unit`, or an error on overflow / a calendar unit.
            pub fn to_unit(&self, unit: TimeUnit) -> Result<Self, TemporalError> {
                let converted = TimeUnit::convert(self.value as i128, self.unit, unit)
                    .ok_or(TemporalError::UnsupportedUnit { ty: $name, unit })?;
                let value = <$int>::try_from(converted)
                    .map_err(|_| TemporalError::OutOfRange { ty: $name })?;
                Self::new(value, unit)
            }

            /// `self + other`, aligning to the finer of the two units. Errors on overflow.
            pub fn checked_add(&self, other: &Self) -> Result<Self, TemporalError> {
                self.combine(other, |a, b| a.checked_add(b))
            }
            /// `self - other`, aligning to the finer of the two units. Errors on overflow.
            pub fn checked_sub(&self, other: &Self) -> Result<Self, TemporalError> {
                self.combine(other, |a, b| a.checked_sub(b))
            }
            /// `-self`, or [`Overflow`](TemporalError::Overflow) at the minimum.
            pub fn checked_neg(&self) -> Result<Self, TemporalError> {
                self.value
                    .checked_neg()
                    .map(|value| Self {
                        value,
                        unit: self.unit,
                    })
                    .ok_or(TemporalError::Overflow {
                        ty: $name,
                        op: "neg",
                    })
            }

            fn combine(
                &self,
                other: &Self,
                op: fn($int, $int) -> Option<$int>,
            ) -> Result<Self, TemporalError> {
                let unit = self.unit.min(other.unit); // the finer resolution
                let (a, b) = (self.to_unit(unit)?, other.to_unit(unit)?);
                op(a.value, b.value)
                    .map(|value| Self { value, unit })
                    .ok_or(TemporalError::Overflow {
                        ty: $name,
                        op: "add",
                    })
            }

            /// Parses a **flexible** duration string, keeping the input's natural granularity: a
            /// single `<count><unit>` (`"90s"`, `"-1500ms"`, `"5 min"`), a compound run
            /// (`"1h30m15s"`, `"2d 3h"`, `"1 hour 30 minutes"`), a clock (`"1:30:00"`,
            /// `"30:00.5"`), or ISO-8601 (`"PT1H30M"`, `"P1DT2H"`, `"P2W"`). A leading `-`/`+`
            /// negates the whole span; the result unit is the coarsest that stays exact. Errors on
            /// a calendar unit (`mo`/`y` — no fixed length), an unparseable string, or overflow.
            ///
            /// ```
            #[doc = concat!("use yggdryl_core::io::fixed::temporal::{", stringify!($Ty), ", TimeUnit};")]
            #[doc = concat!("let d = ", stringify!($Ty), "::parse_str(\"1h30m\").unwrap();")]
            /// assert_eq!((d.value(), d.unit()), (90, TimeUnit::Minute));
            #[doc = concat!("assert_eq!(", stringify!($Ty), "::parse_str(\"PT1.5S\").unwrap().value(), 1500);  // ms")]
            /// ```
            pub fn parse_str(text: &str) -> Result<Self, TemporalError> {
                let (nanos, unit) = super::parse::parse_duration(text)
                    .ok_or(TemporalError::ParseError { ty: $name })?;
                let count = unit
                    .from_nanos(nanos)
                    .ok_or(TemporalError::ParseError { ty: $name })?;
                let value =
                    <$int>::try_from(count).map_err(|_| TemporalError::OutOfRange { ty: $name })?;
                Self::new(value, unit)
            }

            /// The value's little-endian bytes: the count then a one-byte unit tag.
            pub fn serialize_bytes(&self) -> Vec<u8> {
                let mut bytes = self.value.to_le_bytes().to_vec();
                bytes.push(super::time::unit_tag(self.unit));
                bytes
            }
            /// Reconstructs a span from [`serialize_bytes`](Self::serialize_bytes).
            pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, TemporalError> {
                let array: [u8; $width] = bytes
                    .get(..$width)
                    .and_then(|s| s.try_into().ok())
                    .ok_or(TemporalError::ParseError { ty: $name })?;
                let unit = bytes
                    .get($width)
                    .and_then(|&tag| super::time::unit_from_tag(tag))
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

        // Ordering is by the elapsed span (then unit), consistent with the structural equality.
        impl Ord for $Ty {
            fn cmp(&self, other: &Self) -> core::cmp::Ordering {
                let key = |d: &Self| {
                    (
                        d.to_nanos().unwrap_or(d.value as i128),
                        d.value as i128,
                        d.unit,
                    )
                };
                key(self).cmp(&key(other))
            }
        }
        impl PartialOrd for $Ty {
            fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }

        impl core::fmt::Display for $Ty {
            /// A count with its unit abbreviation, e.g. `"90s"`, `"-1500ms"`.
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}{}", self.value, self.unit.abbreviation())
            }
        }

        impl core::str::FromStr for $Ty {
            type Err = TemporalError;
            /// The flexible [`parse_str`](Self::parse_str) — single-unit, compound, clock, or
            /// ISO-8601 (e.g. `"90s"`, `"-1500ms"`, `"1h30m"`, `"PT1H30M"`).
            fn from_str(text: &str) -> Result<Self, Self::Err> {
                Self::parse_str(text)
            }
        }
    };
}

duration_type!(Duration32, i32, 4, "duration32");
duration_type!(Duration64, i64, 8, "duration64");

/// Cross-width conversions (widening is always in range; narrowing may overflow).
impl Duration32 {
    /// Widen to a [`Duration64`] (always in range).
    pub fn to_duration64(&self) -> Duration64 {
        Duration64::new(self.value as i64, self.unit)
            .expect("a valid span stays valid when widened")
    }
}
impl Duration64 {
    /// Narrow to a [`Duration32`], or [`OutOfRange`](TemporalError::OutOfRange) if it does not fit.
    pub fn to_duration32(&self) -> Result<Duration32, TemporalError> {
        let value = i32::try_from(self.value)
            .map_err(|_| TemporalError::OutOfRange { ty: "duration32" })?;
        Duration32::new(value, self.unit)
    }
}
