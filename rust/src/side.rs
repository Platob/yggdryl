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
    /// is the member's fixed four-letter code - `BUYS`, `SSHT`, `CRSX` - the
    /// prefix a sided cross code is stored under, never the one-character
    /// wire code, which [`Self::fix_code`] answers on its own; the long name
    /// is the member's description. What a column stores is the integer
    /// code, which every engine reads.
    ///
    /// ```
    /// use yggdryl::Side;
    ///
    /// assert_eq!(Side::from_spelling("1"), Some(Side::Buy));
    /// assert_eq!(Side::Buy.code(), 1);
    /// assert_eq!(Side::from_code(2), Some(Side::Sell));
    /// assert_eq!(Side::SShort.as_str(), "SSHT");
    /// assert_eq!(Side::SShort.description(), "Sell short.");
    /// assert_eq!(Side::Unknown.merge_with(Side::Sell), Side::Sell);
    /// ```
    pub enum Side: u8, kind = "side", extension = SIDE_EXTENSION_NAME {
        #[default]
        Unknown = 0 as "UNKN": "A side stated as none, which a merge takes the other side over.",
        Buy = 1 as "BUYS": "Buy.",
        Sell = 2 as "SELL": "Sell.",
        BuyMinus = 3 as "BUYM": "Buy minus.",
        SellPlus = 4 as "SELP": "Sell plus.",
        SShort = 5 as "SSHT": "Sell short.",
        SShortEx = 6 as "SSEX": "Sell short exempt.",
        Undisc = 7 as "UNDI": "Undisclosed.",
        Cross = 8 as "CROS": "Cross.",
        CrossSh = 9 as "CRSH": "Cross short.",
        CrossShX = 10 as "CRSX": "Cross short exempt.",
        AsDef = 11 as "ASDF": "As defined.",
        Opposite = 12 as "OPPO": "Opposite.",
        Subscr = 13 as "SUBS": "Subscribe.",
        Redeem = 14 as "REDM": "Redeem.",
        Lend = 15 as "LEND": "Lend.",
        Borrow = 16 as "BORR": "Borrow.",
        SellUnd = 17 as "SELU": "Sell undisclosed.",
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
    /// Everything on neither side - a cross, `UNDI`, `ASDF`, `OPPO`, a side
    /// stated as none - takes neither: a cross is both sides at once and
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
    /// - the stored four-letter code itself - `BUYS`, `SSHT`, `ASDF` - and
    ///   the names the members were stored under before it, `BUY`,
    ///   `SSHORT`, `ASDEF`, read and never written;
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
    /// // The wire code, the specification's name and the stored code.
    /// assert_eq!(Side::from_spelling("1"), Some(Side::Buy));
    /// assert_eq!(Side::from_spelling("SellShort"), Some(Side::SShort));
    /// assert_eq!(Side::from_spelling("SSHT"), Some(Side::SShort));
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

/// Every name that reaches a side, folded: the four-letter codes, the
/// specification's names, and the names the members were stored under
/// before their codes, read and never written.
static SIDE_NAMES: &[(&str, Side)] = &[
    ("asdef", Side::AsDef),
    ("asdefined", Side::AsDef),
    ("asdf", Side::AsDef),
    ("borr", Side::Borrow),
    ("borrow", Side::Borrow),
    ("buy", Side::Buy),
    ("buym", Side::BuyMinus),
    ("buyminus", Side::BuyMinus),
    ("buys", Side::Buy),
    ("cros", Side::Cross),
    ("cross", Side::Cross),
    ("crossshort", Side::CrossSh),
    ("crossshortexempt", Side::CrossShX),
    ("crosssh", Side::CrossSh),
    ("crossshx", Side::CrossShX),
    ("crsh", Side::CrossSh),
    ("crsx", Side::CrossShX),
    ("lend", Side::Lend),
    ("oppo", Side::Opposite),
    ("opposite", Side::Opposite),
    ("redeem", Side::Redeem),
    ("redm", Side::Redeem),
    ("selp", Side::SellPlus),
    ("sell", Side::Sell),
    ("sellplus", Side::SellPlus),
    ("sellshort", Side::SShort),
    ("sellshortexempt", Side::SShortEx),
    ("sellund", Side::SellUnd),
    ("sellundisclosed", Side::SellUnd),
    ("selu", Side::SellUnd),
    ("ssex", Side::SShortEx),
    ("ssht", Side::SShort),
    ("sshort", Side::SShort),
    ("sshortex", Side::SShortEx),
    ("subs", Side::Subscr),
    ("subscr", Side::Subscr),
    ("subscribe", Side::Subscr),
    ("undi", Side::Undisc),
    ("undisc", Side::Undisc),
    ("undisclosed", Side::Undisc),
    ("unkn", Side::Unknown),
    ("unknown", Side::Unknown),
];

/// The Arrow extension name of FIX's side of a trade, over `int32` storage.
pub(crate) const SIDE_EXTENSION_NAME: &str = "yggdryl.side";

// /// A side-typed field: FIX's side of a trade.
define_field_types!(SideType, Side);
