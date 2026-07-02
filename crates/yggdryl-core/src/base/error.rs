//! The [`BaseError`] type.

use crate::charset::CharsetError;

/// An error from [`Base`](super::Base) serialization.
#[derive(Debug)]
#[non_exhaustive]
pub enum BaseError {
    /// JSON serialization or deserialization failed.
    Json(serde_json::Error),
    /// Charset encoding or decoding failed.
    Charset(CharsetError),
}

impl std::fmt::Display for BaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BaseError::Json(e) => write!(f, "JSON error: {e}"),
            BaseError::Charset(e) => write!(f, "charset error: {e}"),
        }
    }
}

impl std::error::Error for BaseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BaseError::Json(e) => Some(e),
            BaseError::Charset(e) => Some(e),
        }
    }
}

impl From<serde_json::Error> for BaseError {
    fn from(error: serde_json::Error) -> Self {
        BaseError::Json(error)
    }
}

impl From<CharsetError> for BaseError {
    fn from(error: CharsetError) -> Self {
        BaseError::Charset(error)
    }
}
