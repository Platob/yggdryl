//! The `Time` napi class — a time of day with nanosecond resolution.

use std::collections::HashMap;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_core::Time as CoreTime;

use crate::{err, to_mapping};

/// A time of day (no date or timezone), with nanosecond resolution.
#[napi]
pub struct Time {
    pub(crate) inner: CoreTime,
}

#[napi]
impl Time {
    /// Build from `hour:minute:second` plus optional sub-second nanoseconds.
    #[napi(constructor)]
    pub fn new(hour: u32, minute: u32, second: u32, nano: Option<u32>) -> Result<Self> {
        CoreTime::from_hms_nano(hour, minute, second, nano.unwrap_or(0))
            .map(|inner| Time { inner })
            .map_err(err)
    }

    /// Parse `HH:MM[:SS[.fraction]]`.
    #[napi(factory, js_name = "fromStr")]
    pub fn from_str(value: String) -> Result<Self> {
        CoreTime::from_str(&value)
            .map(|inner| Time { inner })
            .map_err(err)
    }

    /// Build from an object (`hour` / `minute` / `second` / `nanosecond`).
    #[napi(factory, js_name = "fromMapping")]
    pub fn from_mapping(fields: HashMap<String, String>) -> Result<Self> {
        CoreTime::from_mapping(&to_mapping(fields))
            .map(|inner| Time { inner })
            .map_err(err)
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

    /// Render to an object (`hour` / `minute` / `second` / `nanosecond`).
    #[napi(js_name = "toMapping")]
    pub fn to_mapping(&self) -> HashMap<String, String> {
        self.inner.to_mapping().into_iter().collect()
    }

    /// The canonical string as bytes.
    #[napi(js_name = "toBytes")]
    pub fn to_bytes(&self) -> Buffer {
        self.inner.to_bytes().into()
    }

    /// Compare from midnight: `-1`, `0` or `1`.
    #[napi]
    pub fn compare(&self, other: &Time) -> i32 {
        match self.inner.cmp(&other.inner) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }

    /// `true` if the two times are equal.
    #[napi]
    pub fn equals(&self, other: &Time) -> bool {
        self.inner == other.inner
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.to_str()
    }

    /// Serialise to JSON as the canonical string.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> String {
        self.inner.to_str()
    }

    /// Reconstruct from the string produced by `toJSON`.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: String) -> Result<Self> {
        Time::from_str(value)
    }
}
