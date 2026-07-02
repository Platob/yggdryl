//! The [`BaseError`] type.

/// An error from [`Base`](super::Base) serialization.
#[derive(Debug)]
#[non_exhaustive]
pub enum BaseError {
    /// JSON serialization or deserialization failed.
    Json(serde_json::Error),
    /// A value type's byte form could not be decoded.
    InvalidBytes {
        /// What made the bytes invalid.
        reason: String,
    },
}

impl std::fmt::Display for BaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BaseError::Json(e) => write!(f, "JSON error: {e}"),
            BaseError::InvalidBytes { reason } => write!(f, "invalid bytes: {reason}"),
        }
    }
}

impl std::error::Error for BaseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BaseError::Json(e) => Some(e),
            BaseError::InvalidBytes { .. } => None,
        }
    }
}

impl From<serde_json::Error> for BaseError {
    fn from(error: serde_json::Error) -> Self {
        BaseError::Json(error)
    }
}
