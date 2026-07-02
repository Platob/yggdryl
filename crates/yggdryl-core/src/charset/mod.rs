//! The [`Charset`] trait — encode text (such as JSON) to bytes and back — plus
//! the [`Utf8`] and [`Latin1`] implementations.

mod error;
mod latin1;
mod utf8;

pub use error::CharsetError;
pub use latin1::Latin1;
pub use utf8::Utf8;

/// A character encoding: the conversion between text and its byte representation.
///
/// Implementations are zero-sized markers (for example [`Utf8`] and [`Latin1`])
/// passed wherever bytes are produced or consumed — notably
/// [`Base::to_bson`](crate::Base::to_bson) / [`from_bson`](crate::Base::from_bson).
///
/// ```
/// use yggdryl_core::{Charset, Utf8};
///
/// assert_eq!(Utf8.name(), "UTF-8");
/// assert_eq!(Utf8.encode_bytes("hi")?, b"hi".to_vec());
/// assert_eq!(Utf8.decode_bytes(b"hi")?, "hi");
/// # Ok::<(), yggdryl_core::CharsetError>(())
/// ```
pub trait Charset {
    /// This charset's canonical name, used in diagnostics such as [`CharsetError`].
    fn name(&self) -> &'static str;

    /// Encode `text` to bytes in this charset.
    ///
    /// Returns [`CharsetError::Unrepresentable`] when a character has no encoding
    /// in this charset.
    fn encode_bytes(&self, text: &str) -> Result<Vec<u8>, CharsetError>;

    /// Decode `bytes` from this charset into a string.
    ///
    /// Returns [`CharsetError::InvalidBytes`] when the bytes are not a valid
    /// encoding in this charset.
    fn decode_bytes(&self, bytes: &[u8]) -> Result<String, CharsetError>;
}
