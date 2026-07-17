//! [`UriError`] — the failure modes of the URI/URL parser and byte codec.

use core::fmt;

/// An error raised while parsing or decoding a [`Uri`](crate::io::Uri) /
/// [`Url`](crate::io::Url).
///
/// Every variant names the offending piece and the fix: the
/// bad scheme, the out-of-range port with its value, or why the bytes could not be
/// decoded. In the bindings it surfaces as a Python `ValueError` / a thrown `Error`.
///
/// ```
/// use yggdryl_core::io::{Uri, UriError};
///
/// // A port outside `0..=65535` names the offending value.
/// let err = Uri::parse("http://h:99999/").unwrap_err();
/// assert!(matches!(err, UriError::InvalidPort { .. }));
/// assert!(err.to_string().contains("99999"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum UriError {
    /// A `:` was found with nothing before it (e.g. `"://host"`). A URI scheme must
    /// be at least one letter; drop the leading colon or supply a scheme.
    EmptyScheme,
    /// The scheme has an illegal character or does not start with a letter. A scheme
    /// is `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )`; use only those characters.
    InvalidScheme {
        /// The offending scheme text.
        scheme: String,
    },
    /// The port is non-numeric or outside `0..=65535`. Pass a decimal port in range.
    InvalidPort {
        /// The offending port text.
        port: String,
    },
    /// A [`Url`](crate::io::Url) requires an absolute URI but the input had no scheme.
    /// Prefix the input with a scheme (`"https://…"`), or parse it as a
    /// [`Uri`](crate::io::Uri) instead.
    MissingScheme {
        /// The scheme-less input that was supplied.
        input: String,
    },
    /// The bytes handed to `deserialize_bytes` are not valid UTF-8, so they cannot be
    /// a URI string. Pass the UTF-8 bytes of a URI (as produced by `serialize_bytes`).
    NonUtf8 {
        /// The number of bytes that were supplied.
        len: usize,
    },
}

impl fmt::Display for UriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyScheme => write!(
                f,
                "empty scheme: a `:` was found with nothing before it; a URI scheme \
                 must be at least one letter (drop the leading colon or supply a scheme)"
            ),
            Self::InvalidScheme { scheme } => write!(
                f,
                "invalid scheme {scheme:?}: a scheme is ALPHA *( ALPHA / DIGIT / \"+\" \
                 / \"-\" / \".\" ) and must start with a letter; use only those characters"
            ),
            Self::InvalidPort { port } => write!(
                f,
                "invalid port {port:?}: expected a decimal number in 0..=65535; pass a \
                 port in range"
            ),
            Self::MissingScheme { input } => write!(
                f,
                "not an absolute URL {input:?}: a Url requires a scheme; prefix it with \
                 one (e.g. \"https://…\") or parse it as a Uri instead"
            ),
            Self::NonUtf8 { len } => write!(
                f,
                "cannot decode a URI from {len} bytes: the bytes are not valid UTF-8; \
                 pass the UTF-8 bytes of a URI (as produced by `serialize_bytes`)"
            ),
        }
    }
}

impl std::error::Error for UriError {}
