//! The [`Charset`] codec: encode text (such as JSON) to bytes and back.

mod error;
pub use error::CharsetError;

/// A character encoding for turning text into bytes and back.
///
/// [`Utf8`](Charset::Utf8) is the default; the other variants cover the common
/// fixed- and variable-width encodings the standard library round-trips without
/// extra dependencies.
///
/// ```
/// use yggdryl_core::Charset;
///
/// assert_eq!(Charset::default(), Charset::Utf8);
/// assert_eq!(Charset::Utf8.encode("é")?, vec![0xC3, 0xA9]);
/// assert_eq!(Charset::Latin1.encode("é")?, vec![0xE9]);
/// assert_eq!(Charset::Latin1.decode(&[0xE9])?, "é");
/// # Ok::<(), yggdryl_core::CharsetError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Charset {
    /// UTF-8 (the default).
    #[default]
    Utf8,
    /// UTF-16, little-endian.
    Utf16Le,
    /// UTF-16, big-endian.
    Utf16Be,
    /// ISO-8859-1 (Latin-1): one byte per code point in `U+0000..=U+00FF`.
    Latin1,
    /// US-ASCII: one byte per code point in `U+0000..=U+007F`.
    Ascii,
}

impl Charset {
    /// Encode `text` to bytes in this charset.
    ///
    /// Returns [`CharsetError::Unrepresentable`] when a character has no encoding
    /// in the target charset (for example a non-ASCII character for
    /// [`Ascii`](Charset::Ascii)).
    pub fn encode(self, text: &str) -> Result<Vec<u8>, CharsetError> {
        crate::log_event!(trace, "Charset::encode charset={self:?} len={}", text.len());
        match self {
            Charset::Utf8 => Ok(text.as_bytes().to_vec()),
            Charset::Utf16Le => Ok(text.encode_utf16().flat_map(u16::to_le_bytes).collect()),
            Charset::Utf16Be => Ok(text.encode_utf16().flat_map(u16::to_be_bytes).collect()),
            Charset::Latin1 => self.encode_single_byte(text, 0xFF),
            Charset::Ascii => self.encode_single_byte(text, 0x7F),
        }
    }

    /// Decode `bytes` from this charset into a string.
    ///
    /// Returns [`CharsetError::InvalidBytes`] when the bytes are not a valid
    /// encoding in this charset.
    pub fn decode(self, bytes: &[u8]) -> Result<String, CharsetError> {
        crate::log_event!(
            trace,
            "Charset::decode charset={self:?} len={}",
            bytes.len()
        );
        match self {
            Charset::Utf8 => {
                String::from_utf8(bytes.to_vec()).map_err(|e| CharsetError::InvalidBytes {
                    charset: self,
                    reason: e.to_string(),
                })
            }
            Charset::Utf16Le => self.decode_utf16(bytes, u16::from_le_bytes),
            Charset::Utf16Be => self.decode_utf16(bytes, u16::from_be_bytes),
            Charset::Latin1 => Ok(bytes.iter().map(|&b| b as char).collect()),
            Charset::Ascii => self.decode_ascii(bytes),
        }
    }

    /// Encode one byte per character, rejecting any code point above `max`.
    fn encode_single_byte(self, text: &str, max: u32) -> Result<Vec<u8>, CharsetError> {
        let mut out = Vec::with_capacity(text.len());
        for (index, ch) in text.chars().enumerate() {
            let code = ch as u32;
            if code > max {
                return Err(CharsetError::Unrepresentable {
                    charset: self,
                    index,
                    ch,
                });
            }
            out.push(code as u8);
        }
        Ok(out)
    }

    /// Decode US-ASCII: every byte must be in `0x00..=0x7F`.
    fn decode_ascii(self, bytes: &[u8]) -> Result<String, CharsetError> {
        if let Some(index) = bytes.iter().position(|&b| b > 0x7F) {
            return Err(CharsetError::InvalidBytes {
                charset: self,
                reason: format!(
                    "byte {:#04x} at index {index} is outside US-ASCII",
                    bytes[index]
                ),
            });
        }
        // Every byte is ASCII, hence already valid UTF-8.
        Ok(String::from_utf8(bytes.to_vec()).expect("ASCII bytes are valid UTF-8"))
    }

    /// Decode UTF-16 from `bytes`, reading each unit with `read` (endian-specific).
    fn decode_utf16(self, bytes: &[u8], read: fn([u8; 2]) -> u16) -> Result<String, CharsetError> {
        if !bytes.len().is_multiple_of(2) {
            return Err(CharsetError::InvalidBytes {
                charset: self,
                reason: format!(
                    "odd length {}; UTF-16 needs an even byte count",
                    bytes.len()
                ),
            });
        }
        let units = bytes.chunks_exact(2).map(|pair| read([pair[0], pair[1]]));
        char::decode_utf16(units)
            .collect::<Result<String, _>>()
            .map_err(|e| CharsetError::InvalidBytes {
                charset: self,
                reason: format!("unpaired surrogate {:#06x}", e.unpaired_surrogate()),
            })
    }
}
