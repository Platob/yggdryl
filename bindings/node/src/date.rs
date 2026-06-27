//! The `Date` napi class — a proleptic-Gregorian calendar date.

use std::collections::HashMap;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_core::Date as CoreDate;

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

    /// Parse an ISO `YYYY-MM-DD` date.
    #[napi(factory, js_name = "fromStr")]
    pub fn from_str(value: String) -> Result<Self> {
        CoreDate::from_str(&value)
            .map(|inner| Date { inner })
            .map_err(err)
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
        Date::from_str(value)
    }
}
