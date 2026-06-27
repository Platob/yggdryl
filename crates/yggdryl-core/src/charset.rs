//! The [`Charset`] of a string — the character encoding, shared across the
//! workspace (a [`Varchar`](https://docs.rs/yggdryl-schema) carries one).

use std::fmt;

/// Error returned when a [`Charset`] name is not recognised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharsetError(pub String);

impl fmt::Display for CharsetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown charset '{}', expected 'utf8', 'utf16', 'utf32', 'ascii' or 'latin1'",
            self.0
        )
    }
}

impl std::error::Error for CharsetError {}

/// The character set of a string. UTF-8 is the default and the only one with an
/// Arrow equivalent; the others are carried as metadata for non-Arrow back-ends.
///
/// ```
/// use yggdryl_core::Charset;
/// assert_eq!(Charset::default(), Charset::Utf8);
/// assert_eq!(Charset::from_str("latin1").unwrap(), Charset::Latin1);
/// assert_eq!(Charset::Utf16.as_str(), "utf16");
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Charset {
    /// UTF-8 (the default).
    #[default]
    Utf8,
    /// UTF-16.
    Utf16,
    /// UTF-32.
    Utf32,
    /// 7-bit ASCII.
    Ascii,
    /// ISO-8859-1 (Latin-1).
    Latin1,
}

impl Charset {
    /// Parses a charset name (case-insensitive), accepting common aliases.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Charset, CharsetError> {
        match value
            .trim()
            .to_ascii_lowercase()
            .replace(['-', '_'], "")
            .as_str()
        {
            "utf8" => Ok(Charset::Utf8),
            "utf16" | "utf16le" | "utf16be" => Ok(Charset::Utf16),
            "utf32" => Ok(Charset::Utf32),
            "ascii" | "usascii" => Ok(Charset::Ascii),
            "latin1" | "iso88591" => Ok(Charset::Latin1),
            _ => Err(CharsetError(value.to_string())),
        }
    }

    /// The canonical lowercase name (`"utf8"` / `"utf16"` / `"utf32"` / `"ascii"` /
    /// `"latin1"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Charset::Utf8 => "utf8",
            Charset::Utf16 => "utf16",
            Charset::Utf32 => "utf32",
            Charset::Ascii => "ascii",
            Charset::Latin1 => "latin1",
        }
    }
}

impl fmt::Display for Charset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
