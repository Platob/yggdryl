//! [`Headers`] — an ordered, case-insensitive, multi-value map of **byte-string** keys to
//! **byte-string** values, with `&str` conveniences, following HTTP header conventions.

use core::fmt;
use std::borrow::Cow;

use crate::dtype::DataTypeId;
use crate::io::{IoError, Serializable};
use crate::mediatype::MediaType;
use crate::mimetype::MimeType;

/// One `name: value` entry in the overflow map — both stored as owned byte strings.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Entry {
    name: Box<[u8]>,
    value: Box<[u8]>,
}

/// The six **promoted** keys that live in a hard-typed [`Headers`] field rather than the overflow
/// map. Matched case-insensitively; each is single-valued (see the type docs).
#[derive(Clone, Copy)]
enum Promoted {
    ContentType,
    ContentEncoding,
    ContentLength,
    ElemTypeId,
    Name,
    Mtime,
}

/// The **canonical iteration/serialization order** of the promoted fields — a fixed order so the
/// wire form is deterministic (equal maps serialize to equal bytes).
const PROMOTED_ORDER: [Promoted; 6] = [
    Promoted::ContentType,
    Promoted::ContentEncoding,
    Promoted::ContentLength,
    Promoted::ElemTypeId,
    Promoted::Name,
    Promoted::Mtime,
];

/// The project's **one** metadata map. The six common single-valued keys — `Content-Type`,
/// `Content-Encoding`, `Content-Length`, `X-Elem-Type-Id`, `X-Name`, `X-Mtime-Us` — are stored in
/// **hard-typed fields** (`Option<Box<str>>` / `Option<u64>` / `Option<i64>` / [`DataTypeId`]), so
/// their typed accessors ([`content_length`](Headers::content_length) → `u64`,
/// [`content_type`](Headers::content_type) → `&str`, …) read and write with **no parse and no
/// per-value allocation**. Every **other** name lives in an insertion-ordered, case-insensitive,
/// multi-value overflow map (`other`).
///
/// The generic map view still sees **everything**: [`get`](Headers::get) /
/// [`get_bytes`](Headers::get_bytes) / [`iter`](Headers::iter) return a [`Cow`] — a **borrow** for
/// the string keys and the overflow entries, a small **rendered** buffer for the numeric keys
/// (`Content-Length`, `X-Mtime-Us`, `X-Elem-Type-Id`) on demand. HTTP headers, schema/field
/// metadata, source annotations — all of it is a `Headers`; every
/// [`IOBase`](crate::io::memory::IOBase) source carries one
/// ([`headers`](crate::io::memory::IOBase::headers) /
/// [`headers_mut`](crate::io::memory::IOBase::headers_mut)).
///
/// DESIGN: the promoted keys are **single-valued** (they are unique in HTTP), so `append` on one
/// behaves as `insert` (replace). They render **first**, in a fixed order, ahead of the overflow
/// entries — reordering a unique field relative to other names is RFC 7230 §3.2.2 compliant. A
/// value that does not fit its typed field (a non-UTF-8 `Content-Type`, a non-numeric
/// `Content-Length`) **falls back** to the overflow map under its name, so no data is lost.
/// Equality is by this canonical content (the typed fields plus the ordered overflow), and equal
/// maps hash equal.
///
/// ```
/// use yggdryl_core::headers::Headers;
///
/// let mut headers = Headers::new();
/// headers.insert(Headers::CONTENT_TYPE, "application/json");
/// headers.append("Set-Cookie", "a=1");
/// headers.append("Set-Cookie", "b=2");
///
/// assert_eq!(headers.get("content-type").as_deref(), Some("application/json")); // case-insensitive
/// let cookies = headers.get_all("set-cookie");
/// assert_eq!(cookies.iter().map(|c| c.as_ref()).collect::<Vec<&str>>(), ["a=1", "b=2"]); // multi-value
/// assert!(headers.contains("Content-Type"));
/// ```
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Headers {
    content_type: Option<Box<str>>,
    content_encoding: Option<Box<str>>,
    content_length: Option<u64>,
    elem_type_id: DataTypeId,
    name: Option<Box<str>>,
    mtime: Option<i64>,
    other: Vec<Entry>,
}

impl Default for Headers {
    fn default() -> Self {
        Self::new()
    }
}

impl Headers {
    // ---- common HTTP header names (canonical casing; matched case-insensitively) --------

    /// The `Content-Type` header name.
    pub const CONTENT_TYPE: &'static str = "Content-Type";
    /// The `Content-Length` header name.
    pub const CONTENT_LENGTH: &'static str = "Content-Length";
    /// The `Content-Encoding` header name.
    pub const CONTENT_ENCODING: &'static str = "Content-Encoding";
    /// The `Host` header name.
    pub const HOST: &'static str = "Host";
    /// The `Accept` header name.
    pub const ACCEPT: &'static str = "Accept";
    /// The `Accept-Encoding` header name.
    pub const ACCEPT_ENCODING: &'static str = "Accept-Encoding";
    /// The `Authorization` header name.
    pub const AUTHORIZATION: &'static str = "Authorization";
    /// The `User-Agent` header name.
    pub const USER_AGENT: &'static str = "User-Agent";
    /// The `Location` header name.
    pub const LOCATION: &'static str = "Location";
    /// The `Connection` header name.
    pub const CONNECTION: &'static str = "Connection";
    /// The `Cache-Control` header name.
    pub const CACHE_CONTROL: &'static str = "Cache-Control";
    /// The `Cookie` header name.
    pub const COOKIE: &'static str = "Cookie";
    /// The `Set-Cookie` header name.
    pub const SET_COOKIE: &'static str = "Set-Cookie";
    /// The `Last-Modified` header name (RFC HTTP-date form).
    pub const LAST_MODIFIED: &'static str = "Last-Modified";
    /// The modification-time header this map uses for the **epoch-microseconds** form —
    /// [`mtime`](Headers::mtime) / [`set_mtime`](Headers::set_mtime).
    pub const MTIME: &'static str = "X-Mtime-Us";
    /// The storage **element data type** header — a [`DataTypeId`](crate::dtype::DataTypeId) as its
    /// `u16` id ([`elem_type_id`](Headers::elem_type_id) / [`set_elem_type_id`](Headers::set_elem_type_id)).
    pub const ELEM_TYPE_ID: &'static str = "X-Elem-Type-Id";
    /// The resource **name** header ([`name`](Headers::name) / [`set_name`](Headers::set_name)).
    pub const NAME: &'static str = "X-Name";

    // ---- construction -------------------------------------------------------------------

    /// An empty header map (no allocation — `const`, so it can back a `static` empty map).
    pub const fn new() -> Self {
        Self {
            content_type: None,
            content_encoding: None,
            content_length: None,
            elem_type_id: DataTypeId::Unknown,
            name: None,
            mtime: None,
            other: Vec::new(),
        }
    }

    /// An empty map with room for `capacity` **overflow** entries before its first reallocation
    /// (the promoted keys never allocate the overflow map).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            other: Vec::with_capacity(capacity),
            ..Self::new()
        }
    }

    // ---- promoted-key routing (internal) ------------------------------------------------

    /// Whether a promoted field currently holds a value.
    fn promoted_present(&self, key: Promoted) -> bool {
        match key {
            Promoted::ContentType => self.content_type.is_some(),
            Promoted::ContentEncoding => self.content_encoding.is_some(),
            Promoted::ContentLength => self.content_length.is_some(),
            Promoted::ElemTypeId => self.elem_type_id != DataTypeId::Unknown,
            Promoted::Name => self.name.is_some(),
            Promoted::Mtime => self.mtime.is_some(),
        }
    }

    /// Clears a promoted field to its absent state.
    fn clear_promoted(&mut self, key: Promoted) {
        match key {
            Promoted::ContentType => self.content_type = None,
            Promoted::ContentEncoding => self.content_encoding = None,
            Promoted::ContentLength => self.content_length = None,
            Promoted::ElemTypeId => self.elem_type_id = DataTypeId::Unknown,
            Promoted::Name => self.name = None,
            Promoted::Mtime => self.mtime = None,
        }
    }

    /// The canonical name of a promoted key.
    fn promoted_name(key: Promoted) -> &'static str {
        match key {
            Promoted::ContentType => Self::CONTENT_TYPE,
            Promoted::ContentEncoding => Self::CONTENT_ENCODING,
            Promoted::ContentLength => Self::CONTENT_LENGTH,
            Promoted::ElemTypeId => Self::ELEM_TYPE_ID,
            Promoted::Name => Self::NAME,
            Promoted::Mtime => Self::MTIME,
        }
    }

    /// The promoted field's value rendered to bytes — a **borrow** for the string keys, a small
    /// **owned** decimal for the numeric keys; `None` when the field is absent.
    fn promoted_bytes(&self, key: Promoted) -> Option<Cow<'_, [u8]>> {
        match key {
            Promoted::ContentType => self
                .content_type
                .as_ref()
                .map(|s| Cow::Borrowed(s.as_bytes())),
            Promoted::ContentEncoding => self
                .content_encoding
                .as_ref()
                .map(|s| Cow::Borrowed(s.as_bytes())),
            Promoted::Name => self.name.as_ref().map(|s| Cow::Borrowed(s.as_bytes())),
            Promoted::ContentLength => self.content_length.map(|n| Cow::Owned(render_u64(n))),
            Promoted::Mtime => self.mtime.map(|n| Cow::Owned(render_i64(n))),
            Promoted::ElemTypeId => (self.elem_type_id != DataTypeId::Unknown)
                .then(|| Cow::Owned(render_u64(u64::from(self.elem_type_id.as_u16())))),
        }
    }

    /// Stores `value` into the promoted field for `key`, or — when it does not fit the typed field
    /// (non-UTF-8 string, non-numeric number, foreign type id) — **falls back** to the overflow map
    /// under `name` so no data is lost. The field/overflow entries for the key must already be
    /// cleared by the caller.
    fn store_promoted(&mut self, key: Promoted, name: &[u8], value: &[u8]) {
        let push_raw = |this: &mut Self| {
            this.other.push(Entry {
                name: name.into(),
                value: value.into(),
            });
        };
        match key {
            Promoted::ContentType => match core::str::from_utf8(value) {
                Ok(s) => self.content_type = Some(s.into()),
                Err(_) => push_raw(self),
            },
            Promoted::ContentEncoding => match core::str::from_utf8(value) {
                Ok(s) => self.content_encoding = Some(s.into()),
                Err(_) => push_raw(self),
            },
            Promoted::Name => match core::str::from_utf8(value) {
                Ok(s) => self.name = Some(s.into()),
                Err(_) => push_raw(self),
            },
            Promoted::ContentLength => {
                match core::str::from_utf8(value)
                    .ok()
                    .and_then(|s| s.trim().parse::<u64>().ok())
                {
                    Some(n) => self.content_length = Some(n),
                    None => push_raw(self),
                }
            }
            Promoted::Mtime => {
                match core::str::from_utf8(value)
                    .ok()
                    .and_then(|s| s.trim().parse::<i64>().ok())
                {
                    Some(n) => self.mtime = Some(n),
                    None => push_raw(self),
                }
            }
            Promoted::ElemTypeId => {
                match core::str::from_utf8(value)
                    .ok()
                    .and_then(|s| s.trim().parse::<u16>().ok())
                    .map(DataTypeId::from_u16)
                {
                    Some(dt) if dt != DataTypeId::Unknown => self.elem_type_id = dt,
                    _ => push_raw(self), // non-numeric or a foreign id — keep the raw value
                }
            }
        }
    }

    /// Removes every overflow entry whose name matches `name` (case-insensitively) — the cheap
    /// no-op when the overflow map is empty (the common case for a promoted-only set).
    fn clear_other_named(&mut self, name: &[u8]) {
        if !self.other.is_empty() {
            self.other
                .retain(|entry| !entry.name.eq_ignore_ascii_case(name));
        }
    }

    // ---- read (bytes + str) -------------------------------------------------------------

    /// The number of entries (the present promoted keys plus the overflow entries; a repeated
    /// overflow name counts once per occurrence).
    pub fn len(&self) -> usize {
        let promoted = PROMOTED_ORDER
            .iter()
            .filter(|&&key| self.promoted_present(key))
            .count();
        promoted + self.other.len()
    }

    /// Whether the map has no entries.
    pub fn is_empty(&self) -> bool {
        self.other.is_empty() && !PROMOTED_ORDER.iter().any(|&key| self.promoted_present(key))
    }

    /// The raw value of the **first** entry whose name matches `name` (case-insensitively) — a
    /// borrow for a string/overflow value, a small rendered buffer for a numeric promoted key.
    pub fn get_bytes(&self, name: &[u8]) -> Option<Cow<'_, [u8]>> {
        if let Some(key) = promoted_key(name) {
            if let Some(value) = self.promoted_bytes(key) {
                return Some(value);
            }
        }
        self.other
            .iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(name))
            .map(|entry| Cow::Borrowed(&*entry.value))
    }

    /// The **first** value for `name` as text, or `None` if absent or not valid UTF-8. Use
    /// [`get_bytes`](Headers::get_bytes) for the raw bytes.
    pub fn get(&self, name: &str) -> Option<Cow<'_, str>> {
        match self.get_bytes(name.as_bytes())? {
            Cow::Borrowed(bytes) => core::str::from_utf8(bytes).ok().map(Cow::Borrowed),
            Cow::Owned(bytes) => String::from_utf8(bytes).ok().map(Cow::Owned),
        }
    }

    /// Every raw value for `name`, in insertion order (a promoted key yields at most one).
    pub fn get_all_bytes(&self, name: &[u8]) -> Vec<Cow<'_, [u8]>> {
        if let Some(key) = promoted_key(name) {
            if let Some(value) = self.promoted_bytes(key) {
                return vec![value]; // single-valued promoted key
            }
        }
        self.other
            .iter()
            .filter(|entry| entry.name.eq_ignore_ascii_case(name))
            .map(|entry| Cow::Borrowed(&*entry.value))
            .collect()
    }

    /// Every value for `name` as text, in insertion order (non-UTF-8 values are skipped).
    pub fn get_all(&self, name: &str) -> Vec<Cow<'_, str>> {
        self.get_all_bytes(name.as_bytes())
            .into_iter()
            .filter_map(|value| match value {
                Cow::Borrowed(bytes) => core::str::from_utf8(bytes).ok().map(Cow::Borrowed),
                Cow::Owned(bytes) => String::from_utf8(bytes).ok().map(Cow::Owned),
            })
            .collect()
    }

    /// Whether any entry matches `name` (case-insensitively).
    pub fn contains(&self, name: &str) -> bool {
        let name = name.as_bytes();
        if let Some(key) = promoted_key(name) {
            if self.promoted_present(key) {
                return true;
            }
        }
        self.other
            .iter()
            .any(|entry| entry.name.eq_ignore_ascii_case(name))
    }

    /// The `(name, value)` entries as raw bytes, in canonical order — the present promoted keys
    /// first (fixed order), then the overflow entries in insertion order. A numeric promoted value
    /// is rendered on demand (an owned [`Cow`]); everything else borrows.
    pub fn iter(&self) -> impl Iterator<Item = (&[u8], Cow<'_, [u8]>)> {
        let promoted = PROMOTED_ORDER.iter().filter_map(move |&key| {
            self.promoted_bytes(key)
                .map(|value| (Self::promoted_name(key).as_bytes(), value))
        });
        let overflow = self
            .other
            .iter()
            .map(|entry| (&*entry.name, Cow::Borrowed(&*entry.value)));
        promoted.chain(overflow)
    }

    // ---- write (bytes + str) ------------------------------------------------------------

    /// Appends a `name: value` entry, **keeping** any existing entries with the same name
    /// (multi-value append). A **promoted** key is single-valued, so append replaces it.
    pub fn append_bytes(&mut self, name: &[u8], value: &[u8]) {
        if let Some(key) = promoted_key(name) {
            self.clear_promoted(key);
            self.clear_other_named(name);
            self.store_promoted(key, name, value);
        } else {
            self.other.push(Entry {
                name: name.into(),
                value: value.into(),
            });
        }
    }

    /// [`append_bytes`](Headers::append_bytes) with `&str` arguments.
    pub fn append(&mut self, name: &str, value: &str) {
        self.append_bytes(name.as_bytes(), value.as_bytes());
    }

    /// **Sets** `name` to a single `value` — removes every existing entry with that name,
    /// then stores one (HTTP "replace" semantics).
    pub fn insert_bytes(&mut self, name: &[u8], value: &[u8]) {
        if let Some(key) = promoted_key(name) {
            self.clear_promoted(key);
            self.clear_other_named(name);
            self.store_promoted(key, name, value);
        } else {
            self.clear_other_named(name);
            self.other.push(Entry {
                name: name.into(),
                value: value.into(),
            });
        }
    }

    /// [`insert_bytes`](Headers::insert_bytes) with `&str` arguments.
    pub fn insert(&mut self, name: &str, value: &str) {
        self.insert_bytes(name.as_bytes(), value.as_bytes());
    }

    /// A fresh map with `name` set to a single `value` — the one-line, non-mutating builder
    /// (`headers.with("a", "1").with("b", "2")`).
    pub fn with(mut self, name: &str, value: &str) -> Self {
        self.insert(name, value);
        self
    }

    /// Removes **every** entry matching `name`; returns how many were removed.
    pub fn remove_bytes(&mut self, name: &[u8]) -> usize {
        let mut removed = 0;
        if let Some(key) = promoted_key(name) {
            if self.promoted_present(key) {
                self.clear_promoted(key);
                removed += 1;
            }
        }
        let before = self.other.len();
        self.other
            .retain(|entry| !entry.name.eq_ignore_ascii_case(name));
        removed + (before - self.other.len())
    }

    /// [`remove_bytes`](Headers::remove_bytes) with a `&str` name.
    pub fn remove(&mut self, name: &str) -> usize {
        self.remove_bytes(name.as_bytes())
    }

    /// Removes all entries.
    pub fn clear(&mut self) {
        *self = Self::new();
    }

    /// An explicit copy of this map — the cross-language name for a clone.
    pub fn copy(&self) -> Self {
        self.clone()
    }

    /// Returns a copy of this map overlaid by `other`: every name `other` carries **replaces**
    /// that name here (all occurrences), and names only this map carries are kept.
    pub fn merge_with(&self, other: &Headers) -> Headers {
        let mut merged = self.clone();
        // Drop every name `other` carries, then append all of `other`'s entries (preserving its
        // multi-value overflow keys and its promoted keys).
        for (name, _) in other.iter() {
            merged.remove_bytes(name);
        }
        for (name, value) in other.iter() {
            merged.append_bytes(name, &value);
        }
        merged
    }

    // ---- typed conveniences for common headers -----------------------------------------

    /// The `Content-Type` value, if present — a direct borrow of the typed field.
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    /// Sets the `Content-Type` header — a direct typed-field store (no parse).
    pub fn set_content_type(&mut self, value: &str) {
        self.content_type = Some(value.into());
        self.clear_other_named(Self::CONTENT_TYPE.as_bytes());
    }

    /// The `Content-Encoding` value, if present (e.g. `"gzip"`).
    pub fn content_encoding(&self) -> Option<&str> {
        self.content_encoding.as_deref()
    }

    /// Sets the `Content-Encoding` header.
    pub fn set_content_encoding(&mut self, value: &str) {
        self.content_encoding = Some(value.into());
        self.clear_other_named(Self::CONTENT_ENCODING.as_bytes());
    }

    /// The `Content-Length` value as a `u64`, if present — the hard-typed field, **no parse**.
    ///
    /// ```
    /// use yggdryl_core::headers::Headers;
    ///
    /// let mut headers = Headers::new();
    /// headers.insert(Headers::CONTENT_LENGTH, "1024");
    /// assert_eq!(headers.content_length(), Some(1024));
    /// ```
    pub fn content_length(&self) -> Option<u64> {
        self.content_length
    }

    /// Sets `Content-Length` to `len` — a **direct `u64` store**: no decimal render, no value
    /// allocation. The size half of the content-mutation header sync.
    pub fn set_content_length(&mut self, len: u64) {
        self.content_length = Some(len);
        self.clear_other_named(Self::CONTENT_LENGTH.as_bytes());
    }

    /// The storage **element data type** — the [`DataTypeId`](crate::dtype::DataTypeId) in the typed
    /// field, or [`DataTypeId::Unknown`](crate::dtype::DataTypeId::Unknown) when none is set.
    ///
    /// ```
    /// use yggdryl_core::headers::Headers;
    /// use yggdryl_core::dtype::DataTypeId;
    ///
    /// let mut h = Headers::new();
    /// assert_eq!(h.elem_type_id(), DataTypeId::Unknown);
    /// h.set_elem_type_id(DataTypeId::I64);
    /// assert_eq!(h.elem_type_id(), DataTypeId::I64);
    /// assert_eq!(h.elem_byte_size(), 8);
    /// ```
    pub fn elem_type_id(&self) -> DataTypeId {
        self.elem_type_id
    }

    /// Sets the storage [`DataTypeId`](crate::dtype::DataTypeId) — a direct field store;
    /// [`Unknown`](crate::dtype::DataTypeId::Unknown) clears it (no declared type).
    pub fn set_elem_type_id(&mut self, dtype: DataTypeId) {
        self.elem_type_id = dtype;
        self.clear_other_named(Self::ELEM_TYPE_ID.as_bytes());
    }

    /// The **element storage width** in bytes derived from [`elem_type_id`](Headers::elem_type_id)
    /// (`i64` → 8), or `0` when the type is unknown.
    pub fn elem_byte_size(&self) -> u64 {
        self.elem_type_id.byte_size()
    }

    /// The **element bit width** derived from [`elem_type_id`](Headers::elem_type_id) (`bool` → 1),
    /// or `0` when the type is unknown.
    pub fn elem_bit_size(&self) -> u64 {
        self.elem_type_id.bit_size()
    }

    /// The resource **name** declared under [`NAME`](Headers::NAME), if any.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Sets the resource **name**.
    pub fn set_name(&mut self, name: &str) {
        self.name = Some(name.into());
        self.clear_other_named(Self::NAME.as_bytes());
    }

    /// Sets [`mtime`](Headers::mtime) to **now** (epoch microseconds from the system clock) —
    /// the timestamp half of the content-mutation header sync. Best-effort: a clock before the
    /// epoch stores `0`.
    pub fn touch_mtime(&mut self) {
        let micros = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros().min(i64::MAX as u128) as i64)
            .unwrap_or(0);
        self.set_mtime(micros);
    }

    // ---- media type: the one place Content-Type / Content-Encoding are interpreted ------

    /// The **primary [`MimeType`]** of `Content-Type`, if present and valid — the single most
    /// specific type this map declares. `None` when there is no (valid) `Content-Type`.
    ///
    /// ```
    /// use yggdryl_core::headers::Headers;
    ///
    /// let mut headers = Headers::new();
    /// headers.set_content_type("application/json; charset=utf-8");
    /// assert_eq!(headers.mime_type().unwrap().essence(), "application/json");
    /// ```
    pub fn mime_type(&self) -> Option<MimeType> {
        // Content-Type may be a comma list (set from a MediaType) — the primary is the first.
        let content_type = self.content_type()?;
        let primary = content_type.split(',').next().unwrap_or(content_type);
        MimeType::parse_str(primary).ok()
    }

    /// Sets `Content-Type` to `mime`'s essence — the centralized mime mutator.
    pub fn set_mime_type(&mut self, mime: &MimeType) {
        self.set_content_type(mime.essence());
    }

    /// The full **[`MediaType`]** this map declares: the `Content-Type` (a comma-list is kept
    /// as multiple entries), extended by the `Content-Encoding` layers resolved to their
    /// mime types (`gzip` → `application/gzip`). `None` when there is no `Content-Type`.
    ///
    /// ```
    /// use yggdryl_core::headers::Headers;
    ///
    /// let mut headers = Headers::new();
    /// headers.set_content_type("application/x-tar");
    /// headers.set_content_encoding("gzip");
    /// assert_eq!(headers.media_type().unwrap().essences(),
    ///            vec!["application/x-tar", "application/gzip"]);
    /// ```
    pub fn media_type(&self) -> Option<MediaType> {
        let mut media = MediaType::parse_str(self.content_type()?).ok()?;
        if let Some(encoding) = self.content_encoding() {
            // Each comma-separated encoding token maps to its mime type when known.
            for token in encoding.split(',') {
                if let Some(mime) = MimeType::from_extension(token.trim())
                    .or_else(|| MimeType::parse_str(&format!("application/{}", token.trim())).ok())
                {
                    media.push(mime);
                }
            }
        }
        Some(media)
    }

    /// Sets `Content-Type` to `media`'s comma-joined essences — the centralized media mutator
    /// (the inverse of [`media_type`](Headers::media_type)'s `Content-Type` half).
    pub fn set_media_type(&mut self, media: &MediaType) {
        self.set_content_type(&media.to_string());
    }

    // ---- modification time (epoch microseconds) -----------------------------------------

    /// The modification time as **total epoch microseconds** (signed — before 1970 is
    /// negative), from the typed field, if set.
    ///
    /// ```
    /// use yggdryl_core::headers::Headers;
    ///
    /// let mut headers = Headers::new();
    /// headers.set_mtime(1_600_000_000_000_000);
    /// assert_eq!(headers.mtime(), Some(1_600_000_000_000_000));
    /// ```
    pub fn mtime(&self) -> Option<i64> {
        self.mtime
    }

    /// Sets the modification time to `micros` total epoch microseconds — a **direct `i64` store**
    /// (no decimal render, no allocation).
    pub fn set_mtime(&mut self, micros: i64) {
        self.mtime = Some(micros);
        self.clear_other_named(Self::MTIME.as_bytes());
    }

    // ---- HTTP text form -----------------------------------------------------------------

    /// Renders the header block in HTTP wire form — `Name: Value\r\n` per entry (no trailing
    /// blank line), in canonical order. One pre-sized allocation.
    ///
    /// ```
    /// use yggdryl_core::headers::Headers;
    ///
    /// let mut headers = Headers::new();
    /// headers.insert("Host", "example.com");
    /// assert_eq!(headers.to_http_bytes(), b"Host: example.com\r\n");
    /// ```
    pub fn to_http_bytes(&self) -> Vec<u8> {
        let pairs: Vec<(&[u8], Cow<'_, [u8]>)> = self.iter().collect();
        let size: usize = pairs
            .iter()
            .map(|(name, value)| name.len() + value.len() + 4) // ": " + "\r\n"
            .sum();
        let mut out = Vec::with_capacity(size);
        for (name, value) in &pairs {
            out.extend_from_slice(name);
            out.extend_from_slice(b": ");
            out.extend_from_slice(value);
            out.extend_from_slice(b"\r\n");
        }
        out
    }

    /// Parses an HTTP header block: one `Name: Value` per line (`\r\n` or `\n`), splitting on
    /// the first colon and trimming optional whitespace around the value. **Lenient** — a
    /// blank line stops parsing (the header/body boundary) and a line with no colon is
    /// skipped rather than erroring.
    ///
    /// ```
    /// use yggdryl_core::headers::Headers;
    ///
    /// let headers = Headers::parse_http(b"Host: example.com\r\nAccept: */*\r\n");
    /// assert_eq!(headers.get("host").as_deref(), Some("example.com"));
    /// assert_eq!(headers.get("accept").as_deref(), Some("*/*"));
    /// ```
    pub fn parse_http(bytes: &[u8]) -> Self {
        let mut headers = Self::new();
        for line in bytes.split(|&byte| byte == b'\n') {
            let line = trim_ascii(strip_cr(line));
            if line.is_empty() {
                break; // the blank line ends the header block
            }
            if let Some(colon) = line.iter().position(|&byte| byte == b':') {
                let name = trim_ascii(&line[..colon]);
                let value = trim_ascii(&line[colon + 1..]);
                if !name.is_empty() {
                    headers.append_bytes(name, value);
                }
            }
        }
        headers
    }

    // ---- binary codec (the Serializable value form) -------------------------------------

    /// The map as a length-prefixed binary frame —
    /// `[count: u32][ (name_len: u32) name (value_len: u32) value ]*`, little-endian, in canonical
    /// order — built in **one pre-sized allocation**. Unlike the HTTP text form this round-trips
    /// **arbitrary** bytes (names/values may contain `:` or `\r\n`) and multi-value overflow
    /// entries; [`deserialize_bytes`](Headers::deserialize_bytes) is the exact inverse.
    ///
    /// ```
    /// use yggdryl_core::headers::Headers;
    ///
    /// let mut headers = Headers::new();
    /// headers.append("Set-Cookie", "a=1");
    /// headers.append("Set-Cookie", "b=2");
    /// assert_eq!(Headers::deserialize_bytes(&headers.serialize_bytes()).unwrap(), headers);
    /// ```
    pub fn serialize_bytes(&self) -> Vec<u8> {
        let pairs: Vec<(&[u8], Cow<'_, [u8]>)> = self.iter().collect();
        let size: usize = 4 + pairs
            .iter()
            .map(|(name, value)| 8 + name.len() + value.len())
            .sum::<usize>();
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(&(pairs.len() as u32).to_le_bytes());
        for (name, value) in &pairs {
            out.extend_from_slice(&(name.len() as u32).to_le_bytes());
            out.extend_from_slice(name);
            out.extend_from_slice(&(value.len() as u32).to_le_bytes());
            out.extend_from_slice(value);
        }
        out
    }

    /// Reconstructs a map from bytes produced by [`serialize_bytes`](Headers::serialize_bytes).
    /// Errors with [`IoError::UnexpectedEof`] (naming the shortfall) if the frame is truncated.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        let mut offset = 0usize;
        let count = take_u32(bytes, &mut offset)? as usize;
        let mut headers = Self::with_capacity(count.min(bytes.len() / 8));
        for _ in 0..count {
            let name_len = take_u32(bytes, &mut offset)? as usize;
            let name = take_bytes(bytes, &mut offset, name_len)?;
            let value_len = take_u32(bytes, &mut offset)? as usize;
            let value = take_bytes(bytes, &mut offset, value_len)?;
            // Route by name: a promoted key lands in its typed field (or the overflow fallback);
            // every other name is pushed directly, preserving order and duplicates.
            if let Some(key) = promoted_key(name) {
                headers.clear_promoted(key);
                headers.clear_other_named(name);
                headers.store_promoted(key, name, value);
            } else {
                headers.other.push(Entry {
                    name: name.into(),
                    value: value.into(),
                });
            }
        }
        Ok(headers)
    }
}

impl Serializable for Headers {
    type Error = IoError;

    fn serialize_bytes(&self) -> Vec<u8> {
        Headers::serialize_bytes(self)
    }

    fn deserialize_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        Headers::deserialize_bytes(bytes)
    }
}

/// Matches `name` (case-insensitively) against the six promoted keys.
fn promoted_key(name: &[u8]) -> Option<Promoted> {
    PROMOTED_ORDER
        .into_iter()
        .find(|&key| name.eq_ignore_ascii_case(Headers::promoted_name(key).as_bytes()))
}

/// Renders an unsigned `value` as a fresh decimal byte buffer (for the numeric map view).
fn render_u64(value: u64) -> Vec<u8> {
    let mut buf = [0u8; 20];
    write_u64(&mut buf, value).to_vec()
}

/// Renders a signed `value` as a fresh decimal byte buffer (for the numeric map view).
fn render_i64(value: i64) -> Vec<u8> {
    let mut buf = [0u8; 20];
    write_i64(&mut buf, value).to_vec()
}

/// Formats an unsigned `value` as decimal into `buf` and returns the written slice — the
/// allocation-free integer render.
fn write_u64(buf: &mut [u8; 20], value: u64) -> &[u8] {
    let mut i = buf.len();
    let mut mag = value;
    loop {
        i -= 1;
        buf[i] = b'0' + (mag % 10) as u8;
        mag /= 10;
        if mag == 0 {
            break;
        }
    }
    &buf[i..]
}

/// Formats `value` as decimal into `buf` and returns the written slice — an allocation-free
/// integer render (no `format!`/`String`).
fn write_i64(buf: &mut [u8; 20], value: i64) -> &[u8] {
    let mut i = buf.len();
    let negative = value < 0;
    // Work on the magnitude via i128 so i64::MIN does not overflow on negation.
    let mut mag = (value as i128).unsigned_abs();
    loop {
        i -= 1;
        buf[i] = b'0' + (mag % 10) as u8;
        mag /= 10;
        if mag == 0 {
            break;
        }
    }
    if negative {
        i -= 1;
        buf[i] = b'-';
    }
    &buf[i..]
}

/// Reads a little-endian `u32` at `*offset`, advancing it, or reports the shortfall.
fn take_u32(bytes: &[u8], offset: &mut usize) -> Result<u32, IoError> {
    let slice = take_bytes(bytes, offset, 4)?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// Borrows exactly `len` bytes at `*offset`, advancing it, or reports the shortfall.
fn take_bytes<'a>(bytes: &'a [u8], offset: &mut usize, len: usize) -> Result<&'a [u8], IoError> {
    let end = offset.checked_add(len).filter(|&end| end <= bytes.len());
    match end {
        Some(end) => {
            let slice = &bytes[*offset..end];
            *offset = end;
            Ok(slice)
        }
        None => Err(IoError::UnexpectedEof {
            offset: *offset as u64,
            requested: len,
            available: bytes.len().saturating_sub(*offset),
        }),
    }
}

/// Drops a single trailing `\r` (for `\r\n` line endings).
fn strip_cr(line: &[u8]) -> &[u8] {
    match line {
        [head @ .., b'\r'] => head,
        _ => line,
    }
}

/// Trims leading/trailing ASCII whitespace without allocating.
fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while let [first, rest @ ..] = bytes {
        if first.is_ascii_whitespace() {
            bytes = rest;
        } else {
            break;
        }
    }
    while let [rest @ .., last] = bytes {
        if last.is_ascii_whitespace() {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}

impl fmt::Debug for Headers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Header sets are small; render name -> value with lossy UTF-8 for readability.
        f.debug_map()
            .entries(self.iter().map(|(name, value)| {
                (
                    String::from_utf8_lossy(name).into_owned(),
                    String::from_utf8_lossy(&value).into_owned(),
                )
            }))
            .finish()
    }
}
