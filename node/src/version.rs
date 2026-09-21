//! The native four-byte numeric version value.

use napi::bindgen_prelude::Result;
use napi_derive::napi;
use yggdryl::{Scalar, Version};

use crate::{exact_i32, napi_error, ordering_value};

/// An immutable native version with major, minor, and patch components.
#[napi(js_name = "Version")]
#[derive(Clone)]
pub struct JsVersion {
    pub(crate) inner: Version,
}

#[napi]
impl JsVersion {
    /// Major and minor fit unsigned bytes; patch fits an unsigned 16-bit value.
    #[napi(constructor)]
    pub fn new(major: f64, minor: Option<f64>, patch: Option<f64>) -> Result<Self> {
        let major = u8::try_from(exact_i32(major, "major")?)
            .map_err(|_| napi_error("major must be in 0..255"))?;
        let minor = u8::try_from(exact_i32(minor.unwrap_or(0.0), "minor")?)
            .map_err(|_| napi_error("minor must be in 0..255"))?;
        let patch = u16::try_from(exact_i32(patch.unwrap_or(0.0), "patch")?)
            .map_err(|_| napi_error("patch must be in 0..65535"))?;
        Ok(Self {
            inner: Version::new(major, minor, patch),
        })
    }

    /// Parse the native numeric version grammar.
    #[allow(clippy::should_implement_trait)] // JavaScript passes an owned String through NAPI.
    #[napi(factory)]
    pub fn from_str(text: String) -> Result<Self> {
        text.parse().map(|inner| Self { inner }).map_err(napi_error)
    }

    /// The unsigned 8-bit major component.
    #[napi(getter)]
    pub fn major(&self) -> u8 {
        self.inner.major()
    }

    /// The unsigned 8-bit minor component.
    #[napi(getter)]
    pub fn minor(&self) -> u8 {
        self.inner.minor()
    }

    /// The unsigned 16-bit patch component.
    #[napi(getter)]
    pub fn patch(&self) -> u16 {
        self.inner.patch()
    }

    /// Compare the complete native values.
    #[napi]
    pub fn equals(&self, other: &JsVersion) -> bool {
        self.inner == other.inner
    }

    /// Compare native values in canonical order.
    #[napi]
    pub fn compare(&self, other: &JsVersion) -> i32 {
        ordering_value(self.inner.cmp(&other.inner))
    }

    /// Deterministic hash bits from the native value.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        Scalar::from(self.inner).stable_hash()
    }

    /// Copy the native four-byte value.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        Self { inner: self.inner }
    }

    /// Render the canonical native text.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    /// Project the native JSON representation.
    #[napi(js_name = "toJSON")]
    pub fn js_json(&self) -> String {
        self.inner.to_string()
    }
}
