//! The `yggdryl.io` namespace's [`Headers`] — the project's **one** metadata map.
//!
//! Mirrors `yggdryl_core::io::Headers`: an ordered, case-insensitive (ASCII, per HTTP),
//! multi-value map of **byte-string** names to **byte-string** values. String accessors sit
//! over the byte storage for the common textual case, while the `*Bytes` twins reach the raw
//! bytes for anything that is not UTF-8. Every method is a thin delegation to the core; the
//! binary codec (`serializeBytes` / `deserializeBytes`) round-trips arbitrary bytes, insertion
//! order, and multi-value entries exactly, and the HTTP text form (`toHttpBytes` /
//! `Headers.parseHttp`) speaks the wire convention. `Headers` is a **mutable** map (like a JS
//! `Map`), so it carries content `equals` but deliberately no `hashCode`.

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;

use yggdryl_core::io as core;

/// Maps any core error to a thrown JS `Error` (its guided text).
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// An ordered, case-insensitive, multi-value map of byte-string names to byte-string values,
/// following HTTP header conventions — the project's one metadata map (HTTP headers,
/// schema/field metadata, source annotations all live here). Every `memory` source carries one
/// (`Heap.headers`).
#[napi(namespace = "io")]
#[derive(Default)]
pub struct Headers {
    pub(crate) inner: core::Headers,
}

#[napi(namespace = "io")]
impl Headers {
    /// An empty header map (no allocation).
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            inner: core::Headers::new(),
        }
    }

    /// An empty map with room for `capacity` entries before its first reallocation.
    #[napi(factory)]
    pub fn with_capacity(capacity: u32) -> Self {
        Self {
            inner: core::Headers::with_capacity(capacity as usize),
        }
    }

    /// Parses an HTTP header block: one `Name: Value` per line (`\r\n` or `\n`). **Lenient** —
    /// a blank line stops parsing (the header/body boundary) and a line with no colon is
    /// skipped rather than throwing.
    #[napi(factory)]
    pub fn parse_http(data: Buffer) -> Self {
        Self {
            inner: core::Headers::parse_http(data.as_ref()),
        }
    }

    // ---- read (string + bytes) ---------------------------------------------------------

    /// The number of entries (a repeated name counts once per occurrence).
    #[napi]
    pub fn len(&self) -> u32 {
        self.inner.len() as u32
    }

    /// Whether the map has no entries.
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// The **first** value for `name` (case-insensitive), or `null` if absent or not valid
    /// UTF-8. Use `getBytes` for the raw bytes.
    #[napi]
    pub fn get(&self, name: String) -> Option<String> {
        self.inner.get(&name).map(str::to_string)
    }

    /// Every value for `name` as a string, in insertion order (non-UTF-8 values are skipped).
    #[napi]
    pub fn get_all(&self, name: String) -> Vec<String> {
        self.inner
            .get_all(&name)
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    /// The raw value of the **first** entry whose name matches `name` (case-insensitively),
    /// or `null` if absent.
    #[napi]
    pub fn get_bytes(&self, name: Buffer) -> Option<Buffer> {
        self.inner
            .get_bytes(name.as_ref())
            .map(|value| value.to_vec().into())
    }

    /// Every raw value for `name`, in insertion order.
    #[napi]
    pub fn get_all_bytes(&self, name: Buffer) -> Vec<Buffer> {
        self.inner
            .get_all_bytes(name.as_ref())
            .into_iter()
            .map(|value| value.to_vec().into())
            .collect()
    }

    /// Whether any entry matches `name` (case-insensitively).
    #[napi]
    pub fn contains(&self, name: String) -> bool {
        self.inner.contains(&name)
    }

    /// The entry names as UTF-8 strings, in insertion order (a repeated name appears once per
    /// occurrence; a non-UTF-8 name is skipped — reach it with `getAllBytes`).
    #[napi]
    pub fn keys(&self) -> Vec<String> {
        self.inner
            .iter()
            .filter_map(|(name, _)| std::str::from_utf8(name).ok().map(str::to_string))
            .collect()
    }

    /// The entries as raw byte pairs — a two-element `[name, value]` `Buffer` array per entry,
    /// in insertion order. The complete view: every entry appears, including non-UTF-8 names
    /// that `keys` skips. (A plain array stands in for a tuple, which napi cannot express.)
    #[napi(ts_return_type = "Array<[Buffer, Buffer]>")]
    pub fn items(&self) -> Vec<Vec<Buffer>> {
        self.inner
            .iter()
            .map(|(name, value)| vec![name.to_vec().into(), value.to_vec().into()])
            .collect()
    }

    // ---- write (string + bytes) --------------------------------------------------------

    /// Appends a `name: value` entry, **keeping** any existing entries with the same name
    /// (multi-value append).
    #[napi]
    pub fn append(&mut self, name: String, value: String) {
        self.inner.append(&name, &value);
    }

    /// `append` with raw byte-string arguments.
    #[napi]
    pub fn append_bytes(&mut self, name: Buffer, value: Buffer) {
        self.inner.append_bytes(name.as_ref(), value.as_ref());
    }

    /// **Sets** `name` to a single `value` — removes every existing entry with that name,
    /// then appends one (HTTP "replace" semantics).
    #[napi]
    pub fn insert(&mut self, name: String, value: String) {
        self.inner.insert(&name, &value);
    }

    /// `insert` with raw byte-string arguments.
    #[napi]
    pub fn insert_bytes(&mut self, name: Buffer, value: Buffer) {
        self.inner.insert_bytes(name.as_ref(), value.as_ref());
    }

    /// A fresh map with `name` set to a single `value` — the one-line, non-mutating builder
    /// (`headers.with('A', '1').with('B', '2')`).
    #[napi]
    pub fn with(&self, name: String, value: String) -> Headers {
        Headers {
            inner: self.inner.clone().with(&name, &value),
        }
    }

    /// Removes **every** entry matching `name` (case-insensitively); returns how many were
    /// removed.
    #[napi]
    pub fn remove(&mut self, name: String) -> u32 {
        self.inner.remove(&name) as u32
    }

    /// `remove` with a raw byte-string name — the only way to remove an entry whose name is
    /// not valid UTF-8; returns how many were removed.
    #[napi]
    pub fn remove_bytes(&mut self, name: Buffer) -> u32 {
        self.inner.remove_bytes(name.as_ref()) as u32
    }

    /// Removes all entries.
    #[napi]
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// An explicit copy of this map — the cross-language name for a clone.
    #[napi]
    pub fn copy(&self) -> Headers {
        Headers {
            inner: self.inner.copy(),
        }
    }

    /// Returns a copy of this map overlaid by `other`: every name `other` carries **replaces**
    /// that name here (all occurrences), and names only this map carries are kept.
    #[napi]
    pub fn merge_with(&self, other: &Headers) -> Headers {
        Headers {
            inner: self.inner.merge_with(&other.inner),
        }
    }

    // ---- typed conveniences for common headers -----------------------------------------

    /// The `Content-Type` value, if present and UTF-8.
    #[napi]
    pub fn content_type(&self) -> Option<String> {
        self.inner.content_type().map(str::to_string)
    }

    /// The `Content-Length` value parsed as an integer, if present and numeric. The returned
    /// JS `number` is exact only up to 2^53 (lengths that large are far beyond any real body).
    #[napi]
    pub fn content_length(&self) -> Option<i64> {
        self.inner.content_length().map(|length| length as i64)
    }

    // ---- HTTP text form + binary codec --------------------------------------------------

    /// Renders the header block in HTTP wire form — `Name: Value\r\n` per entry (no trailing
    /// blank line). One pre-sized allocation.
    #[napi]
    pub fn to_http_bytes(&self) -> Buffer {
        self.inner.to_http_bytes().into()
    }

    /// The map as a length-prefixed binary frame — unlike the HTTP text form this round-trips
    /// **arbitrary** bytes, insertion order, and multi-value entries; `deserializeBytes` is the
    /// exact inverse.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs a map from bytes produced by `serializeBytes`, throwing a guided `Error`
    /// naming the shortfall if the frame is truncated.
    #[napi(factory)]
    pub fn deserialize_bytes(data: Buffer) -> napi::Result<Headers> {
        core::Headers::deserialize_bytes(data.as_ref())
            .map(|inner| Headers { inner })
            .map_err(to_error)
    }

    // ---- value semantics ----------------------------------------------------------------

    /// Content equality — equal iff the entries (names, values, order, and duplicates) are
    /// equal byte-for-byte.
    #[napi]
    pub fn equals(&self, other: &Headers) -> bool {
        self.inner == other.inner
    }

    /// A readable `{"name": "value", …}` rendering (lossy UTF-8), in insertion order.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!("{:?}", self.inner)
    }
}
