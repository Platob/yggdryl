//! The `Timezone` napi class (UTC / fixed offset / named IANA zone with DST rules).

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_core::Timezone as CoreTimezone;

use crate::err;

/// A timezone: `"UTC"`, a `"+HH:MM"` offset, or a named IANA zone (DST-aware).
#[napi]
pub struct Timezone {
    pub(crate) inner: CoreTimezone,
}

#[napi]
impl Timezone {
    /// Parse `"UTC"` / `"Z"`, a `±HH:MM` offset, an IANA name or a POSIX TZ string.
    #[napi(constructor)]
    pub fn new(name: String) -> Result<Self> {
        CoreTimezone::from_str(&name)
            .map(|inner| Timezone { inner })
            .map_err(err)
    }

    /// Alias for the constructor.
    #[napi(factory, js_name = "fromStr")]
    pub fn from_str(name: String) -> Result<Self> {
        Timezone::new(name)
    }

    /// The UTC zone.
    #[napi(factory)]
    pub fn utc() -> Self {
        Timezone {
            inner: CoreTimezone::Utc,
        }
    }

    /// A fixed offset east of UTC, in seconds.
    #[napi(factory)]
    pub fn fixed(offset_seconds: i32) -> Self {
        Timezone {
            inner: if offset_seconds == 0 {
                CoreTimezone::Utc
            } else {
                CoreTimezone::Fixed(offset_seconds)
            },
        }
    }

    /// The offset east of UTC (seconds) at the given UTC epoch second (DST-aware).
    #[napi(js_name = "offsetSeconds")]
    pub fn offset_seconds(&self, utc_epoch_seconds: i64) -> i32 {
        self.inner.offset_seconds(utc_epoch_seconds)
    }

    /// The canonical name / offset string.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name()
    }

    /// Whether this is UTC.
    #[napi(getter, js_name = "isUtc")]
    pub fn is_utc(&self) -> bool {
        self.inner.is_utc()
    }

    /// Whether this is a fixed offset.
    #[napi(getter, js_name = "isFixed")]
    pub fn is_fixed(&self) -> bool {
        self.inner.is_fixed()
    }

    /// `true` if the two zones are equal.
    #[napi]
    pub fn equals(&self, other: &Timezone) -> bool {
        self.inner == other.inner
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.name()
    }

    /// Serialise to JSON as the canonical zone string.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> String {
        self.inner.name()
    }

    /// Reconstruct from the string produced by `toJSON`.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: String) -> Result<Self> {
        Timezone::new(value)
    }
}
