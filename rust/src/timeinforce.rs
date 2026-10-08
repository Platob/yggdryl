//! How long an order stands: FIX's `TimeInForce(59)` code set as one enum,
//! stored as a `uint8`.

use crate::code::folded_spelling;
use crate::enums::enum_leaf;
use crate::typed::define_field_types;

enum_leaf! {
    /// How long an order stands: `DAY`, `GTC`, `IOC`, `FOK` and the rest of
    /// FIX's `TimeInForce(59)` code set, one member per wire value in wire
    /// order, beside `UKNW` for none stated and `OTHER` for a venue's own
    /// value no member names.
    ///
    /// [`Self::from_fix`] reads a wire value and [`Self::fix_code`] answers
    /// it back; a FIX registry maps any field's values onto members of its
    /// own choosing through `FIX:timeinforce`
    /// ([`FixRegistry::timeinforce_of`](crate::FixRegistry::timeinforce_of)).
    ///
    /// ```
    /// use yggdryl::TimeInForce;
    ///
    /// assert_eq!(TimeInForce::from_fix("1"), TimeInForce::GoodTillCancel);
    /// assert_eq!(TimeInForce::GoodTillCancel.as_str(), "GTC");
    /// assert_eq!(TimeInForce::GoodTillCancel.code(), 2);
    /// assert_eq!(TimeInForce::GoodTillCancel.fix_code(), Some("1"));
    /// assert_eq!(TimeInForce::from_spelling("day"), Some(TimeInForce::Day));
    /// assert_eq!(TimeInForce::from_spelling("ImmediateOrCancel"), Some(TimeInForce::ImmediateOrCancel));
    /// // A wire value is a spelling too: one field states them all.
    /// assert_eq!(TimeInForce::from_spelling("3"), Some(TimeInForce::ImmediateOrCancel));
    /// // A bridge's spelling reads as its member, a venue's own value as the
    /// // catch-all.
    /// assert_eq!(TimeInForce::from_fix("day"), TimeInForce::Day);
    /// assert_eq!(TimeInForce::from_fix("Z"), TimeInForce::Other);
    /// ```
    #[non_exhaustive]
    pub enum TimeInForce: u8, kind = "timeinforce", extension = TIMEINFORCE_EXTENSION_NAME, aliases = timeinforce_aliases,
    market = TIMEINFORCE_KIND [0xc5, 31, 56, 30] {
        #[default]
        Unknown = 0 as "UKNW": "No time in force stated.",
        Day = 1 as "DAY": "Good for the trading day.",
        GoodTillCancel = 2 as "GTC": "Good till canceled.",
        AtTheOpening = 3 as "OPG": "At the opening.",
        ImmediateOrCancel = 4 as "IOC": "Immediate or cancel: what does not fill at once is canceled.",
        FillOrKill = 5 as "FOK": "Fill or kill: filled whole at once, or canceled.",
        GoodTillCrossing = 6 as "GTX": "Good till crossing.",
        GoodTillDate = 7 as "GTD": "Good till a date.",
        AtTheClose = 8 as "ATC": "At the close.",
        GoodThroughCrossing = 9 as "GTHX": "Good through crossing.",
        AtCrossing = 10 as "ATX": "At crossing.",
        GoodForTime = 11 as "GFT": "Good for a time.",
        GoodForAuction = 12 as "GFA": "Good for the auction.",
        GoodForMonth = 13 as "GFM": "Good for the month.",
        Other = 99 as "OTHER": "A time in force no member names.",
    }
}

impl TimeInForce {
    /// The member one `TimeInForce(59)` value stands for: the wire value
    /// first, else any spelling [`Self::from_spelling`] reads - a bridge
    /// writing `day` where the standard writes `0` - and [`Self::Other`] for
    /// a value no member names, a venue's own.
    #[must_use]
    pub fn from_fix(wire: &str) -> Self {
        Self::from_spelling(wire).unwrap_or(Self::Other)
    }

    /// The `TimeInForce(59)` wire value this member stands for - `"1"` for
    /// `GTC` - or `None` for `UKNW` and `OTHER`, which stand for no one
    /// value.
    #[must_use]
    pub fn fix_code(self) -> Option<&'static str> {
        FIX_CODES
            .iter()
            .find(|(_, _, member)| *member == self)
            .map(|(code, _, _)| *code)
    }

    /// The member one spelling names, or `None` where none does.
    ///
    /// Three vocabularies reach one member: the stored name in any case -
    /// `GTC`, `gtc` - the FIX specification's own name for the value,
    /// folded the way every name in this crate folds - `GoodTillCancel`,
    /// `good_till_cancel` - and the `TimeInForce(59)` wire value itself,
    /// unfolded because `A` and `a` differ in FIX. One field states them
    /// all, so a wire value names one member; a stored code is an integer
    /// and never text, and [`Self::from_code`] reads it.
    #[must_use]
    pub fn from_spelling(spelling: &str) -> Option<Self> {
        if let Some(held) = Self::from_name(spelling) {
            return Some(held);
        }
        if let Some((_, _, member)) = FIX_CODES.iter().find(|(code, _, _)| *code == spelling) {
            return Some(*member);
        }
        let folded = folded_spelling(spelling);
        Self::ALL
            .iter()
            .copied()
            .find(|member| member.as_str().eq_ignore_ascii_case(folded.as_str()))
            .or_else(|| {
                FIX_CODES
                    .iter()
                    .find(|(_, name, _)| folded_spelling(name) == folded)
                    .map(|(_, _, member)| *member)
            })
            .or_else(|| Self::from_pattern(spelling))
    }
}

/// The standard's name for each member, which its spelling patterns read
/// the words of.
fn timeinforce_aliases() -> Vec<(&'static str, TimeInForce)> {
    FIX_CODES
        .iter()
        .map(|(_, name, member)| (*name, *member))
        .collect()
}

/// FIX's `TimeInForce(59)` code set: each wire value, the name the standard
/// gives it and the member it stands for, in wire order.
static FIX_CODES: &[(&str, &str, TimeInForce)] = &[
    ("0", "Day", TimeInForce::Day),
    ("1", "GoodTillCancel", TimeInForce::GoodTillCancel),
    ("2", "AtTheOpening", TimeInForce::AtTheOpening),
    ("3", "ImmediateOrCancel", TimeInForce::ImmediateOrCancel),
    ("4", "FillOrKill", TimeInForce::FillOrKill),
    ("5", "GoodTillCrossing", TimeInForce::GoodTillCrossing),
    ("6", "GoodTillDate", TimeInForce::GoodTillDate),
    ("7", "AtTheClose", TimeInForce::AtTheClose),
    ("8", "GoodThroughCrossing", TimeInForce::GoodThroughCrossing),
    ("9", "AtCrossing", TimeInForce::AtCrossing),
    ("A", "GoodForTime", TimeInForce::GoodForTime),
    ("B", "GoodForAuction", TimeInForce::GoodForAuction),
    ("C", "GoodForMonth", TimeInForce::GoodForMonth),
];

/// The Arrow extension name of how long an order stands, over `uint8`
/// storage.
pub(crate) const TIMEINFORCE_EXTENSION_NAME: &str = "yggdryl.timeinforce";

impl crate::DataType {
    /// Creates the time-in-force datatype: how long an order stands.
    ///
    /// ```
    /// use yggdryl::{DataType, TIMEINFORCE_KIND};
    ///
    /// assert_eq!(DataType::timeinforce(), TIMEINFORCE_KIND.dtype());
    /// assert_eq!(DataType::timeinforce().to_string(), "timeinforce");
    /// assert!(DataType::timeinforce().is_enum());
    /// ```
    #[must_use]
    pub const fn timeinforce() -> Self {
        Self::Market(crate::MarketType::new(&TIMEINFORCE_KIND))
    }
}

// /// A field declared as how long an order stands.
define_field_types!(
    TimeInForceType,
    TimeInForceField,
    market = TIMEINFORCE_KIND,
    TimeInForce
);
