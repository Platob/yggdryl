//! Node wrapper for the binary [`yggdryl_core::BinaryType`] data type.

use std::collections::{BTreeMap, HashMap};

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;
use yggdryl_core::{BinaryBased, BinaryType as CoreBinaryType, DataType};

use crate::to_napi_err;

/// Arrow's variable-length binary type (`binary` / `large_binary`). The in-memory
/// binary *value* is `Binary`.
#[napi]
pub struct BinaryType {
    pub(crate) inner: CoreBinaryType,
}

#[napi]
impl BinaryType {
    #[napi(constructor)]
    pub fn new(large: Option<bool>) -> Self {
        BinaryType {
            inner: if large.unwrap_or(false) {
                CoreBinaryType::large()
            } else {
                CoreBinaryType::new()
            },
        }
    }

    /// The canonical type name (`"binary"` or `"large_binary"`).
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.type_name().to_string()
    }

    /// Whether offsets are 64-bit (`large_binary`).
    #[napi(getter)]
    pub fn is_large(&self) -> bool {
        self.inner.is_large()
    }

    /// Whether the bytes are guaranteed UTF-8 (always `false` for binary).
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
    pub fn from_str(value: String) -> napi::Result<BinaryType> {
        CoreBinaryType::from_str(&value)
            .map(|inner| BinaryType { inner })
            .map_err(to_napi_err)
    }

    /// The component map `{"type": …}`.
    #[napi]
    pub fn to_mapping(&self) -> HashMap<String, String> {
        self.inner.to_mapping().into_iter().collect()
    }

    /// Reconstructs the type from its component map.
    #[napi(factory)]
    pub fn from_mapping(mapping: HashMap<String, String>) -> napi::Result<BinaryType> {
        let mapping: BTreeMap<String, String> = mapping.into_iter().collect();
        CoreBinaryType::from_mapping(&mapping)
            .map(|inner| BinaryType { inner })
            .map_err(to_napi_err)
    }

    /// The canonical byte form.
    #[napi]
    pub fn to_bytes(&self) -> Buffer {
        self.inner.to_bytes().into()
    }

    /// Reconstructs the type from its byte form.
    #[napi(factory)]
    pub fn from_bytes(data: Buffer) -> napi::Result<BinaryType> {
        CoreBinaryType::from_bytes(data.as_ref())
            .map(|inner| BinaryType { inner })
            .map_err(to_napi_err)
    }

    /// The JSON value (used by `JSON.stringify`).
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self.inner).expect("BinaryType serializes to JSON")
    }

    /// Reconstructs the type from its JSON value.
    #[napi(js_name = "fromJSON", factory)]
    pub fn from_json(value: serde_json::Value) -> napi::Result<BinaryType> {
        serde_json::from_value(value)
            .map(|inner| BinaryType { inner })
            .map_err(to_napi_err)
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.to_str().into_owned()
    }

    /// Structural equality with another `BinaryType`.
    #[napi]
    pub fn equals(&self, other: &BinaryType) -> bool {
        self.inner == other.inner
    }
}
