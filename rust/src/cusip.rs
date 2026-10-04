//! CUSIP securities identifiers.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::code::code_value;
use crate::code::identifier_value;
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{DataType, Result, Scalar, Value};

/// One CUSIP securities identifier, held by its shape.
///
/// Nine bytes: six of issuer, two of issue and one check digit, which is
/// the modulus-10 "double-add-double" digit of the eight before it read
/// with each letter as ten plus its alphabet position - every second
/// character doubled, the digits of each product summed. The shape is what
/// [`Cusip::new`] admits; whether the digit closes the identifier
/// ([`Cusip::is_closed`]) is the reading its [`rank`](CodeValue::rank) counts,
/// so a typo is a value of rank zero - which every merge replaces by a
/// closing one whatever the order - rather than a refusal, and a derivation
/// that needs the identifier to be real asks `is_closed` first.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Cusip(SmolStr);

impl Cusip {
    /// Validate and construct a CUSIP: nine ASCII bytes of the identifier's
    /// shape, upper-cased.
    ///
    /// Lower case is read as the upper case it spells, because the
    /// identifier is case-insensitive by construction: the check digit
    /// reads a letter by its position, which case does not change. The
    /// check digit is admitted as stated - an identifier it does not close
    /// is a value of rank zero, never a refusal.
    ///
    /// ```
    /// use yggdryl::{CodeValue, Cusip};
    ///
    /// let apple = Cusip::new("037833100").unwrap();
    /// assert_eq!(apple.as_str(), "037833100");
    /// assert_eq!(apple.issuer(), "037833");
    /// assert_eq!(apple.issue(), "10");
    /// assert_eq!(apple.check_digit(), 0);
    /// assert!(apple.is_real());
    /// assert_eq!(Cusip::new("38259p508").unwrap().as_str(), "38259P508");
    /// // One digit off is a typo: an identifier that does not close, and
    /// // ranks below one that does.
    /// let typo = Cusip::new("037833101").unwrap();
    /// assert!(!Cusip::is_closed(typo.as_str()));
    /// assert_eq!(typo.rank(), 0);
    /// assert!(Cusip::new("03783310").is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the text is not nine ASCII bytes of the
    /// identifier's shape: eight alphanumerics and a closing digit.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = crate::ascii_text(CUSIP_WIDTH, value.as_ref().as_bytes())?;
        let mut bytes = [0_u8; CUSIP_WIDTH];
        for (target, byte) in bytes.iter_mut().zip(value.bytes()) {
            *target = byte.to_ascii_uppercase();
        }
        let folded = std::str::from_utf8(&bytes[..value.len()]).expect("validated ASCII");
        if let Some(reason) = Self::refusal(folded) {
            return Err(crate::Error::InvalidDataType {
                kind: "cusip",
                reason: smol_str::format_smolstr!("{reason}, got {value:?}"),
            });
        }
        Ok(Self(SmolStr::new(folded)))
    }

    /// Borrow the identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Borrow the shared storage without copying the identifier.
    #[must_use]
    pub const fn storage(&self) -> &SmolStr {
        &self.0
    }

    /// The six-character issuer number.
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.as_str()[..6]
    }

    /// The two-character issue number.
    #[must_use]
    pub fn issue(&self) -> &str {
        &self.as_str()[6..8]
    }

    /// The check digit that closes the identifier.
    #[must_use]
    pub fn check_digit(&self) -> u8 {
        self.as_str().as_bytes()[8] - b'0'
    }

    /// Whether `text` is an identifier exactly as this type stores it:
    /// upper case, and of the shape [`Self::new`] admits.
    ///
    /// What a column holds is the canonical spelling, so bytes arriving
    /// through a cast are held to it rather than folded on every read. This
    /// is the strict question about the spelling and says nothing of the
    /// check digit, which is [`Self::is_closed`]' question.
    #[must_use]
    pub fn is_canonical(text: &str) -> bool {
        text.len() == CUSIP_WIDTH
            && text.is_ascii()
            && !text.bytes().any(|byte| byte.is_ascii_lowercase())
            && Self::refusal(text).is_none()
    }

    /// Whether `text`, upper case, is an identifier its check digit closes:
    /// the digit of the eight leading characters is the ninth. Lower case
    /// closes nothing.
    ///
    /// ```
    /// use yggdryl::Cusip;
    ///
    /// assert!(Cusip::is_closed("037833100"));
    /// assert!(!Cusip::is_closed("037833101"));
    /// ```
    #[must_use]
    pub fn is_closed(text: &str) -> bool {
        let bytes = text.as_bytes();
        bytes.len() == CUSIP_WIDTH
            && bytes[8].is_ascii_digit()
            && Self::closing_digit(&text[..8]) == Some(bytes[8] - b'0')
    }

    /// The check digit that closes eight leading characters, or `None`
    /// where they are not eight upper-case alphanumerics.
    ///
    /// Each character reads as a digit or as ten plus its alphabet
    /// position; every second value is doubled, the digits of every value
    /// are summed, and the digit is what closes that sum to a multiple of
    /// ten.
    #[must_use]
    pub fn closing_digit(body: &str) -> Option<u8> {
        let bytes = body.as_bytes();
        if bytes.len() != CUSIP_WIDTH - 1 {
            return None;
        }
        let mut sum = 0_u32;
        for (index, byte) in bytes.iter().enumerate() {
            let mut value = identifier_value(*byte)?;
            if index % 2 == 1 {
                value *= 2;
            }
            sum += value / 10 + value % 10;
        }
        u8::try_from((10 - sum % 10) % 10).ok()
    }

    /// Why an upper-cased, nine-byte spelling is not an identifier's
    /// shape, or nothing.
    fn refusal(folded: &str) -> Option<&'static str> {
        let bytes = folded.as_bytes();
        if bytes.len() != CUSIP_WIDTH {
            return Some("expected nine characters");
        }
        if !bytes[..8].iter().all(u8::is_ascii_alphanumeric) {
            return Some("expected eight alphanumerics before the check digit");
        }
        if !bytes[8].is_ascii_digit() {
            return Some("expected a closing check digit");
        }
        None
    }

    /// [`CodeValue::rank`]: one where the held identifier closes.
    fn ranked(&self) -> u8 {
        u8::from(Self::is_closed(self.as_str()))
    }
}

impl fmt::Display for Cusip {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

code_value!(
    Cusip,
    Cusip,
    CUSIP_WIDTH,
    rank = Cusip::ranked,
    max_rank = 1
);

/// The Arrow extension name of the CUSIP securities identifier.
pub(crate) const CUSIP_EXTENSION_NAME: &str = "yggdryl.cusip";

/// The most bytes a CUSIP securities identifier may be.
///
/// Six of issuer, two of issue and one check digit: nine, which the CUSIP
/// Global Services standard fixes and the check digit closes.
pub(crate) const CUSIP_WIDTH: usize = 9;

impl DataType {
    /// Creates the nine-character CUSIP securities identifier.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::cusip(), DataType::Cusip);
    /// assert_eq!(DataType::cusip().to_string(), "cusip");
    /// assert_eq!(DataType::cusip().code_width(), Some(9));
    /// ```
    #[must_use]
    pub const fn cusip() -> Self {
        Self::Cusip
    }
}

// /// A CUSIP-typed field: the nine-character North American securities identifier.
define_field_types!(CusipType, Cusip);
