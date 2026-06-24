//! The `MediaType` napi class.

use std::collections::HashMap;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_media::{FromInput, MediaType as CoreMediaType, ToOutput};

use crate::to_mapping;

/// A common media (MIME) type, parsed from a string or inferred from a file
/// extension or magic bytes.
#[napi]
pub struct MediaType {
    pub(crate) inner: CoreMediaType,
}

#[napi]
impl MediaType {
    /// Parse a `type/subtype` MIME string, throwing on failure. Any `;parameters`
    /// are dropped; with `safe = false` the input is taken as-is.
    #[napi(constructor)]
    pub fn new(value: String, safe: Option<bool>) -> Result<Self> {
        CoreMediaType::from_str(&value, safe.unwrap_or(true))
            .map(|inner| MediaType { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Alias for the constructor.
    #[napi(factory, js_name = "fromStr")]
    pub fn from_str(value: String, safe: Option<bool>) -> Result<Self> {
        MediaType::new(value, safe)
    }

    /// Build a `MediaType` from an object of components (`type`, `subtype`).
    #[napi(factory, js_name = "fromMapping")]
    pub fn from_mapping(fields: HashMap<String, String>, safe: Option<bool>) -> Result<Self> {
        CoreMediaType::from_mapping(&to_mapping(fields), safe.unwrap_or(true))
            .map(|inner| MediaType { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Infer the media type from a file `extension`, or `null` if unknown.
    #[napi(js_name = "fromExtension")]
    pub fn from_extension(extension: String) -> Option<MediaType> {
        CoreMediaType::from_extension(&extension).map(|inner| MediaType { inner })
    }

    /// Infer the media type from a file's leading `data` bytes (magic bytes), or
    /// `null` if none match. Recognises Arrow IPC, Parquet, ZIP, gzip, etc.
    #[napi(js_name = "fromMagic")]
    pub fn from_magic(data: Buffer) -> Option<MediaType> {
        CoreMediaType::from_magic(data.as_ref()).map(|inner| MediaType { inner })
    }

    /// Infer the media type from a path's file extension, or `null`.
    #[napi(js_name = "fromPath")]
    pub fn from_path(path: String) -> Option<MediaType> {
        CoreMediaType::from_path(&path).map(|inner| MediaType { inner })
    }

    /// Render to a component object (the inverse of `fromMapping`).
    #[napi(js_name = "toMapping")]
    pub fn to_mapping(&self) -> HashMap<String, String> {
        self.inner.to_mapping().into_iter().collect()
    }

    /// The canonical `type/subtype` MIME string.
    #[napi(getter)]
    pub fn mime(&self) -> String {
        self.inner.mime().to_string()
    }

    /// The top-level type, e.g. `"image"` for `image/png`.
    #[napi(getter, js_name = "type")]
    pub fn type_(&self) -> String {
        self.inner.type_().to_string()
    }

    /// The subtype, e.g. `"png"` for `image/png`.
    #[napi(getter)]
    pub fn subtype(&self) -> String {
        self.inner.subtype().to_string()
    }

    /// The canonical (first) file extension, if any.
    #[napi(getter)]
    pub fn extension(&self) -> Option<String> {
        self.inner.extension().map(|s| s.to_string())
    }

    /// The file extensions associated with this type (the first is canonical).
    #[napi(getter)]
    pub fn extensions(&self) -> Vec<String> {
        self.inner
            .extensions()
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    /// Whether this is a registry type rather than a fallback `Other`.
    #[napi(getter, js_name = "isKnown")]
    pub fn is_known(&self) -> bool {
        self.inner.is_known()
    }

    /// `true` if the two media types are equal.
    #[napi]
    pub fn equals(&self, other: &MediaType) -> bool {
        self.inner == other.inner
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.mime().to_string()
    }
}
