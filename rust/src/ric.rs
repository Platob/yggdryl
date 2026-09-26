//! Refinitiv Identification Codes: LSEG's ticker-like instrument codes.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use smol_str::{SmolStr, format_smolstr};

use crate::code::code_value;
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{DataType, Result, Scalar, Value};

/// One validated Refinitiv Identification Code.
///
/// A RIC - first the Reuters Instrument Code - names an instrument on LSEG's
/// data services: a ticker, then a period and the mnemonic of the exchange
/// it trades on - `IBM.N` on the New York Stock Exchange, `VOD.L` in London,
/// `0005.HK` in Hong Kong - or one of the forms that name no exchange: an
/// index opening on a period (`.SPX`), a quote closing on `=` (`EUR=`), a
/// chain carrying `#` (`0#.FTSE`), a continuation future (`ESc1`).
///
/// No registry publishes the set and no check digit closes a code, so the
/// rule is its shape: one token of printable ASCII - no space, no control
/// byte - of at most thirty-two bytes. Case is kept, because it is part of
/// the code: `ESc1` is not `ESC1`, and an exchange mnemonic tells `b` from
/// `B`.
#[repr(transparent)]
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Ric(SmolStr);

impl<'de> Deserialize<'de> for Ric {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        SmolStr::deserialize(deserializer)
            .and_then(|value| Self::new(value).map_err(serde::de::Error::custom))
    }
}

impl Ric {
    /// Validate and construct a Refinitiv Identification Code.
    ///
    /// ```
    /// use yggdryl::Ric;
    ///
    /// let vodafone = Ric::new("VOD.L").unwrap();
    /// assert_eq!(vodafone.as_str(), "VOD.L");
    /// assert_eq!(vodafone.exchange_code(), Some("L"));
    /// assert_eq!(Ric::new("ESc1").unwrap().as_str(), "ESc1");
    /// assert!(Ric::new("VOD L").is_err());
    /// assert!(Ric::new("").is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the text is not ASCII that fits thirty-two
    /// bytes, is empty, or holds a space or a control byte.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = crate::ascii_text(RIC_WIDTH, value.as_ref().as_bytes())?;
        if let Some(reason) = Self::refusal(value) {
            return Err(crate::Error::InvalidDataType {
                kind: "ric",
                reason,
            });
        }
        Ok(Self(SmolStr::new(value)))
    }

    /// Borrow the validated code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Borrow the shared storage without copying the code.
    #[must_use]
    pub const fn storage(&self) -> &SmolStr {
        &self.0
    }

    /// The exchange mnemonic after the ticker's period, `None` for a code
    /// that names no exchange.
    ///
    /// The text after the last period, when a ticker stands before it and
    /// something follows it. An index opens on its period and a chain
    /// carries `#`, so neither names an exchange; nor does a code with no
    /// period at all. The mnemonic is the one FIX 4.2's Appendix C lists,
    /// which [`Mic::from_reuters_exchange_code`](crate::Mic::from_reuters_exchange_code)
    /// resolves.
    ///
    /// ```
    /// use yggdryl::{Mic, Ric};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let hsbc = Ric::new("0005.HK")?;
    /// assert_eq!(hsbc.exchange_code(), Some("HK"));
    /// assert_eq!(Mic::from_reuters_exchange_code("HK")?.as_str(), "XHKG");
    /// assert_eq!(Ric::new(".SPX")?.exchange_code(), None);
    /// assert_eq!(Ric::new("0#.FTSE")?.exchange_code(), None);
    /// assert_eq!(Ric::new("EUR=")?.exchange_code(), None);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn exchange_code(&self) -> Option<&str> {
        let text = self.as_str();
        if text.contains('#') {
            return None;
        }
        match text.rfind('.') {
            Some(period) if period > 0 && period + 1 < text.len() => Some(&text[period + 1..]),
            _ => None,
        }
    }

    /// Whether `text` is already the spelling a code stores: one this type
    /// accepts, with no padding for [`Self::new`] to trim.
    #[must_use]
    pub fn is_canonical(text: &str) -> bool {
        crate::ascii_text(RIC_WIDTH, text.as_bytes())
            .is_ok_and(|canonical| canonical == text && Self::refusal(canonical).is_none())
    }

    /// Why ASCII text that fits the width is still not a code, if it is not.
    fn refusal(text: &str) -> Option<SmolStr> {
        if text.is_empty() {
            return Some(SmolStr::new_static(
                "expected a Refinitiv Identification Code, got \"\"",
            ));
        }
        text.bytes()
            .position(|byte| !byte.is_ascii_graphic())
            .map(|position| {
                format_smolstr!(
                    "expected one token of printable ASCII, got 0x{:02X} at {position} in {text:?}",
                    text.as_bytes()[position]
                )
            })
    }
}

impl fmt::Display for Ric {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

code_value!(Ric, Ric, RIC_WIDTH);

/// The Arrow extension name of a Refinitiv Identification Code.
pub(crate) const RIC_EXTENSION_NAME: &str = "yggdryl.ric";

/// The most bytes a Refinitiv Identification Code may be.
///
/// A bound rather than a shape, as for a Bloomberg identifier: a ticker and
/// an exchange mnemonic have no fixed length between them. Thirty-two is the
/// width a security identifier source other than a checked one is held to.
pub(crate) const RIC_WIDTH: usize = 32;

impl DataType {
    /// Creates the Refinitiv Identification Code datatype.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::ric(), DataType::Ric);
    /// assert_eq!(DataType::ric().to_string(), "ric");
    /// assert_eq!(DataType::ric().code_width(), Some(32));
    /// ```
    #[must_use]
    pub const fn ric() -> Self {
        Self::Ric
    }
}

define_field_types!(RicType, Ric);
