//! [`FieldError`] — the failure modes of the field layer.

use core::fmt;

use yggdryl_dtype::DTypeError;
use yggdryl_http::HeadersError;

/// An error raised while decoding or converting a [`Field`](crate::Field).
///
/// Each message names the remedy — the missing nullable flag, the offending UTF-8
/// offset, or the underlying data-type mismatch — so the fix is knowable from the error
/// alone (rule 12). In the bindings it surfaces as a Python `ValueError` / a thrown
/// `Error`.
///
/// ```
/// use yggdryl_field::FieldError;
///
/// assert!(FieldError::EmptyPayload.to_string().contains("nullable flag"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FieldError {
    /// A serialised field with no bytes at all — it needs at least its 1-byte nullable
    /// flag. Pass a payload produced by `serialize_bytes`.
    EmptyPayload,
    /// A field name that is not valid UTF-8. Pass a valid UTF-8 name.
    InvalidUtf8 {
        /// The byte offset at which decoding failed.
        valid_up_to: usize,
    },
    /// A serialised field or metadata payload that ended mid-frame (a length prefix
    /// promised more bytes than remained). Pass a payload produced by `serialize_bytes`.
    Truncated {
        /// What was being decoded when the bytes ran out, e.g. `"field name"` or
        /// `"metadata"`.
        context: &'static str,
    },
    /// The underlying data type did not match — e.g. an Arrow field whose type is not
    /// the expected one. Carries the [`DTypeError`].
    Dtype(DTypeError),
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPayload => write!(
                f,
                "a serialised field needs at least its 1-byte nullable flag; got an \
                 empty payload"
            ),
            Self::InvalidUtf8 { valid_up_to } => write!(
                f,
                "invalid UTF-8 in the field name at byte {valid_up_to}; pass a valid \
                 UTF-8 name"
            ),
            Self::Truncated { context } => write!(
                f,
                "serialised {context} is truncated; pass a payload produced by \
                 serialize_bytes"
            ),
            Self::Dtype(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for FieldError {}

impl From<DTypeError> for FieldError {
    fn from(error: DTypeError) -> Self {
        Self::Dtype(error)
    }
}

impl From<HeadersError> for FieldError {
    fn from(_: HeadersError) -> Self {
        // A header block that ran past its bytes is a truncated field payload.
        Self::Truncated { context: "headers" }
    }
}
