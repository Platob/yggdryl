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
    /// A typed view (such as [`IOSlice`](super::IOSlice)) could not convert an item
    /// count to bytes because the element width is unknown — the underlying resource
    /// holds no items to infer it from and does not override
    /// [`element_width`](super::IOBase::element_width).
    IndeterminateElementWidth,
    /// The bytes cannot be read as UTF-8 text (a raw byte write may have broken the
    /// encoding of a [`StringBuffer`](super::StringBuffer)).
    InvalidUtf8 {
        /// The byte offset where the first invalid UTF-8 sequence begins.
        offset: usize,
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
            IOError::IndeterminateElementWidth => write!(
                f,
                "cannot infer the element width from an empty resource; override `IOBase::element_width`"
            ),
            IOError::InvalidUtf8 { offset } => {
                write!(f, "the bytes at offset {offset} are not valid UTF-8")
            }
        }
    }
}

impl std::error::Error for IOError {}
