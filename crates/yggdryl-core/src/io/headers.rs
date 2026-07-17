//! [`Headers`] — an ordered, case-insensitive, multi-value map of **byte-string** keys to
//! **byte-string** values, with `&str` conveniences, following HTTP header conventions.

use core::fmt;

use crate::io::{IOCursor, IoError};

/// One `name: value` entry — both stored as owned byte strings (`stored as bytes/bytes`).
#[derive(Clone, PartialEq, Eq, Hash)]
struct Entry {
    name: Box<[u8]>,
    value: Box<[u8]>,
}

/// A generic **header map**: byte-string keys → byte-string values, kept in **insertion
/// order**, **case-insensitive** on the name (ASCII, per HTTP), and **multi-value** (a name
/// may repeat). `&str` accessors sit over the byte storage for the common textual case, while
/// `*_bytes` accessors reach the raw bytes for anything that is not UTF-8.
///
/// DESIGN: entries live in one insertion-ordered `Vec`, scanned linearly. For the small `n`
/// of a real header set (typically well under 30) a linear scan is faster and more
/// cache-friendly than hashing, and it preserves order and duplicates exactly — the two
/// things HTTP requires. Names are compared with [`eq_ignore_ascii_case`](slice::eq_ignore_ascii_case),
/// so matching allocates nothing.
///
/// ```
/// use yggdryl_core::io::Headers;
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

    // ---- construction -------------------------------------------------------------------

    /// An empty header map.
    pub fn new() -> Self {
        Self::default()
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

    // ---- typed conveniences for common headers -----------------------------------------

    /// The `Content-Type` value, if present and UTF-8.
    pub fn content_type(&self) -> Option<&str> {
        self.get(Self::CONTENT_TYPE)
    }

    /// The `Content-Length` value parsed as a `u64`, if present and numeric.
    ///
    /// ```
    /// use yggdryl_core::io::Headers;
    ///
    /// let mut headers = Headers::new();
    /// headers.insert(Headers::CONTENT_LENGTH, "1024");
    /// assert_eq!(headers.content_length(), Some(1024));
    /// ```
    pub fn content_length(&self) -> Option<u64> {
        self.get(Self::CONTENT_LENGTH)?.trim().parse().ok()
    }

    // ---- HTTP text form -----------------------------------------------------------------

    /// Renders the header block in HTTP wire form — `Name: Value\r\n` per entry (no trailing
    /// blank line). One pre-sized allocation.
    ///
    /// ```
    /// use yggdryl_core::io::Headers;
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
    /// use yggdryl_core::io::Headers;
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

    // ---- binary codec over the IO abstraction ------------------------------------------

    /// Writes the map to `sink` in a robust length-prefixed binary form —
    /// `[count: u32][ (name_len: u32) name (value_len: u32) value ]*` — through the
    /// [`IOCursor`] abstraction. Unlike the HTTP text form this round-trips **arbitrary**
    /// bytes (names/values may contain `:` or `\r\n`).
    pub fn write_to<W: IOCursor>(&self, sink: &mut W) -> Result<(), IoError> {
        sink.write_all(&(self.entries.len() as u32).to_le_bytes())?;
        for entry in &self.entries {
            sink.write_all(&(entry.name.len() as u32).to_le_bytes())?;
            sink.write_all(&entry.name)?;
            sink.write_all(&(entry.value.len() as u32).to_le_bytes())?;
            sink.write_all(&entry.value)?;
        }
        Ok(())
    }

    /// Reads a map written by [`write_to`](Headers::write_to) from `source`. Errors
    /// ([`IoError::UnexpectedEof`]) if the frame is truncated.
    pub fn read_from<R: IOCursor>(source: &mut R) -> Result<Self, IoError> {
        let count = source.read_u32()? as usize;
        let mut headers = Self::with_capacity(count);
        for _ in 0..count {
            let name_len = source.read_u32()? as usize;
            let name = read_bytes(source, name_len)?;
            let value_len = source.read_u32()? as usize;
            let value = read_bytes(source, value_len)?;
            headers.entries.push(Entry {
                name: name.into_boxed_slice(),
                value: value.into_boxed_slice(),
            });
        }
        Ok(headers)
    }

    /// The map as a fresh byte vector — the [`write_to`](Headers::write_to) frame collected into a
    /// `Vec`, the **exact inverse** of [`deserialize_bytes`](Headers::deserialize_bytes). This is
    /// the value codec: it round-trips **arbitrary** bytes, insertion order, and multi-value
    /// entries (unlike the lossy HTTP text or single-value Arrow metadata forms).
    ///
    /// ```
    /// use yggdryl_core::io::Headers;
    ///
    /// let mut headers = Headers::new();
    /// headers.append("Set-Cookie", "a=1");
    /// headers.append("Set-Cookie", "b=2");
    /// assert_eq!(Headers::deserialize_bytes(&headers.serialize_bytes()).unwrap(), headers);
    /// ```
    pub fn serialize_bytes(&self) -> Vec<u8> {
        let mut sink = crate::io::Bytes::new();
        // Writing into an in-memory `Bytes` cursor is infallible.
        self.write_to(&mut sink)
            .expect("in-memory Headers serialization is infallible");
        sink.to_vec()
    }

    /// Reconstructs a map from bytes produced by [`serialize_bytes`](Headers::serialize_bytes).
    /// Errors ([`IoError::UnexpectedEof`]) if the frame is truncated.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        let mut source = crate::io::Bytes::from_slice(bytes);
        Self::read_from(&mut source)
    }

    /// The (UTF-8) entries as Arrow's `HashMap<String, String>` field metadata (feature `arrow`).
    /// Non-UTF-8 or repeated keys collapse (Arrow metadata is single-value `String → String`).
    #[cfg(feature = "arrow")]
    pub(crate) fn to_arrow_metadata(&self) -> std::collections::HashMap<String, String> {
        self.entries
            .iter()
            .filter_map(|entry| {
                let name = std::str::from_utf8(&entry.name).ok()?;
                let value = std::str::from_utf8(&entry.value).ok()?;
                Some((name.to_string(), value.to_string()))
            })
            .collect()
    }

    /// Field metadata from Arrow's `HashMap<String, String>` (feature `arrow`). Entries are
    /// inserted in **sorted key order**, so a decode is deterministic regardless of the map's
    /// iteration order.
    #[cfg(feature = "arrow")]
    pub(crate) fn from_arrow_metadata(map: &std::collections::HashMap<String, String>) -> Self {
        let mut pairs: Vec<(&String, &String)> = map.iter().collect();
        pairs.sort();
        let mut headers = Self::with_capacity(pairs.len());
        for (name, value) in pairs {
            headers.insert(name, value);
        }
        headers
    }
}

/// Reads exactly `len` bytes from a cursor into a fresh `Vec` — through the **bounded**
/// [`read_exact_vec`](IOCursor::read_exact_vec) so a corrupt/hostile declared length errors cleanly
/// instead of allocating it up front (matches the Serie/Scalar readers).
fn read_bytes<R: IOCursor>(source: &mut R, len: usize) -> Result<Vec<u8>, IoError> {
    source.read_exact_vec(len)
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
