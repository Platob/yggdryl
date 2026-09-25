//! FIX's side of a trade.

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::SmolStr;

use crate::code::folded_spelling;
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{DataType, Result, Scalar, Value};

/// FIX's `Side(54)`: which side of the market a trade took.
///
/// One byte. `Unknown` is zero and the seventeen sides the code set names
/// follow in FIX's own order, `1`..=`9` then `A`..=`H`, so a discriminant is
/// the position of its wire code. What a column stores and [`Self::as_str`]
/// answers is the crate's explicit spelling - `BUY`, `SSHORT`, `CROSSSHX` -
/// never the one-character code, which [`Self::fix_code`] answers on its own.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Side {
    /// `UNKNOWN`: a side stated as none, which takes no lane and which a
    /// merge takes the other side over.
    #[default]
    Unknown = 0,
    /// `1`, `BUY`.
    Buy = 1,
    /// `2`, `SELL`.
    Sell,
    /// `3`, `BUYMINUS`.
    BuyMinus,
    /// `4`, `SELLPLUS`.
    SellPlus,
    /// `5`, `SSHORT`: sell short.
    SShort,
    /// `6`, `SSHORTEX`: sell short exempt.
    SShortEx,
    /// `7`, `UNDISC`: undisclosed.
    Undisc,
    /// `8`, `CROSS`.
    Cross,
    /// `9`, `CROSSSH`: cross short.
    CrossSh,
    /// `A`, `CROSSSHX`: cross short exempt.
    CrossShX,
    /// `B`, `ASDEF`: as defined.
    AsDef,
    /// `C`, `OPPOSITE`.
    Opposite,
    /// `D`, `SUBSCR`: subscribe.
    Subscr,
    /// `E`, `REDEEM`.
    Redeem,
    /// `F`, `LEND`.
    Lend,
    /// `G`, `BORROW`.
    Borrow,
    /// `H`, `SELLUND`: sell undisclosed.
    SellUnd,
}

/// The stored spelling of every side, indexed by its discriminant.
///
/// Static rather than built per value, so [`Side::storage`] lends the shared
/// handle [`Scalar::code_storage`] and [`CodeValue::storage`] answer for every
/// other code without a side owning any text.
static SPELLINGS: [SmolStr; 18] = [
    SmolStr::new_static("UNKNOWN"),
    SmolStr::new_static("BUY"),
    SmolStr::new_static("SELL"),
    SmolStr::new_static("BUYMINUS"),
    SmolStr::new_static("SELLPLUS"),
    SmolStr::new_static("SSHORT"),
    SmolStr::new_static("SSHORTEX"),
    SmolStr::new_static("UNDISC"),
    SmolStr::new_static("CROSS"),
    SmolStr::new_static("CROSSSH"),
    SmolStr::new_static("CROSSSHX"),
    SmolStr::new_static("ASDEF"),
    SmolStr::new_static("OPPOSITE"),
    SmolStr::new_static("SUBSCR"),
    SmolStr::new_static("REDEEM"),
    SmolStr::new_static("LEND"),
    SmolStr::new_static("BORROW"),
    SmolStr::new_static("SELLUND"),
];

impl Side {
    /// Every side, indexed by its discriminant.
    const ALL: [Self; 18] = [
        Self::Unknown,
        Self::Buy,
        Self::Sell,
        Self::BuyMinus,
        Self::SellPlus,
        Self::SShort,
        Self::SShortEx,
        Self::Undisc,
        Self::Cross,
        Self::CrossSh,
        Self::CrossShX,
        Self::AsDef,
        Self::Opposite,
        Self::Subscr,
        Self::Redeem,
        Self::Lend,
        Self::Borrow,
        Self::SellUnd,
    ];

    /// The side one spelling names, refused where none does: [`Self::read`]
    /// over any text, which is what the value contract and a pickle hand it.
    ///
    /// # Errors
    ///
    /// Returns an error naming the spelling.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        Self::read(value.as_ref())
    }

    /// The stored spelling: `BUY`, `SSHORT`, `UNKNOWN`.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        SPELLINGS[*self as usize].as_str()
    }

    /// The shared handle of the stored spelling, lent without copying.
    #[must_use]
    pub const fn storage(&self) -> &'static SmolStr {
        &SPELLINGS[*self as usize]
    }

    /// The `Side(54)` wire character: `1`..=`9` then `A`..=`H`, in the order
    /// of the variants; `None` for a side stated as none, which no message
    /// carries.
    ///
    /// ```
    /// use yggdryl::Side;
    ///
    /// assert_eq!(Side::Buy.fix_code(), Some('1'));
    /// assert_eq!(Side::CrossShX.fix_code(), Some('A'));
    /// assert_eq!(Side::SellUnd.fix_code(), Some('H'));
    /// assert_eq!(Side::Unknown.fix_code(), None);
    /// ```
    #[must_use]
    pub const fn fix_code(&self) -> Option<char> {
        match *self as u8 {
            0 => None,
            code @ 1..=9 => Some((b'0' + code) as char),
            code => Some((b'A' + code - 10) as char),
        }
    }

    /// Whether this side takes the bid lane of a quote: a party willing to
    /// pay.
    ///
    /// Domain knowledge, written where a reviewer can check it: Orchestra
    /// does not publish which side takes which lane.
    ///
    /// ```
    /// use yggdryl::Side;
    ///
    /// assert!(Side::read("Buy").unwrap().is_bid());
    /// assert!(!Side::read("SellShort").unwrap().is_bid());
    /// assert!(!Side::read("Cross").unwrap().is_bid());
    /// ```
    #[must_use]
    pub const fn is_bid(&self) -> bool {
        matches!(self, Self::Buy | Self::BuyMinus)
    }

    /// Whether this side takes the ask lane of a quote: a party willing to be
    /// paid.
    ///
    /// Everything in neither lane - a cross, `UNDISC`, `ASDEF`, `OPPOSITE`, a
    /// side stated as none - takes no lane: a cross is both sides at once and
    /// `OPPOSITE` means "whatever the other leg was".
    ///
    /// ```
    /// use yggdryl::Side;
    ///
    /// assert!(Side::read("SellShortExempt").unwrap().is_ask());
    /// assert!(!Side::read("Buy").unwrap().is_ask());
    /// assert!(!Side::read("Opposite").unwrap().is_ask());
    /// ```
    #[must_use]
    pub const fn is_ask(&self) -> bool {
        matches!(
            self,
            Self::Sell | Self::SellPlus | Self::SShort | Self::SShortEx | Self::SellUnd
        )
    }

    /// The better of two sides: this one, unless it is `Unknown`.
    #[must_use]
    const fn merged(self, other: Self) -> Self {
        match self {
            Self::Unknown => other,
            stated => stated,
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
    /// - the stored value itself - `BUY`, `SSHORT`, `ASDEF`;
    /// - a FIX `Side(54)` wire code - `1`, `2`, `5`, `H`;
    /// - the specification's own name for it - `Buy`, `SellShort`, `AsDefined`.
    ///
    /// Names fold the way every other name in this crate folds: ASCII case
    /// insensitive, with `_`, `-` and spaces ignored, so `SellShort`,
    /// `sell_short` and `SELL SHORT` are one spelling. A wire code does
    /// **not** fold, because `A` and `a` are different codes in FIX.
    ///
    /// ```
    /// use yggdryl::Side;
    ///
    /// // The wire code, the specification's name and the stored value.
    /// assert_eq!(Side::from_spelling("1"), Some(Side::Buy));
    /// assert_eq!(Side::from_spelling("SellShort"), Some(Side::SShort));
    /// assert_eq!(Side::from_spelling("sshort"), Some(Side::SShort));
    /// assert_eq!(Side::from_spelling("H"), Some(Side::SellUnd));
    /// assert!(Side::from_spelling("X").is_none());
    /// ```
    #[must_use]
    pub fn from_spelling(spelling: &str) -> Option<Self> {
        if let Some(index) = SPELLINGS.iter().position(|held| held.as_str() == spelling) {
            return Some(Self::ALL[index]);
        }
        if let Some(side) = SIDE_CODES
            .iter()
            .find(|(code, _)| *code == spelling)
            .map(|(_, side)| *side)
        {
            return Some(side);
        }
        let folded = folded_spelling(spelling);
        SIDE_NAMES
            .iter()
            .find(|(name, _)| *name == folded.as_str())
            .map(|(_, side)| *side)
    }
}

/// FIX's `Side(54)` wire codes, unfolded, and the side each names.
static SIDE_CODES: &[(&str, Side)] = &[
    ("1", Side::Buy),
    ("2", Side::Sell),
    ("3", Side::BuyMinus),
    ("4", Side::SellPlus),
    ("5", Side::SShort),
    ("6", Side::SShortEx),
    ("7", Side::Undisc),
    ("8", Side::Cross),
    ("9", Side::CrossSh),
    ("A", Side::CrossShX),
    ("B", Side::AsDef),
    ("C", Side::Opposite),
    ("D", Side::Subscr),
    ("E", Side::Redeem),
    ("F", Side::Lend),
    ("G", Side::Borrow),
    ("H", Side::SellUnd),
];

/// Every name that reaches a side, folded: the specification's, and the
/// stored values in any case.
static SIDE_NAMES: &[(&str, Side)] = &[
    ("asdef", Side::AsDef),
    ("asdefined", Side::AsDef),
    ("borrow", Side::Borrow),
    ("buy", Side::Buy),
    ("buyminus", Side::BuyMinus),
    ("cross", Side::Cross),
    ("crossshort", Side::CrossSh),
    ("crossshortexempt", Side::CrossShX),
    ("crosssh", Side::CrossSh),
    ("crossshx", Side::CrossShX),
    ("lend", Side::Lend),
    ("opposite", Side::Opposite),
    ("redeem", Side::Redeem),
    ("sell", Side::Sell),
    ("sellplus", Side::SellPlus),
    ("sellshort", Side::SShort),
    ("sellshortexempt", Side::SShortEx),
    ("sellund", Side::SellUnd),
    ("sellundisclosed", Side::SellUnd),
    ("sshort", Side::SShort),
    ("sshortex", Side::SShortEx),
    ("subscr", Side::Subscr),
    ("subscribe", Side::Subscr),
    ("undisc", Side::Undisc),
    ("undisclosed", Side::Undisc),
    ("unknown", Side::Unknown),
];

impl fmt::Display for Side {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for Side {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Side {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        // Owned where the deserializer cannot lend, as a parsed JSON value
        // cannot, so a document reads through every door.
        let value = <Cow<'_, str>>::deserialize(deserializer)?;
        Self::read(&value).map_err(serde::de::Error::custom)
    }
}

impl Value for Side {
    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::Side)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Side(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Side(value) => Some(value),
            _ => None,
        }
    }
}

impl CodeValue for Side {
    const WIDTH: usize = SIDE_WIDTH;

    fn as_str(&self) -> &str {
        Side::as_str(self)
    }

    fn storage(&self) -> &SmolStr {
        Side::storage(self)
    }

    fn merge_with(self, other: &Self) -> Self {
        self.merged(*other)
    }
}

impl From<Side> for Scalar {
    fn from(value: Side) -> Self {
        Self::Side(value)
    }
}

/// The Arrow extension name of FIX's side of a trade.
pub(crate) const SIDE_EXTENSION_NAME: &str = "yggdryl.side";

/// The most bytes a side of the market may be.
///
/// The stored values are the crate's own explicit spellings - `BUY`, `SELL`,
/// `SSHORTEX`, `CROSSSHX` - rather than FIX's one-character codes, and the
/// longest of them is eight. Fixed here because widening later would change
/// a discriminant, which is a wire contract.
pub(crate) const SIDE_WIDTH: usize = 8;

// /// A side-typed field: FIX's side of a trade.
define_field_types!(SideType, Side);
