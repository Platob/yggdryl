//! How long an order stands.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::code::folded_spelling;
use crate::code::{code_leaf, code_value};
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{DataType, Result, Scalar, Value};

code_leaf!(TimeInForce, TIMEINFORCE_WIDTH);

impl TimeInForce {
    /// The time in force one spelling names, refused where none does.
    ///
    /// [`Self::from_spelling`] as a refusal: the only spelling that names no
    /// time in force is one no stored value can hold, past the eight ASCII
    /// bytes `TimeInForce(59)` is fixed at.
    ///
    /// # Errors
    ///
    /// Returns an error naming the field and the spelling.
    pub fn read(spelling: &str) -> Result<Self> {
        Self::from_spelling(spelling).ok_or_else(|| crate::Error::InvalidDataType {
            kind: "timeinforce",
            reason: smol_str::format_smolstr!(
                "TimeInForce(59) holds a code, a name or a stored value of at most \
                 {TIMEINFORCE_WIDTH} ASCII bytes, got {spelling:?}"
            ),
        })
    }

    /// The time in force one spelling names, or `None` where none does.
    ///
    /// Three vocabularies reach one value, and the value stored is the wire
    /// code, exactly as a column holds it:
    ///
    /// - a FIX `TimeInForce(59)` wire code - `0`, `1`, `A` - unfolded,
    ///   because `A` and `a` are different codes in FIX;
    /// - the name the shipped code set gives it - `Day`, `GoodTillCancel` -
    ///   folded the way every name in this crate folds, ASCII case
    ///   insensitive with `_`, `-` and spaces ignored;
    /// - any other stored value of at most eight ASCII bytes, kept as stated:
    ///   the code set is a vocabulary and never a gate, so a venue's own
    ///   `GTX` is held rather than refused.
    ///
    /// ```
    /// use yggdryl::TimeInForce;
    ///
    /// assert_eq!(TimeInForce::from_spelling("day").unwrap().as_str(), "0");
    /// assert_eq!(TimeInForce::from_spelling("GoodTillCancel").unwrap().as_str(), "1");
    /// assert_eq!(TimeInForce::from_spelling("1").unwrap().as_str(), "1");
    /// assert_eq!(TimeInForce::from_spelling("GTX").unwrap().as_str(), "GTX");
    /// assert!(TimeInForce::from_spelling("TOOLONGTIF").is_none());
    /// ```
    #[must_use]
    pub fn from_spelling(spelling: &str) -> Option<Self> {
        if let Some((code, _)) = TIMEINFORCE_CODES.iter().find(|(code, _)| *code == spelling) {
            return Some(Self(SmolStr::new_static(code)));
        }
        let folded = folded_spelling(spelling);
        if let Some((code, _)) = TIMEINFORCE_CODES
            .iter()
            .find(|(_, name)| folded_spelling(name) == folded)
        {
            return Some(Self(SmolStr::new_static(code)));
        }
        Self::new(spelling).ok()
    }
}

/// FIX's `TimeInForce(59)` code set as shipped under
/// `config/fix/codesets/timeinforcecodeset.json`: each wire code and the name
/// the standard gives it, in code order.
pub const TIMEINFORCE_CODES: [(&str, &str); 13] = [
    ("0", "Day"),
    ("1", "GoodTillCancel"),
    ("2", "AtTheOpening"),
    ("3", "ImmediateOrCancel"),
    ("4", "FillOrKill"),
    ("5", "GoodTillCrossing"),
    ("6", "GoodTillDate"),
    ("7", "AtTheClose"),
    ("8", "GoodThroughCrossing"),
    ("9", "AtCrossing"),
    ("A", "GoodForTime"),
    ("B", "GoodForAuction"),
    ("C", "GoodForMonth"),
];

code_value!(TimeInForce, TimeInForce, TIMEINFORCE_WIDTH);

/// The Arrow extension name of how long an order stands.
pub(crate) const TIMEINFORCE_EXTENSION_NAME: &str = "yggdryl.timeinforce";

/// The most bytes how long an order stands may be.
///
/// The standard's values are one character; eight leaves room for venue codes.
pub(crate) const TIMEINFORCE_WIDTH: usize = 8;

// /// A field declared as how long an order stands.
define_field_types!(TimeInForceType, TimeInForce);
