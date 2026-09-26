//! ANSI X9.145 Financial Instrument Global Identifiers.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use smol_str::SmolStr;

use crate::code::{code_value, identifier_value};
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{DataType, Result, Scalar, Value};

/// One validated Financial Instrument Global Identifier.
///
/// A FIGI is twelve ASCII characters: two consonants, `G`, eight consonants
/// or digits, and one decimal check digit. Lowercase input is normalized once
/// at construction; the stored spelling is uppercase and fits inline in the
/// crate's compact string.
#[repr(transparent)]
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Figi(SmolStr);

impl<'de> Deserialize<'de> for Figi {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        SmolStr::deserialize(deserializer)
            .and_then(|value| Self::new(value).map_err(serde::de::Error::custom))
    }
}

impl Figi {
    /// Validate and construct a Financial Instrument Global Identifier.
    ///
    /// ```
    /// use yggdryl::Figi;
    ///
    /// let figi = Figi::new("BBG000BLNQ16").unwrap();
    /// assert_eq!(figi.as_str(), "BBG000BLNQ16");
    /// assert_eq!(Figi::new("bbg000blnq16").unwrap(), figi);
    /// assert_eq!(figi.check_digit(), 6);
    /// assert!(Figi::new("BBG000BLNQ15").is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the text is not twelve ASCII bytes of the
    /// standard's shape, uses a reserved prefix, or its check digit does not
    /// close the identifier.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = crate::ascii_text(FIGI_WIDTH, value.as_ref().as_bytes())?;
        let mut bytes = [0_u8; FIGI_WIDTH];
        for (target, byte) in bytes.iter_mut().zip(value.bytes()) {
            *target = byte.to_ascii_uppercase();
        }
        let folded = std::str::from_utf8(&bytes[..value.len()]).expect("validated ASCII");
        if let Some(reason) = Self::refusal(folded) {
            return Err(crate::Error::InvalidDataType {
                kind: "figi",
                reason: smol_str::format_smolstr!("{reason}, got {value:?}"),
            });
        }
        Ok(Self(SmolStr::new(folded)))
    }

    /// Borrow the canonical identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Borrow the compact storage without copying the identifier.
    #[must_use]
    pub const fn storage(&self) -> &SmolStr {
        &self.0
    }

    /// The decimal digit that closes this identifier.
    #[must_use]
    pub fn check_digit(&self) -> u8 {
        self.as_str().as_bytes()[FIGI_WIDTH - 1] - b'0'
    }

    /// Whether `text` is a FIGI this type accepts, in either ASCII case.
    #[must_use]
    pub fn is_valid(text: &str) -> bool {
        if text.len() != FIGI_WIDTH || !text.is_ascii() {
            return false;
        }
        let mut bytes = [0_u8; FIGI_WIDTH];
        for (target, byte) in bytes.iter_mut().zip(text.bytes()) {
            *target = byte.to_ascii_uppercase();
        }
        let folded = std::str::from_utf8(&bytes).expect("validated ASCII");
        Self::refusal(folded).is_none()
    }

    /// Whether `text` is exactly the uppercase spelling this type stores.
    #[must_use]
    pub fn is_canonical(text: &str) -> bool {
        text.len() == FIGI_WIDTH
            && text.is_ascii()
            && !text.bytes().any(|byte| byte.is_ascii_lowercase())
            && Self::refusal(text).is_none()
    }

    /// The check digit for eleven leading FIGI characters.
    ///
    /// Each character is read as one value from zero to thirty-five. Values
    /// at odd zero-based positions are doubled, then the decimal digits of
    /// every value are summed before the modulo-ten close.
    #[must_use]
    pub fn closing_digit(body: &str) -> Option<u8> {
        let bytes = body.as_bytes();
        if bytes.len() != FIGI_WIDTH - 1 || !Self::body_has_shape(bytes) {
            return None;
        }
        let sum = bytes
            .iter()
            .enumerate()
            .try_fold(0_u32, |sum, (index, byte)| {
                let value = identifier_value(*byte)? * if index % 2 == 1 { 2 } else { 1 };
                Some(sum + value / 10 + value % 10)
            })?;
        u8::try_from((10 - sum % 10) % 10).ok()
    }

    fn is_consonant(byte: u8) -> bool {
        byte.is_ascii_uppercase() && !matches!(byte, b'A' | b'E' | b'I' | b'O' | b'U')
    }

    fn is_reserved_prefix(bytes: &[u8]) -> bool {
        matches!(
            (bytes[0], bytes[1]),
            (b'B', b'S')
                | (b'B', b'M')
                | (b'G', b'G')
                | (b'G', b'B')
                | (b'G', b'H')
                | (b'K', b'Y')
                | (b'V', b'G')
        )
    }

    fn body_has_shape(bytes: &[u8]) -> bool {
        bytes.len() == FIGI_WIDTH - 1
            && Self::is_consonant(bytes[0])
            && Self::is_consonant(bytes[1])
            && !Self::is_reserved_prefix(bytes)
            && bytes[2] == b'G'
            && bytes[3..]
                .iter()
                .all(|byte| byte.is_ascii_digit() || Self::is_consonant(*byte))
    }

    fn refusal(folded: &str) -> Option<&'static str> {
        let bytes = folded.as_bytes();
        if bytes.len() != FIGI_WIDTH {
            return Some("expected twelve characters");
        }
        if !Self::is_consonant(bytes[0]) || !Self::is_consonant(bytes[1]) {
            return Some("expected a two-consonant prefix");
        }
        if Self::is_reserved_prefix(bytes) {
            return Some("expected a non-reserved prefix");
        }
        if bytes[2] != b'G' {
            return Some("expected G as the third character");
        }
        if !bytes[3..11]
            .iter()
            .all(|byte| byte.is_ascii_digit() || Self::is_consonant(*byte))
        {
            return Some("expected eight consonants or digits after the prefix and G");
        }
        if !bytes[11].is_ascii_digit() {
            return Some("expected a closing check digit");
        }
        match Self::closing_digit(&folded[..11]) {
            Some(digit) if digit == bytes[11] - b'0' => None,
            _ => Some("the check digit does not close the identifier"),
        }
    }
}

impl fmt::Display for Figi {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

code_value!(Figi, Figi, FIGI_WIDTH);

/// The Arrow extension name of a Financial Instrument Global Identifier.
pub(crate) const FIGI_EXTENSION_NAME: &str = "yggdryl.figi";

/// The exact width of a Financial Instrument Global Identifier.
pub(crate) const FIGI_WIDTH: usize = 12;

impl DataType {
    /// Creates ANSI X9.145's twelve-character identifier datatype.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::figi(), DataType::Figi);
    /// assert_eq!(DataType::figi().to_string(), "figi");
    /// assert_eq!(DataType::figi().code_width(), Some(12));
    /// ```
    #[must_use]
    pub const fn figi() -> Self {
        Self::Figi
    }
}

define_field_types!(FigiType, Figi);
