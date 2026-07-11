//! [`HeadersBased`] — the shared get / add / update / delete surface for header-carrying
//! types (a field, a buffer).

use crate::Headers;

/// A type that carries optional [`Headers`](crate::Headers).
///
/// Implementors supply only the storage slot — [`headers`](HeadersBased::headers) and
/// [`headers_mut`](HeadersBased::headers_mut) — and the trait provides the whole
/// get / add-or-update / delete surface over it (byte and UTF-8 string keys, plus the
/// common-key conveniences), the [`with_headers`](HeadersBased::with_headers) builder,
/// and **zero-copy** per-key value mutation via
/// [`get_header_mut`](HeadersBased::get_header_mut). This is what lets the field and
/// buffer layers share one header implementation instead of repeating it per type.
///
/// ```
/// use yggdryl_http::{Headers, HeadersBased};
///
/// #[derive(Default)]
/// struct Column { headers: Option<Headers> }
/// impl HeadersBased for Column {
///     fn headers(&self) -> Option<&Headers> { self.headers.as_ref() }
///     fn headers_mut(&mut self) -> &mut Option<Headers> { &mut self.headers }
/// }
///
/// let mut column = Column::default();
/// assert_eq!(column.set_header_str("unit", "ms"), None); // add
/// assert_eq!(column.get_header_str("unit"), Some(b"ms".as_slice()));
/// column.set_content_type("application/x.int64");        // pre-built content-type mutator
/// assert_eq!(column.content_type(), Some(b"application/x.int64".as_slice()));
/// assert_eq!(column.remove_header_str("unit"), Some(b"ms".to_vec())); // delete
/// ```
pub trait HeadersBased {
    /// The attached headers, or `None`.
    fn headers(&self) -> Option<&Headers>;

    /// A mutable handle to the header slot, so entries can be added or removed.
    fn headers_mut(&mut self) -> &mut Option<Headers>;

    /// Replaces all headers with `headers` (builder).
    fn with_headers(mut self, headers: Headers) -> Self
    where
        Self: Sized,
    {
        *self.headers_mut() = Some(headers);
        self
    }

    /// The value for the byte `key`, or `None`.
    fn get_header(&self, key: &[u8]) -> Option<&[u8]> {
        self.headers().and_then(|headers| headers.get(key))
    }

    /// The value for the UTF-8 string `key`, or `None`.
    fn get_header_str(&self, key: &str) -> Option<&[u8]> {
        self.get_header(key.as_bytes())
    }

    /// A **mutable** handle to the value for the byte `key` (zero-copy in-place mutation),
    /// or `None` if the key (or the whole map) is absent.
    fn get_header_mut(&mut self, key: &[u8]) -> Option<&mut Vec<u8>> {
        self.headers_mut()
            .as_mut()
            .and_then(|headers| headers.get_mut(key))
    }

    /// Adds or updates the byte `key` → `value`, returning the previous value if present.
    fn set_header(&mut self, key: Vec<u8>, value: Vec<u8>) -> Option<Vec<u8>> {
        self.headers_mut()
            .get_or_insert_with(Headers::new)
            .insert(key, value)
    }

    /// Adds or updates the UTF-8 string `key` → `value`.
    fn set_header_str(&mut self, key: &str, value: &str) -> Option<Vec<u8>> {
        self.set_header(key.as_bytes().to_vec(), value.as_bytes().to_vec())
    }

    /// Removes the byte `key`, returning its value if present. The slot is cleared to
    /// `None` once its last entry is removed.
    fn remove_header(&mut self, key: &[u8]) -> Option<Vec<u8>> {
        let slot = self.headers_mut();
        let removed = slot.as_mut().and_then(|headers| headers.remove(key));
        if slot.as_ref().is_some_and(Headers::is_empty) {
            *slot = None;
        }
        removed
    }

    /// Removes the UTF-8 string `key`.
    fn remove_header_str(&mut self, key: &str) -> Option<Vec<u8>> {
        self.remove_header(key.as_bytes())
    }

    // ---- common-key conveniences (read-through) ----------------------------------

    /// The `name` header value (distinct from a field's own name).
    fn header_name(&self) -> Option<&[u8]> {
        self.get_header(Headers::NAME)
    }

    /// Sets the `name` header.
    fn set_header_name(&mut self, value: Vec<u8>) -> Option<Vec<u8>> {
        self.set_header(Headers::NAME.to_vec(), value)
    }

    /// The `comment` header value.
    fn comment(&self) -> Option<&[u8]> {
        self.get_header(Headers::COMMENT)
    }

    /// Sets the `comment` header.
    fn set_comment(&mut self, value: impl Into<Vec<u8>>) -> Option<Vec<u8>> {
        self.set_header(Headers::COMMENT.to_vec(), value.into())
    }

    /// The `content-type` header value.
    fn content_type(&self) -> Option<&[u8]> {
        self.get_header(Headers::CONTENT_TYPE)
    }

    /// Sets the `content-type` header.
    fn set_content_type(&mut self, value: impl Into<Vec<u8>>) -> Option<Vec<u8>> {
        self.set_header(Headers::CONTENT_TYPE.to_vec(), value.into())
    }

    /// The `content-encoding` header value.
    fn content_encoding(&self) -> Option<&[u8]> {
        self.get_header(Headers::CONTENT_ENCODING)
    }

    /// Sets the `content-encoding` header.
    fn set_content_encoding(&mut self, value: impl Into<Vec<u8>>) -> Option<Vec<u8>> {
        self.set_header(Headers::CONTENT_ENCODING.to_vec(), value.into())
    }
}
