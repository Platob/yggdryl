//! The registered codes' values: an identity, held in the width it fixes.
//!
//! A code is at most twelve US-ASCII bytes, so every value here lives inside
//! the crate's compact string and never touches the heap: the text is
//! validated once when it is built and never changed after. Equality, order
//! and hashing read the text; the family enum keeps the identity in front of
//! it, so a currency is never a country however alike their bytes look.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::codes::{
    BLOOMBERG_WIDTH, CFI_WIDTH, COUNTRY_WIDTH, CURRENCY_WIDTH, CUSIP_WIDTH, ISIN_WIDTH, MIC_WIDTH,
    SEDOL_WIDTH, SIDE_WIDTH, STATE_WIDTH, TIMEINFORCE_WIDTH,
};
use crate::{DataType, DataTypeId, Result, Scalar, Value, types};

/// Borrowing access shared by every code representation.
pub trait CodeValue: crate::Value {
    /// The fixed storage width, in bytes.
    const WIDTH: usize;

    /// Borrow the validated code.
    fn as_str(&self) -> &str;
    /// Borrow the shared storage behind the validated code.
    ///
    /// The stored text is already trimmed and checked, so a rewrite that
    /// keeps it clones this handle rather than re-validating and copying.
    fn storage(&self) -> &SmolStr;

    /// The better statement of this code and another of the same kind: this
    /// one, unless it states less than `other` does.
    ///
    /// What "less" means is each code's own, and the codes that can state
    /// nothing say so: a [`Cfi`] fills every `X` position from the other
    /// where the two describe one instrument; a
    /// [`State`] that reached none, `00UNKNOWN`, takes the other, and
    /// otherwise the further along stands; a [`Side`] `UNKNOWN`, a
    /// [`Currency`] `XXX` and a [`Mic`] `XXXX` take the other. Every other
    /// code is an identifier with nothing partial about it, so this one
    /// stands as it is. This is what a graph element folds two statements
    /// of one fact with.
    #[must_use]
    fn merge_with(self, other: &Self) -> Self {
        let _ = other;
        self
    }
}

macro_rules! code_leaf {
    ($name:ident, $width:expr) => {
        #[doc = concat!("One validated `", stringify!($name), "` code.")]
        #[repr(transparent)]
        #[derive(
            Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
        )]
        #[serde(transparent)]
        pub struct $name(SmolStr);

        impl $name {
            /// Validate and construct this registered code.
            ///
            /// # Errors
            ///
            /// Returns an error naming the width when the text is not ASCII
            /// text that fits it.
            pub fn new(value: impl AsRef<str>) -> Result<Self> {
                let value = types::ascii_text($width, value.as_ref().as_bytes())?;
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
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

code_leaf!(Country, COUNTRY_WIDTH);
code_leaf!(Currency, CURRENCY_WIDTH);
code_leaf!(Mic, MIC_WIDTH);
code_leaf!(Cfi, CFI_WIDTH);
code_leaf!(Bloomberg, BLOOMBERG_WIDTH);

impl Currency {
    /// ISO 4217's code for no currency.
    const NONE: &str = "XXX";

    /// The currency stated as none: ISO 4217's `XXX`, which a merge takes
    /// the other currency over.
    #[must_use]
    pub fn none() -> Self {
        Self(SmolStr::new_static(Self::NONE))
    }

    /// The better of two currencies: this one, unless it is `XXX`.
    fn merged(self, other: &Self) -> Self {
        if self.as_str() == Self::NONE {
            other.clone()
        } else {
            self
        }
    }
}

impl Mic {
    /// ISO 10383's code for no market.
    const NONE: &str = "XXXX";

    /// The market stated as none: ISO 10383's `XXXX`, which a merge takes
    /// the other market over.
    #[must_use]
    pub fn none() -> Self {
        Self(SmolStr::new_static(Self::NONE))
    }

    /// The better of two markets: this one, unless it is `XXXX`.
    fn merged(self, other: &Self) -> Self {
        if self.as_str() == Self::NONE {
            other.clone()
        } else {
            self
        }
    }
}

impl Bloomberg {
    /// Whether `text` is already the canonical spelling of an identifier.
    ///
    /// The one code here with no shape to check: a Bloomberg identifier is a
    /// ticker, a market and a yellow key with spaces between them, or a
    /// FIGI, and the standard that would say which is a terminal's rather
    /// than a registry's. So canonical is what it is for every code - ASCII
    /// that fits the width, in upper case - and no more, because refusing a
    /// spelling nobody published would be a guess.
    #[must_use]
    pub fn is_canonical(text: &str) -> bool {
        !text.is_empty()
            && text.len() <= BLOOMBERG_WIDTH
            && text.is_ascii()
            && !text.bytes().any(|byte| byte.is_ascii_lowercase())
    }
}

/// One validated ISO 6166 international securities identification number.
///
/// Twelve bytes: a two-letter prefix, nine alphanumerics of national number
/// and one check digit, which is the Luhn digit of the eleven before it read
/// with each letter expanded to the two digits of its alphabet position.
/// A spelling whose check digit does not close it is refused, because an
/// identifier that fails its own checksum is not that identifier - it is a
/// typo, and a typo typed as a security joins to the wrong one.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Isin(SmolStr);

impl Isin {
    /// Validate and construct a securities identification number.
    ///
    /// Lower case is read as the upper case it spells, because the number
    /// is case-insensitive by construction: the check digit expands a letter
    /// by its position, which case does not change.
    ///
    /// ```
    /// use yggdryl::types::Isin;
    ///
    /// let apple = Isin::new("US0378331005").unwrap();
    /// assert_eq!(apple.as_str(), "US0378331005");
    /// assert_eq!(apple.prefix(), "US");
    /// assert_eq!(apple.nsin(), "037833100");
    /// assert_eq!(apple.check_digit(), 5);
    /// assert_eq!(Isin::new("us0378331005").unwrap(), apple);
    /// // One digit off is a typo, not a security.
    /// assert!(Isin::new("US0378331006").is_err());
    /// assert!(Isin::new("US037833100").is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the text is not twelve ASCII bytes of the
    /// number's shape, or when its check digit does not close it.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = types::ascii_text(ISIN_WIDTH, value.as_ref().as_bytes())?;
        let folded = value.to_ascii_uppercase();
        if let Some(reason) = Self::refusal(&folded) {
            return Err(crate::Error::InvalidDataType {
                kind: "isin",
                reason: smol_str::format_smolstr!("{reason}, got {value:?}"),
            });
        }
        Ok(Self(SmolStr::new(folded)))
    }

    /// Borrow the validated number.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Borrow the shared storage without copying the number.
    #[must_use]
    pub const fn storage(&self) -> &SmolStr {
        &self.0
    }

    /// The two-letter prefix: the country of the numbering agency, or one of
    /// the international prefixes such as `XS`.
    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.as_str()[..2]
    }

    /// The nine-character national securities identifying number.
    #[must_use]
    pub fn nsin(&self) -> &str {
        &self.as_str()[2..11]
    }

    /// The check digit that closes the number.
    #[must_use]
    pub fn check_digit(&self) -> u8 {
        self.as_str().as_bytes()[11] - b'0'
    }

    /// Whether `text` spells a number this type would accept, in either case.
    #[must_use]
    pub fn is_valid(text: &str) -> bool {
        text.len() == ISIN_WIDTH
            && text.is_ascii()
            && Self::refusal(&text.to_ascii_uppercase()).is_none()
    }

    /// Whether `text` is a number exactly as this type stores it: upper
    /// case, and closed by its check digit.
    ///
    /// What a column holds is the canonical spelling, so bytes arriving
    /// through a cast are held to it rather than folded on every read.
    #[must_use]
    pub fn is_canonical(text: &str) -> bool {
        text.len() == ISIN_WIDTH
            && text.is_ascii()
            && !text.bytes().any(|byte| byte.is_ascii_lowercase())
            && Self::refusal(text).is_none()
    }

    /// The check digit that closes eleven leading characters, or `None`
    /// where they are not two letters and nine alphanumerics.
    ///
    /// ISO 6166 reads the eleven as digits - a letter as the two digits of
    /// its position from `A` at ten - and closes them with the Luhn digit,
    /// doubling every second digit from the right.
    #[must_use]
    pub fn closing_digit(body: &str) -> Option<u8> {
        let bytes = body.as_bytes();
        if bytes.len() != ISIN_WIDTH - 1
            || !bytes[..2].iter().all(u8::is_ascii_uppercase)
            || !bytes[2..].iter().all(u8::is_ascii_alphanumeric)
        {
            return None;
        }
        // Eleven characters expand to at most twenty-two digits.
        let mut digits = [0_u8; 2 * (ISIN_WIDTH - 1)];
        let mut held = 0;
        for byte in bytes {
            match byte {
                b'0'..=b'9' => {
                    digits[held] = byte - b'0';
                    held += 1;
                }
                b'A'..=b'Z' => {
                    let position = byte - b'A' + 10;
                    digits[held] = position / 10;
                    digits[held + 1] = position % 10;
                    held += 2;
                }
                _ => return None,
            }
        }
        let mut sum = 0_u32;
        for (from_right, digit) in digits[..held].iter().rev().enumerate() {
            let mut value = u32::from(*digit);
            if from_right % 2 == 0 {
                value *= 2;
                if value > 9 {
                    value -= 9;
                }
            }
            sum += value;
        }
        u8::try_from((10 - sum % 10) % 10).ok()
    }

    /// Why an upper-cased, twelve-byte spelling is not a number, or nothing.
    fn refusal(folded: &str) -> Option<&'static str> {
        let bytes = folded.as_bytes();
        if bytes.len() != ISIN_WIDTH {
            return Some("expected twelve characters");
        }
        if !bytes[..2].iter().all(u8::is_ascii_uppercase) {
            return Some("expected a two-letter prefix");
        }
        if !bytes[2..11].iter().all(u8::is_ascii_alphanumeric) {
            return Some("expected nine alphanumerics after the prefix");
        }
        if !bytes[11].is_ascii_digit() {
            return Some("expected a closing check digit");
        }
        match Self::closing_digit(&folded[..11]) {
            Some(digit) if digit == bytes[11] - b'0' => None,
            _ => Some("the check digit does not close the number"),
        }
    }
}

impl fmt::Display for Isin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The value one character of a securities identifier reads as: a digit as
/// itself, a letter as ten plus its position in the alphabet, from `A` at
/// ten to `Z` at thirty-five.
///
/// The one reading CUSIP and SEDOL share, over an upper-cased byte; anything
/// else is not part of an identifier.
const fn identifier_value(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some((byte - b'0') as u32),
        b'A'..=b'Z' => Some((byte - b'A') as u32 + 10),
        _ => None,
    }
}

/// One validated CUSIP securities identifier.
///
/// Nine bytes: six of issuer, two of issue and one check digit, which is
/// the modulus-10 "double-add-double" digit of the eight before it read
/// with each letter as ten plus its alphabet position - every second
/// character doubled, the digits of each product summed. A spelling whose
/// check digit does not close it is refused for the reason an ISIN's is: an
/// identifier that fails its own checksum is a typo, and a typo typed as a
/// security joins to the wrong one.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Cusip(SmolStr);

impl Cusip {
    /// Validate and construct a CUSIP.
    ///
    /// Lower case is read as the upper case it spells, because the
    /// identifier is case-insensitive by construction: the check digit
    /// reads a letter by its position, which case does not change.
    ///
    /// ```
    /// use yggdryl::types::Cusip;
    ///
    /// let apple = Cusip::new("037833100").unwrap();
    /// assert_eq!(apple.as_str(), "037833100");
    /// assert_eq!(apple.issuer(), "037833");
    /// assert_eq!(apple.issue(), "10");
    /// assert_eq!(apple.check_digit(), 0);
    /// assert_eq!(Cusip::new("38259p508").unwrap().as_str(), "38259P508");
    /// // One digit off is a typo, not a security.
    /// assert!(Cusip::new("037833101").is_err());
    /// assert!(Cusip::new("03783310").is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the text is not nine ASCII bytes of the
    /// identifier's shape, or when its check digit does not close it.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = types::ascii_text(CUSIP_WIDTH, value.as_ref().as_bytes())?;
        let folded = value.to_ascii_uppercase();
        if let Some(reason) = Self::refusal(&folded) {
            return Err(crate::Error::InvalidDataType {
                kind: "cusip",
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

    /// Whether `text` spells an identifier this type would accept, in
    /// either case.
    #[must_use]
    pub fn is_valid(text: &str) -> bool {
        text.len() == CUSIP_WIDTH
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
        text.len() == CUSIP_WIDTH
            && text.is_ascii()
            && !text.bytes().any(|byte| byte.is_ascii_lowercase())
            && Self::refusal(text).is_none()
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

    /// Why an upper-cased, nine-byte spelling is not an identifier, or
    /// nothing.
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
        match Self::closing_digit(&folded[..8]) {
            Some(digit) if digit == bytes[8] - b'0' => None,
            _ => Some("the check digit does not close the identifier"),
        }
    }
}

impl fmt::Display for Cusip {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

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
pub struct Sedol(SmolStr);

impl Sedol {
    /// The weight each of the six leading characters carries.
    const WEIGHTS: [u32; SEDOL_WIDTH - 1] = [1, 3, 1, 7, 3, 9];

    /// Validate and construct a SEDOL.
    ///
    /// Lower case is read as the upper case it spells, because the
    /// identifier is case-insensitive by construction: the check digit
    /// reads a letter by its position, which case does not change.
    ///
    /// ```
    /// use yggdryl::types::Sedol;
    ///
    /// let shell = Sedol::new("B0YBKJ7").unwrap();
    /// assert_eq!(shell.as_str(), "B0YBKJ7");
    /// assert_eq!(shell.check_digit(), 7);
    /// assert_eq!(Sedol::new("b0ybkj7").unwrap(), shell);
    /// // One digit off is a typo, not a security.
    /// assert!(Sedol::new("B0YBKJ8").is_err());
    /// assert!(Sedol::new("B0YBKJ").is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the text is not seven ASCII bytes of the
    /// identifier's shape, or when its check digit does not close it.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = types::ascii_text(SEDOL_WIDTH, value.as_ref().as_bytes())?;
        let folded = value.to_ascii_uppercase();
        if let Some(reason) = Self::refusal(&folded) {
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

impl fmt::Display for Sedol {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

code_leaf!(Side, SIDE_WIDTH);
code_leaf!(State, STATE_WIDTH);
code_leaf!(TimeInForce, TIMEINFORCE_WIDTH);

impl Side {
    /// The spelling of a side stated as none.
    const UNKNOWN: &str = "UNKNOWN";

    /// The side stated as none: `UNKNOWN`, which takes no lane and which a
    /// merge takes the other side over.
    #[must_use]
    pub fn unknown() -> Self {
        Self(SmolStr::new_static(Self::UNKNOWN))
    }

    /// The sides that take the bid lane of a quote: a party willing to pay.
    ///
    /// Domain knowledge, written where a reviewer can check it: Orchestra
    /// does not publish which side takes which lane.
    const BID: [&str; 2] = ["BUY", "BUYMINUS"];

    /// The sides that take the ask lane of a quote: a party willing to be
    /// paid. Everything in neither listing - a cross, `UNDISC`, `ASDEF`,
    /// `OPPOSITE`, a side stated as none - takes no lane: a cross is both
    /// sides at once and `OPPOSITE` means "whatever the other leg was".
    const ASK: [&str; 5] = ["SELL", "SELLPLUS", "SSHORT", "SSHORTEX", "SELLUND"];

    /// Whether this side takes the bid lane of a quote.
    ///
    /// ```
    /// use yggdryl::types::Side;
    ///
    /// assert!(Side::read("Buy").unwrap().is_bid());
    /// assert!(!Side::read("SellShort").unwrap().is_bid());
    /// assert!(!Side::read("Cross").unwrap().is_bid());
    /// ```
    #[must_use]
    pub fn is_bid(&self) -> bool {
        Self::BID.contains(&self.as_str())
    }

    /// Whether this side takes the ask lane of a quote.
    ///
    /// ```
    /// use yggdryl::types::Side;
    ///
    /// assert!(Side::read("SellShortExempt").unwrap().is_ask());
    /// assert!(!Side::read("Buy").unwrap().is_ask());
    /// assert!(!Side::read("Opposite").unwrap().is_ask());
    /// ```
    #[must_use]
    pub fn is_ask(&self) -> bool {
        Self::ASK.contains(&self.as_str())
    }

    /// The better of two sides: this one, unless it is `UNKNOWN`.
    fn merged(self, other: &Self) -> Self {
        if self.as_str() == Self::UNKNOWN {
            other.clone()
        } else {
            self
        }
    }

    /// The side one spelling names, refused where none does.
    ///
    /// [`Self::from_spelling`] as the value contract reads it: a column typed
    /// `side` holds the explicit values only, so text that names no side
    /// leaves the column null rather than storing a spelling nothing reads.
    ///
    /// # Errors
    ///
    /// Returns an error naming the spelling.
    pub fn read(spelling: &str) -> Result<Self> {
        Self::from_spelling(spelling).ok_or_else(|| crate::Error::InvalidDataType {
            kind: "side",
            reason: smol_str::format_smolstr!(
                "expected a side code, name or stored value, got {spelling:?}"
            ),
        })
    }

    /// The side one spelling names, or `None` where none does.
    ///
    /// Three vocabularies reach one value, because they name one thing:
    ///
    /// - a FIX `Side(54)` wire code - `1`, `2`, `5`, `H`;
    /// - the specification's own name for it - `Buy`, `SellShort`, `AsDefined`;
    /// - the stored value itself - `BUY`, `SSHORT`, `ASDEF`.
    ///
    /// Names fold the way every other name in this crate folds: ASCII case
    /// insensitive, with `_`, `-` and spaces ignored, so `SellShort`,
    /// `sell_short` and `SELL SHORT` are one spelling. A wire code does
    /// **not** fold, because `A` and `a` are different codes in FIX.
    ///
    /// ```
    /// use yggdryl::types::Side;
    ///
    /// // The wire code, the specification's name and the stored value.
    /// assert_eq!(Side::from_spelling("1").unwrap().as_str(), "BUY");
    /// assert_eq!(Side::from_spelling("SellShort").unwrap().as_str(), "SSHORT");
    /// assert_eq!(Side::from_spelling("sshort").unwrap().as_str(), "SSHORT");
    /// assert_eq!(Side::from_spelling("H").unwrap().as_str(), "SELLUND");
    /// assert!(Side::from_spelling("X").is_none());
    /// ```
    #[must_use]
    pub fn from_spelling(spelling: &str) -> Option<Self> {
        if crate::types::StringEnum::SIDES.contains(&spelling) {
            return Self::new(spelling).ok();
        }
        if let Some(held) = SIDE_CODES
            .iter()
            .find(|(code, _)| *code == spelling)
            .map(|(_, side)| *side)
        {
            return Self::new(held).ok();
        }
        let folded = folded_spelling(spelling);
        SIDE_NAMES
            .iter()
            .find(|(name, _)| *name == folded.as_str())
            .map(|(_, side)| *side)
            .and_then(|held| Self::new(held).ok())
    }
}

/// FIX's `Side(54)` wire codes, unfolded, and the explicit value each names.
static SIDE_CODES: &[(&str, &str)] = &[
    ("1", "BUY"),
    ("2", "SELL"),
    ("3", "BUYMINUS"),
    ("4", "SELLPLUS"),
    ("5", "SSHORT"),
    ("6", "SSHORTEX"),
    ("7", "UNDISC"),
    ("8", "CROSS"),
    ("9", "CROSSSH"),
    ("A", "CROSSSHX"),
    ("B", "ASDEF"),
    ("C", "OPPOSITE"),
    ("D", "SUBSCR"),
    ("E", "REDEEM"),
    ("F", "LEND"),
    ("G", "BORROW"),
    ("H", "SELLUND"),
];

/// Every name that reaches a side, folded: the specification's, and the
/// stored values in any case.
static SIDE_NAMES: &[(&str, &str)] = &[
    ("asdef", "ASDEF"),
    ("asdefined", "ASDEF"),
    ("borrow", "BORROW"),
    ("buy", "BUY"),
    ("buyminus", "BUYMINUS"),
    ("cross", "CROSS"),
    ("crossshort", "CROSSSH"),
    ("crossshortexempt", "CROSSSHX"),
    ("crosssh", "CROSSSH"),
    ("crossshx", "CROSSSHX"),
    ("lend", "LEND"),
    ("opposite", "OPPOSITE"),
    ("redeem", "REDEEM"),
    ("sell", "SELL"),
    ("sellplus", "SELLPLUS"),
    ("sellshort", "SSHORT"),
    ("sellshortexempt", "SSHORTEX"),
    ("sellund", "SELLUND"),
    ("sellundisclosed", "SELLUND"),
    ("sshort", "SSHORT"),
    ("sshortex", "SSHORTEX"),
    ("subscr", "SUBSCR"),
    ("subscribe", "SUBSCR"),
    ("undisc", "UNDISC"),
    ("undisclosed", "UNDISC"),
    ("unknown", "UNKNOWN"),
];

impl State {
    /// The state stated as none: `00UNKNOWN`, rank zero, which every other
    /// state is further along than.
    #[must_use]
    pub fn unknown() -> Self {
        Self(SmolStr::new_static("00UNKNOWN"))
    }

    /// The better of two states: this one, unless it reached none - rank
    /// `00` - or the other reached further.
    fn merged(self, other: &Self) -> Self {
        match (self.rank(), other.rank()) {
            (None | Some(0), _) => other.clone(),
            (Some(this), Some(that)) if that > this => other.clone(),
            _ => self,
        }
    }

    /// The rank a stored state opens with, first to terminal.
    ///
    /// The two leading digits read as the number they spell, `0` to `99`, or
    /// `None` where the value does not open with two digits. A sort of the
    /// raw column is already in this order, because the digits lead and are
    /// fixed at two.
    #[must_use]
    pub fn rank(&self) -> Option<u8> {
        match self.as_str().as_bytes() {
            [tens @ b'0'..=b'9', ones @ b'0'..=b'9', ..] => {
                Some((tens - b'0') * 10 + (ones - b'0'))
            }
            _ => None,
        }
    }

    /// Whether this state can still change.
    ///
    /// Every rank below `80`, the first terminal band. A reader asking "is
    /// this still going" asks this rather than listing names.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.rank().is_some_and(|rank| rank < 80)
    }

    /// Whether this state ended having done what was asked: rank `80`-`89`.
    #[must_use]
    pub fn is_done(&self) -> bool {
        matches!(self.rank(), Some(80..=89))
    }

    /// Whether this state ended because someone stopped it: rank `90`-`94`.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        matches!(self.rank(), Some(90..=94))
    }

    /// Whether this state ended because it could not be done: rank `95`-`99`.
    #[must_use]
    pub fn is_failed(&self) -> bool {
        matches!(self.rank(), Some(95..=99))
    }

    /// The state one spelling names, refused where none does.
    ///
    /// [`Self::from_spelling`] as the value contract reads it: a column typed
    /// `state` holds ranked values only, so text that names no state leaves
    /// the column null rather than storing a value nothing can rank.
    ///
    /// # Errors
    ///
    /// Returns an error naming the spelling.
    pub fn read(spelling: &str) -> Result<Self> {
        Self::from_spelling(spelling).ok_or_else(|| crate::Error::InvalidDataType {
            kind: "state",
            reason: smol_str::format_smolstr!(
                "expected a state code, name or stored value, got {spelling:?}"
            ),
        })
    }

    /// The state one spelling names, or `None` where none does.
    ///
    /// Four vocabularies reach one value, because they name one thing:
    ///
    /// - a FIX `OrdStatus(39)` or `ExecType(150)` wire code - `0`, `1`, `F`;
    /// - the specification's own name for it - `PartiallyFilled`, `DoneForDay`;
    /// - the word a scheduler uses - `running`, `succeeded`, `timed out`;
    /// - the short name a FIX bridge logs - `PartFill`, `PendNew`, `DoneDay`.
    ///
    /// Names fold the way every other name in this crate folds: ASCII case
    /// insensitive, with `_`, `-` and spaces ignored, so `DoneForDay`,
    /// `done_for_day` and `DONE FOR DAY` are one spelling. A wire code does
    /// **not** fold, because `A` and `a` are different codes in FIX and a
    /// folded lookup would answer the wrong state for one of them.
    ///
    /// ```
    /// use yggdryl::types::State;
    ///
    /// // The wire code, the specification's name and the scheduler's word.
    /// assert_eq!(State::from_spelling("1").unwrap().as_str(), "40PARTFILL");
    /// assert_eq!(State::from_spelling("PartiallyFilled").unwrap().as_str(), "40PARTFILL");
    /// assert_eq!(State::from_spelling("running").unwrap().as_str(), "30RUNNING");
    ///
    /// // The stored bytes sort from the first state to the terminal ones,
    /// // which is the whole reason the rank leads.
    /// let mut held = ["80FILLED", "20NEW", "95REJECTED", "40PARTFILL"];
    /// held.sort_unstable();
    /// assert_eq!(held, ["20NEW", "40PARTFILL", "80FILLED", "95REJECTED"]);
    ///
    /// // And the three endings are told apart without reading the name.
    /// assert!(State::from_spelling("New").unwrap().is_live());
    /// assert!(State::from_spelling("Filled").unwrap().is_done());
    /// assert!(State::from_spelling("Rejected").unwrap().is_failed());
    /// ```
    #[must_use]
    pub fn from_spelling(spelling: &str) -> Option<Self> {
        // A stored value names itself, which is what makes reading one back
        // free and the whole mapping idempotent.
        if crate::types::StringEnum::STATES.contains(&spelling) {
            return Self::new(spelling).ok();
        }
        if let Some(held) = STATE_CODES
            .iter()
            .find(|(code, _)| *code == spelling)
            .map(|(_, state)| *state)
        {
            return Self::new(held).ok();
        }
        let folded = folded_spelling(spelling);
        STATE_NAMES
            .iter()
            .find(|(name, _)| *name == folded.as_str())
            .map(|(_, state)| *state)
            .and_then(|held| Self::new(held).ok())
    }
}

/// One spelling folded the way every name in this crate folds.
fn folded_spelling(spelling: &str) -> SmolStr {
    let mut held = smol_str::SmolStrBuilder::new();
    for byte in spelling.bytes() {
        if matches!(byte, b'_' | b'-' | b' ') {
            continue;
        }
        held.push(char::from(byte.to_ascii_lowercase()));
    }
    held.finish()
}

/// FIX's `OrdStatus(39)` and `ExecType(150)` wire codes, unfolded.
///
/// The two code sets agree on every value they share, which is why one table
/// answers both: `0` is New in each, `1` PartiallyFilled, `2` Filled. Where
/// only `ExecType` defines a value - `F` Trade, `L` Triggered - the state is
/// what that report says the order is doing.
static STATE_CODES: &[(&str, &str)] = &[
    ("0", "20NEW"),
    ("1", "40PARTFILL"),
    ("2", "80FILLED"),
    ("3", "80DONEDAY"),
    ("4", "90CANCELED"),
    ("5", "70REPLACED"),
    ("6", "60PENDCXL"),
    ("7", "50STOPPED"),
    ("8", "95REJECTED"),
    ("9", "50SUSPEND"),
    ("A", "10PENDNEW"),
    ("B", "80CALCULAT"),
    ("C", "95EXPIRED"),
    ("D", "20ACCEPTED"),
    ("E", "60PENDRPL"),
    ("F", "40TRADE"),
    ("G", "40TRDCORR"),
    ("H", "40TRDCXL"),
    ("I", "30STATUS"),
    ("J", "40TRDHOLD"),
    ("K", "80TRDRELS"),
    ("L", "30TRIGGER"),
];

/// Every name that reaches a state, folded: FIX's, a scheduler's, and the
/// short names a FIX bridge logs - `PartFill`, `PendNew`, `DoneDay`,
/// `Cancel`, `Reject`.
static STATE_NAMES: &[(&str, &str)] = &[
    ("accepted", "20ACCEPTED"),
    ("acceptedforbidding", "20ACCEPTED"),
    ("calculated", "80CALCULAT"),
    ("cancel", "90CANCELED"),
    ("canceled", "90CANCELED"),
    ("cancelled", "90CANCELED"),
    ("complete", "80COMPLETE"),
    ("completed", "80COMPLETE"),
    ("doneday", "80DONEDAY"),
    ("doneforday", "80DONEDAY"),
    ("expired", "95EXPIRED"),
    ("failed", "95FAILED"),
    ("failure", "95FAILED"),
    ("filled", "80FILLED"),
    ("inprogress", "40INPROGR"),
    ("new", "20NEW"),
    ("orderstatus", "30STATUS"),
    ("partfill", "40PARTFILL"),
    ("partfilled", "40PARTFILL"),
    ("partiallyfilled", "40PARTFILL"),
    ("paused", "50PAUSED"),
    ("pendcancel", "60PENDCXL"),
    ("pending", "10PENDING"),
    ("pendingcancel", "60PENDCXL"),
    ("pendingnew", "10PENDNEW"),
    ("pendingreplace", "60PENDRPL"),
    ("pendnew", "10PENDNEW"),
    ("pendreplace", "60PENDRPL"),
    ("queued", "10QUEUED"),
    ("reject", "95REJECTED"),
    ("rejected", "95REJECTED"),
    ("replaced", "70REPLACED"),
    ("restated", "70RESTATED"),
    ("running", "30RUNNING"),
    ("starting", "20STARTING"),
    ("stopped", "50STOPPED"),
    ("submitted", "20SUBMITTD"),
    ("succeeded", "80SUCCESS"),
    ("success", "80SUCCESS"),
    ("suspended", "50SUSPEND"),
    ("timedout", "95TIMEOUT"),
    ("timeout", "95TIMEOUT"),
    ("trade", "40TRADE"),
    ("tradecancel", "40TRDCXL"),
    ("tradecorrect", "40TRDCORR"),
    ("tradehasbeenreleasedtoclearing", "80TRDRELS"),
    ("tradeinaclearinghold", "40TRDHOLD"),
    ("triggeredoractivatedbysystem", "30TRIGGER"),
    ("unknown", "00UNKNOWN"),
];

/// One registered code, whichever registry it is drawn from.
///
/// The identity leads: a currency and a country whose bytes agree are two
/// values, and they order by which code they are before they order by text.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[non_exhaustive]
pub enum Code {
    /// ISO 3166-1 alpha-2 country code.
    Country(Country),
    /// ISO 4217 currency code.
    Currency(Currency),
    /// ISO 10383 market identifier code.
    Mic(Mic),
    /// ISO 10962 classification code.
    Cfi(Cfi),
    /// ISO 6166 securities identification number.
    Isin(Isin),
    /// CUSIP securities identifier.
    Cusip(Cusip),
    /// SEDOL securities identifier.
    Sedol(Sedol),
    Bloomberg(Bloomberg),
    /// FIX's side of a trade.
    Side(Side),
    /// What state one thing is in, ranked so the bytes sort by lifecycle.
    State(State),
    /// How long an order stands.
    TimeInForce(TimeInForce),
}

const _: () = assert!(std::mem::size_of::<Code>() == 32);

impl Code {
    /// Borrow the validated text independently of the code's identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.storage().as_str()
    }

    /// The better statement of this code and another: [`CodeValue::merge_with`]
    /// where the two are one kind of code, and this one where they are not.
    #[must_use]
    pub fn merge_with(self, other: &Self) -> Self {
        match (self, other) {
            (Self::Country(this), Self::Country(that)) => Self::Country(this.merge_with(that)),
            (Self::Currency(this), Self::Currency(that)) => Self::Currency(this.merge_with(that)),
            (Self::Mic(this), Self::Mic(that)) => Self::Mic(this.merge_with(that)),
            (Self::Cfi(this), Self::Cfi(that)) => Self::Cfi(this.merge_with(that)),
            (Self::Isin(this), Self::Isin(that)) => Self::Isin(this.merge_with(that)),
            (Self::Cusip(this), Self::Cusip(that)) => Self::Cusip(this.merge_with(that)),
            (Self::Sedol(this), Self::Sedol(that)) => Self::Sedol(this.merge_with(that)),
            (Self::Bloomberg(this), Self::Bloomberg(that)) => {
                Self::Bloomberg(this.merge_with(that))
            }
            (Self::Side(this), Self::Side(that)) => Self::Side(this.merge_with(that)),
            (Self::State(this), Self::State(that)) => Self::State(this.merge_with(that)),
            (Self::TimeInForce(this), Self::TimeInForce(that)) => {
                Self::TimeInForce(this.merge_with(that))
            }
            (this, _) => this,
        }
    }

    /// Borrow the shared storage independently of the code's identity.
    ///
    /// Every member holds the same trimmed, validated text, so a string
    /// adopting a code's text clones this handle instead of the characters.
    #[must_use]
    pub const fn storage(&self) -> &SmolStr {
        match self {
            Self::Country(value) => value.storage(),
            Self::Currency(value) => value.storage(),
            Self::Mic(value) => value.storage(),
            Self::Cfi(value) => value.storage(),
            Self::Isin(value) => value.storage(),
            Self::Cusip(value) => value.storage(),
            Self::Sedol(value) => value.storage(),
            Self::Bloomberg(value) => value.storage(),
            Self::Side(value) => value.storage(),
            Self::State(value) => value.storage(),
            Self::TimeInForce(value) => value.storage(),
        }
    }

    /// The fixed storage width of this code, in bytes.
    #[must_use]
    pub const fn width(&self) -> usize {
        match self {
            Self::Country(_) => COUNTRY_WIDTH,
            Self::Currency(_) => CURRENCY_WIDTH,
            Self::Mic(_) => MIC_WIDTH,
            Self::Cfi(_) => CFI_WIDTH,
            Self::Isin(_) => ISIN_WIDTH,
            Self::Cusip(_) => CUSIP_WIDTH,
            Self::Sedol(_) => SEDOL_WIDTH,
            Self::Bloomberg(_) => BLOOMBERG_WIDTH,
            Self::Side(_) => SIDE_WIDTH,
            Self::State(_) => STATE_WIDTH,
            Self::TimeInForce(_) => TIMEINFORCE_WIDTH,
        }
    }

    /// The identifier this code carries.
    #[must_use]
    pub const fn identifier(&self) -> DataTypeId {
        match self {
            Self::Country(_) => DataTypeId::Country,
            Self::Currency(_) => DataTypeId::Currency,
            Self::Mic(_) => DataTypeId::Mic,
            Self::Cfi(_) => DataTypeId::Cfi,
            Self::Isin(_) => DataTypeId::Isin,
            Self::Cusip(_) => DataTypeId::Cusip,
            Self::Sedol(_) => DataTypeId::Sedol,
            Self::Bloomberg(_) => DataTypeId::Bloomberg,
            Self::Side(_) => DataTypeId::Side,
            Self::State(_) => DataTypeId::State,
            Self::TimeInForce(_) => DataTypeId::TimeInForce,
        }
    }

    /// The datatype this code is a value of.
    #[must_use]
    pub const fn datatype(&self) -> DataType {
        match self {
            Self::Country(_) => DataType::Country,
            Self::Currency(_) => DataType::Currency,
            Self::Mic(_) => DataType::Mic,
            Self::Cfi(_) => DataType::Cfi,
            Self::Isin(_) => DataType::Isin,
            Self::Cusip(_) => DataType::Cusip,
            Self::Sedol(_) => DataType::Sedol,
            Self::Bloomberg(_) => DataType::Bloomberg,
            Self::Side(_) => DataType::Side,
            Self::State(_) => DataType::State,
            Self::TimeInForce(_) => DataType::TimeInForce,
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

macro_rules! code_value {
    ($leaf:ident, $id:ident, $width:expr $(, merge = $merge:expr)?) => {
        impl Value for $leaf {

            fn dtype(&self) -> Result<DataType> {
                Ok(DataType::$id)
            }

            fn into_scalar(self) -> Scalar {
                Scalar::Code(Code::$leaf(self))
            }

            fn from_scalar(value: &Scalar) -> Option<&Self> {
                match value {
                    Scalar::Code(Code::$leaf(value)) => Some(value),
                    _ => None,
                }
            }
        }

        impl CodeValue for $leaf {
            const WIDTH: usize = $width;

            fn as_str(&self) -> &str {
                <$leaf>::as_str(self)
            }

            fn storage(&self) -> &SmolStr {
                <$leaf>::storage(self)
            }

            $(
                fn merge_with(self, other: &Self) -> Self {
                    $merge(self, other)
                }
            )?
        }

        impl From<$leaf> for Scalar {
            fn from(value: $leaf) -> Self {
                Self::Code(Code::$leaf(value))
            }
        }
    };
}

code_value!(Country, Country, COUNTRY_WIDTH);
code_value!(Currency, Currency, CURRENCY_WIDTH, merge = Currency::merged);
code_value!(Mic, Mic, MIC_WIDTH, merge = Mic::merged);
code_value!(Cfi, Cfi, CFI_WIDTH, merge = Cfi::filled);
code_value!(Bloomberg, Bloomberg, BLOOMBERG_WIDTH);
code_value!(Isin, Isin, ISIN_WIDTH);
code_value!(Cusip, Cusip, CUSIP_WIDTH);
code_value!(Sedol, Sedol, SEDOL_WIDTH);
code_value!(Side, Side, SIDE_WIDTH, merge = Side::merged);
code_value!(State, State, STATE_WIDTH, merge = State::merged);
code_value!(TimeInForce, TimeInForce, TIMEINFORCE_WIDTH);

impl From<Code> for Scalar {
    fn from(value: Code) -> Self {
        Self::Code(value)
    }
}
