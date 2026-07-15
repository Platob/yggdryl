//! The `yggdryl.temporal` namespace — the temporal value types (`Date32`/`Date64`, `Time32`/
//! `Time64`, `Ts32`/`Ts64`/`Ts96`, `Duration32`/`Duration64`) and the [`Tz`]
//! timezone, mirroring `yggdryl_core::io::fixed::temporal`.
//!
//! Resolutions (**time units**) and **timezones** cross as strings — `"ns"`/`"ms"`/`"s"` and
//! `"UTC"`/`"Europe/Paris"`/`"+02:00"`/`""` (naive). Component tuples cross as `number[]`, and a
//! timestamp's epoch count as a `bigint` (it can exceed 64 bits for `Ts96`).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::{BigInt, Buffer};
use napi_derive::napi;

use yggdryl_core::io::fixed::temporal as core;
use yggdryl_core::io::fixed::temporal::Temporal as _;

fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

fn parse_unit(text: &str) -> napi::Result<core::TimeUnit> {
    core::TimeUnit::parse(text).ok_or_else(|| to_error(format!("unknown time unit: {text:?}")))
}

fn parse_tz(text: &str) -> napi::Result<core::Tz> {
    core::Tz::parse(text).ok_or_else(|| to_error(format!("unknown timezone: {text:?}")))
}

fn java_hash<T: Hash>(value: &T) -> i32 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    let hash = hasher.finish();
    (hash as u32 ^ (hash >> 32) as u32) as i32
}

/// A **timezone** — `naive`, `UTC`, a fixed offset, or a DST-aware IANA zone.
#[napi(namespace = "temporal")]
pub struct Tz {
    pub(crate) inner: core::Tz,
}

#[napi(namespace = "temporal")]
impl Tz {
    #[napi(factory)]
    pub fn naive() -> Self {
        Self {
            inner: core::Tz::NAIVE,
        }
    }
    #[napi(factory)]
    pub fn utc() -> Self {
        Self {
            inner: core::Tz::UTC,
        }
    }
    #[napi(factory)]
    pub fn fixed_offset(seconds: i32) -> Self {
        Self {
            inner: core::Tz::fixed_offset(seconds),
        }
    }
    #[napi(factory)]
    pub fn iana(name: String) -> napi::Result<Self> {
        core::Tz::iana(&name)
            .map(|inner| Self { inner })
            .ok_or_else(|| to_error(format!("unknown IANA zone: {name:?}")))
    }
    #[napi(factory)]
    pub fn parse(text: String) -> napi::Result<Self> {
        Ok(Self {
            inner: parse_tz(&text)?,
        })
    }

    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name()
    }
    #[napi]
    pub fn offset_seconds_at(&self, epoch_seconds: i64) -> i32 {
        self.inner.offset_seconds_at(epoch_seconds)
    }
    #[napi]
    pub fn is_naive(&self) -> bool {
        self.inner.is_naive()
    }
    #[napi]
    pub fn is_utc(&self) -> bool {
        self.inner.is_utc()
    }
    #[napi]
    pub fn is_iana(&self) -> bool {
        self.inner.is_iana()
    }
    #[napi]
    pub fn equals(&self, other: &Tz) -> bool {
        self.inner == other.inner
    }
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        self.inner.to_string()
    }
}

/// The shared value-type surface (codec, unit, compare, hash, toString) for a temporal core type.
macro_rules! temporal_common {
    ($Ty:ident, $core:ty) => {
        #[napi(namespace = "temporal")]
        impl $Ty {
            #[napi]
            pub fn serialize_bytes(&self) -> Buffer {
                self.inner.serialize_bytes().to_vec().into()
            }
            #[napi(factory)]
            pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
                <$core>::deserialize_bytes(&bytes)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }
            /// The resolution unit as a string.
            #[napi(getter)]
            pub fn unit(&self) -> String {
                self.inner.time_unit().abbreviation().to_string()
            }
            #[napi]
            pub fn equals(&self, other: &$Ty) -> bool {
                self.inner == other.inner
            }
            /// `-1` / `0` / `1` for `self` less than / equal / greater than `other`.
            #[napi]
            pub fn compare_to(&self, other: &$Ty) -> i32 {
                self.inner.cmp(&other.inner) as i32
            }
            #[napi]
            pub fn hash_code(&self) -> i32 {
                java_hash(&self.inner)
            }
            #[napi]
            pub fn copy(&self) -> Self {
                Self { inner: self.inner }
            }
            /// The type signature with its inner params and value, e.g.
            /// `ts64[ns, UTC](2024-07-15T12:00:00Z)`.
            #[napi]
            pub fn signature(&self) -> String {
                format!("{:?}", self.inner)
            }
            #[napi(js_name = "toString")]
            pub fn text(&self) -> String {
                self.inner.to_string()
            }
        }
    };
}

// ---- Date -----------------------------------------------------------------------------

macro_rules! date_type {
    ($Ty:ident, $core:ty) => {
        #[doc = concat!("A calendar date (", stringify!($Ty), "), naive.")]
        #[napi(namespace = "temporal")]
        pub struct $Ty {
            pub(crate) inner: $core,
        }

        #[napi(namespace = "temporal")]
        impl $Ty {
            #[napi(factory)]
            pub fn from_ymd(year: i32, month: u32, day: u32) -> napi::Result<Self> {
                <$core>::from_ymd(year, month, day)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }
            #[napi(factory)]
            pub fn from_days(days: i64) -> Self {
                Self {
                    inner: <$core>::from_days(days as _),
                }
            }
            #[napi(factory)]
            pub fn from_string(text: String) -> napi::Result<Self> {
                text.parse::<$core>()
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }
            /// The `[year, month, day]`.
            #[napi]
            pub fn to_ymd(&self) -> Vec<i32> {
                let (y, m, d) = self.inner.to_ymd();
                vec![y, m as i32, d as i32]
            }
            #[napi(getter)]
            pub fn days(&self) -> i64 {
                self.inner.days() as i64
            }
            #[napi(getter)]
            pub fn year(&self) -> i32 {
                self.inner.year()
            }
            #[napi(getter)]
            pub fn month(&self) -> u32 {
                self.inner.month()
            }
            #[napi(getter)]
            pub fn day(&self) -> u32 {
                self.inner.day()
            }
            #[napi]
            pub fn weekday(&self) -> u32 {
                self.inner.weekday()
            }
            #[napi]
            pub fn is_leap_year(&self) -> bool {
                self.inner.is_leap_year()
            }
            /// This date at midnight, as a `Ts64` in `unit` (default `"s"`) and zone `tz`.
            #[napi]
            pub fn at_midnight(
                &self,
                unit: Option<String>,
                tz: Option<String>,
            ) -> napi::Result<Ts64> {
                self.inner
                    .at_midnight(
                        parse_unit(&unit.unwrap_or_else(|| "s".to_string()))?,
                        parse_tz(&tz.unwrap_or_default())?,
                    )
                    .map(|inner| Ts64 { inner })
                    .map_err(to_error)
            }
            /// This date at the wall-clock `time`, as a `Ts64` in `unit` (default `"s"`), zone `tz`.
            #[napi]
            pub fn at_time(
                &self,
                time: &Time64,
                unit: Option<String>,
                tz: Option<String>,
            ) -> napi::Result<Ts64> {
                self.inner
                    .at_time(
                        &time.inner,
                        parse_unit(&unit.unwrap_or_else(|| "s".to_string()))?,
                        parse_tz(&tz.unwrap_or_default())?,
                    )
                    .map(|inner| Ts64 { inner })
                    .map_err(to_error)
            }
            /// The elapsed span from the epoch to this date, as a `Duration64` of days.
            #[napi]
            pub fn to_duration(&self) -> Duration64 {
                Duration64 {
                    inner: self.inner.to_duration(),
                }
            }
        }
        temporal_common!($Ty, $core);
    };
}

date_type!(Date32, core::Date32);
date_type!(Date64, core::Date64);

#[napi(namespace = "temporal")]
impl Date32 {
    #[napi]
    pub fn to_date64(&self) -> Date64 {
        Date64 {
            inner: self.inner.to_date64(),
        }
    }
}
#[napi(namespace = "temporal")]
impl Date64 {
    #[napi]
    pub fn to_date32(&self) -> napi::Result<Date32> {
        self.inner
            .to_date32()
            .map(|inner| Date32 { inner })
            .map_err(to_error)
    }
}

// ---- Time -----------------------------------------------------------------------------

macro_rules! time_type {
    ($Ty:ident, $core:ty) => {
        #[doc = concat!("A wall-clock time of day (", stringify!($Ty), "), naive.")]
        #[napi(namespace = "temporal")]
        pub struct $Ty {
            pub(crate) inner: $core,
        }

        #[napi(namespace = "temporal")]
        impl $Ty {
            #[napi(factory)]
            pub fn create(value: i64, unit: String) -> napi::Result<Self> {
                <$core>::new(value as _, parse_unit(&unit)?)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }
            #[napi(factory)]
            pub fn from_string(text: String) -> napi::Result<Self> {
                text.parse::<$core>()
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }
            #[napi(getter)]
            pub fn value(&self) -> i64 {
                self.inner.value() as i64
            }
            /// The `[hour, minute, second, nanosecond]`.
            #[napi]
            pub fn to_hms(&self) -> Vec<i64> {
                let (h, m, s, n) = self.inner.to_hms();
                vec![h as i64, m as i64, s as i64, n as i64]
            }
            #[napi(getter)]
            pub fn hour(&self) -> u32 {
                self.inner.hour()
            }
            #[napi(getter)]
            pub fn minute(&self) -> u32 {
                self.inner.minute()
            }
            #[napi(getter)]
            pub fn second(&self) -> u32 {
                self.inner.second()
            }
            #[napi(getter)]
            pub fn nanosecond(&self) -> u32 {
                self.inner.nanosecond()
            }
            #[napi]
            pub fn nanos_of_day(&self) -> i64 {
                self.inner.nanos_of_day()
            }
            #[napi]
            pub fn to_unit(&self, unit: String) -> napi::Result<Self> {
                self.inner
                    .to_unit(parse_unit(&unit)?)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }
            /// This time-of-day as an elapsed span since midnight, as a `Duration64`.
            #[napi]
            pub fn to_duration(&self) -> Duration64 {
                Duration64 {
                    inner: self.inner.to_duration(),
                }
            }
            /// This time on the epoch date (`1970-01-01`), as a `Ts64` in `unit` (default `"s"`),
            /// zone `tz`.
            #[napi]
            pub fn to_timestamp(
                &self,
                unit: Option<String>,
                tz: Option<String>,
            ) -> napi::Result<Ts64> {
                self.inner
                    .to_timestamp(
                        parse_unit(&unit.unwrap_or_else(|| "s".to_string()))?,
                        parse_tz(&tz.unwrap_or_default())?,
                    )
                    .map(|inner| Ts64 { inner })
                    .map_err(to_error)
            }
        }
        temporal_common!($Ty, $core);
    };
}

time_type!(Time32, core::Time32);
time_type!(Time64, core::Time64);

#[napi(namespace = "temporal")]
impl Time32 {
    #[napi(factory)]
    pub fn from_hms(hour: u32, minute: u32, second: u32) -> napi::Result<Self> {
        core::Time32::from_hms(hour, minute, second)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }
    #[napi]
    pub fn to_time64(&self, unit: String) -> napi::Result<Time64> {
        self.inner
            .to_time64(parse_unit(&unit)?)
            .map(|inner| Time64 { inner })
            .map_err(to_error)
    }
}
#[napi(namespace = "temporal")]
impl Time64 {
    #[napi(factory)]
    pub fn from_hms_nano(
        hour: u32,
        minute: u32,
        second: u32,
        nanosecond: u32,
    ) -> napi::Result<Self> {
        core::Time64::from_hms_nano(hour, minute, second, nanosecond)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }
    #[napi]
    pub fn to_time32(&self, unit: String) -> napi::Result<Time32> {
        self.inner
            .to_time32(parse_unit(&unit)?)
            .map(|inner| Time32 { inner })
            .map_err(to_error)
    }
}

// ---- Timestamp ------------------------------------------------------------------------

macro_rules! timestamp_type {
    ($Ty:ident, $core:ty) => {
        #[doc = concat!("An instant (", stringify!($Ty), ") — a count since the epoch, a unit, and a timezone.")]
        #[napi(namespace = "temporal")]
        pub struct $Ty {
            pub(crate) inner: $core,
        }

        #[napi(namespace = "temporal")]
        impl $Ty {
            #[napi(factory)]
            pub fn from_epoch(value: BigInt, unit: String, tz: Option<String>) -> napi::Result<Self> {
                let (epoch, lossless) = value.get_i128();
                if !lossless {
                    return Err(to_error("epoch value exceeds the 128-bit range"));
                }
                <$core>::from_epoch(epoch, parse_unit(&unit)?, parse_tz(&tz.unwrap_or_default())?)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }
            #[napi(factory)]
            #[allow(clippy::too_many_arguments)]
            pub fn from_datetime(
                year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32,
                nanosecond: u32, unit: String, tz: Option<String>,
            ) -> napi::Result<Self> {
                <$core>::from_datetime(year, month, day, hour, minute, second, nanosecond, parse_unit(&unit)?, parse_tz(&tz.unwrap_or_default())?)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }
            #[napi(factory)]
            pub fn from_string(text: String) -> napi::Result<Self> {
                text.parse::<$core>().map(|inner| Self { inner }).map_err(to_error)
            }
            #[napi(getter)]
            pub fn epoch_value(&self) -> i128 {
                self.inner.epoch_value()
            }
            /// The timezone name (empty for naive).
            #[napi(getter)]
            pub fn timezone(&self) -> String {
                self.inner.tz().name()
            }
            #[napi]
            pub fn epoch_seconds(&self) -> i64 {
                self.inner.epoch_seconds()
            }
            #[napi]
            pub fn offset_seconds(&self) -> i32 {
                self.inner.offset_seconds()
            }
            /// The `[year, month, day, hour, minute, second, nanosecond]` in the zone.
            #[napi]
            pub fn to_datetime(&self) -> Vec<i64> {
                let (y, mo, d, h, mi, s, n) = self.inner.to_datetime();
                vec![y as i64, mo as i64, d as i64, h as i64, mi as i64, s as i64, n as i64]
            }
            #[napi(getter)]
            pub fn year(&self) -> i32 {
                self.inner.year()
            }
            #[napi(getter)]
            pub fn month(&self) -> u32 {
                self.inner.month()
            }
            #[napi(getter)]
            pub fn day(&self) -> u32 {
                self.inner.day()
            }
            #[napi(getter)]
            pub fn hour(&self) -> u32 {
                self.inner.hour()
            }
            #[napi(getter)]
            pub fn minute(&self) -> u32 {
                self.inner.minute()
            }
            #[napi(getter)]
            pub fn second(&self) -> u32 {
                self.inner.second()
            }
            #[napi(getter)]
            pub fn nanosecond(&self) -> u32 {
                self.inner.nanosecond()
            }
            #[napi]
            pub fn with_timezone(&self, tz: String) -> napi::Result<Self> {
                Ok(Self { inner: self.inner.with_timezone(parse_tz(&tz)?) })
            }
            #[napi]
            pub fn to_unit(&self, unit: String) -> napi::Result<Self> {
                self.inner.to_unit(parse_unit(&unit)?).map(|inner| Self { inner }).map_err(to_error)
            }
            #[napi]
            pub fn to_date(&self) -> napi::Result<Date32> {
                self.inner.to_date().map(|inner| Date32 { inner }).map_err(to_error)
            }
            #[napi]
            pub fn to_time(&self) -> napi::Result<Time64> {
                self.inner.to_time().map(|inner| Time64 { inner }).map_err(to_error)
            }
            /// The elapsed span since the epoch, as a `Duration64` in this instant's unit.
            #[napi]
            pub fn to_duration(&self) -> napi::Result<Duration64> {
                self.inner.to_duration().map(|inner| Duration64 { inner }).map_err(to_error)
            }
        }
        temporal_common!($Ty, $core);
    };
}

timestamp_type!(Ts32, core::Ts32);
timestamp_type!(Ts64, core::Ts64);
timestamp_type!(Ts96, core::Ts96);

#[napi(namespace = "temporal")]
impl Ts32 {
    #[napi]
    pub fn to_ts64(&self) -> Ts64 {
        Ts64 {
            inner: self.inner.to_ts64(),
        }
    }
    #[napi]
    pub fn to_ts96(&self) -> Ts96 {
        Ts96 {
            inner: self.inner.to_ts96(),
        }
    }
}
#[napi(namespace = "temporal")]
impl Ts64 {
    #[napi]
    pub fn to_ts32(&self) -> napi::Result<Ts32> {
        self.inner
            .to_ts32()
            .map(|inner| Ts32 { inner })
            .map_err(to_error)
    }
    #[napi]
    pub fn to_ts96(&self) -> Ts96 {
        Ts96 {
            inner: self.inner.to_ts96(),
        }
    }
}
#[napi(namespace = "temporal")]
impl Ts96 {
    #[napi]
    pub fn to_ts64(&self) -> napi::Result<Ts64> {
        self.inner
            .to_ts64()
            .map(|inner| Ts64 { inner })
            .map_err(to_error)
    }
}

// ---- Duration -------------------------------------------------------------------------

macro_rules! duration_type {
    ($Ty:ident, $core:ty) => {
        #[doc = concat!("A signed elapsed span (", stringify!($Ty), ").")]
        #[napi(namespace = "temporal")]
        pub struct $Ty {
            pub(crate) inner: $core,
        }

        #[napi(namespace = "temporal")]
        impl $Ty {
            #[napi(factory)]
            pub fn create(value: i64, unit: String) -> napi::Result<Self> {
                <$core>::new(value as _, parse_unit(&unit)?)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }
            #[napi(factory)]
            pub fn seconds(value: i64) -> Self {
                Self {
                    inner: <$core>::seconds(value as _),
                }
            }
            #[napi(factory)]
            pub fn milliseconds(value: i64) -> Self {
                Self {
                    inner: <$core>::milliseconds(value as _),
                }
            }
            #[napi(factory)]
            pub fn nanoseconds(value: i64) -> Self {
                Self {
                    inner: <$core>::nanoseconds(value as _),
                }
            }
            #[napi(factory)]
            pub fn from_string(text: String) -> napi::Result<Self> {
                text.parse::<$core>()
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }
            #[napi(getter)]
            pub fn value(&self) -> i64 {
                self.inner.value() as i64
            }
            #[napi]
            pub fn to_nanos(&self) -> Option<i128> {
                self.inner.to_nanos()
            }
            #[napi]
            pub fn is_negative(&self) -> bool {
                self.inner.is_negative()
            }
            #[napi]
            pub fn is_zero(&self) -> bool {
                self.inner.is_zero()
            }
            #[napi]
            pub fn to_unit(&self, unit: String) -> napi::Result<Self> {
                self.inner
                    .to_unit(parse_unit(&unit)?)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }
            /// The instant at `epoch + self`, as a `Ts64` in this span's unit and zone `tz`.
            #[napi]
            pub fn to_timestamp(&self, tz: Option<String>) -> napi::Result<Ts64> {
                self.inner
                    .to_timestamp(parse_tz(&tz.unwrap_or_default())?)
                    .map(|inner| Ts64 { inner })
                    .map_err(to_error)
            }
            /// This span reduced to a time of day (modulo 24h), as a `Time64`.
            #[napi]
            pub fn to_time(&self) -> napi::Result<Time64> {
                self.inner
                    .to_time()
                    .map(|inner| Time64 { inner })
                    .map_err(to_error)
            }
            /// This span as days since the epoch (truncating), as a `Date32`.
            #[napi]
            pub fn to_date(&self) -> napi::Result<Date32> {
                self.inner
                    .to_date()
                    .map(|inner| Date32 { inner })
                    .map_err(to_error)
            }
            #[napi]
            pub fn add(&self, other: &$Ty) -> napi::Result<Self> {
                self.inner
                    .checked_add(&other.inner)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }
            #[napi]
            pub fn sub(&self, other: &$Ty) -> napi::Result<Self> {
                self.inner
                    .checked_sub(&other.inner)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }
            #[napi]
            pub fn neg(&self) -> napi::Result<Self> {
                self.inner
                    .checked_neg()
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }
        }
        temporal_common!($Ty, $core);
    };
}

duration_type!(Duration32, core::Duration32);
duration_type!(Duration64, core::Duration64);

#[napi(namespace = "temporal")]
impl Duration32 {
    /// Widen to a `Duration64`.
    #[napi]
    pub fn to_duration64(&self) -> Duration64 {
        Duration64 {
            inner: self.inner.to_duration64(),
        }
    }
}
#[napi(namespace = "temporal")]
impl Duration64 {
    /// Narrow to a `Duration32`, or throws if out of range.
    #[napi]
    pub fn to_duration32(&self) -> napi::Result<Duration32> {
        self.inner
            .to_duration32()
            .map(|inner| Duration32 { inner })
            .map_err(to_error)
    }
}

// ---- JS Date bridge (a JS Date is milliseconds since the epoch, UTC) -------------------

#[napi(namespace = "temporal")]
impl Ts64 {
    /// This instant in **milliseconds** since the epoch — `new Date(ts.toEpochMillis())` bridges to
    /// a JS `Date`.
    #[napi]
    pub fn to_epoch_millis(&self) -> f64 {
        (self.inner.epoch_nanos() / 1_000_000) as f64
    }
    /// An instant from milliseconds since the epoch (`Ts64.fromEpochMillis(date.getTime())`),
    /// in `tz` (default UTC).
    #[napi(factory)]
    pub fn from_epoch_millis(millis: f64, tz: Option<String>) -> napi::Result<Self> {
        core::Ts64::from_epoch(
            millis as i128,
            core::TimeUnit::Millisecond,
            parse_tz(&tz.unwrap_or_else(|| "UTC".to_string()))?,
        )
        .map(|inner| Self { inner })
        .map_err(to_error)
    }
}

// ---- Generic parse factories (the default width per concept) --------------------------

/// Parses a **date** from a flexible set of common formats into a [`Date32`].
#[napi(namespace = "temporal")]
pub fn date(text: String) -> napi::Result<Date32> {
    core::Date32::parse_str(&text)
        .map(|inner| Date32 { inner })
        .map_err(to_error)
}
/// Parses a **time** of day into a [`Time64`].
#[napi(namespace = "temporal")]
pub fn time(text: String) -> napi::Result<Time64> {
    core::Time64::parse_str(&text)
        .map(|inner| Time64 { inner })
        .map_err(to_error)
}
/// Parses an **instant** into a [`Ts64`], defaulting to `unit` / `tz` while parsing.
#[napi(namespace = "temporal")]
pub fn timestamp(text: String, unit: Option<String>, tz: Option<String>) -> napi::Result<Ts64> {
    core::Ts64::parse_str(
        &text,
        parse_unit(&unit.unwrap_or_else(|| "ns".to_string()))?,
        parse_tz(&tz.unwrap_or_default())?,
    )
    .map(|inner| Ts64 { inner })
    .map_err(to_error)
}
/// Parses a **duration** into a [`Duration64`], flexibly: a single `<count><unit>` (`"90s"`,
/// `"-1500ms"`), a compound run (`"1h30m15s"`, `"2d 3h"`), a clock (`"1:30:00"`), or ISO-8601
/// (`"PT1H30M"`, `"P1DT2H"`). Pass `unit` to cast the result to that resolution (default: the
/// input's natural granularity).
#[napi(namespace = "temporal")]
pub fn duration(text: String, unit: Option<String>) -> napi::Result<Duration64> {
    let parsed = core::Duration64::parse_str(&text).map_err(to_error)?;
    let inner = match unit {
        Some(unit) => parsed.to_unit(parse_unit(&unit)?).map_err(to_error)?,
        None => parsed,
    };
    Ok(Duration64 { inner })
}
