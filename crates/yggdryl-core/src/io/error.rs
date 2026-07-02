//! The [`IOError`] type.

/// An error from an [`IOBase`](super::IOBase) operation.
#[derive(Debug)]
#[non_exhaustive]
pub enum IOError {
    /// A resolved offset lies outside the resource's bounds.
    OutOfBounds {
        /// The offset that fell outside the resource.
        offset: usize,
        /// The resource's length.
        len: usize,
    },
    /// Fewer elements were available than requested.
    UnexpectedEof {
        /// How many elements were requested.
        requested: usize,
        /// How many were actually available.
        available: usize,
    },
}

impl std::fmt::Display for IOError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IOError::OutOfBounds { offset, len } => {
                write!(f, "offset {offset} is out of bounds for length {len}")
            }
            IOError::UnexpectedEof {
                requested,
                available,
            } => write!(
                f,
                "expected {requested} element(s) but only {available} were available"
            ),
        }
    }
}

impl std::error::Error for IOError {}
