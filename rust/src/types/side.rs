//! FIX's side of a trade.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::types;
use crate::types::code::folded_spelling;
use crate::types::code::{CodeValue, code_leaf, code_value};
use crate::types::typed::define_field_types;
use crate::{DataType, Result, Scalar, Value};

code_leaf!(Side, SIDE_WIDTH);

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

code_value!(Side, Side, SIDE_WIDTH, merge = Side::merged);

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
