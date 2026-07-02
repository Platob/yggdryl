//! The [`CharsetError`] type.

use super::Charset;

/// An error from [`Charset::encode`](super::Charset::encode) or
/// [`Charset::decode`](super::Charset::decode).
#[derive(Debug)]
#[non_exhaustive]
pub enum CharsetError {
    /// A character cannot be represented in the target charset.
    Unrepresentable {
        /// The charset that rejected the character.
        charset: Charset,
        /// Index of the offending character in the input string.
        index: usize,
        /// The character that has no encoding in `charset`.
        ch: char,
    },
    /// The bytes are not a valid encoding in the source charset.
    InvalidBytes {
        /// The charset the bytes were decoded with.
        charset: Charset,
        /// What made the bytes invalid, and where.
        reason: String,
    },
}

impl std::fmt::Display for CharsetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CharsetError::Unrepresentable { charset, index, ch } => write!(
                f,
                "character {ch:?} (U+{:04X}) at index {index} cannot be encoded as {charset:?}",
                *ch as u32
            ),
            CharsetError::InvalidBytes { charset, reason } => {
                write!(f, "invalid {charset:?} bytes: {reason}")
            }
        }
    }
}

impl std::error::Error for CharsetError {}
