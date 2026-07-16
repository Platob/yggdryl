//! The `yggdryl.temporal` submodule — the temporal value types (`Date32`/`Date64`, `Time32`/
//! `Time64`, `Ts32`/`Ts64`/`Ts96`, `Duration32`/`Duration64`) and the [`Tz`]
//! timezone, mirroring `yggdryl_core::io::fixed::temporal`.
//!
//! Resolutions (**time units**) and **timezones** cross as strings — `"ns"`/`"ms"`/`"s"`/`"second"`
//! and `"UTC"`/`"Europe/Paris"`/`"+02:00"`/`""` (naive) — so the Python surface stays idiomatic; the
//! [`Tz`] class is there for zone queries (offset lookups, DST). Each value type behaves like a
//! native Python value: rich comparison, `__hash__`, `__str__`, pickling, and `copy`.

#![allow(clippy::useless_conversion)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::pyclass::CompareOp;
use pyo3::types::{PyBytes, PyDict};

use yggdryl_core::io::fixed::temporal as core;
use yggdryl_core::io::fixed::temporal::Temporal as _;

/// Maps a core [`TemporalError`](core::TemporalError) to a Python `ValueError`.
fn temporal_err(error: core::TemporalError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Parses a time-unit string (`"ns"`, `"second"`, …) or raises `ValueError`.
fn parse_unit(text: &str) -> PyResult<core::TimeUnit> {
    core::TimeUnit::parse(text)
        .ok_or_else(|| PyValueError::new_err(format!("unknown time unit: {text:?}")))
}

/// Parses a timezone string (`"UTC"`, `"Europe/Paris"`, `"+02:00"`, `""`) or raises `ValueError`.
fn parse_tz(text: &str) -> PyResult<core::Tz> {
    core::Tz::parse(text)
        .ok_or_else(|| PyValueError::new_err(format!("unknown timezone: {text:?}")))
}

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// A **timezone** — `naive`, `UTC`, a fixed offset, or a DST-aware IANA zone. Construct with a
/// factory (`Tz.utc()`, `Tz.iana("Europe/Paris")`, `Tz.parse("+02:00")`); its `name` is the string
/// the timestamp types accept.
#[pyclass(module = "yggdryl.temporal")]
#[derive(Clone)]
pub struct Tz {
    pub(crate) inner: core::Tz,
}

#[pymethods]
impl Tz {
    /// The naive (zone-unspecified) timezone.
    #[staticmethod]
    fn naive() -> Self {
        Self {
            inner: core::Tz::NAIVE,
        }
    }
    /// UTC.
    #[staticmethod]
    fn utc() -> Self {
        Self {
            inner: core::Tz::UTC,
        }
    }
    /// A fixed offset of `seconds` east of UTC.
    #[staticmethod]
    fn fixed_offset(seconds: i32) -> Self {
        Self {
            inner: core::Tz::fixed_offset(seconds),
        }
    }
    /// A named IANA zone, or `ValueError` if unknown.
    #[staticmethod]
    fn iana(name: &str) -> PyResult<Self> {
        core::Tz::iana(name)
            .map(|inner| Self { inner })
            .ok_or_else(|| PyValueError::new_err(format!("unknown IANA zone: {name:?}")))
    }
    /// Parses a timezone string (IANA name, offset, `UTC`/`Z`, or empty for naive).
    #[staticmethod]
    fn parse(text: &str) -> PyResult<Self> {
        Ok(Self {
            inner: parse_tz(text)?,
        })
    }

    /// The zone name (empty for naive) — the string the timestamp types accept.
    #[getter]
    fn name(&self) -> String {
        self.inner.name()
    }
    /// The offset in seconds east of UTC in effect at `epoch_seconds` (DST-aware).
    fn offset_seconds_at(&self, epoch_seconds: i64) -> i32 {
        self.inner.offset_seconds_at(epoch_seconds)
    }
    fn is_naive(&self) -> bool {
        self.inner.is_naive()
    }
    fn is_utc(&self) -> bool {
        self.inner.is_utc()
    }
    fn is_iana(&self) -> bool {
        self.inner.is_iana()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
    fn __hash__(&self) -> u64 {
        hash_of(&self.inner)
    }
    fn __str__(&self) -> String {
        self.inner.to_string()
    }
    fn __repr__(&self) -> String {
        format!("Tz({:?})", self.inner.name())
    }
}

/// Generates a pyo3 wrapper carrying the common value-type surface (rich comparison, hash, str,
/// pickle, copy) for a temporal core type with a `serialize_bytes` / `deserialize_bytes` codec.
macro_rules! temporal_common {
    ($Ty:ident, $core:ty) => {
        #[pymethods]
        impl $Ty {
            /// The value's canonical bytes.
            fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                PyBytes::new_bound(py, &self.inner.serialize_bytes())
            }
            /// Reconstructs a value from [`serialize_bytes`](Self::serialize_bytes).
            #[staticmethod]
            fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
                <$core>::deserialize_bytes(bytes)
                    .map(|inner| Self { inner })
                    .map_err(temporal_err)
            }
            /// The resolution unit as a string (`"day"`, `"ns"`, …).
            #[getter]
            fn unit(&self) -> String {
                self.inner.time_unit().abbreviation().to_string()
            }
            fn __richcmp__(&self, other: &Self, op: CompareOp) -> bool {
                op.matches(self.inner.cmp(&other.inner))
            }
            fn __hash__(&self) -> u64 {
                hash_of(&self.inner)
            }
            fn __str__(&self) -> String {
                self.inner.to_string()
            }
            fn copy(&self) -> Self {
                self.clone()
            }
            fn __copy__(&self) -> Self {
                self.clone()
            }
            fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
                self.clone()
            }
            fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
                let ctor = py
                    .get_type_bound::<$Ty>()
                    .getattr("deserialize_bytes")?
                    .unbind();
                let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
                    .into_any()
                    .unbind();
                Ok((ctor, (state,)))
            }
        }
    };
}

// ---- Date -----------------------------------------------------------------------------

macro_rules! date_type {
    ($Ty:ident, $core:ty, $lit:literal) => {
        #[doc = concat!("A calendar date (`", $lit, "`), naive.")]
        #[pyclass(module = "yggdryl.temporal")]
        #[derive(Clone)]
        pub struct $Ty {
            pub(crate) inner: $core,
        }

        #[pymethods]
        impl $Ty {
            /// The date `year-month-day`, or `ValueError` for an impossible / out-of-range date.
            #[staticmethod]
            fn from_ymd(year: i32, month: u32, day: u32) -> PyResult<Self> {
                <$core>::from_ymd(year, month, day)
                    .map(|inner| Self { inner })
                    .map_err(temporal_err)
            }
            /// The date `days` days after the epoch.
            #[staticmethod]
            fn from_days(days: i64) -> Self {
                Self {
                    inner: <$core>::from_days(days as _),
                }
            }
            /// Parses an ISO date `YYYY-MM-DD`.
            #[staticmethod]
            fn from_string(text: &str) -> PyResult<Self> {
                text.parse::<$core>()
                    .map(|inner| Self { inner })
                    .map_err(temporal_err)
            }
            /// The `(year, month, day)`.
            fn to_ymd(&self) -> (i32, u32, u32) {
                self.inner.to_ymd()
            }
            /// The day count since the epoch.
            #[getter]
            fn days(&self) -> i64 {
                self.inner.days() as i64
            }
            #[getter]
            fn year(&self) -> i32 {
                self.inner.year()
            }
            #[getter]
            fn month(&self) -> u32 {
                self.inner.month()
            }
            #[getter]
            fn day(&self) -> u32 {
                self.inner.day()
            }
            /// The weekday (`0` = Sunday … `6` = Saturday).
            fn weekday(&self) -> u32 {
                self.inner.weekday()
            }
            fn is_leap_year(&self) -> bool {
                self.inner.is_leap_year()
            }
            /// This date at midnight, as a [`Ts64`] in `unit` (default `"s"`) and zone `tz`.
            #[pyo3(signature = (unit = "s", tz = ""))]
            fn at_midnight(&self, unit: &str, tz: &str) -> PyResult<Ts64> {
                self.inner
                    .at_midnight(parse_unit(unit)?, parse_tz(tz)?)
                    .map(|inner| Ts64 { inner })
                    .map_err(temporal_err)
            }
            /// This date at the wall-clock `time`, as a [`Ts64`] in `unit` (default `"s"`), zone `tz`.
            #[pyo3(signature = (time, unit = "s", tz = ""))]
            fn at_time(&self, time: &Time64, unit: &str, tz: &str) -> PyResult<Ts64> {
                self.inner
                    .at_time(&time.inner, parse_unit(unit)?, parse_tz(tz)?)
                    .map(|inner| Ts64 { inner })
                    .map_err(temporal_err)
            }
            /// The elapsed span from the epoch to this date, as a [`Duration64`] of days.
            fn to_duration(&self) -> Duration64 {
                Duration64 {
                    inner: self.inner.to_duration(),
                }
            }
            fn __repr__(&self) -> String {
                format!("{:?}", self.inner)
            }
        }
        temporal_common!($Ty, $core);
    };
}

date_type!(Date32, core::Date32, "date32");
date_type!(Date64, core::Date64, "date64");

#[pymethods]
impl Date32 {
    /// This date as a [`Date64`].
    fn to_date64(&self) -> Date64 {
        Date64 {
            inner: self.inner.to_date64(),
        }
    }
}
#[pymethods]
impl Date64 {
    /// This date as a [`Date32`], or `ValueError` if out of range.
    fn to_date32(&self) -> PyResult<Date32> {
        self.inner
            .to_date32()
            .map(|inner| Date32 { inner })
            .map_err(temporal_err)
    }
}

// ---- Time -----------------------------------------------------------------------------

macro_rules! time_type {
    ($Ty:ident, $core:ty, $lit:literal) => {
        #[doc = concat!("A wall-clock time of day (`", $lit, "`), naive.")]
        #[pyclass(module = "yggdryl.temporal")]
        #[derive(Clone)]
        pub struct $Ty {
            pub(crate) inner: $core,
        }

        #[pymethods]
        impl $Ty {
            /// A time from a raw count in `unit` (a unit string).
            #[staticmethod]
            fn new(value: i64, unit: &str) -> PyResult<Self> {
                <$core>::new(value as _, parse_unit(unit)?)
                    .map(|inner| Self { inner })
                    .map_err(temporal_err)
            }
            /// Parses an ISO time `HH:MM:SS[.frac]`.
            #[staticmethod]
            fn from_string(text: &str) -> PyResult<Self> {
                text.parse::<$core>()
                    .map(|inner| Self { inner })
                    .map_err(temporal_err)
            }
            /// The raw count of `unit` since midnight.
            #[getter]
            fn value(&self) -> i64 {
                self.inner.value() as i64
            }
            /// The `(hour, minute, second, nanosecond)`.
            fn to_hms(&self) -> (u32, u32, u32, u32) {
                self.inner.to_hms()
            }
            #[getter]
            fn hour(&self) -> u32 {
                self.inner.hour()
            }
            #[getter]
            fn minute(&self) -> u32 {
                self.inner.minute()
            }
            #[getter]
            fn second(&self) -> u32 {
                self.inner.second()
            }
            #[getter]
            fn nanosecond(&self) -> u32 {
                self.inner.nanosecond()
            }
            /// The nanosecond-of-day.
            fn nanos_of_day(&self) -> i64 {
                self.inner.nanos_of_day()
            }
            /// This time re-expressed in `unit` (a unit string).
            fn to_unit(&self, unit: &str) -> PyResult<Self> {
                self.inner
                    .to_unit(parse_unit(unit)?)
                    .map(|inner| Self { inner })
                    .map_err(temporal_err)
            }
            /// This time-of-day as an elapsed span since midnight, as a [`Duration64`].
            fn to_duration(&self) -> Duration64 {
                Duration64 {
                    inner: self.inner.to_duration(),
                }
            }
            /// This time on the epoch date (`1970-01-01`), as a [`Ts64`] in `unit` (default `"s"`),
            /// zone `tz`.
            #[pyo3(signature = (unit = "s", tz = ""))]
            fn to_timestamp(&self, unit: &str, tz: &str) -> PyResult<Ts64> {
                self.inner
                    .to_timestamp(parse_unit(unit)?, parse_tz(tz)?)
                    .map(|inner| Ts64 { inner })
                    .map_err(temporal_err)
            }
            fn __repr__(&self) -> String {
                format!("{:?}", self.inner)
            }
        }
        temporal_common!($Ty, $core);
    };
}

time_type!(Time32, core::Time32, "time32");
time_type!(Time64, core::Time64, "time64");

#[pymethods]
impl Time32 {
    /// A time from `hour:minute:second` (second resolution).
    #[staticmethod]
    fn from_hms(hour: u32, minute: u32, second: u32) -> PyResult<Self> {
        core::Time32::from_hms(hour, minute, second)
            .map(|inner| Self { inner })
            .map_err(temporal_err)
    }
    /// This time as a [`Time64`] in `unit`.
    fn to_time64(&self, unit: &str) -> PyResult<Time64> {
        self.inner
            .to_time64(parse_unit(unit)?)
            .map(|inner| Time64 { inner })
            .map_err(temporal_err)
    }
}
#[pymethods]
impl Time64 {
    /// A time from `hour:minute:second` and a sub-second `nanosecond` (nanosecond resolution).
    #[staticmethod]
    fn from_hms_nano(hour: u32, minute: u32, second: u32, nanosecond: u32) -> PyResult<Self> {
        core::Time64::from_hms_nano(hour, minute, second, nanosecond)
            .map(|inner| Self { inner })
            .map_err(temporal_err)
    }
    /// This time as a [`Time32`] in `unit`.
    fn to_time32(&self, unit: &str) -> PyResult<Time32> {
        self.inner
            .to_time32(parse_unit(unit)?)
            .map(|inner| Time32 { inner })
            .map_err(temporal_err)
    }
}

// ---- Timestamp ------------------------------------------------------------------------

macro_rules! timestamp_type {
    ($Ty:ident, $core:ty, $lit:literal) => {
        #[doc = concat!("An instant (`", $lit, "`) — a count since the epoch, a unit, and a timezone.")]
        #[pyclass(module = "yggdryl.temporal")]
        #[derive(Clone)]
        pub struct $Ty {
            pub(crate) inner: $core,
        }

        #[pymethods]
        impl $Ty {
            /// An instant `value` counts of `unit` since the epoch, in `tz` (a tz string).
            #[staticmethod]
            #[pyo3(signature = (value, unit, tz = ""))]
            fn from_epoch(value: i128, unit: &str, tz: &str) -> PyResult<Self> {
                <$core>::from_epoch(value, parse_unit(unit)?, parse_tz(tz)?)
                    .map(|inner| Self { inner })
                    .map_err(temporal_err)
            }
            /// An instant from wall-clock components interpreted in `tz`.
            #[staticmethod]
            #[pyo3(signature = (year, month, day, hour, minute, second, nanosecond, unit, tz = ""))]
            #[allow(clippy::too_many_arguments)]
            fn from_datetime(
                year: i32,
                month: u32,
                day: u32,
                hour: u32,
                minute: u32,
                second: u32,
                nanosecond: u32,
                unit: &str,
                tz: &str,
            ) -> PyResult<Self> {
                <$core>::from_datetime(
                    year,
                    month,
                    day,
                    hour,
                    minute,
                    second,
                    nanosecond,
                    parse_unit(unit)?,
                    parse_tz(tz)?,
                )
                .map(|inner| Self { inner })
                .map_err(temporal_err)
            }
            /// Parses an ISO timestamp `YYYY-MM-DDThh:mm:ss[.frac][Z|±hh:mm]`.
            #[staticmethod]
            fn from_string(text: &str) -> PyResult<Self> {
                text.parse::<$core>()
                    .map(|inner| Self { inner })
                    .map_err(temporal_err)
            }
            /// The raw epoch count.
            #[getter]
            fn epoch_value(&self) -> i128 {
                self.inner.epoch_value()
            }
            /// The timezone name (empty for naive).
            #[getter]
            fn timezone(&self) -> String {
                self.inner.tz().name()
            }
            /// The whole seconds since the epoch.
            fn epoch_seconds(&self) -> i64 {
                self.inner.epoch_seconds()
            }
            /// The zone's offset (seconds) at this instant.
            fn offset_seconds(&self) -> i32 {
                self.inner.offset_seconds()
            }
            /// The wall-clock `(year, month, day, hour, minute, second, nanosecond)` in the zone.
            fn to_datetime(&self) -> (i32, u32, u32, u32, u32, u32, u32) {
                self.inner.to_datetime()
            }
            #[getter]
            fn year(&self) -> i32 {
                self.inner.year()
            }
            #[getter]
            fn month(&self) -> u32 {
                self.inner.month()
            }
            #[getter]
            fn day(&self) -> u32 {
                self.inner.day()
            }
            #[getter]
            fn hour(&self) -> u32 {
                self.inner.hour()
            }
            #[getter]
            fn minute(&self) -> u32 {
                self.inner.minute()
            }
            #[getter]
            fn second(&self) -> u32 {
                self.inner.second()
            }
            #[getter]
            fn nanosecond(&self) -> u32 {
                self.inner.nanosecond()
            }
            /// The **same instant** displayed in a different `tz`.
            fn with_timezone(&self, tz: &str) -> PyResult<Self> {
                Ok(Self {
                    inner: self.inner.with_timezone(parse_tz(tz)?),
                })
            }
            /// This instant re-expressed in `unit`.
            fn to_unit(&self, unit: &str) -> PyResult<Self> {
                self.inner
                    .to_unit(parse_unit(unit)?)
                    .map(|inner| Self { inner })
                    .map_err(temporal_err)
            }
            /// The calendar date in the zone.
            fn to_date(&self) -> PyResult<Date32> {
                self.inner
                    .to_date()
                    .map(|inner| Date32 { inner })
                    .map_err(temporal_err)
            }
            /// The time of day (nanosecond resolution) in the zone.
            fn to_time(&self) -> PyResult<Time64> {
                self.inner
                    .to_time()
                    .map(|inner| Time64 { inner })
                    .map_err(temporal_err)
            }
            /// The elapsed span since the epoch, as a [`Duration64`] in this instant's unit.
            fn to_duration(&self) -> PyResult<Duration64> {
                self.inner
                    .to_duration()
                    .map(|inner| Duration64 { inner })
                    .map_err(temporal_err)
            }
            fn __repr__(&self) -> String {
                format!("{:?}", self.inner)
            }
        }
        temporal_common!($Ty, $core);
    };
}

timestamp_type!(Ts32, core::Ts32, "ts32");
timestamp_type!(Ts64, core::Ts64, "ts64");
timestamp_type!(Ts96, core::Ts96, "ts96");

#[pymethods]
impl Ts32 {
    /// Widen to a [`Ts64`].
    fn to_ts64(&self) -> Ts64 {
        Ts64 {
            inner: self.inner.to_ts64(),
        }
    }
    /// Widen to a [`Ts96`].
    fn to_ts96(&self) -> Ts96 {
        Ts96 {
            inner: self.inner.to_ts96(),
        }
    }
}
#[pymethods]
impl Ts64 {
    /// Narrow to a [`Ts32`], or `ValueError` if out of range.
    fn to_ts32(&self) -> PyResult<Ts32> {
        self.inner
            .to_ts32()
            .map(|inner| Ts32 { inner })
            .map_err(temporal_err)
    }
    /// Widen to a [`Ts96`].
    fn to_ts96(&self) -> Ts96 {
        Ts96 {
            inner: self.inner.to_ts96(),
        }
    }
}
#[pymethods]
impl Ts96 {
    /// Narrow to a [`Ts64`], or `ValueError` if out of range.
    fn to_ts64(&self) -> PyResult<Ts64> {
        self.inner
            .to_ts64()
            .map(|inner| Ts64 { inner })
            .map_err(temporal_err)
    }
}

// ---- Duration -------------------------------------------------------------------------

macro_rules! duration_type {
    ($Ty:ident, $core:ty, $lit:literal) => {
        #[doc = concat!("A signed elapsed span (`", $lit, "`).")]
        #[pyclass(module = "yggdryl.temporal")]
        #[derive(Clone)]
        pub struct $Ty {
            pub(crate) inner: $core,
        }

        #[pymethods]
        impl $Ty {
            /// A span of `value` counts of `unit` (a unit string).
            #[staticmethod]
            fn new(value: i64, unit: &str) -> PyResult<Self> {
                <$core>::new(value as _, parse_unit(unit)?)
                    .map(|inner| Self { inner })
                    .map_err(temporal_err)
            }
            /// A span of `value` seconds.
            #[staticmethod]
            fn seconds(value: i64) -> Self {
                Self {
                    inner: <$core>::seconds(value as _),
                }
            }
            /// A span of `value` milliseconds.
            #[staticmethod]
            fn milliseconds(value: i64) -> Self {
                Self {
                    inner: <$core>::milliseconds(value as _),
                }
            }
            /// A span of `value` nanoseconds.
            #[staticmethod]
            fn nanoseconds(value: i64) -> Self {
                Self {
                    inner: <$core>::nanoseconds(value as _),
                }
            }
            /// Parses `<count><unit>` (e.g. `"90s"`, `"-1500ms"`).
            #[staticmethod]
            fn from_string(text: &str) -> PyResult<Self> {
                text.parse::<$core>()
                    .map(|inner| Self { inner })
                    .map_err(temporal_err)
            }
            #[getter]
            fn value(&self) -> i64 {
                self.inner.value() as i64
            }
            /// The span in nanoseconds, or `None` for a calendar unit.
            fn to_nanos(&self) -> Option<i128> {
                self.inner.to_nanos()
            }
            fn is_negative(&self) -> bool {
                self.inner.is_negative()
            }
            fn is_zero(&self) -> bool {
                self.inner.is_zero()
            }
            /// This span re-expressed in `unit`.
            fn to_unit(&self, unit: &str) -> PyResult<Self> {
                self.inner
                    .to_unit(parse_unit(unit)?)
                    .map(|inner| Self { inner })
                    .map_err(temporal_err)
            }
            /// The instant at `epoch + self`, as a [`Ts64`] in this span's unit and zone `tz`.
            #[pyo3(signature = (tz = ""))]
            fn to_timestamp(&self, tz: &str) -> PyResult<Ts64> {
                self.inner
                    .to_timestamp(parse_tz(tz)?)
                    .map(|inner| Ts64 { inner })
                    .map_err(temporal_err)
            }
            /// This span reduced to a time of day (modulo 24h), as a [`Time64`].
            fn to_time(&self) -> PyResult<Time64> {
                self.inner
                    .to_time()
                    .map(|inner| Time64 { inner })
                    .map_err(temporal_err)
            }
            /// This span as days since the epoch (truncating), as a [`Date32`].
            fn to_date(&self) -> PyResult<Date32> {
                self.inner
                    .to_date()
                    .map(|inner| Date32 { inner })
                    .map_err(temporal_err)
            }
            fn __add__(&self, other: &Self) -> PyResult<Self> {
                self.inner
                    .checked_add(&other.inner)
                    .map(|inner| Self { inner })
                    .map_err(temporal_err)
            }
            fn __sub__(&self, other: &Self) -> PyResult<Self> {
                self.inner
                    .checked_sub(&other.inner)
                    .map(|inner| Self { inner })
                    .map_err(temporal_err)
            }
            fn __neg__(&self) -> PyResult<Self> {
                self.inner
                    .checked_neg()
                    .map(|inner| Self { inner })
                    .map_err(temporal_err)
            }
            fn __repr__(&self) -> String {
                format!("{:?}", self.inner)
            }
        }
        temporal_common!($Ty, $core);
    };
}

duration_type!(Duration32, core::Duration32, "duration32");
duration_type!(Duration64, core::Duration64, "duration64");

#[pymethods]
impl Duration32 {
    /// Widen to a [`Duration64`].
    fn to_duration64(&self) -> Duration64 {
        Duration64 {
            inner: self.inner.to_duration64(),
        }
    }
}
#[pymethods]
impl Duration64 {
    /// Narrow to a [`Duration32`], or `ValueError` if out of range.
    fn to_duration32(&self) -> PyResult<Duration32> {
        self.inner
            .to_duration32()
            .map(|inner| Duration32 { inner })
            .map_err(temporal_err)
    }
}

// ---- Native Python datetime interop (datetime.date / time / datetime / timedelta) -----

#[pymethods]
impl Date32 {
    /// This date as a native Python `datetime.date`.
    fn to_pydate<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let (year, month, day) = self.inner.to_ymd();
        py.import_bound("datetime")?
            .getattr("date")?
            .call1((year, month, day))
    }
    /// A date from a native Python `datetime.date` (or `datetime.datetime`).
    #[staticmethod]
    pub(crate) fn from_pydate(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        core::Date32::from_ymd(
            value.getattr("year")?.extract()?,
            value.getattr("month")?.extract()?,
            value.getattr("day")?.extract()?,
        )
        .map(|inner| Self { inner })
        .map_err(temporal_err)
    }
}

#[pymethods]
impl Time64 {
    /// This time as a native Python `datetime.time` (microsecond resolution).
    fn to_pytime<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let (hour, minute, second, nanosecond) = self.inner.to_hms();
        py.import_bound("datetime")?.getattr("time")?.call1((
            hour,
            minute,
            second,
            nanosecond / 1000,
        ))
    }
    /// A time from a native Python `datetime.time`.
    #[staticmethod]
    pub(crate) fn from_pytime(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let micros: u32 = value.getattr("microsecond")?.extract()?;
        core::Time64::from_hms_nano(
            value.getattr("hour")?.extract()?,
            value.getattr("minute")?.extract()?,
            value.getattr("second")?.extract()?,
            micros * 1000,
        )
        .map(|inner| Self { inner })
        .map_err(temporal_err)
    }
}

#[pymethods]
impl Ts64 {
    /// This instant as a native Python `datetime.datetime` (naive, or with a fixed-offset tzinfo).
    fn to_pydatetime<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let (year, month, day, hour, minute, second, nanosecond) = self.inner.to_datetime();
        let datetime = py.import_bound("datetime")?;
        let micros = nanosecond / 1000;
        if self.inner.tz().is_naive() {
            datetime
                .getattr("datetime")?
                .call1((year, month, day, hour, minute, second, micros))
        } else {
            let delta = datetime
                .getattr("timedelta")?
                .call1((0, self.inner.offset_seconds()))?;
            let tzinfo = datetime.getattr("timezone")?.call1((delta,))?;
            let kwargs = PyDict::new_bound(py);
            kwargs.set_item("tzinfo", tzinfo)?;
            datetime.getattr("datetime")?.call(
                (year, month, day, hour, minute, second, micros),
                Some(&kwargs),
            )
        }
    }
    /// An instant from a native Python `datetime.datetime`, at `unit` resolution (default
    /// microseconds); a `tzinfo` becomes a fixed-offset zone, else naive.
    #[staticmethod]
    #[pyo3(signature = (value, unit = "us"))]
    pub(crate) fn from_pydatetime(value: &Bound<'_, PyAny>, unit: &str) -> PyResult<Self> {
        let micros: u32 = value.getattr("microsecond")?.extract()?;
        let offset = value.call_method0("utcoffset")?;
        let tz = if offset.is_none() {
            core::Tz::NAIVE
        } else {
            let total: f64 = offset.call_method0("total_seconds")?.extract()?;
            core::Tz::fixed_offset(total as i32)
        };
        core::Ts64::from_datetime(
            value.getattr("year")?.extract()?,
            value.getattr("month")?.extract()?,
            value.getattr("day")?.extract()?,
            value.getattr("hour")?.extract()?,
            value.getattr("minute")?.extract()?,
            value.getattr("second")?.extract()?,
            micros * 1000,
            parse_unit(unit)?,
            tz,
        )
        .map(|inner| Self { inner })
        .map_err(temporal_err)
    }
}

#[pymethods]
impl Duration64 {
    /// This span as a native Python `datetime.timedelta` (microsecond resolution).
    fn to_timedelta<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let nanos = self
            .inner
            .to_nanos()
            .ok_or_else(|| PyValueError::new_err("a calendar-unit duration has no timedelta"))?;
        let kwargs = PyDict::new_bound(py);
        kwargs.set_item("microseconds", nanos / 1000)?;
        py.import_bound("datetime")?
            .getattr("timedelta")?
            .call((), Some(&kwargs))
    }
    /// A span from a native Python `datetime.timedelta` (microsecond resolution).
    #[staticmethod]
    pub(crate) fn from_timedelta(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let days: i64 = value.getattr("days")?.extract()?;
        let seconds: i64 = value.getattr("seconds")?.extract()?;
        let micros: i64 = value.getattr("microseconds")?.extract()?;
        let total = days as i128 * 86_400_000_000 + seconds as i128 * 1_000_000 + micros as i128;
        let value = i64::try_from(total)
            .map_err(|_| PyValueError::new_err("timedelta out of range for duration64"))?;
        core::Duration64::new(value, core::TimeUnit::Microsecond)
            .map(|inner| Self { inner })
            .map_err(temporal_err)
    }
}

// ---- Generic parse factories (the default width per concept) --------------------------

/// Parses a **date** from a flexible set of common formats into a [`Date32`].
#[pyfunction]
fn date(text: &str) -> PyResult<Date32> {
    core::Date32::parse_str(text)
        .map(|inner| Date32 { inner })
        .map_err(temporal_err)
}
/// Parses a **time** of day into a [`Time64`].
#[pyfunction]
fn time(text: &str) -> PyResult<Time64> {
    core::Time64::parse_str(text)
        .map(|inner| Time64 { inner })
        .map_err(temporal_err)
}
/// Parses an **instant** into a [`Ts64`], defaulting to `unit` / `tz` while parsing.
#[pyfunction]
#[pyo3(signature = (text, unit = "ns", tz = ""))]
fn timestamp(text: &str, unit: &str, tz: &str) -> PyResult<Ts64> {
    core::Ts64::parse_str(text, parse_unit(unit)?, parse_tz(tz)?)
        .map(|inner| Ts64 { inner })
        .map_err(temporal_err)
}
/// Parses a **duration** into a [`Duration64`], flexibly: a single `<count><unit>` (`"90s"`,
/// `"-1500ms"`), a compound run (`"1h30m15s"`, `"2d 3h"`), a clock (`"1:30:00"`), or ISO-8601
/// (`"PT1H30M"`, `"P1DT2H"`). Pass `unit` to cast the result to that resolution (default: the
/// input's natural granularity).
#[pyfunction]
#[pyo3(signature = (text, unit = ""))]
fn duration(text: &str, unit: &str) -> PyResult<Duration64> {
    let parsed = core::Duration64::parse_str(text).map_err(temporal_err)?;
    let inner = if unit.is_empty() {
        parsed
    } else {
        parsed.to_unit(parse_unit(unit)?).map_err(temporal_err)?
    };
    Ok(Duration64 { inner })
}

/// Populates the `temporal` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Tz>()?;
    module.add_class::<Date32>()?;
    module.add_class::<Date64>()?;
    module.add_class::<Time32>()?;
    module.add_class::<Time64>()?;
    module.add_class::<Ts32>()?;
    module.add_class::<Ts64>()?;
    module.add_class::<Ts96>()?;
    module.add_class::<Duration32>()?;
    module.add_class::<Duration64>()?;
    module.add_function(wrap_pyfunction!(date, module)?)?;
    module.add_function(wrap_pyfunction!(time, module)?)?;
    module.add_function(wrap_pyfunction!(timestamp, module)?)?;
    module.add_function(wrap_pyfunction!(duration, module)?)?;
    Ok(())
}
