//! The [`CharsetError`] type.

/// An error from a [`Charset`](super::Charset) conversion.
#[derive(Debug)]
#[non_exhaustive]
pub enum CharsetError {
    /// A character cannot be represented in the target charset.
    Unrepresentable {
        /// Name of the charset that rejected the character.
        charset: &'static str,
        /// Index of the offending character in the input string.
        index: usize,
        /// The character that has no encoding in `charset`.
        ch: char,
    },
    /// The bytes are not a valid encoding in the source charset.
    InvalidBytes {
        /// Name of the charset the bytes were decoded with.
        charset: &'static str,
        /// What made the bytes invalid.
        reason: String,
    },
}

impl std::fmt::Display for CharsetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CharsetError::Unrepresentable { charset, index, ch } => write!(
                f,
                "character {ch:?} (U+{:04X}) at index {index} cannot be encoded as {charset}",
                *ch as u32
            ),
            CharsetError::InvalidBytes { charset, reason } => {
                write!(f, "invalid {charset} bytes: {reason}")
            }
        }
    }
}

impl std::error::Error for CharsetError {}
