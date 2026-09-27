//! FIX's side of a trade: an enum stored as the `int32` code of its member.

use crate::code::folded_spelling;
use crate::enums::enum_leaf;
use crate::typed::define_field_types;

enum_leaf! {
    /// FIX's `Side(54)`: which side of the market a trade took.
    ///
    /// One byte in memory, an `int32` code in a column. `Unknown` is zero and
    /// the seventeen sides the code set names follow in FIX's own order,
    /// `1`..=`9` then `A`..=`H`, so a code is the position of its wire
    /// character. What [`Self::as_str`] answers and every text format writes
    /// is the crate's explicit spelling - `BUY`, `SSHORT`, `CROSSSHX` -
    /// never the one-character code, which [`Self::fix_code`] answers on its
    /// own; what a column stores is the code, which every engine reads.
    ///
    /// ```
    /// use yggdryl::Side;
    ///
    /// assert_eq!(Side::from_spelling("1"), Some(Side::Buy));
    /// assert_eq!(Side::Buy.code(), 1);
    /// assert_eq!(Side::from_code(2), Some(Side::Sell));
    /// assert_eq!(Side::SShort.as_str(), "SSHORT");
    /// assert_eq!(Side::Unknown.merge_with(Side::Sell), Side::Sell);
    /// ```
    pub enum Side: u8, kind = "side", extension = SIDE_EXTENSION_NAME {
        #[default]
        Unknown = 0 as "UNKNOWN": "A side stated as none, which a merge takes the other side over.",
        Buy = 1 as "BUY": "Buy.",
        Sell = 2 as "SELL": "Sell.",
        BuyMinus = 3 as "BUYMINUS": "Buy minus.",
        SellPlus = 4 as "SELLPLUS": "Sell plus.",
        SShort = 5 as "SSHORT": "Sell short.",
        SShortEx = 6 as "SSHORTEX": "Sell short exempt.",
        Undisc = 7 as "UNDISC": "Undisclosed.",
        Cross = 8 as "CROSS": "Cross.",
        CrossSh = 9 as "CROSSSH": "Cross short.",
        CrossShX = 10 as "CROSSSHX": "Cross short exempt.",
        AsDef = 11 as "ASDEF": "As defined.",
        Opposite = 12 as "OPPOSITE": "Opposite.",
        Subscr = 13 as "SUBSCR": "Subscribe.",
        Redeem = 14 as "REDEEM": "Redeem.",
        Lend = 15 as "LEND": "Lend.",
        Borrow = 16 as "BORROW": "Borrow.",
        SellUnd = 17 as "SELLUND": "Sell undisclosed.",
    }
}

impl Side {
    /// The side one spelling names, refused where none does: [`Self::read`]
    /// over any text, which is what the value contract and a pickle hand it.
    ///
    /// # Errors
    ///
    /// Returns an error naming the spelling.
    pub fn new(value: impl AsRef<str>) -> crate::Result<Self> {
        Self::read(value.as_ref())
    }

    /// The `Side(54)` wire character: `1`..=`9` then `A`..=`H`, in the order
    /// of the members; `None` for a side stated as none, which no message
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
    pub const fn fix_code(self) -> Option<char> {
        match self as u8 {
            0 => None,
            code @ 1..=9 => Some((b'0' + code) as char),
            code => Some((b'A' + code - 10) as char),
        }
    }

    /// Whether this side takes the bid of a quote or a book: a party
    /// willing to pay.
    ///
    /// Domain knowledge, written where a reviewer can check it: Orchestra
    /// does not publish which side takes the bid and which the ask.
    ///
    /// ```
    /// use yggdryl::Side;
    ///
    /// assert!(Side::read("Buy").unwrap().is_bid());
    /// assert!(!Side::read("SellShort").unwrap().is_bid());
    /// assert!(!Side::read("Cross").unwrap().is_bid());
    /// ```
    #[must_use]
    pub const fn is_bid(self) -> bool {
        matches!(self, Self::Buy | Self::BuyMinus)
    }

    /// Whether this side takes the ask of a quote or a book: a party willing
    /// to be paid.
    ///
    /// Everything on neither side - a cross, `UNDISC`, `ASDEF`, `OPPOSITE`, a
    /// side stated as none - takes neither: a cross is both sides at once and
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
    pub const fn is_ask(self) -> bool {
        matches!(
            self,
            Self::Sell | Self::SellPlus | Self::SShort | Self::SShortEx | Self::SellUnd
        )
    }

    /// The better of two sides: this one, unless it is `Unknown`.
    ///
    /// What a graph element folds two statements of one side with: a side
    /// stated as none takes the other, and anything stated stands.
    #[must_use]
    pub const fn merge_with(self, other: Self) -> Self {
        match self {
            Self::Unknown => other,
            stated => stated,
        }
    }

    /// The side one spelling names, or `None` where none does.
    ///
    /// Three vocabularies reach one value, because they name one thing:
    ///
    /// - the stored name itself - `BUY`, `SSHORT`, `ASDEF`;
    /// - a FIX `Side(54)` wire code - `1`, `2`, `5`, `H`;
    /// - the specification's own name for it - `Buy`, `SellShort`, `AsDefined`.
    ///
    /// Names fold the way every other name in this crate folds: ASCII case
    /// insensitive, with `_`, `-` and spaces ignored, so `SellShort`,
    /// `sell_short` and `SELL SHORT` are one spelling. A wire code does
    /// **not** fold, because `A` and `a` are different codes in FIX. A stored
    /// code is an integer, never text: [`Self::from_code`] reads it.
    ///
    /// ```
    /// use yggdryl::Side;
    ///
    /// // The wire code, the specification's name and the stored name.
    /// assert_eq!(Side::from_spelling("1"), Some(Side::Buy));
    /// assert_eq!(Side::from_spelling("SellShort"), Some(Side::SShort));
    /// assert_eq!(Side::from_spelling("sshort"), Some(Side::SShort));
    /// assert_eq!(Side::from_spelling("H"), Some(Side::SellUnd));
    /// assert!(Side::from_spelling("X").is_none());
    /// ```
    #[must_use]
    pub fn from_spelling(spelling: &str) -> Option<Self> {
        if let Some(held) = Self::from_name(spelling) {
            return Some(held);
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
/// stored names in any case.
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

/// The Arrow extension name of FIX's side of a trade, over `int32` storage.
pub(crate) const SIDE_EXTENSION_NAME: &str = "yggdryl.side";

// /// A side-typed field: FIX's side of a trade.
define_field_types!(SideType, Side);
