//! Node wrapper for [`yggdryl_core::Utf8`].

use std::collections::{BTreeMap, HashMap};

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;
use yggdryl_core::{BinaryBased, DataType, Utf8 as CoreUtf8};

use crate::to_napi_err;

/// Arrow's variable-length UTF-8 string type (`string` / `large_string`).
///
/// Named `Utf8` to stay clear of the JavaScript `String` global; `fromBytes` and
/// the core parser also accept the aliases `"utf8"` / `"large_utf8"`.
#[napi]
pub struct Utf8 {
    pub(crate) inner: CoreUtf8,
}

#[napi]
impl Utf8 {
    #[napi(constructor)]
    pub fn new(large: Option<bool>) -> Self {
        Utf8 {
            inner: if large.unwrap_or(false) {
                CoreUtf8::large()
            } else {
                CoreUtf8::new()
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

    /// The component map `{"type": …}`.
    #[napi]
    pub fn to_mapping(&self) -> HashMap<String, String> {
        self.inner.to_mapping().into_iter().collect()
    }

    /// Reconstructs the type from its component map.
    #[napi(factory)]
    pub fn from_mapping(mapping: HashMap<String, String>) -> napi::Result<Utf8> {
        let mapping: BTreeMap<String, String> = mapping.into_iter().collect();
        CoreUtf8::from_mapping(&mapping)
            .map(|inner| Utf8 { inner })
            .map_err(to_napi_err)
    }

    /// The canonical byte form.
    #[napi]
    pub fn to_bytes(&self) -> Buffer {
        self.inner.to_bytes().into()
    }

    /// Reconstructs the type from its byte form.
    #[napi(factory)]
    pub fn from_bytes(data: Buffer) -> napi::Result<Utf8> {
        CoreUtf8::from_bytes(data.as_ref())
            .map(|inner| Utf8 { inner })
            .map_err(to_napi_err)
    }

    /// The JSON value (used by `JSON.stringify`).
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self.inner).expect("Utf8 serializes to JSON")
    }

    /// Reconstructs the type from its JSON value.
    #[napi(js_name = "fromJSON", factory)]
    pub fn from_json(value: serde_json::Value) -> napi::Result<Utf8> {
        serde_json::from_value(value)
            .map(|inner| Utf8 { inner })
            .map_err(to_napi_err)
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.to_str().into_owned()
    }

    /// Structural equality with another `Utf8`.
    #[napi]
    pub fn equals(&self, other: &Utf8) -> bool {
        self.inner == other.inner
    }
}
