//! What kind of market data an element is: FIX's MsgCat code set as a
//! lifecycle enum, stored as an `int32`.

use crate::code::folded_spelling;
use crate::enums::enum_leaf;
use crate::typed::define_field_types;

enum_leaf! {
    /// The business category of a market data element: the FIX MsgCat code
    /// set, one member per category the standard files its message types
    /// under, and `UNKN` for a type it files under none.
    ///
    /// This is the one owner of that set. A FIX dictionary's `FIX:msgcat`
    /// resolves to a member through [`Self::from_name`], the crate's
    /// `msgcatcodeset` renders from [`Self::ALL`], and a market data row
    /// states its leaf's category as one of these - an order `ORDR`, a quote
    /// `QUOT`, an execution `EXEC`, a trade `TRAD`, a book `BOOK` - so a
    /// reader tells the leaves apart by one column every FIX engine already
    /// speaks.
    ///
    /// The code is the set's own value, `UNKN` at zero and `TRAD` at
    /// twenty-one, and what a column stores; the four-letter code is the
    /// stored name.
    ///
    /// ```
    /// use yggdryl::MarketDataKind;
    ///
    /// assert_eq!(MarketDataKind::from_spelling("ORDR"), Some(MarketDataKind::Order));
    /// assert_eq!(MarketDataKind::from_spelling("quotation"), Some(MarketDataKind::Quotation));
    /// assert_eq!(MarketDataKind::Trade.code(), 21);
    /// assert_eq!(MarketDataKind::from_code(3), Some(MarketDataKind::Book));
    /// assert_eq!(MarketDataKind::Execution.as_str(), "EXEC");
    /// // A stored code is an integer, never text.
    /// assert_eq!(MarketDataKind::from_spelling("10"), None);
    /// ```
    #[non_exhaustive]
    pub enum MarketDataKind: i32, kind = "marketdatakind", extension = MARKETDATAKIND_EXTENSION_NAME {
        #[default]
        Unknown = 0 as "UNKN": "No published category: a message type the dictionary does not file.",
        Account = 1 as "ACCT": "Account reporting.",
        Allocation = 2 as "ALLO": "Allocation instructions, reports and acknowledgements.",
        Book = 3 as "BOOK": "Market data: books, their snapshots, increments and requests.",
        Certificate = 4 as "CERT": "Certificate handling.",
        Collateral = 5 as "COLL": "Collateral management.",
        Communication = 6 as "COMM": "Communication: news and email.",
        Confirmation = 7 as "CONF": "Confirmation and affirmation.",
        Execution = 8 as "EXEC": "Execution reports and their acknowledgements.",
        MarketStructure = 9 as "MKST": "Market structure reference data.",
        Order = 10 as "ORDR": "Order handling: single, list, cross, multileg and mass.",
        Payment = 11 as "PAYM": "Pay management.",
        Position = 12 as "POSN": "Position maintenance.",
        Parties = 13 as "PRTY": "Parties reference data and risk limits reporting.",
        Quotation = 14 as "QUOT": "Quotation and negotiation.",
        Registration = 15 as "REGI": "Registration instructions.",
        Risk = 16 as "RISK": "Party risk limits.",
        Securities = 17 as "SECU": "Securities reference data.",
        Session = 18 as "SESS": "Session, application sequencing and business rejects.",
        Settlement = 19 as "SETL": "Settlement instructions and obligations.",
        Stream = 20 as "STRM": "Stream assignment.",
        Trade = 21 as "TRAD": "Trade capture and matching.",
    }
}

impl MarketDataKind {
    /// The kind one spelling names, or `None` where none does.
    ///
    /// Two vocabularies reach one value: the four-letter code in any case -
    /// `ORDR`, `ordr` - and the member's own word folded the way every name
    /// in this crate folds, ASCII case insensitive with `_`, `-` and spaces
    /// ignored - `order`, `Quotation`, `market_structure`. A stored code is
    /// an integer, never text: [`Self::from_code`] reads it.
    #[must_use]
    pub fn from_spelling(spelling: &str) -> Option<Self> {
        if let Some(held) = Self::from_name(spelling) {
            return Some(held);
        }
        let folded = folded_spelling(spelling);
        Self::ALL.iter().copied().find(|held| {
            held.as_str().eq_ignore_ascii_case(folded.as_str())
                || held.word().eq_ignore_ascii_case(folded.as_str())
        })
    }

    /// The member's own word, folded: what [`Self::from_spelling`] reads
    /// beside the four-letter code.
    const fn word(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Account => "account",
            Self::Allocation => "allocation",
            Self::Book => "book",
            Self::Certificate => "certificate",
            Self::Collateral => "collateral",
            Self::Communication => "communication",
            Self::Confirmation => "confirmation",
            Self::Execution => "execution",
            Self::MarketStructure => "marketstructure",
            Self::Order => "order",
            Self::Payment => "payment",
            Self::Position => "position",
            Self::Parties => "parties",
            Self::Quotation => "quotation",
            Self::Registration => "registration",
            Self::Risk => "risk",
            Self::Securities => "securities",
            Self::Session => "session",
            Self::Settlement => "settlement",
            Self::Stream => "stream",
            Self::Trade => "trade",
        }
    }
}

/// The Arrow extension name of a market data element's kind, over `int32`
/// storage.
pub(crate) const MARKETDATAKIND_EXTENSION_NAME: &str = "yggdryl.marketdatakind";

impl crate::DataType {
    /// Creates the market data kind datatype: FIX's MsgCat code set.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::marketdatakind(), DataType::MarketDataKind);
    /// assert_eq!(DataType::marketdatakind().to_string(), "marketdatakind");
    /// assert!(DataType::marketdatakind().is_enum());
    /// ```
    #[must_use]
    pub const fn marketdatakind() -> Self {
        Self::MarketDataKind
    }
}

// /// A field declared as a market data element's kind.
define_field_types!(MarketDataKindType, MarketDataKind);
