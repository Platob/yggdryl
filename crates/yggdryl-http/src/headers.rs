//! [`Headers`] — an ordered, byte-based HTTP-style header map.

use std::collections::BTreeMap;

use crate::HeadersError;

/// An ordered **bytes → bytes** map, modelled on an HTTP header block.
///
/// Keys and values are arbitrary bytes, so binary annotations round-trip losslessly; the
/// map is ordered (a `BTreeMap`) so [`serialize_bytes`](Headers::serialize_bytes) is
/// deterministic and two maps are equal **iff** their serialised bytes are equal. Beyond
/// the byte API it offers UTF-8 **string** accessors/mutators, **zero-copy** in-place
/// value mutation via [`get_mut`](Headers::get_mut) (no clone, no re-insert), and
/// pre-built accessors for the common keys ([`NAME`](Headers::NAME),
/// [`COMMENT`](Headers::COMMENT), [`CONTENT_TYPE`](Headers::CONTENT_TYPE),
/// [`CONTENT_ENCODING`](Headers::CONTENT_ENCODING)).
///
/// ```
/// use yggdryl_http::Headers;
///
/// let mut headers = Headers::new();
/// headers.set_content_type("text/plain");        // pre-built string mutator
/// assert_eq!(headers.content_type(), Some(b"text/plain".as_slice()));
///
/// // Zero-copy mutation: extend the value's bytes in place.
/// headers.get_mut(Headers::CONTENT_TYPE).unwrap().extend_from_slice(b"; charset=utf-8");
/// assert_eq!(headers.content_type(), Some(b"text/plain; charset=utf-8".as_slice()));
///
/// assert_eq!(Headers::deserialize_bytes(&headers.serialize_bytes()).unwrap(), headers);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Headers {
    entries: BTreeMap<Vec<u8>, Vec<u8>>,
}

impl Headers {
    /// The `name` header key.
    pub const NAME: &'static [u8] = b"name";
    /// The `comment` header key.
    pub const COMMENT: &'static [u8] = b"comment";
    /// The `content-type` header key.
    pub const CONTENT_TYPE: &'static [u8] = b"content-type";
    /// The `content-encoding` header key.
    pub const CONTENT_ENCODING: &'static [u8] = b"content-encoding";

    /// Creates an empty header map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a header map from `(key, value)` byte pairs (later duplicates win).
    pub fn from_pairs(pairs: impl IntoIterator<Item = (Vec<u8>, Vec<u8>)>) -> Self {
        Self {
            entries: pairs.into_iter().collect(),
        }
    }

    // ---- byte access -------------------------------------------------------------

    /// The value for the byte `key`, or `None`.
    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.entries.get(key).map(Vec::as_slice)
    }

    /// A **mutable** handle to the value for `key`, for **zero-copy** in-place mutation
    /// (append/patch the bytes without cloning the map or re-inserting).
    pub fn get_mut(&mut self, key: &[u8]) -> Option<&mut Vec<u8>> {
        self.entries.get_mut(key)
    }

    /// Whether `key` is present.
    pub fn contains(&self, key: &[u8]) -> bool {
        self.entries.contains_key(key)
    }

    /// Inserts (or replaces) `key` → `value`, returning the previous value if any.
    pub fn insert(
        &mut self,
        key: impl Into<Vec<u8>>,
        value: impl Into<Vec<u8>>,
    ) -> Option<Vec<u8>> {
        self.entries.insert(key.into(), value.into())
    }

    /// Removes `key`, returning its value if it was present.
    pub fn remove(&mut self, key: &[u8]) -> Option<Vec<u8>> {
        self.entries.remove(key)
    }

    /// Removes all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// The number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the map holds no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterates the `(key, value)` pairs in key order.
    pub fn pairs(&self) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.entries
            .iter()
            .map(|(key, value)| (key.as_slice(), value.as_slice()))
    }

    // ---- string access (UTF-8 convenience) ---------------------------------------

    /// The value for the UTF-8 string `key`, or `None`.
    pub fn get_str(&self, key: &str) -> Option<&[u8]> {
        self.get(key.as_bytes())
    }

    /// Inserts (or replaces) the UTF-8 string `key` → `value`.
    pub fn set_str(&mut self, key: &str, value: &str) -> Option<Vec<u8>> {
        self.insert(key.as_bytes().to_vec(), value.as_bytes().to_vec())
    }

    /// Removes the UTF-8 string `key`.
    pub fn remove_str(&mut self, key: &str) -> Option<Vec<u8>> {
        self.remove(key.as_bytes())
    }

    // ---- common-key accessors ----------------------------------------------------

    /// The `name` header value, or `None`.
    pub fn name(&self) -> Option<&[u8]> {
        self.get(Self::NAME)
    }

    /// Sets the `name` header.
    pub fn set_name(&mut self, value: impl Into<Vec<u8>>) -> Option<Vec<u8>> {
        self.insert(Self::NAME.to_vec(), value.into())
    }

    /// The `comment` header value, or `None`.
    pub fn comment(&self) -> Option<&[u8]> {
        self.get(Self::COMMENT)
    }

    /// Sets the `comment` header.
    pub fn set_comment(&mut self, value: impl Into<Vec<u8>>) -> Option<Vec<u8>> {
        self.insert(Self::COMMENT.to_vec(), value.into())
    }

    /// The `content-type` header value, or `None`.
    pub fn content_type(&self) -> Option<&[u8]> {
        self.get(Self::CONTENT_TYPE)
    }

    /// Sets the `content-type` header.
    pub fn set_content_type(&mut self, value: impl Into<Vec<u8>>) -> Option<Vec<u8>> {
        self.insert(Self::CONTENT_TYPE.to_vec(), value.into())
    }

    /// The `content-encoding` header value, or `None`.
    pub fn content_encoding(&self) -> Option<&[u8]> {
        self.get(Self::CONTENT_ENCODING)
    }

    /// Sets the `content-encoding` header.
    pub fn set_content_encoding(&mut self, value: impl Into<Vec<u8>>) -> Option<Vec<u8>> {
        self.insert(Self::CONTENT_ENCODING.to_vec(), value.into())
    }

    // ---- byte codec --------------------------------------------------------------

    /// Serialises the map to bytes: a `u32` entry count, then each entry as a
    /// length-prefixed key and value (`[key_len u32][key][val_len u32][val]`),
    /// little-endian.
    pub fn serialize_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for (key, value) in &self.entries {
            out.extend_from_slice(&(key.len() as u32).to_le_bytes());
            out.extend_from_slice(key);
            out.extend_from_slice(&(value.len() as u32).to_le_bytes());
            out.extend_from_slice(value);
        }
        out
    }

    /// Reconstructs a map from [`serialize_bytes`](Headers::serialize_bytes).
    ///
    /// # Errors
    /// [`HeadersError::Truncated`](crate::HeadersError::Truncated) if a length prefix
    /// runs past the end of `bytes`.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, HeadersError> {
        let mut cursor = Cursor { bytes };
        let count = cursor.u32()?;
        let mut entries = BTreeMap::new();
        for _ in 0..count {
            let key = cursor.chunk()?.to_vec();
            let value = cursor.chunk()?.to_vec();
            entries.insert(key, value);
        }
        Ok(Self { entries })
    }
}

/// A tiny forward byte reader for the length-prefixed header frames, mapping a short read
/// to [`HeadersError::Truncated`].
struct Cursor<'a> {
    bytes: &'a [u8],
}

impl<'a> Cursor<'a> {
    /// Reads a little-endian `u32` length prefix.
    fn u32(&mut self) -> Result<u32, HeadersError> {
        let (head, rest) = self
            .bytes
            .split_first_chunk::<4>()
            .ok_or(HeadersError::Truncated)?;
        self.bytes = rest;
        Ok(u32::from_le_bytes(*head))
    }

    /// Reads a `u32`-length-prefixed byte chunk.
    fn chunk(&mut self) -> Result<&'a [u8], HeadersError> {
        let len = self.u32()? as usize;
        if self.bytes.len() < len {
            return Err(HeadersError::Truncated);
        }
        let (chunk, rest) = self.bytes.split_at(len);
        self.bytes = rest;
        Ok(chunk)
    }
}
