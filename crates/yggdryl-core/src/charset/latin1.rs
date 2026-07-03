//! The [`Latin1`] charset.

use super::{Charset, CharsetError};

/// The ISO-8859-1 (Latin-1) charset: one byte per code point in `U+0000..=U+00FF`.
///
/// Encoding rejects any character above `U+00FF`; decoding always succeeds, since
/// every byte maps directly to a code point.
///
/// ```
/// use yggdryl_core::{Charset, Latin1};
///
/// assert_eq!(Latin1.encode_bytes("é")?, vec![0xE9]);
/// assert_eq!(Latin1.decode_bytes(&[0xE9])?, "é");
/// # Ok::<(), yggdryl_core::CharsetError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Latin1;

impl Charset for Latin1 {
    fn name(&self) -> &'static str {
        "ISO-8859-1"
    }

    fn encode_bytes(&self, text: &str) -> Result<Vec<u8>, CharsetError> {
        crate::log_event!(trace, "Latin1::encode_bytes len={}", text.len());
        let mut out = Vec::with_capacity(text.len());
        for (index, ch) in text.chars().enumerate() {
            let code = ch as u32;
            if code > 0xFF {
                return Err(CharsetError::Unrepresentable {
                    charset: self.name(),
                    index,
                    ch,
                });
            }
            out.push(code as u8);
        }
        Ok(out)
    }

    fn decode_bytes(&self, bytes: &[u8]) -> Result<String, CharsetError> {
        crate::log_event!(trace, "Latin1::decode_bytes len={}", bytes.len());
        // Every byte in `0x00..=0xFF` maps directly to `U+0000..=U+00FF`.
        Ok(bytes.iter().map(|&b| b as char).collect())
    }
}
