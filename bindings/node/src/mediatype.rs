//! The `yggdryl.mediatype` namespace — an **ordered list of [`MimeType`]s**: the layered type
//! description of a resource (a content type plus any encodings/wrappers, or the stack a
//! multi-extension file name implies, e.g. `archive.tar.gz` → `application/x-tar` then
//! `application/gzip`).
//!
//! Mirrors `yggdryl_core::mediatype::MediaType`. A value type with the usual surface — a byte
//! codec (`serializeBytes` / `deserializeBytes` over the comma-joined essences), content
//! equality (`equals`), a Java-style `hashCode`, and `toString` — so it round-trips and works
//! as a map key exactly like the core value. A bad mime item handed to `parse` /
//! `deserializeBytes` throws a guided `Error` carrying the core's text unchanged.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;

use crate::mimetype::MimeType;
use yggdryl_core::io::Serializable;
use yggdryl_core::mediatype::{self as core};

/// Maps any core error to a thrown JS `Error` (its guided text).
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// A Java-style `i32` content hash of a value, folding the 64-bit hash halves.
fn java_hash<T: Hash>(value: &T) -> i32 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    let hash = hasher.finish();
    (hash as u32 ^ (hash >> 32) as u32) as i32
}

/// An ordered list of [`MimeType`]s describing a resource, **primary first**. A single-type
/// media (`application/json`) is a one-element list; a wrapped one (`.tar.gz`) lists the
/// content type then its encodings. A value type — equal, hashable, byte-serializable.
#[napi(namespace = "mediatype")]
pub struct MediaType {
    pub(crate) inner: core::MediaType,
}

#[napi(namespace = "mediatype")]
impl MediaType {
    /// A media type from an ordered list of [`MimeType`]s (primary first), or an empty media
    /// type when `types` is omitted.
    #[napi(constructor)]
    pub fn new(types: Option<Vec<&MimeType>>) -> Self {
        match types {
            Some(types) => Self {
                inner: core::MediaType::from_types(types.into_iter().map(|m| m.inner.clone())),
            },
            None => Self {
                inner: core::MediaType::new(),
            },
        }
    }

    /// Parses a **comma-separated mime list** (like an HTTP `Accept` / `Content-Type` value),
    /// dropping each item's parameters (`;q=…`) and skipping empty items. Throws a guided
    /// `Error` if any non-empty item is not a `type/subtype` essence.
    #[napi(factory)]
    pub fn parse(s: String) -> napi::Result<MediaType> {
        core::MediaType::parse_str(&s)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// A single-type media over `mime`.
    #[napi(factory)]
    pub fn of(mime: &MimeType) -> MediaType {
        Self {
            inner: core::MediaType::of(mime.inner.clone()),
        }
    }

    /// Builds a media type from a file's **extensions** (outermost-last): each known extension
    /// maps to its [`MimeType`], an unknown one is skipped (`["tar", "gz"]` →
    /// `[application/x-tar, application/gzip]`).
    #[napi(factory)]
    pub fn from_extensions(exts: Vec<String>) -> MediaType {
        Self {
            inner: core::MediaType::from_extensions(exts),
        }
    }

    /// The **primary** type (the first), or `null` when empty.
    #[napi]
    pub fn primary(&self) -> Option<MimeType> {
        self.inner.primary().map(|inner| MimeType {
            inner: inner.clone(),
        })
    }

    /// The listed types, primary first.
    #[napi]
    pub fn types(&self) -> Vec<MimeType> {
        self.inner
            .types()
            .iter()
            .map(|inner| MimeType {
                inner: inner.clone(),
            })
            .collect()
    }

    /// The listed essences, primary first (`["application/x-tar", "application/gzip"]`).
    #[napi]
    pub fn essences(&self) -> Vec<String> {
        self.inner
            .essences()
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    /// The number of listed types — an `i64` (a JS number).
    #[napi]
    pub fn len(&self) -> i64 {
        self.inner.len() as i64
    }

    /// Whether the list has no types.
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Whether any listed type has the given `essence` (case-insensitive).
    #[napi]
    pub fn contains(&self, essence: String) -> bool {
        self.inner.contains(&essence)
    }

    /// Appends a type to the list (in place).
    #[napi]
    pub fn push(&mut self, mime: &MimeType) {
        self.inner.push(mime.inner.clone());
    }

    /// An explicit copy of this media type — the cross-language name for a clone.
    #[napi]
    pub fn copy(&self) -> MediaType {
        Self {
            inner: self.inner.clone(),
        }
    }

    /// The value form — the comma-joined essences (the inverse of `parse`). Each entry's
    /// extensions/magic are dropped, like `MimeType.serializeBytes`.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        Serializable::serialize_bytes(&self.inner).into()
    }

    /// Reconstructs a media type from the comma-joined essence bytes produced by
    /// `serializeBytes`, throwing a guided `Error` on non-UTF-8 bytes or a bad item.
    #[napi(factory)]
    pub fn deserialize_bytes(data: Buffer) -> napi::Result<MediaType> {
        <core::MediaType as Serializable>::deserialize_bytes(data.as_ref())
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Content equality (equal iff the comma-joined essences are equal).
    #[napi]
    pub fn equals(&self, other: &MediaType) -> bool {
        self.inner == other.inner
    }

    /// Java-style `i32` content hash.
    #[napi]
    pub fn hash_code(&self) -> i32 {
        java_hash(&self.inner)
    }

    /// The comma-joined essences (`"application/x-tar, application/gzip"`).
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        self.inner.to_string()
    }
}
