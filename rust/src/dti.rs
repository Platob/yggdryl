//! ISO 24165 digital token identifiers.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use smol_str::SmolStr;

use crate::code::{code_value, folded_code};
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{DataType, Result, Scalar, Value};

/// The thirty symbols a DTI is written in: the digits and the consonants
/// but `Y`, so no word can be spelled; a symbol's value is its place here.
const ALPHABET: &[u8; 30] = b"0123456789BCDFGHJKLMNPQRSTVWXZ";

/// One ISO 24165 Digital Token Identifier, held by its shape.
///
/// Nine ASCII characters of a thirty-symbol alphabet -
/// `0123456789BCDFGHJKLMNPQRSTVWXZ`, the digits and the consonants but `Y`:
/// an eight-character base that never opens with `0`,
/// then one check character under ISO/IEC 7064 hybrid MOD 31,30. The shape
/// is what [`Dti::new`] admits; whether the check character closes the
/// base ([`Dti::is_closed`]) is its [`rank`](CodeValue::rank), so a typo is
/// a value of rank zero - which every merge replaces by a closing one
/// whatever the order - never a refusal. The check is the rule and nothing
/// else is: a code the registry assigned whose stored character the
/// algorithm does not give ranks as a typo. Lower case folds once at
/// construction; nine bytes stay inline.
#[repr(transparent)]
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Dti(SmolStr);

impl<'de> Deserialize<'de> for Dti {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        SmolStr::deserialize(deserializer)
            .and_then(|value| Self::new(value).map_err(serde::de::Error::custom))
    }
}

impl Dti {
    /// Validate and construct a Digital Token Identifier: nine ASCII bytes
    /// of its alphabet, upper-cased, the check character admitted as stated.
    ///
    /// ```
    /// use yggdryl::{CodeValue, Dti};
    ///
    /// let token = Dti::new("x9j9k872s")?;
    /// assert_eq!(token.as_str(), "X9J9K872S");
    /// assert_eq!(token.check_character(), 'S');
    /// assert!(token.is_real());
    /// // One character off is a typo: a code that does not close.
    /// assert_eq!(Dti::new("X9J9K872T")?.rank(), 0);
    /// assert!(Dti::new("A9J9K872S").is_err(), "a vowel");
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the text is not nine ASCII bytes of the
    /// alphabet, or opens with `0`.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        folded_code::<DTI_WIDTH>("dti", value.as_ref(), Self::refusal).map(Self)
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

    /// The check character as stated: the ninth.
    #[must_use]
    pub fn check_character(&self) -> char {
        char::from(self.as_str().as_bytes()[DTI_WIDTH - 1])
    }

    /// Whether `text` is exactly the spelling this type stores: upper case
    /// and of the shape [`Self::new`] admits, whatever the check character
    /// says - [`Self::is_closed`]' question.
    #[must_use]
    pub fn is_canonical(text: &str) -> bool {
        Self::refusal(text).is_none()
    }

    /// Whether `text`, upper case, is a code its check character closes:
    /// ISO 7064 hybrid MOD 31,30 over the eight leading characters gives
    /// the ninth. Lower case closes nothing.
    ///
    /// ```
    /// use yggdryl::Dti;
    ///
    /// assert!(Dti::is_closed("X9J9K872S"));
    /// assert!(!Dti::is_closed("X9J9K872T"));
    /// assert!(!Dti::is_closed("x9j9k872s"));
    /// ```
    #[must_use]
    pub fn is_closed(text: &str) -> bool {
        Self::is_canonical(text)
            && Self::closing_character(&text[..DTI_WIDTH - 1])
                == Some(char::from(text.as_bytes()[DTI_WIDTH - 1]))
    }

    /// The check character that closes eight leading characters, or `None`
    /// where they are not eight upper-case symbols of the alphabet.
    ///
    /// Hybrid MOD 31,30: from `p = 30`, each symbol of value `v` sets
    /// `s = (p + v) mod 30` - thirty where that is zero - and `p = 2s mod
    /// 31`; the check symbol's value is `(31 - p) mod 30`.
    ///
    /// ```
    /// use yggdryl::Dti;
    ///
    /// assert_eq!(Dti::closing_character("X9J9K872"), Some('S'));
    /// assert_eq!(Dti::closing_character("X9J9K87"), None);
    /// ```
    #[must_use]
    pub fn closing_character(body: &str) -> Option<char> {
        let bytes = body.as_bytes();
        if bytes.len() != DTI_WIDTH - 1 {
            return None;
        }
        let product = bytes.iter().try_fold(30_u32, |product, byte| {
            let sum = (product + Self::value(*byte)?) % 30;
            Some(if sum == 0 { 30 } else { sum } * 2 % 31)
        })?;
        Some(char::from(ALPHABET[((31 - product) % 30) as usize]))
    }

    /// A symbol's value: its place in the alphabet.
    fn value(byte: u8) -> Option<u32> {
        ALPHABET
            .iter()
            .position(|symbol| *symbol == byte)
            .and_then(|place| u32::try_from(place).ok())
    }

    /// [`CodeValue::rank`]: one where the check character closes.
    fn ranked(&self) -> u8 {
        u8::from(Self::is_closed(self.as_str()))
    }

    fn refusal(folded: &str) -> Option<&'static str> {
        let bytes = folded.as_bytes();
        if bytes.len() != DTI_WIDTH {
            return Some("expected nine characters");
        }
        if bytes[0] == b'0' {
            return Some("expected a first character other than 0");
        }
        if !bytes.iter().all(|byte| ALPHABET.contains(byte)) {
            return Some("expected digits or consonants other than Y");
        }
        None
    }
}

impl fmt::Display for Dti {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

code_value!(Dti, Dti, DTI_WIDTH, rank = Dti::ranked, max_rank = 1);

/// The Arrow extension name of a digital token identifier.
pub(crate) const DTI_EXTENSION_NAME: &str = "yggdryl.dti";

/// The exact width of a digital token identifier.
pub(crate) const DTI_WIDTH: usize = 9;

impl DataType {
    /// Creates ISO 24165's nine-character digital token identifier datatype.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::dti(), DataType::Dti);
    /// assert_eq!(DataType::dti().to_string(), "dti");
    /// assert_eq!(DataType::dti().code_width(), Some(9));
    /// ```
    #[must_use]
    pub const fn dti() -> Self {
        Self::Dti
    }
}

define_field_types!(DtiType, Dti);
