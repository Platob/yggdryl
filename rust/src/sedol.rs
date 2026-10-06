//! SEDOL securities identifiers.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::code::code_value;
use crate::code::identifier_value;
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{DataType, Result, Scalar, Value};

/// One SEDOL securities identifier, held by its shape.
///
/// Seven bytes: six alphanumerics and one check digit, the modulus-10
/// digit of the six before it read with each letter as ten plus its
/// alphabet position and weighted `1, 3, 1, 7, 3, 9` in turn. The shape is
/// what [`Sedol::new`] admits; whether the digit closes the identifier
/// ([`Sedol::is_closed`]) is the reading its [`rank`](CodeValue::rank) counts,
/// so a typo is a value of rank zero - which every merge replaces by a
/// closing one whatever the order - rather than a refusal, as a CUSIP's and
/// an ISIN's are.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Sedol(SmolStr);

impl Sedol {
    /// The weight each of the six leading characters carries.
    const WEIGHTS: [u32; SEDOL_WIDTH - 1] = [1, 3, 1, 7, 3, 9];

    /// Validate and construct a SEDOL: seven ASCII bytes of the
    /// identifier's shape, upper-cased.
    ///
    /// Lower case is read as the upper case it spells, because the
    /// identifier is case-insensitive by construction: the check digit
    /// reads a letter by its position, which case does not change. The
    /// check digit is admitted as stated - an identifier it does not close
    /// is a value of rank zero, never a refusal.
    ///
    /// ```
    /// use yggdryl::{CodeValue, Sedol};
    ///
    /// let shell = Sedol::new("B0YBKJ7").unwrap();
    /// assert_eq!(shell.as_str(), "B0YBKJ7");
    /// assert_eq!(shell.check_digit(), 7);
    /// assert!(shell.is_real());
    /// assert_eq!(Sedol::new("b0ybkj7").unwrap(), shell);
    /// // One digit off is a typo: an identifier that does not close, and
    /// // ranks below one that does.
    /// let typo = Sedol::new("B0YBKJ8").unwrap();
    /// assert!(!Sedol::is_closed(typo.as_str()));
    /// assert_eq!(typo.rank(), 0);
    /// assert!(Sedol::new("B0YBKJ").is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the text is not seven ASCII bytes of the
    /// identifier's shape: six alphanumerics and a closing digit.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        crate::code::folded_code::<SEDOL_WIDTH>("sedol", value.as_ref(), Self::refusal).map(Self)
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

    /// The check digit that closes the identifier.
    #[must_use]
    pub fn check_digit(&self) -> u8 {
        self.as_str().as_bytes()[6] - b'0'
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
        text.len() == SEDOL_WIDTH
            && text.is_ascii()
            && !text.bytes().any(|byte| byte.is_ascii_lowercase())
            && Self::refusal(text).is_none()
    }

    /// Whether `text`, upper case, is an identifier its check digit closes:
    /// the digit of the six leading characters is the seventh. Lower case
    /// closes nothing.
    ///
    /// ```
    /// use yggdryl::Sedol;
    ///
    /// assert!(Sedol::is_closed("B0YBKJ7"));
    /// assert!(!Sedol::is_closed("B0YBKJ8"));
    /// ```
    #[must_use]
    pub fn is_closed(text: &str) -> bool {
        let bytes = text.as_bytes();
        bytes.len() == SEDOL_WIDTH
            && bytes[6].is_ascii_digit()
            && Self::closing_digit(&text[..6]) == Some(bytes[6] - b'0')
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

    /// Why an upper-cased, seven-byte spelling is not an identifier's
    /// shape, or nothing.
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
        None
    }

    /// [`CodeValue::rank`]: one where the held identifier closes.
    fn ranked(&self) -> u8 {
        u8::from(Self::is_closed(self.as_str()))
    }
}

impl fmt::Display for Sedol {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

code_value!(
    Sedol,
    Sedol,
    SEDOL_WIDTH,
    rank = Sedol::ranked,
    max_rank = 1
);

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
    /// assert_eq!(DataType::sedol(), DataType::Sedol);
    /// assert_eq!(DataType::sedol().to_string(), "sedol");
    /// assert_eq!(DataType::sedol().code_width(), Some(7));
    /// ```
    #[must_use]
    pub const fn sedol() -> Self {
        Self::Sedol
    }
}

// /// A SEDOL-typed field: the seven-character London Stock Exchange securities identifier.
define_field_types!(SedolType, Sedol);
