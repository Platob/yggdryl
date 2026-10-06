//! ISO 17442 legal entity identifiers.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use smol_str::SmolStr;

use crate::code::{code_value, folded_code, identifier_value};
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{DataType, Result, Scalar, Value};

/// One ISO 17442 Legal Entity Identifier, held by its shape.
///
/// Twenty ASCII characters: eighteen letters or digits - the issuing LOU's
/// four-character prefix, then the entity-specific part - and two decimal
/// check digits under ISO/IEC 7064 MOD 97-10, which close the whole code to
/// a remainder of one. ISO 17442:2012 reserved positions five and six as
/// `00`; ISO 17442-1:2020 made them part of the entity-specific code, so
/// `HWUPKR0MPOU8FGXBT394` is an LEI. The shape is what [`Lei::new`] admits;
/// whether the digits close ([`Lei::is_closed`]) is its
/// [`rank`](CodeValue::rank), so a typo is a value of rank zero - which
/// every merge replaces by a closing one whatever the order - never a
/// refusal. Lower case folds once at construction; the twenty bytes stay
/// inline in the crate's compact string.
#[repr(transparent)]
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Lei(SmolStr);

impl<'de> Deserialize<'de> for Lei {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        SmolStr::deserialize(deserializer)
            .and_then(|value| Self::new(value).map_err(serde::de::Error::custom))
    }
}

impl Lei {
    /// Validate and construct a Legal Entity Identifier: twenty ASCII bytes
    /// of its shape, upper-cased, the check digits admitted as stated.
    ///
    /// ```
    /// use yggdryl::{CodeValue, Lei};
    ///
    /// let apple = Lei::new("hwupkr0mpou8fgxbt394")?;
    /// assert_eq!(apple.as_str(), "HWUPKR0MPOU8FGXBT394");
    /// assert_eq!(apple.prefix(), "HWUP");
    /// assert_eq!(apple.check_digits(), 94);
    /// assert!(apple.is_real());
    /// // One digit off is a typo: a code that does not close, and ranks
    /// // below one that does.
    /// assert_eq!(Lei::new("HWUPKR0MPOU8FGXBT395")?.rank(), 0);
    /// assert!(Lei::new("HWUPKR0MPOU8FGXBT3").is_err(), "eighteen characters");
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the text is not twenty ASCII bytes: eighteen
    /// letters or digits, then two digits.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        folded_code::<LEI_WIDTH>("lei", value.as_ref(), Self::refusal).map(Self)
    }

    /// Borrow the identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Borrow the compact storage without copying the identifier.
    #[must_use]
    pub const fn storage(&self) -> &SmolStr {
        &self.0
    }

    /// The issuing LOU's prefix: the first four characters.
    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.as_str()[..4]
    }

    /// The two check digits as one number, `0` to `99` as stated.
    #[must_use]
    pub fn check_digits(&self) -> u8 {
        let bytes = self.as_str().as_bytes();
        (bytes[LEI_WIDTH - 2] - b'0') * 10 + (bytes[LEI_WIDTH - 1] - b'0')
    }

    /// Whether `text` is exactly the spelling this type stores: upper case
    /// and of the shape [`Self::new`] admits, whatever the check digits say -
    /// [`Self::is_closed`]' question.
    #[must_use]
    pub fn is_canonical(text: &str) -> bool {
        text.is_ascii()
            && !text.bytes().any(|byte| byte.is_ascii_lowercase())
            && Self::refusal(text).is_none()
    }

    /// Whether `text`, upper case, is a code its check digits close under
    /// ISO 7064 MOD 97-10: the twenty characters read as one decimal number,
    /// a letter as its two digits from `A` at ten, leave a remainder of one.
    /// Lower case closes nothing.
    ///
    /// ```
    /// use yggdryl::Lei;
    ///
    /// assert!(Lei::is_closed("HWUPKR0MPOU8FGXBT394"));
    /// assert!(!Lei::is_closed("HWUPKR0MPOU8FGXBT395"));
    /// assert!(!Lei::is_closed("hwupkr0mpou8fgxbt394"));
    /// ```
    #[must_use]
    pub fn is_closed(text: &str) -> bool {
        Self::is_canonical(text) && Self::remainder(text.as_bytes()) == Some(1)
    }

    /// The two check digits that close eighteen leading characters, from
    /// `2` to `98`, or `None` where they are not eighteen upper-case letters
    /// or digits.
    ///
    /// ```
    /// use yggdryl::Lei;
    ///
    /// assert_eq!(Lei::closing_digits("HWUPKR0MPOU8FGXBT3"), Some(94));
    /// assert_eq!(Lei::closing_digits("hwupkr0mpou8fgxbt3"), None);
    /// ```
    #[must_use]
    pub fn closing_digits(body: &str) -> Option<u8> {
        let bytes = body.as_bytes();
        if bytes.len() != LEI_WIDTH - 2 || !bytes.iter().all(|byte| Self::is_body_byte(*byte)) {
            return None;
        }
        // The body followed by `00`: two more decimal places.
        let remainder = Self::remainder(bytes)? * 100 % 97;
        u8::try_from(98 - remainder).ok()
    }

    /// The remainder modulo 97 of the characters read as one decimal
    /// number, a letter contributing its two digits (`A` is 10).
    fn remainder(bytes: &[u8]) -> Option<u32> {
        bytes.iter().try_fold(0_u32, |remainder, byte| {
            let value = identifier_value(*byte)?;
            let shift = if value >= 10 { 100 } else { 10 };
            Some((remainder * shift + value) % 97)
        })
    }

    /// [`CodeValue::rank`]: one where the check digits close.
    fn ranked(&self) -> u8 {
        u8::from(Self::is_closed(self.as_str()))
    }

    fn is_body_byte(byte: u8) -> bool {
        byte.is_ascii_uppercase() || byte.is_ascii_digit()
    }

    fn refusal(folded: &str) -> Option<&'static str> {
        let bytes = folded.as_bytes();
        if bytes.len() != LEI_WIDTH {
            return Some("expected twenty characters");
        }
        if !bytes[..LEI_WIDTH - 2]
            .iter()
            .all(|byte| Self::is_body_byte(*byte))
        {
            return Some("expected eighteen letters or digits before the check digits");
        }
        if !bytes[LEI_WIDTH - 2..].iter().all(u8::is_ascii_digit) {
            return Some("expected two closing check digits");
        }
        None
    }
}

impl fmt::Display for Lei {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

code_value!(Lei, Lei, LEI_WIDTH, rank = Lei::ranked, max_rank = 1);

/// The Arrow extension name of a legal entity identifier.
pub(crate) const LEI_EXTENSION_NAME: &str = "yggdryl.lei";

/// The exact width of a legal entity identifier.
pub(crate) const LEI_WIDTH: usize = 20;

impl DataType {
    /// Creates ISO 17442's twenty-character legal entity identifier
    /// datatype.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::lei(), DataType::Lei);
    /// assert_eq!(DataType::lei().to_string(), "lei");
    /// assert_eq!(DataType::lei().code_width(), Some(20));
    /// ```
    #[must_use]
    pub const fn lei() -> Self {
        Self::Lei
    }
}

define_field_types!(LeiType, Lei);
