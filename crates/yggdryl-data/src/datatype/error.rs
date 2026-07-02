//! The [`DataError`] type.

/// An error from a data-model operation, such as decoding a native value from bytes.
#[derive(Debug)]
#[non_exhaustive]
pub enum DataError {
    /// The bytes handed to a native decoder had the wrong length for the type.
    InvalidByteLength {
        /// The number of bytes the type requires.
        expected: usize,
        /// The number of bytes actually provided.
        got: usize,
    },
}

impl std::fmt::Display for DataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataError::InvalidByteLength { expected, got } => {
                write!(f, "expected {expected} byte(s) but got {got}")
            }
        }
    }
}

impl std::error::Error for DataError {}
