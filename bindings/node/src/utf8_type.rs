//! Node wrapper for the string [`yggdryl_core::Utf8Type`] data type.

use std::collections::{BTreeMap, HashMap};

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;
use yggdryl_core::{BinaryBased, DataType, Jsonable, Utf8Type as CoreUtf8Type};

use crate::to_napi_err;

/// Arrow's variable-length UTF-8 string type (`string` / `large_string`). The
/// in-memory string *value* is `Utf8`. `fromStr` accepts the aliases `"utf8"` /
/// `"large_utf8"`.
#[napi]
pub struct Utf8Type {
    pub(crate) inner: CoreUtf8Type,
}

#[napi]
impl Utf8Type {
    #[napi(constructor)]
    pub fn new(large: Option<bool>) -> Self {
        Utf8Type {
            inner: if large.unwrap_or(false) {
                CoreUtf8Type::large()
            } else {
                CoreUtf8Type::new()
            },
        }
    }

    /// The canonical type name (`"string"` or `"large_string"`).
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.type_name().to_string()
    }

    /// Whether offsets are 64-bit (`large_string`).
    #[napi(getter)]
    pub fn is_large(&self) -> bool {
        self.inner.is_large()
    }

    /// Whether the bytes are guaranteed UTF-8 (always `true` for strings).
    #[napi(getter)]
    pub fn is_utf8(&self) -> bool {
        self.inner.is_utf8()
    }

    /// The canonical string form.
    #[napi]
    pub fn to_str(&self) -> String {
        self.inner.to_str().into_owned()
    }

    /// Reconstructs the type from its canonical string (accepts the aliases).
    #[napi(js_name = "fromStr", factory)]
    pub fn from_str(value: String) -> napi::Result<Utf8Type> {
        CoreUtf8Type::from_str(&value)
            .map(|inner| Utf8Type { inner })
            .map_err(to_napi_err)
    }

    /// The component map `{"type": …}`.
    #[napi]
    pub fn to_mapping(&self) -> HashMap<String, String> {
        self.inner.to_mapping().into_iter().collect()
    }

    /// Reconstructs the type from its component map.
    #[napi(factory)]
    pub fn from_mapping(mapping: HashMap<String, String>) -> napi::Result<Utf8Type> {
        let mapping: BTreeMap<String, String> = mapping.into_iter().collect();
        CoreUtf8Type::from_mapping(&mapping)
            .map(|inner| Utf8Type { inner })
            .map_err(to_napi_err)
    }

    /// The canonical byte form.
    #[napi]
    pub fn to_bytes(&self) -> Buffer {
        self.inner.to_bytes().into()
    }

    /// Reconstructs the type from its byte form.
    #[napi(factory)]
    pub fn from_bytes(data: Buffer) -> napi::Result<Utf8Type> {
        CoreUtf8Type::from_bytes(data.as_ref())
            .map(|inner| Utf8Type { inner })
            .map_err(to_napi_err)
    }

    /// The JSON value (used by `JSON.stringify`).
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self.inner).expect("Utf8Type serializes to JSON")
    }

    /// The JSON string, formatted per the global `JsonParams`.
    #[napi(js_name = "toJsonString")]
    pub fn to_json_string(&self) -> String {
        self.inner.to_json()
    }

    /// Reconstructs the type from its JSON value.
    #[napi(js_name = "fromJSON", factory)]
    pub fn from_json(value: serde_json::Value) -> napi::Result<Utf8Type> {
        serde_json::from_value(value)
            .map(|inner| Utf8Type { inner })
            .map_err(to_napi_err)
    }

    /// The JSON bytes (JSON text encoded with the global charset).
    #[napi]
    pub fn to_bson(&self) -> Buffer {
        self.inner.to_bson().into()
    }

    /// Reconstructs the type from its JSON bytes.
    #[napi(factory)]
    pub fn from_bson(data: Buffer) -> napi::Result<Utf8Type> {
        CoreUtf8Type::from_bson(data.as_ref())
            .map(|inner| Utf8Type { inner })
            .map_err(to_napi_err)
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.to_str().into_owned()
    }

    /// Structural equality with another `Utf8Type`.
    #[napi]
    pub fn equals(&self, other: &Utf8Type) -> bool {
        self.inner == other.inner
    }
}
