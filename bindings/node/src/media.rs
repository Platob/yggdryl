//! The `MediaType` napi class: an ordered stack of `MimeType`.

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_media::{FromInput, MediaType as CoreMediaType, ToOutput};

use crate::mime::MimeType;

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
    pub fn from_str(value: String, safe: Option<bool>) -> Result<Self> {
        CoreMediaType::from_str(&value, safe.unwrap_or(true))
            .map(|inner| MediaType { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
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
