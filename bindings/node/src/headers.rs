//! The `yggdryl.io` namespace's [`Headers`] — the project's **one** metadata map.
//!
//! Mirrors `yggdryl_core::headers::Headers`: an ordered, case-insensitive (ASCII, per HTTP),
//! multi-value map of **byte-string** names to **byte-string** values. String accessors sit
//! over the byte storage for the common textual case, while the `*Bytes` twins reach the raw
//! bytes for anything that is not UTF-8. Every method is a thin delegation to the core; the
//! binary codec (`serializeBytes` / `deserializeBytes`) round-trips arbitrary bytes, insertion
//! order, and multi-value entries exactly, and the HTTP text form (`toHttpBytes` /
//! `Headers.parseHttp`) speaks the wire convention. `Headers` is a **mutable** map (like a JS
//! `Map`), so it carries content `equals` but deliberately no `hashCode`.

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;

use crate::datatype_id::DataTypeId;
use crate::mediatype::MediaType;
use crate::mimetype::MimeType;
use yggdryl_core::headers as core;

/// Maps any core error to a thrown JS `Error` (its guided text).
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// An ordered, case-insensitive, multi-value map of byte-string names to byte-string values,
/// following HTTP header conventions — the project's one metadata map (HTTP headers,
/// schema/field metadata, source annotations all live here). Every `memory` source carries one
/// (`Heap.headers`).
#[napi(namespace = "headers")]
#[derive(Default)]
pub struct Headers {
    pub(crate) inner: core::Headers,
}

#[napi(namespace = "headers")]
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
        self.inner.get(&name).map(|value| value.into_owned())
    }

    /// Every value for `name` as a string, in insertion order (non-UTF-8 values are skipped).
    #[napi]
    pub fn get_all(&self, name: String) -> Vec<String> {
        self.inner
            .get_all(&name)
            .into_iter()
            .map(|value| value.into_owned())
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

    /// Sets the `Content-Type` header (replace semantics).
    #[napi]
    pub fn set_content_type(&mut self, value: String) {
        self.inner.set_content_type(&value);
    }

    /// The `Content-Encoding` value, if present and UTF-8 (e.g. `"gzip"`).
    #[napi]
    pub fn content_encoding(&self) -> Option<String> {
        self.inner.content_encoding().map(str::to_string)
    }

    /// Sets the `Content-Encoding` header (replace semantics).
    #[napi]
    pub fn set_content_encoding(&mut self, value: String) {
        self.inner.set_content_encoding(&value);
    }

    // ---- promoted single-valued HTTP request/response headers --------------------------

    /// The `Host` value, if present and UTF-8.
    #[napi]
    pub fn host(&self) -> Option<String> {
        self.inner.host().map(str::to_string)
    }

    /// Sets the `Host` header (replace semantics).
    #[napi]
    pub fn set_host(&mut self, value: String) {
        self.inner.set_host(&value);
    }

    /// The `User-Agent` value, if present and UTF-8.
    #[napi]
    pub fn user_agent(&self) -> Option<String> {
        self.inner.user_agent().map(str::to_string)
    }

    /// Sets the `User-Agent` header (replace semantics).
    #[napi]
    pub fn set_user_agent(&mut self, value: String) {
        self.inner.set_user_agent(&value);
    }

    /// The `Accept` value, if present and UTF-8.
    #[napi]
    pub fn accept(&self) -> Option<String> {
        self.inner.accept().map(str::to_string)
    }

    /// Sets the `Accept` header (replace semantics).
    #[napi]
    pub fn set_accept(&mut self, value: String) {
        self.inner.set_accept(&value);
    }

    /// The `Accept-Encoding` value, if present and UTF-8.
    #[napi]
    pub fn accept_encoding(&self) -> Option<String> {
        self.inner.accept_encoding().map(str::to_string)
    }

    /// Sets the `Accept-Encoding` header (replace semantics).
    #[napi]
    pub fn set_accept_encoding(&mut self, value: String) {
        self.inner.set_accept_encoding(&value);
    }

    /// The `Authorization` value, if present and UTF-8.
    #[napi]
    pub fn authorization(&self) -> Option<String> {
        self.inner.authorization().map(str::to_string)
    }

    /// Sets the `Authorization` header (replace semantics).
    #[napi]
    pub fn set_authorization(&mut self, value: String) {
        self.inner.set_authorization(&value);
    }

    /// The `Location` value, if present and UTF-8.
    #[napi]
    pub fn location(&self) -> Option<String> {
        self.inner.location().map(str::to_string)
    }

    /// Sets the `Location` header (replace semantics).
    #[napi]
    pub fn set_location(&mut self, value: String) {
        self.inner.set_location(&value);
    }

    /// The `Connection` value, if present and UTF-8.
    #[napi]
    pub fn connection(&self) -> Option<String> {
        self.inner.connection().map(str::to_string)
    }

    /// Sets the `Connection` header (replace semantics).
    #[napi]
    pub fn set_connection(&mut self, value: String) {
        self.inner.set_connection(&value);
    }

    /// The `Cache-Control` value, if present and UTF-8.
    #[napi]
    pub fn cache_control(&self) -> Option<String> {
        self.inner.cache_control().map(str::to_string)
    }

    /// Sets the `Cache-Control` header (replace semantics).
    #[napi]
    pub fn set_cache_control(&mut self, value: String) {
        self.inner.set_cache_control(&value);
    }

    /// The `Last-Modified` value (RFC HTTP-date form), if present and UTF-8.
    #[napi]
    pub fn last_modified(&self) -> Option<String> {
        self.inner.last_modified().map(str::to_string)
    }

    /// Sets the `Last-Modified` header (replace semantics).
    #[napi]
    pub fn set_last_modified(&mut self, value: String) {
        self.inner.set_last_modified(&value);
    }

    // ---- element data type + resource name ---------------------------------------------

    /// The storage **element [`DataTypeId`]** declared under `X-Type-Id`, or
    /// [`DataTypeId.Unknown`] (raw bytes) when none is set. Total (never throws — an unrecognized
    /// id reads as `Unknown`).
    #[napi]
    pub fn type_id(&self) -> DataTypeId {
        DataTypeId {
            inner: self.inner.type_id(),
        }
    }

    /// Sets the storage [`DataTypeId`] (its `u16` id). [`DataTypeId.Unknown`] **removes** the
    /// header (no declared type).
    #[napi]
    pub fn set_type_id(&mut self, dtype: &DataTypeId) {
        self.inner.set_type_id(dtype.inner);
    }

    /// The **element storage width** in bytes derived from `typeId` (`i64` → 8), or `0` when
    /// the type is unknown. An `i64` (a JS number).
    #[napi]
    pub fn type_byte_size(&self) -> i64 {
        self.inner.type_byte_size() as i64
    }

    /// The **element bit width** derived from `typeId` (`bool` → 1), or `0` when the type is
    /// unknown. An `i64` (a JS number).
    #[napi]
    pub fn type_bit_size(&self) -> i64 {
        self.inner.type_bit_size() as i64
    }

    /// The resource **name** declared under `X-Name`, or `null` if absent.
    #[napi]
    pub fn name(&self) -> Option<String> {
        self.inner.name().map(str::to_string)
    }

    /// Sets the resource **name** (replace semantics).
    #[napi]
    pub fn set_name(&mut self, name: String) {
        self.inner.set_name(&name);
    }

    // ---- media type: the one place Content-Type / Content-Encoding are interpreted ------

    /// The **primary [`MimeType`]** of `Content-Type`, if present and valid — the single most
    /// specific type this map declares, or `null` when there is no (valid) `Content-Type`.
    #[napi]
    pub fn mime_type(&self) -> Option<MimeType> {
        self.inner.mime_type().map(|inner| MimeType { inner })
    }

    /// Sets `Content-Type` to `mime`'s essence — the centralized mime mutator.
    #[napi]
    pub fn set_mime_type(&mut self, mime: &MimeType) {
        self.inner.set_mime_type(&mime.inner);
    }

    /// The full **[`MediaType`]** this map declares: the `Content-Type` (a comma-list is kept as
    /// multiple entries), extended by the `Content-Encoding` layers resolved to their mime types
    /// (`gzip` → `application/gzip`). `null` when there is no `Content-Type`.
    #[napi]
    pub fn media_type(&self) -> Option<MediaType> {
        self.inner.media_type().map(|inner| MediaType { inner })
    }

    /// Sets `Content-Type` to `media`'s comma-joined essences — the centralized media mutator.
    #[napi]
    pub fn set_media_type(&mut self, media: &MediaType) {
        self.inner.set_media_type(&media.inner);
    }

    // ---- modification time (epoch microseconds) -----------------------------------------

    /// The modification time as **total epoch microseconds** (signed — before 1970 is
    /// negative), from the `MTIME` header, if present and an integer. An `i64` (a JS number,
    /// exact to ±2^53): a signed epoch-microseconds count needs the full 64-bit range, so it is
    /// never a `u32` (which would wrap) nor lost through a float.
    #[napi]
    pub fn mtime(&self) -> Option<i64> {
        self.inner.mtime()
    }

    /// Sets the modification time to `micros` total epoch microseconds — written as a compact
    /// decimal into the `MTIME` header. Keep `micros` within ±2^53 so the JS `number` stays
    /// exact.
    #[napi]
    pub fn set_mtime(&mut self, micros: i64) {
        self.inner.set_mtime(micros);
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

// ---- common header-name constants ---------------------------------------------------

/// The `Last-Modified` header name (RFC HTTP-date form).
#[napi(namespace = "headers")]
pub const LAST_MODIFIED: &str = core::Headers::LAST_MODIFIED;

/// The modification-time header name this map uses for the **epoch-microseconds** form —
/// `mtime` / `setMtime`.
#[napi(namespace = "headers")]
pub const MTIME: &str = core::Headers::MTIME;
