//! The `Duration` napi class — a signed span of time (nanoseconds).

use std::collections::HashMap;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_core::{Duration as CoreDuration, TimeUnit};

use crate::{bigint_i128, err, to_mapping};

/// A signed span of time with nanosecond resolution.
#[napi]
pub struct Duration {
    pub(crate) inner: CoreDuration,
}

#[napi]
impl Duration {
    /// Build from a count of nanoseconds (a BigInt).
    #[napi(constructor)]
    pub fn new(nanos: Option<BigInt>) -> Result<Self> {
        let nanos = match nanos {
            Some(b) => bigint_i128(b)?,
            None => 0,
        };
        Ok(Duration {
            inner: CoreDuration::from_nanos(nanos),
        })
    }

    /// Parse a span flexibly (compact `"1h30m"` / `"1s500ms"` / `"-2d"`, ISO-8601
    /// `"PT15M"` / `"P1D"`, or seconds), throwing on malformed input.
    #[napi(factory, js_name = "fromStr")]
    pub fn from_str(value: String) -> Result<Self> {
        CoreDuration::from_str(&value)
            .map(|inner| Duration { inner })
            .map_err(err)
    }

    /// A span of `seconds` seconds.
    #[napi(factory, js_name = "fromSecs")]
    pub fn from_secs(seconds: i64) -> Self {
        Duration {
            inner: CoreDuration::from_secs(seconds),
        }
    }

    /// A span of `millis` milliseconds.
    #[napi(factory, js_name = "fromMillis")]
    pub fn from_millis(millis: i64) -> Self {
        Duration {
            inner: CoreDuration::from_millis(millis),
        }
    }

    /// A span of `micros` microseconds.
    #[napi(factory, js_name = "fromMicros")]
    pub fn from_micros(micros: i64) -> Self {
        Duration {
            inner: CoreDuration::from_micros(micros),
        }
    }

    /// A span of `nanos` nanoseconds (a BigInt).
    #[napi(factory, js_name = "fromNanos")]
    pub fn from_nanos(nanos: BigInt) -> Result<Self> {
        Ok(Duration {
            inner: CoreDuration::from_nanos(bigint_i128(nanos)?),
        })
    }

    /// A span of `value` of the given unit (`"s"` / `"ms"` / `"us"` / `"ns"`).
    #[napi(factory, js_name = "fromUnit")]
    pub fn from_unit(value: i64, unit: String) -> Result<Self> {
        let unit = TimeUnit::from_str(&unit).map_err(err)?;
        Ok(Duration {
            inner: CoreDuration::from_unit(value, unit),
        })
    }

    /// Build from an object (`nanoseconds`).
    #[napi(factory, js_name = "fromMapping")]
    pub fn from_mapping(fields: HashMap<String, String>) -> Result<Self> {
        CoreDuration::from_mapping(&to_mapping(fields))
            .map(|inner| Duration { inner })
            .map_err(err)
    }

    /// The whole seconds (truncated toward zero).
    #[napi(js_name = "asSeconds")]
    pub fn as_seconds(&self) -> i64 {
        self.inner.as_seconds()
    }

    /// The total milliseconds (a BigInt, truncated toward zero).
    #[napi(js_name = "asMillis")]
    pub fn as_millis(&self) -> BigInt {
        BigInt::from(self.inner.as_millis())
    }

    /// The total microseconds (a BigInt, truncated toward zero).
    #[napi(js_name = "asMicros")]
    pub fn as_micros(&self) -> BigInt {
        BigInt::from(self.inner.as_micros())
    }

    /// The total nanoseconds (a BigInt).
    #[napi(js_name = "asNanos")]
    pub fn as_nanos(&self) -> BigInt {
        BigInt::from(self.inner.as_nanos())
    }

    /// The span as fractional seconds.
    #[napi(js_name = "asSecondsF64")]
    pub fn as_seconds_f64(&self) -> f64 {
        self.inner.as_seconds_f64()
    }

    #[napi(getter, js_name = "isZero")]
    pub fn is_zero(&self) -> bool {
        self.inner.is_zero()
    }

    #[napi(getter, js_name = "isNegative")]
    pub fn is_negative(&self) -> bool {
        self.inner.is_negative()
    }

    /// The absolute (non-negative) span.
    #[napi]
    pub fn abs(&self) -> Self {
        Duration {
            inner: self.inner.abs(),
        }
    }

    /// The negated span.
    #[napi]
    pub fn negate(&self) -> Self {
        Duration {
            inner: self.inner.negate(),
        }
    }

    /// The sum of two spans.
    #[napi]
    pub fn add(&self, other: &Duration) -> Self {
        Duration {
            inner: self.inner.add(&other.inner),
        }
    }

    /// The span scaled by an integer `factor`.
    #[napi]
    pub fn mul(&self, factor: i64) -> Self {
        Duration {
            inner: self.inner.mul(factor),
        }
    }

    /// The span divided by an integer `divisor` (dividing by zero yields zero).
    #[napi]
    pub fn div(&self, divisor: i64) -> Self {
        Duration {
            inner: self.inner.div(divisor),
        }
    }

    /// The difference of two spans.
    #[napi]
    pub fn sub(&self, other: &Duration) -> Self {
        Duration {
            inner: self.inner.sub(&other.inner),
        }
    }

    /// Render to an object (`nanoseconds`).
    #[napi(js_name = "toMapping")]
    pub fn to_mapping(&self) -> HashMap<String, String> {
        self.inner.to_mapping().into_iter().collect()
    }

    /// Compare by length: `-1`, `0` or `1`.
    #[napi]
    pub fn compare(&self, other: &Duration) -> i32 {
        match self.inner.cmp(&other.inner) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }

    /// `true` if the two spans are equal.
    #[napi]
    pub fn equals(&self, other: &Duration) -> bool {
        self.inner == other.inner
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.to_str()
    }

    /// Serialise to JSON as the compact string.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> String {
        self.inner.to_str()
    }

    /// Reconstruct from the string produced by `toJSON`.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: String) -> Result<Self> {
        CoreDuration::from_str(&value)
            .map(|inner| Duration { inner })
            .map_err(err)
    }
}
