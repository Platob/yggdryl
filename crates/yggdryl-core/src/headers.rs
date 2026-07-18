//! [`Headers`] — an ordered, case-insensitive, multi-value map of **byte-string** keys to
//! **byte-string** values, with `&str` conveniences, following HTTP header conventions.

use core::fmt;

use crate::io::{IoError, Serializable};
use crate::mediatype::MediaType;
use crate::mimetype::MimeType;

/// One `name: value` entry — both stored as owned byte strings.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Entry {
    name: Box<[u8]>,
    value: Box<[u8]>,
}

/// The project's **one** metadata map: byte-string keys → byte-string values, kept in
/// **insertion order**, **case-insensitive** on the name (ASCII, per HTTP), and **multi-value**
/// (a name may repeat). `&str` accessors sit over the byte storage for the common textual case,
/// while `*_bytes` accessors reach the raw bytes for anything that is not UTF-8. HTTP headers,
/// schema/field metadata, source annotations — all of them are a `Headers`; every
/// [`IOBase`](crate::io::memory::IOBase) source carries one
/// ([`headers`](crate::io::memory::IOBase::headers) /
/// [`headers_mut`](crate::io::memory::IOBase::headers_mut)).
///
/// DESIGN: entries live in one insertion-ordered `Vec`, scanned linearly. For the small `n` of
/// a real header set (typically well under 30) a linear scan is faster and more cache-friendly
/// than hashing, and it preserves order and duplicates exactly — the two things HTTP requires.
/// Names are compared with [`eq_ignore_ascii_case`](slice::eq_ignore_ascii_case), so matching
/// allocates nothing.
///
/// ```
/// use yggdryl_core::headers::Headers;
///
/// let mut headers = Headers::new();
/// headers.insert(Headers::CONTENT_TYPE, "application/json");
/// headers.append("Set-Cookie", "a=1");
/// headers.append("Set-Cookie", "b=2");
///
/// assert_eq!(headers.get("content-type"), Some("application/json")); // case-insensitive
/// assert_eq!(headers.get_all("set-cookie"), vec!["a=1", "b=2"]);      // multi-value
/// assert!(headers.contains("Content-Type"));
/// ```
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct Headers {
    entries: Vec<Entry>,
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
    /// The storage **element data type** header — a [`DataTypeId`](crate::datatype_id::DataTypeId) as its
    /// `u16` id ([`type_id`](Headers::type_id) / [`set_type_id`](Headers::set_type_id)).
    pub const TYPE_ID: &'static str = "X-Type-Id";
    /// The resource **name** header ([`name`](Headers::name) / [`set_name`](Headers::set_name)).
    pub const NAME: &'static str = "X-Name";
    /// The **nullable** flag header — whether a typed field/column admits nulls
    /// ([`nullable`](Headers::nullable) / [`set_nullable`](Headers::set_nullable)).
    pub const NULLABLE: &'static str = "X-Nullable";

    // ---- construction -------------------------------------------------------------------

    /// An empty header map (no allocation — `const`, so it can back a `static` empty map).
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// An empty map with room for `capacity` entries before its first reallocation.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
        }
    }

    // ---- read (bytes + str) -------------------------------------------------------------

    /// The number of entries (a repeated name counts once per occurrence).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the map has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The raw value of the **first** entry whose name matches `name` (case-insensitively).
    pub fn get_bytes(&self, name: &[u8]) -> Option<&[u8]> {
        self.entries
            .iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(name))
            .map(|entry| &*entry.value)
    }

    /// The **first** value for `name` as `&str`, or `None` if absent or not valid UTF-8. Use
    /// [`get_bytes`](Headers::get_bytes) for the raw bytes.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.get_bytes(name.as_bytes())
            .and_then(|value| core::str::from_utf8(value).ok())
    }

    /// Every raw value for `name`, in insertion order.
    pub fn get_all_bytes(&self, name: &[u8]) -> Vec<&[u8]> {
        self.entries
            .iter()
            .filter(|entry| entry.name.eq_ignore_ascii_case(name))
            .map(|entry| &*entry.value)
            .collect()
    }

    /// Every value for `name` as `&str`, in insertion order (non-UTF-8 values are skipped).
    pub fn get_all(&self, name: &str) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|entry| entry.name.eq_ignore_ascii_case(name.as_bytes()))
            .filter_map(|entry| core::str::from_utf8(&entry.value).ok())
            .collect()
    }

    /// Whether any entry matches `name` (case-insensitively).
    pub fn contains(&self, name: &str) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.name.eq_ignore_ascii_case(name.as_bytes()))
    }

    /// The `(name, value)` entries as raw bytes, in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.entries
            .iter()
            .map(|entry| (&*entry.name, &*entry.value))
    }

    // ---- write (bytes + str) ------------------------------------------------------------

    /// Appends a `name: value` entry, **keeping** any existing entries with the same name
    /// (multi-value append).
    pub fn append_bytes(&mut self, name: &[u8], value: &[u8]) {
        self.entries.push(Entry {
            name: name.into(),
            value: value.into(),
        });
    }

    /// [`append_bytes`](Headers::append_bytes) with `&str` arguments.
    pub fn append(&mut self, name: &str, value: &str) {
        self.append_bytes(name.as_bytes(), value.as_bytes());
    }

    /// **Sets** `name` to a single `value` — removes every existing entry with that name,
    /// then appends one (HTTP "replace" semantics).
    pub fn insert_bytes(&mut self, name: &[u8], value: &[u8]) {
        self.remove_bytes(name);
        self.append_bytes(name, value);
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
        let before = self.entries.len();
        self.entries
            .retain(|entry| !entry.name.eq_ignore_ascii_case(name));
        before - self.entries.len()
    }

    /// [`remove_bytes`](Headers::remove_bytes) with a `&str` name.
    pub fn remove(&mut self, name: &str) -> usize {
        self.remove_bytes(name.as_bytes())
    }

    /// Removes all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// An explicit copy of this map — the cross-language name for a clone.
    pub fn copy(&self) -> Self {
        self.clone()
    }

    /// Returns a copy of this map overlaid by `other`: every name `other` carries **replaces**
    /// that name here (all occurrences), and names only this map carries are kept.
    pub fn merge_with(&self, other: &Headers) -> Headers {
        let mut merged = self.clone();
        for entry in &other.entries {
            merged.remove_bytes(&entry.name);
        }
        merged.entries.extend(other.entries.iter().cloned());
        merged
    }

    // ---- typed conveniences for common headers -----------------------------------------

    /// The `Content-Type` value, if present and UTF-8.
    pub fn content_type(&self) -> Option<&str> {
        self.get(Self::CONTENT_TYPE)
    }

    /// Sets the `Content-Type` header (replace semantics).
    pub fn set_content_type(&mut self, value: &str) {
        self.insert(Self::CONTENT_TYPE, value);
    }

    /// The `Content-Encoding` value, if present and UTF-8 (e.g. `"gzip"`).
    pub fn content_encoding(&self) -> Option<&str> {
        self.get(Self::CONTENT_ENCODING)
    }

    /// Sets the `Content-Encoding` header (replace semantics).
    pub fn set_content_encoding(&mut self, value: &str) {
        self.insert(Self::CONTENT_ENCODING, value);
    }

    /// The `Content-Length` value parsed as a `u64`, if present and numeric.
    ///
    /// ```
    /// use yggdryl_core::headers::Headers;
    ///
    /// let mut headers = Headers::new();
    /// headers.insert(Headers::CONTENT_LENGTH, "1024");
    /// assert_eq!(headers.content_length(), Some(1024));
    /// ```
    pub fn content_length(&self) -> Option<u64> {
        self.get(Self::CONTENT_LENGTH)?.trim().parse().ok()
    }

    /// Sets `Content-Length` to `len` — a compact decimal rendered into a stack buffer (no
    /// `String` temporary), the alloc-free counterpart of [`content_length`](Headers::content_length).
    /// The size half of the content-mutation header sync.
    pub fn set_content_length(&mut self, len: u64) {
        let mut buf = [0u8; 20];
        let text = write_u64(&mut buf, len);
        self.insert_bytes(Self::CONTENT_LENGTH.as_bytes(), text);
    }

    /// The storage **element data type** — the [`DataTypeId`](crate::datatype_id::DataTypeId) declared
    /// under [`TYPE_ID`](Headers::TYPE_ID), or [`DataTypeId::Unknown`](crate::datatype_id::DataTypeId::Unknown)
    /// when none is set. Total (never fails — an unrecognized id reads as `Unknown`).
    ///
    /// ```
    /// use yggdryl_core::headers::Headers;
    /// use yggdryl_core::datatype_id::DataTypeId;
    ///
    /// let mut h = Headers::new();
    /// assert_eq!(h.type_id(), DataTypeId::Unknown);
    /// h.set_type_id(DataTypeId::I64);
    /// assert_eq!(h.type_id(), DataTypeId::I64);
    /// assert_eq!(h.type_byte_size(), 8);
    /// ```
    pub fn type_id(&self) -> crate::datatype_id::DataTypeId {
        match self.get(Self::TYPE_ID).and_then(|v| v.trim().parse().ok()) {
            Some(id) => crate::datatype_id::DataTypeId::from_u16(id),
            None => crate::datatype_id::DataTypeId::Unknown,
        }
    }

    /// Sets the storage [`DataTypeId`](crate::datatype_id::DataTypeId) (its `u16` id, alloc-free decimal
    /// render). [`Unknown`](crate::datatype_id::DataTypeId::Unknown) **removes** the header (no declared type).
    pub fn set_type_id(&mut self, dtype: crate::datatype_id::DataTypeId) {
        if dtype == crate::datatype_id::DataTypeId::Unknown {
            self.remove(Self::TYPE_ID);
            return;
        }
        let mut buf = [0u8; 20];
        let text = write_u64(&mut buf, dtype.as_u16() as u64);
        self.insert_bytes(Self::TYPE_ID.as_bytes(), text);
    }

    /// The **element storage width** in bytes derived from [`type_id`](Headers::type_id)
    /// (`i64` → 8), or `0` when the type is unknown.
    pub fn type_byte_size(&self) -> u64 {
        self.type_id().byte_size()
    }

    /// The **element bit width** derived from [`type_id`](Headers::type_id) (`bool` → 1),
    /// or `0` when the type is unknown.
    pub fn type_bit_size(&self) -> u64 {
        self.type_id().bit_size()
    }

    /// The resource **name** declared under [`NAME`](Headers::NAME), if any.
    pub fn name(&self) -> Option<&str> {
        self.get(Self::NAME)
    }

    /// Sets the resource **name** (replaces).
    pub fn set_name(&mut self, name: &str) {
        self.insert(Self::NAME, name);
    }

    /// Whether the field/column this metadata describes **admits nulls** — the
    /// [`NULLABLE`](Headers::NULLABLE) flag, `false` when unset (the safe default: a column with no
    /// declared nullability is treated as non-nullable).
    ///
    /// ```
    /// use yggdryl_core::headers::Headers;
    ///
    /// let mut h = Headers::new();
    /// assert!(!h.nullable()); // unset -> non-nullable
    /// h.set_nullable(true);
    /// assert!(h.nullable());
    /// ```
    pub fn nullable(&self) -> bool {
        matches!(self.get(Self::NULLABLE), Some("true" | "1"))
    }

    /// Sets the [`NULLABLE`](Headers::NULLABLE) flag (`"true"` / `"false"`).
    pub fn set_nullable(&mut self, nullable: bool) {
        self.insert(Self::NULLABLE, if nullable { "true" } else { "false" });
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
    /// negative), from the [`MTIME`](Headers::MTIME) header, if present and an integer.
    ///
    /// ```
    /// use yggdryl_core::headers::Headers;
    ///
    /// let mut headers = Headers::new();
    /// headers.set_mtime(1_600_000_000_000_000);
    /// assert_eq!(headers.mtime(), Some(1_600_000_000_000_000));
    /// ```
    pub fn mtime(&self) -> Option<i64> {
        self.get(Self::MTIME)?.trim().parse().ok()
    }

    /// Sets the modification time to `micros` total epoch microseconds — written as a compact
    /// decimal into the [`MTIME`](Headers::MTIME) header (one small allocation via `itoa`-free
    /// integer formatting).
    pub fn set_mtime(&mut self, micros: i64) {
        let mut buf = [0u8; 20]; // enough for i64::MIN's decimal digits + sign
        let text = write_i64(&mut buf, micros);
        self.insert_bytes(Self::MTIME.as_bytes(), text);
    }

    // ---- HTTP text form -----------------------------------------------------------------

    /// Renders the header block in HTTP wire form — `Name: Value\r\n` per entry (no trailing
    /// blank line). One pre-sized allocation.
    ///
    /// ```
    /// use yggdryl_core::headers::Headers;
    ///
    /// let mut headers = Headers::new();
    /// headers.insert("Host", "example.com");
    /// assert_eq!(headers.to_http_bytes(), b"Host: example.com\r\n");
    /// ```
    pub fn to_http_bytes(&self) -> Vec<u8> {
        let size: usize = self
            .entries
            .iter()
            .map(|entry| entry.name.len() + entry.value.len() + 4) // ": " + "\r\n"
            .sum();
        let mut out = Vec::with_capacity(size);
        for entry in &self.entries {
            out.extend_from_slice(&entry.name);
            out.extend_from_slice(b": ");
            out.extend_from_slice(&entry.value);
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
    /// assert_eq!(headers.get("host"), Some("example.com"));
    /// assert_eq!(headers.get("accept"), Some("*/*"));
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
    /// `[count: u32][ (name_len: u32) name (value_len: u32) value ]*`, little-endian — built in
    /// **one pre-sized allocation**. Unlike the HTTP text form this round-trips **arbitrary**
    /// bytes (names/values may contain `:` or `\r\n`), insertion order, and multi-value entries;
    /// [`deserialize_bytes`](Headers::deserialize_bytes) is the exact inverse.
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
        let size: usize = 4 + self
            .entries
            .iter()
            .map(|entry| 8 + entry.name.len() + entry.value.len())
            .sum::<usize>();
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for entry in &self.entries {
            out.extend_from_slice(&(entry.name.len() as u32).to_le_bytes());
            out.extend_from_slice(&entry.name);
            out.extend_from_slice(&(entry.value.len() as u32).to_le_bytes());
            out.extend_from_slice(&entry.value);
        }
        out
    }

    /// Reconstructs a map from bytes produced by [`serialize_bytes`](Headers::serialize_bytes).
    /// Errors with [`IoError::UnexpectedEof`] (naming the shortfall) if the frame is truncated.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        let mut offset = 0usize;
        let count = take_u32(bytes, &mut offset)? as usize;
        // Cap the pre-size by what the frame could possibly hold (2 u32s per entry minimum),
        // so a corrupt count cannot trigger a runaway allocation.
        let mut headers = Self::with_capacity(count.min(bytes.len() / 8));
        for _ in 0..count {
            let name_len = take_u32(bytes, &mut offset)? as usize;
            let name = take_bytes(bytes, &mut offset, name_len)?;
            let value_len = take_u32(bytes, &mut offset)? as usize;
            let value = take_bytes(bytes, &mut offset, value_len)?;
            headers.entries.push(Entry {
                name: name.into(),
                value: value.into(),
            });
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

/// Formats an unsigned `value` as decimal into `buf` — the alloc-free render for
/// `set_content_length`.
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
/// integer render (no `format!`/`String`), for the hot `set_mtime` path.
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
            .entries(self.entries.iter().map(|entry| {
                (
                    String::from_utf8_lossy(&entry.name),
                    String::from_utf8_lossy(&entry.value),
                )
            }))
            .finish()
    }
}
