//! The `DateTime` napi class — an absolute instant with an optional display timezone.

use std::collections::HashMap;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_core::{DateTime as CoreDateTime, Timezone as CoreTimezone};

use crate::date::Date;
use crate::time::Time;
use crate::timezone::Timezone;
use crate::{err, to_mapping};

/// An instant in time (UTC seconds + nanoseconds) with an optional display
/// `Timezone`; DST-aware conversions. A naive instance (no zone) is UTC.
#[napi]
pub struct DateTime {
    pub(crate) inner: CoreDateTime,
}

fn zone(tz: Option<String>) -> Result<Option<CoreTimezone>> {
    match tz {
        Some(name) => CoreTimezone::from_str(&name).map(Some).map_err(err),
        None => Ok(None),
    }
}

#[napi]
impl DateTime {
    /// Build from civil components in `timezone` (a zone name, or null = naive/UTC).
    #[napi(constructor)]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        year: i32,
        month: u32,
        day: u32,
        hour: Option<u32>,
        minute: Option<u32>,
        second: Option<u32>,
        nano: Option<u32>,
        timezone: Option<String>,
    ) -> Result<Self> {
        CoreDateTime::from_ymd_hms(
            year,
            month,
            day,
            hour.unwrap_or(0),
            minute.unwrap_or(0),
            second.unwrap_or(0),
            nano.unwrap_or(0),
            zone(timezone)?,
        )
        .map(|inner| DateTime { inner })
        .map_err(err)
    }

    /// Parse an ISO-8601 datetime (`Z` / `±HH:MM` offset, or none for naive).
    #[napi(factory, js_name = "fromStr")]
    pub fn from_str(value: String) -> Result<Self> {
        CoreDateTime::from_str(&value)
            .map(|inner| DateTime { inner })
            .map_err(err)
    }

    /// The current instant in UTC.
    #[napi(factory)]
    pub fn now() -> Self {
        DateTime {
            inner: CoreDateTime::now(),
        }
    }

    /// Build from UTC epoch seconds, with an optional display zone.
    #[napi(factory, js_name = "fromEpochSeconds")]
    pub fn from_epoch_seconds(seconds: i64, timezone: Option<String>) -> Result<Self> {
        Ok(DateTime {
            inner: CoreDateTime::from_epoch_seconds(seconds, zone(timezone)?),
        })
    }

    /// Build from UTC epoch nanoseconds (a BigInt), with an optional display zone.
    #[napi(factory, js_name = "fromEpochNanos")]
    pub fn from_epoch_nanos(nanos: BigInt, timezone: Option<String>) -> Result<Self> {
        Ok(DateTime {
            inner: CoreDateTime::from_epoch_nanos(nanos.get_i128().0, zone(timezone)?),
        })
    }

    /// Build from an object (date/time components plus optional `timezone`).
    #[napi(factory, js_name = "fromMapping")]
    pub fn from_mapping(fields: HashMap<String, String>) -> Result<Self> {
        CoreDateTime::from_mapping(&to_mapping(fields))
            .map(|inner| DateTime { inner })
            .map_err(err)
    }

    #[napi(getter, js_name = "epochSeconds")]
    pub fn epoch_seconds(&self) -> i64 {
        self.inner.epoch_seconds()
    }

    #[napi(getter, js_name = "epochMillis")]
    pub fn epoch_millis(&self) -> i64 {
        self.inner.epoch_millis()
    }

    /// UTC epoch nanoseconds (a BigInt).
    #[napi(getter, js_name = "epochNanos")]
    pub fn epoch_nanos(&self) -> BigInt {
        BigInt::from(self.inner.epoch_nanos())
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

    /// The offset east of UTC (seconds) at this instant in its display zone.
    #[napi(getter, js_name = "offsetSeconds")]
    pub fn offset_seconds(&self) -> i32 {
        self.inner.offset_seconds()
    }

    /// The display `Timezone`, or null if naive.
    #[napi(getter)]
    pub fn timezone(&self) -> Option<Timezone> {
        self.inner
            .timezone()
            .cloned()
            .map(|inner| Timezone { inner })
    }

    /// The local `Date`.
    #[napi]
    pub fn date(&self) -> Date {
        Date {
            inner: self.inner.date(),
        }
    }

    /// The local `Time`.
    #[napi]
    pub fn time(&self) -> Time {
        Time {
            inner: self.inner.time(),
        }
    }

    /// The same instant displayed in `timezone`.
    #[napi(js_name = "toTimezone")]
    pub fn to_timezone(&self, timezone: String) -> Result<Self> {
        let tz = CoreTimezone::from_str(&timezone).map_err(err)?;
        Ok(DateTime {
            inner: self.inner.to_timezone(tz),
        })
    }

    /// The same instant displayed in UTC.
    #[napi(js_name = "toUtc")]
    pub fn to_utc(&self) -> Self {
        DateTime {
            inner: self.inner.to_utc(),
        }
    }

    /// Parse flexibly (ISO, date-only → midnight, bare integer → epoch seconds);
    /// with `raiseError = false` return `null` instead of throwing.
    #[napi]
    pub fn parse(value: String, raise_error: Option<bool>) -> Result<Option<Self>> {
        match CoreDateTime::from_str(&value) {
            Ok(inner) => Ok(Some(DateTime { inner })),
            Err(e) if raise_error.unwrap_or(true) => Err(err(e)),
            Err(_) => Ok(None),
        }
    }

    /// Render to a component object.
    #[napi(js_name = "toMapping")]
    pub fn to_mapping(&self) -> HashMap<String, String> {
        self.inner.to_mapping().into_iter().collect()
    }

    /// Compare by instant: `-1`, `0` or `1`.
    #[napi]
    pub fn compare(&self, other: &DateTime) -> i32 {
        match self.inner.cmp(&other.inner) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }

    /// `true` if the two instants (and zones) are equal.
    #[napi]
    pub fn equals(&self, other: &DateTime) -> bool {
        self.inner == other.inner
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.to_str()
    }

    /// Serialise to JSON as the canonical ISO string.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> String {
        self.inner.to_str()
    }

    /// Reconstruct from the string produced by `toJSON`.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: String) -> Result<Self> {
        DateTime::from_str(value)
    }
}
