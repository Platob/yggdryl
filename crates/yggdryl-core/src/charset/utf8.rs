//! The [`Utf8`] charset.

use super::{Charset, CharsetError};

/// The UTF-8 charset: Rust's native string encoding, so encoding is a copy of the
/// string's bytes and decoding is a validation.
///
/// ```
/// use yggdryl_core::{Charset, Utf8};
///
/// assert_eq!(Utf8.encode_bytes("é")?, vec![0xC3, 0xA9]);
/// assert_eq!(Utf8.decode_bytes(&[0xC3, 0xA9])?, "é");
/// # Ok::<(), yggdryl_core::CharsetError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Utf8;

impl Charset for Utf8 {
    fn name(&self) -> &'static str {
        "UTF-8"
    }

    fn encode_bytes(&self, text: &str) -> Result<Vec<u8>, CharsetError> {
        crate::log_event!(trace, "Utf8::encode_bytes len={}", text.len());
        Ok(text.as_bytes().to_vec())
    }

    fn decode_bytes(&self, bytes: &[u8]) -> Result<String, CharsetError> {
        crate::log_event!(trace, "Utf8::decode_bytes len={}", bytes.len());
        String::from_utf8(bytes.to_vec()).map_err(|e| CharsetError::InvalidBytes {
            charset: self.name(),
            reason: e.to_string(),
        })
    }
}
