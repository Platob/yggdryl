//! Node wrapper for [`yggdryl_core::StringScalar`].

use napi::Either;
use napi_derive::napi;
use yggdryl_core::{Scalar, StringScalar as CoreStringScalar};

use crate::{anytype_to_either, to_napi_err, Binary, Utf8};

/// A single UTF-8 string value, or null.
#[napi]
pub struct StringScalar {
    pub(crate) inner: CoreStringScalar,
}

#[napi]
impl StringScalar {
    #[napi(constructor)]
    pub fn new(value: Option<String>) -> Self {
        StringScalar {
            inner: match value {
                Some(text) => CoreStringScalar::new(text),
                None => CoreStringScalar::null(),
            },
        }
    }

    /// The null `string` scalar.
    #[napi(factory)]
    pub fn null() -> Self {
        StringScalar {
            inner: CoreStringScalar::null(),
        }
    }

    /// Whether the scalar holds the null value.
    #[napi(getter)]
    pub fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The scalar's data type (a `Utf8` object).
    #[napi(getter)]
    pub fn data_type(&self) -> Either<Binary, Utf8> {
        anytype_to_either(&self.inner.data_type())
    }

    /// The scalar's text, or `null` if null.
    #[napi(getter)]
    pub fn value(&self) -> Option<String> {
        self.inner.as_str().map(str::to_owned)
    }

    /// The number of UTF-8 bytes (`0` if null).
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        self.inner.as_bytes().map_or(0, <[u8]>::len) as u32
    }

    /// The JSON value (used by `JSON.stringify`).
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.inner).expect("StringScalar serializes to JSON")
    }

    /// Reconstructs a scalar from its JSON value.
    #[napi(js_name = "fromJSON", factory)]
    pub fn from_json(value: serde_json::Value) -> napi::Result<StringScalar> {
        serde_json::from_value(value)
            .map(|inner| StringScalar { inner })
            .map_err(to_napi_err)
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.as_str().unwrap_or("").to_string()
    }

    /// Structural equality with another `StringScalar`.
    #[napi]
    pub fn equals(&self, other: &StringScalar) -> bool {
        self.inner == other.inner
    }
}
