//! Node wrapper for [`yggdryl_core::BinaryScalar`].

use napi::bindgen_prelude::Buffer;
use napi::Either;
use napi_derive::napi;
use yggdryl_core::{BinaryScalar as CoreBinaryScalar, Scalar};

use crate::{anytype_to_either, to_napi_err, Binary, Utf8};

/// A single binary value, or null.
#[napi]
pub struct BinaryScalar {
    pub(crate) inner: CoreBinaryScalar,
}

#[napi]
impl BinaryScalar {
    #[napi(constructor)]
    pub fn new(value: Option<Buffer>) -> Self {
        BinaryScalar {
            inner: match value {
                Some(bytes) => CoreBinaryScalar::new(bytes.as_ref()),
                None => CoreBinaryScalar::null(),
            },
        }
    }

    /// The null `binary` scalar.
    #[napi(factory)]
    pub fn null() -> Self {
        BinaryScalar {
            inner: CoreBinaryScalar::null(),
        }
    }

    /// Whether the scalar holds the null value.
    #[napi(getter)]
    pub fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The scalar's data type (a `Binary` object).
    #[napi(getter)]
    pub fn data_type(&self) -> Either<Binary, Utf8> {
        anytype_to_either(&self.inner.data_type())
    }

    /// The scalar's bytes, or `null` if null.
    #[napi(getter)]
    pub fn value(&self) -> Option<Buffer> {
        self.inner.as_bytes().map(|bytes| bytes.to_vec().into())
    }

    /// The number of bytes (`0` if null).
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        self.inner.len().unwrap_or(0) as u32
    }

    /// The JSON value (used by `JSON.stringify`).
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.inner).expect("BinaryScalar serializes to JSON")
    }

    /// Reconstructs a scalar from its JSON value.
    #[napi(js_name = "fromJSON", factory)]
    pub fn from_json(value: serde_json::Value) -> napi::Result<BinaryScalar> {
        serde_json::from_value(value)
            .map(|inner| BinaryScalar { inner })
            .map_err(to_napi_err)
    }

    /// Structural equality with another `BinaryScalar`.
    #[napi]
    pub fn equals(&self, other: &BinaryScalar) -> bool {
        self.inner == other.inner
    }
}
