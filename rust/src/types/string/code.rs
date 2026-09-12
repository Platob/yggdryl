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
    CFI_WIDTH, COUNTRY_WIDTH, CURRENCY_WIDTH, ISIN_WIDTH, MIC_WIDTH, SIDE_WIDTH, STATE_WIDTH,
    TIMEINFORCE_WIDTH,
};
use crate::{DataType, DataTypeId, DataTypeKind, Result, Scalar, ScalarFamily, ScalarValue, types};

/// Borrowing access shared by every code representation.
pub trait CodeValue: crate::ScalarValue {
    /// The fixed storage width, in bytes.
    const WIDTH: usize;

    /// Borrow the validated code.
    fn as_str(&self) -> &str;
    /// Borrow the shared storage behind the validated code.
    ///
    /// The stored text is already trimmed and checked, so a rewrite that
    /// keeps it clones this handle rather than re-validating and copying.
    fn storage(&self) -> &SmolStr;
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

code_leaf!(Side, SIDE_WIDTH);
code_leaf!(State, STATE_WIDTH);
code_leaf!(TimeInForce, TIMEINFORCE_WIDTH);

impl State {
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
    ($leaf:ident, $id:ident, $width:expr) => {
        impl ScalarValue for $leaf {
            type Family = Code;

            const ID: DataTypeId = DataTypeId::$id;
            const KIND: DataTypeKind = DataTypeKind::Code;

            fn dtype(&self) -> Result<DataType> {
                Ok(DataType::$id)
            }

            fn into_family(self) -> Self::Family {
                Code::$leaf(self)
            }

            fn from_family(family: &Self::Family) -> Option<&Self> {
                match family {
                    Code::$leaf(value) => Some(value),
                    _ => None,
                }
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
        }

        impl From<$leaf> for Scalar {
            fn from(value: $leaf) -> Self {
                Self::Code(Code::$leaf(value))
            }
        }
    };
}

code_value!(Country, Country, COUNTRY_WIDTH);
code_value!(Currency, Currency, CURRENCY_WIDTH);
code_value!(Mic, Mic, MIC_WIDTH);
code_value!(Cfi, Cfi, CFI_WIDTH);
code_value!(Isin, Isin, ISIN_WIDTH);
code_value!(Side, Side, SIDE_WIDTH);
code_value!(State, State, STATE_WIDTH);
code_value!(TimeInForce, TimeInForce, TIMEINFORCE_WIDTH);

impl ScalarFamily for Code {
    const KIND: DataTypeKind = DataTypeKind::Code;

    fn id(&self) -> DataTypeId {
        self.identifier()
    }

    fn dtype(&self) -> Result<DataType> {
        Ok(self.datatype())
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Code(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Code(value) => Some(value),
            _ => None,
        }
    }
}

impl From<Code> for Scalar {
    fn from(value: Code) -> Self {
        Self::Code(value)
    }
}
