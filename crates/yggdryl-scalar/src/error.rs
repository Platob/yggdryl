//! The scalar crate's error type.

/// Errors raised when decoding a scalar's bytes into a native value.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScalarError {
    /// The bytes are not valid UTF-8 for a string value.
    NonUtf8,
}

impl std::fmt::Display for ScalarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScalarError::NonUtf8 => {
                f.write_str("scalar bytes are not valid UTF-8 for a string value")
            }
        }
    }
}

impl std::error::Error for ScalarError {}
