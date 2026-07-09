//! [`EncodeError`] — the failure modes of an [`Encoder`](crate::Encoder).

use core::fmt;

/// An error raised while encoding a byte array.
///
/// Every [`Encoder`](crate::Encoder) reports failures through this one enum, so
/// callers handle encoding errors uniformly regardless of the concrete codec. In
/// the bindings it surfaces as a Python `ValueError` / a thrown `Error`.
///
/// ```
/// use yggdryl_core::EncodeError;
///
/// let err = EncodeError::InvalidLevel { level: 12, min: 0, max: 9 };
/// assert!(err.to_string().contains("expected 0..=9"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EncodeError {
    /// A codec was configured with a compression level outside the supported
    /// `min..=max` range. Pass a level within it.
    InvalidLevel {
        /// The offending level.
        level: i64,
        /// The lowest accepted level.
        min: i64,
        /// The highest accepted level.
        max: i64,
    },
    /// The underlying encoder raised an I/O failure; the string is the source
    /// error's message.
    Io(String),
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLevel { level, min, max } => {
                write!(
                    f,
                    "compression level {level} out of range; expected {min}..={max}"
                )
            }
            Self::Io(message) => write!(f, "i/o error while encoding: {message}"),
        }
    }
}

impl std::error::Error for EncodeError {}

impl From<std::io::Error> for EncodeError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
