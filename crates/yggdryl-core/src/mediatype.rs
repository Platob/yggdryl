//! [`MediaType`] — an **ordered list of [`MimeType`]s**: the layered type description of a
//! resource (a content type plus any encodings/wrappers, or the stack a multi-extension file
//! name implies, e.g. `archive.tar.gz` → `application/x-tar` then `application/gzip`). Built
//! from a mime-list string or from a file's extensions, leveraging [`MimeType`].

use core::fmt;

use crate::io::{IoError, Serializable};
use crate::mimetype::MimeType;

/// An ordered list of [`MimeType`]s describing a resource, **primary first**. A single-type
/// media (`application/json`) is a one-element list; a wrapped one (`.tar.gz`) lists the
/// content type then its encodings. A value type — equal, hashable, byte-serializable.
///
/// ```
/// use yggdryl_core::mediatype::MediaType;
///
/// // From a comma-separated mime list (like an Accept value).
/// let m = MediaType::parse_str("application/json, text/html").unwrap();
/// assert_eq!(m.primary().unwrap().essence(), "application/json");
/// assert_eq!(m.len(), 2);
///
/// // From a multi-extension file name — the content type, then its wrapping encodings.
/// let tgz = MediaType::from_extensions(["tar", "gz"]);
/// assert_eq!(tgz.essences(), vec!["application/x-tar", "application/gzip"]);
/// ```
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct MediaType {
    types: Vec<MimeType>,
}

impl MediaType {
    // ---- construction -------------------------------------------------------------------

    /// An empty media type (no known types).
    pub fn new() -> MediaType {
        MediaType { types: Vec::new() }
    }

    /// A single-type media over `mime`.
    pub fn of(mime: MimeType) -> MediaType {
        MediaType { types: vec![mime] }
    }

    /// A media type from an ordered list of [`MimeType`]s.
    pub fn from_types(types: impl IntoIterator<Item = MimeType>) -> MediaType {
        MediaType {
            types: types.into_iter().collect(),
        }
    }

    /// Parses a **comma-separated mime list** (like an HTTP `Accept` / `Content-Type` value),
    /// dropping each item's parameters (`;q=…`). Empty items are skipped.
    ///
    /// # Errors
    /// [`IoError::UnknownName`] if any non-empty item is not a `type/subtype` essence.
    pub fn parse_str(s: &str) -> Result<MediaType, IoError> {
        let mut types = Vec::new();
        for item in s.split(',') {
            if item.trim().is_empty() {
                continue;
            }
            types.push(MimeType::parse_str(item)?);
        }
        Ok(MediaType { types })
    }

    /// Builds a media type from a file's **extensions** (outermost-last, as
    /// [`Uri::extensions`](crate::uri::Uri::extensions) yields): each known extension maps to
    /// its [`MimeType`], an unknown one is skipped. `archive.tar.gz`'s `["tar", "gz"]` becomes
    /// `[application/x-tar, application/gzip]`.
    pub fn from_extensions(exts: impl IntoIterator<Item = impl AsRef<str>>) -> MediaType {
        MediaType {
            types: exts
                .into_iter()
                .filter_map(|ext| MimeType::from_extension(ext.as_ref()))
                .collect(),
        }
    }

    // ---- accessors ----------------------------------------------------------------------

    /// The **primary** type (the first), or `None` when empty.
    pub fn primary(&self) -> Option<&MimeType> {
        self.types.first()
    }

    /// The listed types, primary first.
    pub fn types(&self) -> &[MimeType] {
        &self.types
    }

    /// The listed essences, primary first (`["application/x-tar", "application/gzip"]`).
    pub fn essences(&self) -> Vec<&str> {
        self.types.iter().map(MimeType::essence).collect()
    }

    /// The number of listed types.
    pub fn len(&self) -> usize {
        self.types.len()
    }

    /// Whether the list is empty.
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// Whether any listed type has the given `essence` (case-insensitive).
    pub fn contains(&self, essence: &str) -> bool {
        self.types
            .iter()
            .any(|m| m.essence().eq_ignore_ascii_case(essence))
    }

    // ---- mutation -----------------------------------------------------------------------

    /// Appends a type to the list (chainable via [`with`](MediaType::with)).
    pub fn push(&mut self, mime: MimeType) {
        self.types.push(mime);
    }

    /// [`push`](MediaType::push) as a chainable builder.
    pub fn with(mut self, mime: MimeType) -> MediaType {
        self.push(mime);
        self
    }
}

impl fmt::Display for MediaType {
    /// The comma-joined essences — the inverse of [`parse_str`](MediaType::parse_str).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, mime) in self.types.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            f.write_str(mime.essence())?;
        }
        Ok(())
    }
}

impl fmt::Debug for MediaType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MediaType([{self}])")
    }
}

/// The value form is the comma-joined essence list — the inverse of
/// [`parse_str`](MediaType::parse_str).
impl Serializable for MediaType {
    type Error = IoError;

    fn serialize_bytes(&self) -> Vec<u8> {
        self.to_string().into_bytes()
    }

    fn deserialize_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        let s = core::str::from_utf8(bytes).map_err(|e| IoError::InvalidUtf8 {
            position: e.valid_up_to(),
        })?;
        MediaType::parse_str(s)
    }
}
