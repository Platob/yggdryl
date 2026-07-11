//! [`HeadersError`] — the failure mode of decoding [`Headers`](crate::Headers).

use core::fmt;

/// An error raised while decoding [`Headers`](crate::Headers) from bytes.
///
/// ```
/// use yggdryl_http::HeadersError;
///
/// assert!(HeadersError::Truncated.to_string().contains("truncated"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HeadersError {
    /// A serialised header block that ended mid-frame — a length prefix promised more
    /// bytes than remained. Pass a payload produced by
    /// [`serialize_bytes`](crate::Headers::serialize_bytes).
    Truncated,
}

impl fmt::Display for HeadersError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => write!(
                f,
                "serialised headers are truncated; pass a payload produced by \
                 serialize_bytes"
            ),
        }
    }
}

impl std::error::Error for HeadersError {}
