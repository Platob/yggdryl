//! The `MimeType` napi class and the global registry hooks.

use std::collections::HashMap;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_core::{Category, MimeType as CoreMimeType, Signature};

use crate::to_mapping;

/// A single common media (MIME) type, parsed from a string or inferred from a
/// file extension or magic bytes. The extension/magic registry is global and can
/// be extended with `register` / `unregister`.
#[napi]
pub struct MimeType {
    pub(crate) inner: CoreMimeType,
}

#[napi]
impl MimeType {
    /// Parse a `type/subtype` MIME string, throwing on failure. Any `;parameters`
    /// are dropped; unknown but well-formed types are kept verbatim as `Other`.
    #[napi(constructor)]
    pub fn new(value: String) -> Result<Self> {
        CoreMimeType::from_str(&value)
            .map(|inner| MimeType { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Alias for the constructor.
    #[napi(factory, js_name = "fromStr")]
    pub fn from_str(value: String) -> Result<Self> {
        MimeType::new(value)
    }

    /// Build a `MimeType` from an object of components (`type`, `subtype`).
    #[napi(factory, js_name = "fromMapping")]
    pub fn from_mapping(fields: HashMap<String, String>) -> Result<Self> {
        CoreMimeType::from_mapping(&to_mapping(fields))
            .map(|inner| MimeType { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Build a `MimeType` straight from its `type` and `subtype` parts, without
    /// parsing a string, e.g. `MimeType.fromParts('text', 'csv')`.
    #[napi(factory, js_name = "fromParts")]
    pub fn from_parts(type_: String, subtype: String) -> MimeType {
        MimeType {
            inner: CoreMimeType::from_parts(&type_, &subtype),
        }
    }

    /// Infer the MIME type from a file `extension`, or `null` if unknown.
    #[napi(js_name = "fromExtension")]
    pub fn from_extension(extension: String) -> Option<MimeType> {
        CoreMimeType::from_extension(&extension).map(|inner| MimeType { inner })
    }

    /// Infer the MIME type from a file's leading `data` bytes (magic bytes), or
    /// `null` if none match. Recognises Arrow IPC, Parquet, ZIP, gzip, etc.
    #[napi(js_name = "fromMagic")]
    pub fn from_magic(data: Buffer) -> Option<MimeType> {
        CoreMimeType::from_magic(data.as_ref()).map(|inner| MimeType { inner })
    }

    /// Infer the outermost MIME type from a `path`'s last known file extension, or
    /// `null`. For the full layered view use `MediaType.fromPath`.
    #[napi(js_name = "fromPath")]
    pub fn from_path(path: String) -> Option<MimeType> {
        CoreMimeType::from_path(&path).map(|inner| MimeType { inner })
    }

    /// The fallback MIME type, `application/octet-stream` — the conventional
    /// default when no more specific type is known.
    #[napi(factory)]
    #[allow(clippy::should_implement_trait)]
    pub fn default() -> MimeType {
        MimeType {
            inner: CoreMimeType::default(),
        }
    }

    /// Register (or replace) a MIME type globally. `magic` is a list of byte
    /// prefixes matched at the start of a file; `category` is one of `'blob'` (the
    /// default) / `'directory'` / `'tabular'` / `'code'` / `'codec'`. The change is
    /// process-wide.
    #[napi]
    pub fn register(
        mime: String,
        extensions: Vec<String>,
        magic: Option<Vec<Buffer>>,
        category: Option<String>,
    ) -> Result<()> {
        let exts: Vec<&str> = extensions.iter().map(String::as_str).collect();
        let sigs: Vec<Signature> = magic
            .unwrap_or_default()
            .into_iter()
            .map(|b| Signature::prefix(b.to_vec()))
            .collect();
        let category = match category {
            Some(c) => Category::from_str(&c).map_err(|e| Error::from_reason(e.to_string()))?,
            None => Category::default(),
        };
        CoreMimeType::register(&mime, &exts, &sigs, category);
        Ok(())
    }

    /// Remove a MIME type from the global registry, returning whether it existed.
    #[napi]
    pub fn unregister(mime: String) -> bool {
        CoreMimeType::unregister(&mime)
    }

    /// Restore the global registry to its built-in defaults.
    #[napi(js_name = "resetRegistry")]
    pub fn reset_registry() {
        CoreMimeType::reset_registry()
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
        self.inner.extension()
    }

    /// The file extensions registered for this type (the first is canonical).
    #[napi(getter)]
    pub fn extensions(&self) -> Vec<String> {
        self.inner.extensions()
    }

    /// Whether this is a built-in type rather than a fallback `Other`.
    #[napi(getter, js_name = "isKnown")]
    pub fn is_known(&self) -> bool {
        self.inner.is_known()
    }

    /// The category this type plays: `'blob'` (the default) / `'directory'` /
    /// `'tabular'` / `'code'` / `'codec'`.
    #[napi(getter)]
    pub fn category(&self) -> String {
        self.inner.category().as_str().to_string()
    }

    /// `true` if the two MIME types are equal.
    #[napi]
    pub fn equals(&self, other: &MimeType) -> bool {
        self.inner == other.inner
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.mime().to_string()
    }

    /// Serialise to JSON as the canonical MIME string (used by `JSON.stringify`).
    /// `fromJSON` is the inverse (an unknown `Other` round-trips verbatim).
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> String {
        self.inner.mime().to_string()
    }

    /// Reconstruct from the value produced by `toJSON`.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: String) -> Result<Self> {
        MimeType::new(value)
    }
}
