//! The `Date` napi class — a proleptic-Gregorian calendar date.

use std::collections::HashMap;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_core::{Date as CoreDate, Temporal, Timezone as CoreTimezone};

use crate::datetime::DateTime;
use crate::duration::Duration;
use crate::time::Time;
use crate::timezone::Timezone;
use crate::{err, to_mapping};

/// A calendar date (no time of day or timezone), stored as days since the epoch.
#[napi]
pub struct Date {
    pub(crate) inner: CoreDate,
}

#[napi]
impl Date {
    /// Build from `(year, month, day)`, validating the calendar.
    #[napi(constructor)]
    pub fn new(year: i32, month: u32, day: u32) -> Result<Self> {
        CoreDate::from_ymd(year, month, day)
            .map(|inner| Date { inner })
            .map_err(err)
    }

    /// Parse a date flexibly (ISO `YYYY-MM-DD`, `YYYY/MM/DD`, compact `YYYYMMDD` or a
    /// full datetime), throwing on malformed input.
    #[napi(factory, js_name = "fromStr")]
    pub fn from_str(value: String) -> Result<Self> {
        CoreDate::from_str(&value)
            .map(|inner| Date { inner })
            .map_err(err)
    }

    /// Build from a `DateTime` (its local calendar date) — the `Temporal` redirect.
    #[napi(factory, js_name = "fromDatetime")]
    pub fn from_datetime(value: &DateTime) -> Self {
        Date {
            inner: CoreDate::from_datetime(&value.inner),
        }
    }

    /// Build from a count of days since the UNIX epoch.
    #[napi(factory, js_name = "fromEpochDays")]
    pub fn from_epoch_days(days: i32) -> Self {
        Date {
            inner: CoreDate::from_epoch_days(days),
        }
    }

    /// Build from an object (`year` / `month` / `day`).
    #[napi(factory, js_name = "fromMapping")]
    pub fn from_mapping(fields: HashMap<String, String>) -> Result<Self> {
        CoreDate::from_mapping(&to_mapping(fields))
            .map(|inner| Date { inner })
            .map_err(err)
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

    /// The day of week (0 = Sunday … 6 = Saturday).
    #[napi(getter)]
    pub fn weekday(&self) -> u32 {
        self.inner.weekday()
    }

    /// Days since the UNIX epoch.
    #[napi(getter, js_name = "epochDays")]
    pub fn epoch_days(&self) -> i32 {
        self.inner.epoch_days()
    }

    /// A copy `days` days later (or earlier, if negative).
    #[napi(js_name = "addDays")]
    pub fn add_days(&self, days: i32) -> Self {
        Date {
            inner: self.inner.add_days(days),
        }
    }

    /// This date advanced by a `Duration`'s whole days.
    #[napi]
    pub fn add(&self, span: &Duration) -> Self {
        Date {
            inner: self.inner.add(&span.inner),
        }
    }

    /// This date moved back by a `Duration`'s whole days.
    #[napi]
    pub fn sub(&self, span: &Duration) -> Self {
        Date {
            inner: self.inner.sub(&span.inner),
        }
    }

    /// The signed whole-day `Duration` from `other` to this date.
    #[napi(js_name = "durationSince")]
    pub fn duration_since(&self, other: &Date) -> Duration {
        Duration {
            inner: self.inner.duration_since(&other.inner),
        }
    }

    /// This date floored to a multiple of `unit` (whole days) since the epoch.
    #[napi]
    pub fn truncate(&self, unit: &Duration) -> Self {
        Date {
            inner: self.inner.truncate(&unit.inner),
        }
    }

    /// The timezone this date is anchored to, if any.
    #[napi(getter)]
    pub fn timezone(&self) -> Option<Timezone> {
        self.inner
            .timezone()
            .cloned()
            .map(|inner| Timezone { inner })
    }

    /// A copy anchored to the named timezone.
    #[napi(js_name = "withTimezone")]
    pub fn with_timezone(&self, timezone: String) -> Result<Self> {
        let tz = CoreTimezone::from_str(&timezone).map_err(err)?;
        Ok(Date {
            inner: self.inner.clone().with_timezone(tz),
        })
    }

    /// A copy with no timezone.
    #[napi(js_name = "withoutTimezone")]
    pub fn without_timezone(&self) -> Self {
        Date {
            inner: self.inner.clone().without_timezone(),
        }
    }

    /// Midnight on this date (in its timezone) as a `DateTime`.
    #[napi(js_name = "toDatetime")]
    pub fn to_datetime(&self) -> DateTime {
        DateTime {
            inner: self.inner.to_datetime(),
        }
    }

    /// Combine with a `Time` into a `DateTime` in the date's zone.
    #[napi]
    pub fn at(&self, time: &Time) -> DateTime {
        DateTime {
            inner: self.inner.at(time.inner),
        }
    }

    /// Render to an object (`year` / `month` / `day`).
    #[napi(js_name = "toMapping")]
    pub fn to_mapping(&self) -> HashMap<String, String> {
        self.inner.to_mapping().into_iter().collect()
    }

    /// The canonical string as bytes.
    #[napi(js_name = "toBytes")]
    pub fn to_bytes(&self) -> Buffer {
        self.inner.to_bytes().into()
    }

    /// Compare chronologically: `-1`, `0` or `1`.
    #[napi]
    pub fn compare(&self, other: &Date) -> i32 {
        match self.inner.cmp(&other.inner) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }

    /// `true` if the two dates are equal.
    #[napi]
    pub fn equals(&self, other: &Date) -> bool {
        self.inner == other.inner
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.to_str()
    }

    /// Serialise to JSON as the ISO string.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> String {
        self.inner.to_str()
    }

    /// Reconstruct from the string produced by `toJSON`.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: String) -> Result<Self> {
        CoreDate::from_str(&value)
            .map(|inner| Date { inner })
            .map_err(err)
    }
}
