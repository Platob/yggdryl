//! ISO 9362 business identifier codes.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use smol_str::SmolStr;

use crate::code::{code_value, folded_code};
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{Country, DataType, Result, Scalar, Value};

/// One ISO 9362 Business Identifier Code, held by its shape.
///
/// Eight or eleven ASCII characters: the business party prefix, four
/// letters or digits; the country, two letters; the business party suffix
/// naming its location, two letters or digits; and, on eleven, the branch,
/// three letters or digits. ISO 9362:2009 wrote the prefix in letters alone;
/// ISO 9362:2014 opened it to digits, so `1234DEFF` is a BIC. Eight and
/// eleven are kept as stated - `DEUTDEFF` and `DEUTDEFFXXX` are two
/// spellings of the one primary office, and neither is folded into the
/// other. There is no check character: how real a BIC is is whether ISO 3166
/// lists its country, or it is SWIFT's `XK` for Kosovo, which ISO 3166 does
/// not assign - its [`rank`](CodeValue::rank). Lower case folds once at
/// construction; eleven bytes stay inline.
#[repr(transparent)]
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Bic(SmolStr);

impl<'de> Deserialize<'de> for Bic {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        SmolStr::deserialize(deserializer)
            .and_then(|value| Self::new(value).map_err(serde::de::Error::custom))
    }
}

/// The country SWIFT assigns BICs for Kosovo, which ISO 3166-1 does not
/// list: a BIC under it is as real as one under a listed country.
const KOSOVO: &str = "XK";

impl Bic {
    /// Validate and construct a Business Identifier Code: eight or eleven
    /// ASCII bytes of its shape, upper-cased.
    ///
    /// ```
    /// use yggdryl::{Bic, CodeValue};
    ///
    /// let office = Bic::new("deutdeff500")?;
    /// assert_eq!(office.as_str(), "DEUTDEFF500");
    /// assert_eq!(office.party_prefix(), "DEUT");
    /// assert_eq!(office.country(), "DE");
    /// assert_eq!(office.location(), "FF");
    /// assert_eq!(office.branch(), Some("500"));
    /// assert!(!office.is_primary_office());
    /// assert!(office.is_real());
    /// // An unassigned country is a value of rank zero, not a refusal.
    /// assert_eq!(Bic::new("DEUTZZFF")?.rank(), 0);
    /// assert!(Bic::new("DEUT1EFF").is_err(), "a digit in the country");
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the text is not eight or eleven ASCII bytes:
    /// four letters or digits, two letters, two letters or digits, and an
    /// optional three letters or digits.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        folded_code::<BIC_WIDTH>("bic", value.as_ref(), Self::refusal).map(Self)
    }

    /// Borrow the code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Borrow the compact storage without copying the code.
    #[must_use]
    pub const fn storage(&self) -> &SmolStr {
        &self.0
    }

    /// The business party prefix: the first four characters.
    #[must_use]
    pub fn party_prefix(&self) -> &str {
        &self.as_str()[..4]
    }

    /// The country: the fifth and sixth characters.
    #[must_use]
    pub fn country(&self) -> &str {
        &self.as_str()[4..6]
    }

    /// The business party suffix naming the location: the seventh and
    /// eighth characters.
    #[must_use]
    pub fn location(&self) -> &str {
        &self.as_str()[6..8]
    }

    /// The branch: the last three characters of an eleven-character code,
    /// `None` on eight.
    #[must_use]
    pub fn branch(&self) -> Option<&str> {
        (self.0.len() == BIC_WIDTH).then(|| &self.as_str()[8..])
    }

    /// Whether this names the party's primary office: eight characters, or
    /// the branch `XXX`.
    #[must_use]
    pub fn is_primary_office(&self) -> bool {
        self.branch().is_none_or(|branch| branch == "XXX")
    }

    /// Whether `text` is exactly the spelling this type stores: upper case
    /// and of the shape [`Self::new`] admits, at its stated length.
    #[must_use]
    pub fn is_canonical(text: &str) -> bool {
        text.is_ascii()
            && !text.bytes().any(|byte| byte.is_ascii_lowercase())
            && Self::refusal(text).is_none()
    }

    /// [`CodeValue::rank`]: one where ISO 3166 lists the country, or it is
    /// SWIFT's `XK`.
    fn ranked(&self) -> u8 {
        let country = self.country();
        u8::from(country == KOSOVO || Country::new(country).is_ok_and(|code| code.is_listed()))
    }

    fn is_alphanumeric(bytes: &[u8]) -> bool {
        bytes
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    }

    fn refusal(folded: &str) -> Option<&'static str> {
        let bytes = folded.as_bytes();
        if bytes.len() != 8 && bytes.len() != BIC_WIDTH {
            return Some("expected eight or eleven characters");
        }
        if !Self::is_alphanumeric(&bytes[..4]) {
            return Some("expected a four-character party prefix of letters or digits");
        }
        if !bytes[4..6].iter().all(u8::is_ascii_uppercase) {
            return Some("expected a two-letter country code");
        }
        if !Self::is_alphanumeric(&bytes[6..8]) {
            return Some("expected a two-character location of letters or digits");
        }
        if !Self::is_alphanumeric(&bytes[8..]) {
            return Some("expected a three-character branch of letters or digits");
        }
        None
    }
}

impl fmt::Display for Bic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

code_value!(Bic, Bic, BIC_WIDTH, rank = Bic::ranked, max_rank = 1);

/// The Arrow extension name of a business identifier code.
pub(crate) const BIC_EXTENSION_NAME: &str = "yggdryl.bic";

/// The most bytes a business identifier code may be: eleven, a branch
/// stated; eight without one.
pub(crate) const BIC_WIDTH: usize = 11;

impl DataType {
    /// Creates ISO 9362's eight- or eleven-character business identifier
    /// code datatype.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::bic(), DataType::Bic);
    /// assert_eq!(DataType::bic().to_string(), "bic");
    /// assert_eq!(DataType::bic().code_width(), Some(11));
    /// ```
    #[must_use]
    pub const fn bic() -> Self {
        Self::Bic
    }
}

define_field_types!(BicType, Bic);
