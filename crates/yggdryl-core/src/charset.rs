//! The [`Charset`] text encoding used to turn JSON text into bytes and back.

use crate::error::CharsetError;

/// A character encoding for converting between text and bytes.
///
/// [`Jsonable::to_bson`](crate::Jsonable::to_bson) renders a value to JSON text
/// and encodes it to bytes with the active charset;
/// [`from_bson`](crate::Jsonable::from_bson) decodes it back. `Utf8` is the default
/// and round-trips any text losslessly; `Ascii` and `Latin1` are byte-oriented
/// legacy encodings that replace unrepresentable characters with `?` on encode and
/// decode each byte as the code point of the same value.
///
/// ```
/// use yggdryl_core::Charset;
///
/// assert_eq!(Charset::default(), Charset::Utf8);
/// assert_eq!(Charset::from_str("latin1").unwrap(), Charset::Latin1);
/// assert_eq!(Charset::Latin1.encode("é"), vec![0xe9]);
/// assert_eq!(Charset::Latin1.decode(&[0xe9]).unwrap(), "é");
/// assert_eq!(Charset::Ascii.encode("é"), b"?"); // not representable
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(into = "String", try_from = "String"))]
pub enum Charset {
    /// UTF-8 (the default): lossless for any text.
    #[default]
    Utf8,
    /// 7-bit US-ASCII: code points above `0x7F` become `?` on encode.
    Ascii,
    /// ISO-8859-1 (Latin-1): code points above `0xFF` become `?` on encode.
    Latin1,
}

impl Charset {
    /// The canonical charset name (`"utf8"`, `"ascii"` or `"latin1"`).
    pub fn name(&self) -> &'static str {
        match self {
            Charset::Utf8 => "utf8",
            Charset::Ascii => "ascii",
            Charset::Latin1 => "latin1",
        }
    }

    /// Parses a canonical charset name, accepting the common aliases (`"utf-8"`,
    /// `"us-ascii"`, `"iso-8859-1"`).
    #[allow(clippy::should_implement_trait)] // `from_str` is the crate-wide naming convention.
    pub fn from_str(value: &str) -> Result<Self, CharsetError> {
        crate::log_event!(trace, "Charset::from_str {:?}", value);
        match value {
            "utf8" | "utf-8" => Ok(Charset::Utf8),
            "ascii" | "us-ascii" => Ok(Charset::Ascii),
            "latin1" | "latin-1" | "iso-8859-1" => Ok(Charset::Latin1),
            other => Err(CharsetError::UnknownCharset(other.to_string())),
        }
    }

    /// Encodes `text` to bytes, replacing characters the charset cannot represent
    /// with `?`.
    pub fn encode(&self, text: &str) -> Vec<u8> {
        match self {
            Charset::Utf8 => text.as_bytes().to_vec(),
            Charset::Ascii => text
                .chars()
                .map(|c| if c.is_ascii() { c as u8 } else { b'?' })
                .collect(),
            Charset::Latin1 => text
                .chars()
                .map(|c| if (c as u32) <= 0xff { c as u8 } else { b'?' })
                .collect(),
        }
    }

    /// Decodes `bytes` to text, erroring on bytes the charset cannot represent.
    pub fn decode(&self, bytes: &[u8]) -> Result<String, CharsetError> {
        match self {
            Charset::Utf8 => String::from_utf8(bytes.to_vec())
                .map_err(|_| CharsetError::InvalidBytes { charset: "utf8" }),
            Charset::Ascii => {
                if bytes.iter().all(u8::is_ascii) {
                    Ok(bytes.iter().map(|&b| b as char).collect())
                } else {
                    Err(CharsetError::InvalidBytes { charset: "ascii" })
                }
            }
            Charset::Latin1 => Ok(bytes.iter().map(|&b| b as char).collect()),
        }
    }
}

impl From<Charset> for String {
    fn from(value: Charset) -> Self {
        value.name().to_string()
    }
}

impl TryFrom<String> for Charset {
    type Error = CharsetError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Charset::from_str(&value)
    }
}
