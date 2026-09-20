//! SEDOL securities identifiers.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::code::code_value;
use crate::code::identifier_value;
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{DataType, Result, Scalar, Value};

/// One validated SEDOL securities identifier.
///
/// Seven bytes: six alphanumerics and one check digit, the modulus-10
/// digit of the six before it read with each letter as ten plus its
/// alphabet position and weighted `1, 3, 1, 7, 3, 9` in turn. A spelling
/// whose check digit does not close it is refused, as an ISIN's and a
/// CUSIP's are: an identifier that fails its own checksum is a typo.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct SedolCode(SmolStr);

impl SedolCode {
    /// The weight each of the six leading characters carries.
    const WEIGHTS: [u32; SEDOL_WIDTH - 1] = [1, 3, 1, 7, 3, 9];

    /// Validate and construct a SEDOL.
    ///
    /// Lower case is read as the upper case it spells, because the
    /// identifier is case-insensitive by construction: the check digit
    /// reads a letter by its position, which case does not change.
    ///
    /// ```
    /// use yggdryl::SedolCode;
    ///
    /// let shell = SedolCode::new("B0YBKJ7").unwrap();
    /// assert_eq!(shell.as_str(), "B0YBKJ7");
    /// assert_eq!(shell.check_digit(), 7);
    /// assert_eq!(SedolCode::new("b0ybkj7").unwrap(), shell);
    /// // One digit off is a typo, not a security.
    /// assert!(SedolCode::new("B0YBKJ8").is_err());
    /// assert!(SedolCode::new("B0YBKJ").is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the text is not seven ASCII bytes of the
    /// identifier's shape, or when its check digit does not close it.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = crate::ascii_text(SEDOL_WIDTH, value.as_ref().as_bytes())?;
        let mut bytes = [0_u8; SEDOL_WIDTH];
        for (target, byte) in bytes.iter_mut().zip(value.bytes()) {
            *target = byte.to_ascii_uppercase();
        }
        let folded = std::str::from_utf8(&bytes[..value.len()]).expect("validated ASCII");
        if let Some(reason) = Self::refusal(folded) {
            return Err(crate::Error::InvalidDataType {
                kind: "sedol",
                reason: smol_str::format_smolstr!("{reason}, got {value:?}"),
            });
        }
        Ok(Self(SmolStr::new(folded)))
    }

    /// Borrow the validated identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Borrow the shared storage without copying the identifier.
    #[must_use]
    pub const fn storage(&self) -> &SmolStr {
        &self.0
    }

    /// The check digit that closes the identifier.
    #[must_use]
    pub fn check_digit(&self) -> u8 {
        self.as_str().as_bytes()[6] - b'0'
    }

    /// Whether `text` spells an identifier this type would accept, in
    /// either case.
    #[must_use]
    pub fn is_valid(text: &str) -> bool {
        text.len() == SEDOL_WIDTH
            && text.is_ascii()
            && Self::refusal(&text.to_ascii_uppercase()).is_none()
    }

    /// Whether `text` is an identifier exactly as this type stores it:
    /// upper case, and closed by its check digit.
    ///
    /// What a column holds is the canonical spelling, so bytes arriving
    /// through a cast are held to it rather than folded on every read.
    #[must_use]
    pub fn is_canonical(text: &str) -> bool {
        text.len() == SEDOL_WIDTH
            && text.is_ascii()
            && !text.bytes().any(|byte| byte.is_ascii_lowercase())
            && Self::refusal(text).is_none()
    }

    /// The check digit that closes six leading characters, or `None` where
    /// they are not six upper-case alphanumerics.
    ///
    /// Each character reads as a digit or as ten plus its alphabet
    /// position, weighted `1, 3, 1, 7, 3, 9` in turn, and the digit is what
    /// closes the weighted sum to a multiple of ten.
    #[must_use]
    pub fn closing_digit(body: &str) -> Option<u8> {
        let bytes = body.as_bytes();
        if bytes.len() != SEDOL_WIDTH - 1 {
            return None;
        }
        let mut sum = 0_u32;
        for (byte, weight) in bytes.iter().zip(Self::WEIGHTS) {
            sum += identifier_value(*byte)? * weight;
        }
        u8::try_from((10 - sum % 10) % 10).ok()
    }

    /// Why an upper-cased, seven-byte spelling is not an identifier, or
    /// nothing.
    fn refusal(folded: &str) -> Option<&'static str> {
        let bytes = folded.as_bytes();
        if bytes.len() != SEDOL_WIDTH {
            return Some("expected seven characters");
        }
        if !bytes[..6].iter().all(u8::is_ascii_alphanumeric) {
            return Some("expected six alphanumerics before the check digit");
        }
        if !bytes[6].is_ascii_digit() {
            return Some("expected a closing check digit");
        }
        match Self::closing_digit(&folded[..6]) {
            Some(digit) if digit == bytes[6] - b'0' => None,
            _ => Some("the check digit does not close the identifier"),
        }
    }
}

impl fmt::Display for SedolCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

code_value!(SedolCode, SedolCode, SEDOL_WIDTH);

/// The Arrow extension name of the SEDOL securities identifier.
pub(crate) const SEDOL_EXTENSION_NAME: &str = "yggdryl.sedol";

/// The most bytes a SEDOL securities identifier may be.
///
/// Six alphanumerics and one check digit: seven, which the London Stock
/// Exchange fixes and the check digit closes.
pub(crate) const SEDOL_WIDTH: usize = 7;

impl DataType {
    /// Creates the seven-character SEDOL securities identifier.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::sedol(), DataType::SedolCode);
    /// assert_eq!(DataType::sedol().to_string(), "sedol");
    /// assert_eq!(DataType::sedol().code_width(), Some(7));
    /// ```
    #[must_use]
    pub const fn sedol() -> Self {
        Self::SedolCode
    }
}

// /// A SEDOL-typed field: the seven-character London Stock Exchange securities identifier.
define_field_types!(SedolCodeType, SedolCode);
