//! The `MediaType` napi class: an ordered stack of `MimeType`.

use std::collections::HashMap;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_core::{MediaType as CoreMediaType, ToOutput};

use crate::mime::MimeType;
use crate::to_mapping;

/// An ordered stack of `MimeType`, describing a layered file. Parsing
/// `data.csv.gz` yields `MediaType([MimeType('text/csv'), MimeType('application/gzip')])`.
#[napi]
pub struct MediaType {
    pub(crate) inner: CoreMediaType,
}

#[napi]
impl MediaType {
    /// Build a `MediaType` from an ordered list of `MimeType`.
    #[napi(constructor)]
    pub fn new(types: Vec<&MimeType>) -> Self {
        MediaType {
            inner: CoreMediaType::new(types.into_iter().map(|t| t.inner.clone()).collect()),
        }
    }

    /// Build the stack from a path's file extensions (innermost content first).
    #[napi(factory, js_name = "fromPath")]
    pub fn from_path(path: String) -> Self {
        MediaType {
            inner: CoreMediaType::from_path(&path),
        }
    }

    /// Parse a path or file name into its `MimeType` stack.
    #[napi(factory, js_name = "fromStr")]
    pub fn from_str(value: String) -> Result<Self> {
        CoreMediaType::from_str(&value)
            .map(|inner| MediaType { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Build the stack from an object; reads the `path` key (or `str`).
    #[napi(factory, js_name = "fromMapping")]
    pub fn from_mapping(fields: HashMap<String, String>) -> Result<Self> {
        CoreMediaType::from_mapping(&to_mapping(fields))
            .map(|inner| MediaType { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Build a single-type stack from one file `extension` (empty if unknown).
    #[napi(factory, js_name = "fromExtension")]
    pub fn from_extension(extension: String) -> Self {
        MediaType {
            inner: CoreMediaType::from_extension(&extension),
        }
    }

    /// Build the stack from an ordered list of file `extensions`.
    #[napi(factory, js_name = "fromExtensions")]
    pub fn from_extensions(extensions: Vec<String>) -> Self {
        let exts: Vec<&str> = extensions.iter().map(String::as_str).collect();
        MediaType {
            inner: CoreMediaType::from_extensions(&exts),
        }
    }

    /// The fallback stack, a single `application/octet-stream` — the default when
    /// no type can be inferred.
    #[napi(factory)]
    #[allow(clippy::should_implement_trait)]
    pub fn default() -> MediaType {
        MediaType {
            inner: CoreMediaType::default(),
        }
    }

    /// Render to a component object (the inverse of `fromMapping`).
    #[napi(js_name = "toMapping")]
    pub fn to_mapping(&self) -> HashMap<String, String> {
        self.inner.to_mapping().into_iter().collect()
    }

    /// The ordered `MimeType` list, innermost content first.
    #[napi(getter)]
    pub fn types(&self) -> Vec<MimeType> {
        self.inner
            .types()
            .iter()
            .map(|inner| MimeType {
                inner: inner.clone(),
            })
            .collect()
    }

    /// The innermost (content) type, e.g. `text/csv` for `data.csv.gz`.
    #[napi(getter)]
    pub fn first(&self) -> Option<MimeType> {
        self.inner.first().map(|inner| MimeType {
            inner: inner.clone(),
        })
    }

    /// The outermost (container) type, e.g. `application/gzip` for `data.csv.gz`.
    #[napi(getter)]
    pub fn last(&self) -> Option<MimeType> {
        self.inner.last().map(|inner| MimeType {
            inner: inner.clone(),
        })
    }

    /// The number of types in the stack.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        self.inner.len() as u32
    }

    /// Whether the stack is empty (no known extension was found).
    #[napi(getter, js_name = "isEmpty")]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// `true` if the two stacks are equal.
    #[napi]
    pub fn equals(&self, other: &MediaType) -> bool {
        self.inner == other.inner
    }

    /// Render the canonical extension chain, e.g. `"csv.gz"`.
    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.to_str(true)
    }
}
