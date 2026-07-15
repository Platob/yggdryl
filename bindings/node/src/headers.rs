//! The `yggdryl.io` namespace's [`Headers`] — the **centralized** string key/value metadata
//! holder: an ordered, case-insensitive, multi-value map that backs both HTTP headers and a
//! [`Field`](crate::types::Field)'s metadata (there is no separate `Metadata` type).
//!
//! Mirrors [`Headers`](yggdryl_core::io::Headers) method-for-method — each method is one or two
//! lines over the core. Like the `Bytes` buffer it is a **mutable** container, so it exposes
//! content `equals` + a lossless byte codec but no `hashCode`; a `Field` that embeds it still
//! hashes, via the core's byte-canonical field hash.

use napi::bindgen_prelude::{Buffer, Object};
use napi_derive::napi;

use yggdryl_core::io::Headers as CoreHeaders;

/// Maps any core error to a thrown JS `Error` (its guided text).
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// Builds core [`Headers`](CoreHeaders) from an optional JS object, **preserving the object's
/// key insertion order** (Headers equality is order-significant, so a `Record` must decode
/// deterministically — a Rust `HashMap` would lose the order). Shared by the constructor and
/// [`Field`](crate::types::Field).
pub(crate) fn core_headers(entries: Option<Object>) -> napi::Result<CoreHeaders> {
    let mut headers = CoreHeaders::new();
    if let Some(object) = entries {
        for key in Object::keys(&object)? {
            let value: Option<String> = object.get(&key)?;
            if let Some(value) = value {
                headers.insert(&key, &value);
            }
        }
    }
    Ok(headers)
}

/// A **case-insensitive, ordered, multi-value** string map — a `Field`'s metadata *and* an HTTP
/// header block. `Map`-like (`get` / `set`… here `insert` / `has` / `size`), with multi-value
/// `append`/`getAll`, the HTTP text form (`toHttpBytes`/`parseHttp`), and a lossless byte codec
/// (`serializeBytes`/`deserializeBytes`).
#[napi(namespace = "io")]
#[derive(Clone)]
pub struct Headers {
    pub(crate) inner: CoreHeaders,
}

#[napi(namespace = "io")]
impl Headers {
    /// An empty map, or one seeded from a `Record<string, string>` (each entry `insert`ed, in
    /// the object's key order).
    #[napi(constructor)]
    pub fn new(entries: Option<Object>) -> napi::Result<Self> {
        Ok(Self {
            inner: core_headers(entries)?,
        })
    }

    /// The first value for `key` (case-insensitive), or `undefined` if absent or not UTF-8.
    #[napi]
    pub fn get(&self, key: String) -> Option<String> {
        self.inner.get(&key).map(str::to_string)
    }

    /// Every value for `key`, in insertion order (non-UTF-8 values skipped).
    #[napi]
    pub fn get_all(&self, key: String) -> Vec<String> {
        self.inner
            .get_all(&key)
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    /// Whether any entry matches `key` (case-insensitively).
    #[napi]
    pub fn has(&self, key: String) -> bool {
        self.inner.contains(&key)
    }

    /// **Sets** `key` to a single `value` — removes any existing entries with that name first.
    #[napi]
    pub fn insert(&mut self, key: String, value: String) {
        self.inner.insert(&key, &value);
    }

    /// **Appends** a `key: value` entry, keeping any existing entries with that name (multi-value).
    #[napi]
    pub fn append(&mut self, key: String, value: String) {
        self.inner.append(&key, &value);
    }

    /// Removes **every** entry matching `key`; returns how many were removed.
    #[napi]
    pub fn remove(&mut self, key: String) -> u32 {
        self.inner.remove(&key) as u32
    }

    /// Removes all entries.
    #[napi]
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// The number of entries (a repeated name counts once per occurrence).
    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.len() as u32
    }

    /// Whether the map has no entries.
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// The keys, in insertion order (a repeated name appears once per occurrence).
    #[napi]
    pub fn keys(&self) -> Vec<String> {
        self.inner
            .iter()
            .map(|(key, _)| String::from_utf8_lossy(key).into_owned())
            .collect()
    }

    /// The values, in insertion order.
    #[napi]
    pub fn values(&self) -> Vec<String> {
        self.inner
            .iter()
            .map(|(_, value)| String::from_utf8_lossy(value).into_owned())
            .collect()
    }

    /// A plain `Record<string, string>` copy (a repeated name keeps its last value).
    #[napi]
    pub fn to_object(&self) -> std::collections::HashMap<String, String> {
        self.inner
            .iter()
            .map(|(key, value)| {
                (
                    String::from_utf8_lossy(key).into_owned(),
                    String::from_utf8_lossy(value).into_owned(),
                )
            })
            .collect()
    }

    /// A fresh map with `key` set to a single `value` — the one-line, non-mutating builder.
    #[napi]
    pub fn with_entry(&self, key: String, value: String) -> Self {
        Self {
            inner: self.inner.clone().with(&key, &value),
        }
    }

    // ---- HTTP conveniences -----------------------------------------------------------------

    /// The `Content-Type` value, if present and UTF-8.
    #[napi(getter)]
    pub fn content_type(&self) -> Option<String> {
        self.inner.content_type().map(str::to_string)
    }

    /// The `Content-Length` value parsed as a number, if present and numeric.
    #[napi(getter)]
    pub fn content_length(&self) -> Option<i64> {
        self.inner.content_length().map(|length| length as i64)
    }

    /// The header block in HTTP wire form — `Name: Value\r\n` per entry.
    #[napi]
    pub fn to_http_bytes(&self) -> Buffer {
        self.inner.to_http_bytes().into()
    }

    /// Parses an HTTP header block (`Name: Value` per line, `\r\n` or `\n`), stopping at the
    /// blank line and skipping colon-less lines (lenient).
    #[napi(factory)]
    pub fn parse_http(bytes: Buffer) -> Self {
        Self {
            inner: CoreHeaders::parse_http(bytes.as_ref()),
        }
    }

    // ---- lossless byte codec (round-trips arbitrary bytes + multi-value) --------------------

    /// The map serialized to bytes — the exact inverse of `deserializeBytes`.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs a map from bytes produced by `serializeBytes`. Throws a guided `Error` on a
    /// truncated frame.
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        CoreHeaders::deserialize_bytes(bytes.as_ref())
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// An explicit copy.
    #[napi]
    pub fn copy(&self) -> Self {
        self.clone()
    }

    /// Content equality (same entries, in the same order).
    #[napi]
    pub fn equals(&self, other: &Headers) -> bool {
        self.inner == other.inner
    }

    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        let entries: Vec<String> = self
            .inner
            .iter()
            .map(|(key, value)| {
                format!(
                    "{:?}: {:?}",
                    String::from_utf8_lossy(key),
                    String::from_utf8_lossy(value)
                )
            })
            .collect();
        format!("Headers({{{}}})", entries.join(", "))
    }
}
