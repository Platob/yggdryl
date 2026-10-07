//! The native ISO 10383 market identifier code.

use napi::bindgen_prelude::Result;
use napi_derive::napi;
use yggdryl::{DataType, Mic, Scalar};

use crate::napi_error;

/// A market identifier code as the `mic` datatype reads one.
pub(crate) fn mic_of(text: &str) -> Result<Mic> {
    match DataType::Mic
        .scalar(Scalar::from(text))
        .map_err(napi_error)?
    {
        Scalar::Mic(mic) => Ok(mic),
        other => Err(napi_error(format!(
            "expected a market identifier code, got {}",
            other.kind()
        ))),
    }
}

/// One ISO 10383 market identifier code, held by its shape: at most four
/// ASCII bytes, read as the `mic` datatype reads one. Immutable.
#[napi(js_name = "Mic")]
#[derive(Clone)]
pub struct JsMic {
    inner: Mic,
}

#[napi]
impl JsMic {
    /// The code `code` spells, refused where the `mic` datatype refuses it.
    #[napi(constructor)]
    pub fn new(code: String) -> Result<Self> {
        mic_of(&code).map(|inner| Self { inner })
    }

    /// Whether this is the market stated as none, `XXXX`.
    #[napi(getter)]
    pub fn is_none(&self) -> bool {
        self.inner.is_none()
    }

    /// The operating MIC this code trades under in ISO 10383: itself for an
    /// operating MIC, its market's for a segment, `null` for a code the
    /// registry never assigned. An expired code answers too.
    #[napi(getter)]
    pub fn operating(&self) -> Option<String> {
        self.inner.operating().map(|mic| mic.to_string())
    }

    /// Whether ISO 10383 lists this code as a segment of another market:
    /// false for an operating MIC and for a code it never assigned.
    #[napi(getter)]
    pub fn is_segment(&self) -> bool {
        self.inner.is_segment()
    }

    /// The country ISO 10383 places this code in, or `null` where the
    /// registry never assigned it or places it in no single country - `XOFF`
    /// and `XXXX` answer none.
    #[napi(getter)]
    pub fn country(&self) -> Option<String> {
        self.inner.country().map(|country| country.to_string())
    }

    /// Whether `other` is the same code.
    #[napi]
    pub fn equals(&self, other: &JsMic) -> bool {
        self.inner == other.inner
    }

    /// The code.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    /// The code, as JSON states it.
    #[napi(js_name = "toJSON")]
    pub fn js_json(&self) -> String {
        self.inner.to_string()
    }
}
