//! ISO 18774 financial instrument short names.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use smol_str::SmolStr;

use crate::code::{code_value, folded_code};
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{DataType, Result, Scalar, Value};

/// One ISO 18774 Financial Instrument Short Name, held by its shape.
///
/// At most thirty-five printable ASCII characters, delimiter included: the
/// issuer's short name, a `/`, then the instrument's description -
/// `ACME CORP/SH`. The separator is the first `/`: a description may hold
/// another (`AMORT PN W/P/C`), and dots, ampersands, parentheses, hyphens
/// and colons besides. The issuer takes at most fifteen characters by the
/// ANNA guidelines, save a collective investment vehicle's or an OTC
/// derivative's, whose issuer may run longer, so that bound is not a shape
/// rule. The standard writes upper case alone, so lower case folds once at
/// construction. ISO 18774 draws on ISO/IEC 8859-1, while a code here stores
/// US-ASCII: a Latin-1 letter is refused. No check character: every
/// well-shaped value ranks the same. A short name past twenty-three bytes
/// spills the compact string into one shared allocation, as a Bloomberg
/// identifier or a RIC does.
#[repr(transparent)]
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Fisn(SmolStr);

impl<'de> Deserialize<'de> for Fisn {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        SmolStr::deserialize(deserializer)
            .and_then(|value| Self::new(value).map_err(serde::de::Error::custom))
    }
}

impl Fisn {
    /// Validate and construct a Financial Instrument Short Name: at most
    /// thirty-five printable ASCII bytes, an issuer and a description either
    /// side of the first `/`, upper-cased.
    ///
    /// ```
    /// use yggdryl::{CodeValue, Fisn};
    ///
    /// let name = Fisn::new("acme corp/sh")?;
    /// assert_eq!(name.as_str(), "ACME CORP/SH");
    /// assert_eq!(name.issuer(), "ACME CORP");
    /// assert_eq!(name.description(), "SH");
    /// assert!(name.is_real());
    /// assert_eq!(Fisn::new("ACME CORP/AMORT PN W/P/C")?.description(), "AMORT PN W/P/C");
    /// assert!(Fisn::new("ACME CORP SH").is_err(), "no '/'");
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the text is wider than thirty-five bytes, holds
    /// a byte that is not printable ASCII, or does not state an issuer and a
    /// description either side of a `/`.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        folded_code::<FISN_WIDTH>("fisn", value.as_ref(), Self::refusal).map(Self)
    }

    /// Borrow the short name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Borrow the compact storage without copying the short name.
    #[must_use]
    pub const fn storage(&self) -> &SmolStr {
        &self.0
    }

    /// The issuer's short name: everything before the first `/`.
    #[must_use]
    pub fn issuer(&self) -> &str {
        self.split().0
    }

    /// The instrument's description: everything after the first `/`.
    #[must_use]
    pub fn description(&self) -> &str {
        self.split().1
    }

    /// Whether `text` is exactly the spelling this type stores: upper case
    /// and of the shape [`Self::new`] admits.
    #[must_use]
    pub fn is_canonical(text: &str) -> bool {
        text.len() <= FISN_WIDTH
            && !text.bytes().any(|byte| byte.is_ascii_lowercase())
            && Self::refusal(text).is_none()
    }

    fn split(&self) -> (&str, &str) {
        self.as_str()
            .split_once('/')
            .expect("a short name holds its '/'")
    }

    fn refusal(folded: &str) -> Option<&'static str> {
        if !folded.bytes().all(|byte| matches!(byte, b' '..=b'~')) {
            return Some("expected printable characters");
        }
        match folded.split_once('/') {
            None => Some("expected a '/' between the issuer and the instrument description"),
            Some(("", _)) => Some("expected an issuer name before the '/'"),
            Some((_, "")) => Some("expected an instrument description after the '/'"),
            Some(_) => None,
        }
    }
}

impl fmt::Display for Fisn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

code_value!(Fisn, Fisn, FISN_WIDTH);

/// The Arrow extension name of a financial instrument short name.
pub(crate) const FISN_EXTENSION_NAME: &str = "yggdryl.fisn";

/// The most bytes a financial instrument short name may be.
pub(crate) const FISN_WIDTH: usize = 35;

impl DataType {
    /// Creates ISO 18774's financial instrument short name datatype: at
    /// most thirty-five bytes.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::fisn(), DataType::Fisn);
    /// assert_eq!(DataType::fisn().to_string(), "fisn");
    /// assert_eq!(DataType::fisn().code_width(), Some(35));
    /// ```
    #[must_use]
    pub const fn fisn() -> Self {
        Self::Fisn
    }
}

define_field_types!(FisnType, Fisn);
