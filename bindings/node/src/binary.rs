//! Node wrapper for [`yggdryl_core::Binary`].

use std::collections::{BTreeMap, HashMap};

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;
use yggdryl_core::{Binary as CoreBinary, BinaryBased, DataType};

use crate::to_napi_err;

/// Arrow's variable-length binary type (`binary` / `large_binary`).
#[napi]
pub struct Binary {
    pub(crate) inner: CoreBinary,
}

#[napi]
impl Binary {
    #[napi(constructor)]
    pub fn new(large: Option<bool>) -> Self {
        Binary {
            inner: if large.unwrap_or(false) {
                CoreBinary::large()
            } else {
                CoreBinary::new()
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

    /// The component map `{"type": …}`.
    #[napi]
    pub fn to_mapping(&self) -> HashMap<String, String> {
        self.inner.to_mapping().into_iter().collect()
    }

    /// Reconstructs the type from its component map.
    #[napi(factory)]
    pub fn from_mapping(mapping: HashMap<String, String>) -> napi::Result<Binary> {
        let mapping: BTreeMap<String, String> = mapping.into_iter().collect();
        CoreBinary::from_mapping(&mapping)
            .map(|inner| Binary { inner })
            .map_err(to_napi_err)
    }

    /// The canonical byte form.
    #[napi]
    pub fn to_bytes(&self) -> Buffer {
        self.inner.to_bytes().into()
    }

    /// Reconstructs the type from its byte form.
    #[napi(factory)]
    pub fn from_bytes(data: Buffer) -> napi::Result<Binary> {
        CoreBinary::from_bytes(data.as_ref())
            .map(|inner| Binary { inner })
            .map_err(to_napi_err)
    }

    /// The JSON value (used by `JSON.stringify`).
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self.inner).expect("Binary serializes to JSON")
    }

    /// Reconstructs the type from its JSON value.
    #[napi(js_name = "fromJSON", factory)]
    pub fn from_json(value: serde_json::Value) -> napi::Result<Binary> {
        serde_json::from_value(value)
            .map(|inner| Binary { inner })
            .map_err(to_napi_err)
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.to_str().into_owned()
    }

    /// Structural equality with another `Binary`.
    #[napi]
    pub fn equals(&self, other: &Binary) -> bool {
        self.inner == other.inner
    }
}
