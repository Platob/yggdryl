//! The native version value: a sixteen-bit major and minor, and a text patch.

use napi::bindgen_prelude::{Either, Result};
use napi_derive::napi;
use yggdryl::{Scalar, Version};

use crate::{exact_i64, exact_u64, napi_error, ordering_value};

/// An immutable native version with major, minor, and patch components.
#[napi(js_name = "Version")]
#[derive(Clone)]
pub struct JsVersion {
    pub(crate) inner: Version,
}

/// A major or minor: a whole number the sixteen bits hold, refused by name.
///
/// A whole number of any size past the sixteen bits is refused by the range it
/// missed; `exact_i64` refuses what is no whole number at all.
fn component(value: f64, name: &str) -> Result<u16> {
    let refused = || napi_error(format!("{name} must be in 0..65535"));
    if value.fract() == 0.0 && !(0.0..=65535.0).contains(&value) {
        return Err(refused());
    }
    u16::try_from(exact_i64(value, name)?).map_err(|_| refused())
}

#[napi]
impl JsVersion {
    /// Major and minor are whole numbers in 0..65535. A patch is text as
    /// written or a non-negative whole number, held as its digits; zero or
    /// empty text states none.
    #[napi(constructor)]
    pub fn new(major: f64, minor: Option<f64>, patch: Option<Either<f64, String>>) -> Result<Self> {
        let major = component(major, "major")?;
        let minor = component(minor.unwrap_or(0.0), "minor")?;
        let patch = match patch {
            None => None,
            Some(Either::A(number)) => Some(exact_u64(number, "patch")?.to_string()),
            Some(Either::B(text)) => Some(text),
        };
        Ok(Self {
            inner: Version::new(major, minor, patch.as_deref()),
        })
    }

    /// Parse the native version grammar: a strict major and minor, then the
    /// patch as the tail states it.
    #[allow(clippy::should_implement_trait)] // JavaScript passes an owned String through NAPI.
    #[napi(factory)]
    pub fn from_str(text: String) -> Result<Self> {
        text.parse().map(|inner| Self { inner }).map_err(napi_error)
    }

    /// The sixteen-bit major component.
    #[napi(getter)]
    pub fn major(&self) -> u16 {
        self.inner.major()
    }

    /// The sixteen-bit minor component, zero when omitted.
    #[napi(getter)]
    pub fn minor(&self) -> u16 {
        self.inner.minor()
    }

    /// The patch as text - a number as its digits - or null when none.
    #[napi(getter)]
    pub fn patch(&self) -> Option<String> {
        self.inner.patch().map(str::to_owned)
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
        Scalar::from(self.inner.clone()).stable_hash()
    }

    /// Copy the native value.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
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
