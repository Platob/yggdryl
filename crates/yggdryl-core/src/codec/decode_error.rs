//! [`DecodeError`] — the failure modes of a [`Decoder`](crate::Decoder).

use core::fmt;

/// An error raised while decoding a byte array.
///
/// Every [`Decoder`](crate::Decoder) reports failures through this one enum, so
/// callers handle decoding errors uniformly regardless of the concrete codec. In
/// the bindings it surfaces as a Python `ValueError` / a thrown `Error`.
///
/// ```
/// use yggdryl_core::DecodeError;
///
/// let err = DecodeError::InvalidData("not a gzip stream".into());
/// assert!(err.to_string().contains("not a gzip stream"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecodeError {
    /// The input was malformed for this codec (e.g. a truncated or corrupt
    /// stream); the string names what was expected.
    InvalidData(String),
    /// The underlying decoder raised an I/O failure; the string is the source
    /// error's message.
    Io(String),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidData(message) => write!(f, "invalid encoded data: {message}"),
            Self::Io(message) => write!(f, "i/o error while decoding: {message}"),
        }
    }
}

impl std::error::Error for DecodeError {}

impl From<std::io::Error> for DecodeError {
    fn from(error: std::io::Error) -> Self {
        // A corrupt or truncated stream surfaces from the codec backend as an
        // `InvalidInput` / `InvalidData` / `UnexpectedEof` I/O error (e.g. a bad
        // gzip header); report it as malformed input rather than an environmental
        // I/O failure.
        match error.kind() {
            std::io::ErrorKind::InvalidInput
            | std::io::ErrorKind::InvalidData
            | std::io::ErrorKind::UnexpectedEof => Self::InvalidData(error.to_string()),
            _ => Self::Io(error.to_string()),
        }
    }
}
