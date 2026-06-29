//! Node wrapper for the in-memory string value [`yggdryl_core::Utf8`].

use std::collections::{BTreeMap, HashMap};

use napi::bindgen_prelude::Buffer;
use napi::Either;
use napi_derive::napi;
use yggdryl_core::{Jsonable, Scalar, Utf8 as CoreUtf8, Utf8Type as CoreUtf8Type};

use crate::{anyscalar_to_either, anytype_from_either, to_napi_err, Binary, BinaryType, Utf8Type};

/// A validated, in-memory UTF-8 string value. Equality is content-based; `cast`
/// converts to a `Binary` for byte IO.
#[napi]
pub struct Utf8 {
    pub(crate) inner: CoreUtf8,
}

#[napi]
impl Utf8 {
    #[napi(constructor)]
    pub fn new(value: Option<String>, large: Option<bool>) -> Self {
        let mut inner = CoreUtf8::new(value.unwrap_or_default());
        if large.unwrap_or(false) {
            inner = inner.with_data_type(CoreUtf8Type::large());
        }
        Utf8 { inner }
    }

    /// The scalar's data type (a `Utf8Type` object).
    #[napi(getter)]
    pub fn data_type(&self) -> Utf8Type {
        Utf8Type {
            inner: self.inner.string_type(),
        }
    }

    /// The string value.
    #[napi(getter)]
    pub fn value(&self) -> String {
        self.inner.as_str().to_string()
    }

    /// The number of UTF-8 bytes.
    #[napi(getter)]
    pub fn length(&self) -> f64 {
        self.inner.len() as f64
    }

    /// Returns a copy carrying the given `string` type variant.
    #[napi]
    pub fn with_data_type(&self, data_type: &Utf8Type) -> Utf8 {
        Utf8 {
            inner: self.inner.with_data_type(data_type.inner),
        }
    }

    /// The string's raw UTF-8 bytes.
    #[napi]
    pub fn to_bytes(&self) -> Buffer {
        self.inner.to_bytes().into()
    }

    /// A `string` value holding a copy of `data`, validating UTF-8.
    #[napi(factory)]
    pub fn from_bytes(data: Buffer) -> napi::Result<Utf8> {
        CoreUtf8::from_bytes(data.as_ref())
            .map(|inner| Utf8 { inner })
            .map_err(to_napi_err)
    }

    /// The component map (`type`, plus the `value` text).
    #[napi]
    pub fn to_mapping(&self) -> HashMap<String, String> {
        self.inner.to_mapping().into_iter().collect()
    }

    /// Reconstructs a value from its component map.
    #[napi(factory)]
    pub fn from_mapping(mapping: HashMap<String, String>) -> napi::Result<Utf8> {
        let mapping: BTreeMap<String, String> = mapping.into_iter().collect();
        CoreUtf8::from_mapping(&mapping)
            .map(|inner| Utf8 { inner })
            .map_err(to_napi_err)
    }

    /// The JSON value (used by `JSON.stringify`).
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.inner).expect("Utf8 serializes to JSON")
    }

    /// The JSON string, formatted per the global `JsonParams`.
    #[napi(js_name = "toJsonString")]
    pub fn to_json_string(&self) -> String {
        self.inner.to_json()
    }

    /// Reconstructs a value from its JSON value.
    #[napi(js_name = "fromJSON", factory)]
    pub fn from_json(value: serde_json::Value) -> napi::Result<Utf8> {
        serde_json::from_value(value)
            .map(|inner| Utf8 { inner })
            .map_err(to_napi_err)
    }

    /// The JSON bytes (JSON text encoded with the global charset).
    #[napi]
    pub fn to_bson(&self) -> Buffer {
        self.inner.to_bson().into()
    }

    /// Reconstructs a value from its JSON bytes.
    #[napi(factory)]
    pub fn from_bson(data: Buffer) -> napi::Result<Utf8> {
        CoreUtf8::from_bson(data.as_ref())
            .map(|inner| Utf8 { inner })
            .map_err(to_napi_err)
    }

    /// Casts the value to `dataType`, returning a new `Binary` or `Utf8`.
    #[napi]
    pub fn cast(
        &self,
        data_type: Either<&BinaryType, &Utf8Type>,
    ) -> napi::Result<Either<Binary, Utf8>> {
        let data_type = anytype_from_either(data_type);
        let scalar = self.inner.cast(&data_type).map_err(to_napi_err)?;
        Ok(anyscalar_to_either(scalar))
    }

    /// Sets the data type in place (same-family only); use `cast` to convert.
    #[napi]
    pub fn set_data_type(&mut self, data_type: Either<&BinaryType, &Utf8Type>) -> napi::Result<()> {
        let data_type = anytype_from_either(data_type);
        self.inner.set_data_type(&data_type).map_err(to_napi_err)
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.as_str().to_string()
    }

    /// Structural equality (content + type) with another `Utf8`.
    #[napi]
    pub fn equals(&self, other: &Utf8) -> bool {
        self.inner == other.inner
    }
}
