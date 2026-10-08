//! What kind of market data an element is: FIX's MsgCat code set as a
//! lifecycle enum, stored as a `uint8`.

use crate::Side;
use crate::code::folded_spelling;
use crate::enums::enum_leaf;
use crate::typed::define_field_types;

enum_leaf! {
    /// The business category of a market data element: the FIX MsgCat code
    /// set, one member per category the standard files its message types
    /// under, and `UKNW` for a type it files under none.
    ///
    /// This is the one owner of that set. A FIX dictionary's `FIX:msgcat`
    /// resolves to a member through [`Self::from_name`], the crate's
    /// `marketdatakindcodeset` renders from [`Self::ALL`], and a market data row
    /// states its leaf's category as one of these - an order `ORDR`, a quote
    /// `QUOT`, an execution `EXEC`, a trade `TRAD`, a book `BOOK` - so a
    /// reader tells the leaves apart by one column every FIX engine already
    /// speaks.
    ///
    /// Four batch categories follow the standard's: `ORDB`, `QUOB`, `EXEB`
    /// and `TRDB` file a message that states many orders, quotes,
    /// executions or trades at once - a list, a mass order, a cross, a mass
    /// quote, a match report - which a parse splits into one message of the
    /// single category per entry ([`Self::is_batch`]).
    ///
    /// The code is the set's own value, `UKNW` at zero, `TRAD` at twenty-one
    /// and the batches after it, and what a column stores; the four-letter
    /// code is the stored name.
    ///
    /// ```
    /// use yggdryl::MarketDataKind;
    ///
    /// assert_eq!(MarketDataKind::from_spelling("ORDR"), Some(MarketDataKind::Order));
    /// assert_eq!(MarketDataKind::from_spelling("quotation"), Some(MarketDataKind::Quotation));
    /// assert_eq!(MarketDataKind::Trade.code(), 21);
    /// assert_eq!(MarketDataKind::from_code(3), Some(MarketDataKind::Book));
    /// assert_eq!(MarketDataKind::Execution.as_str(), "EXEC");
    /// assert_eq!(MarketDataKind::from_spelling("order_batch"), Some(MarketDataKind::OrderBatch));
    /// assert_eq!(MarketDataKind::OrderBatch.as_str(), "ORDB");
    /// // A stored code is an integer, never text.
    /// assert_eq!(MarketDataKind::from_spelling("10"), None);
    /// ```
    #[non_exhaustive]
    pub enum MarketDataKind: u8, kind = "marketdatakind", extension = MARKETDATAKIND_EXTENSION_NAME, aliases = marketdatakind_aliases,
    market = MARKETDATAKIND_KIND [0xc2, 28, 73, 64] {
        #[default]
        Unknown = 0 as "UKNW": "No published category: a message type the dictionary does not file.",
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
        OrderBatch = 22 as "ORDB": "Order batches: lists, mass order handling and crosses, one order per entry.",
        QuoteBatch = 23 as "QUOB": "Quote batches: mass quotes and bid lists, one quote per entry.",
        ExecutionBatch = 24 as "EXEB": "Execution batches: several executions reported at once, one per entry.",
        TradeBatch = 25 as "TRDB": "Trade batches: match reports stating several trades, one per entry.",
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
        Self::ALL
            .iter()
            .copied()
            .find(|held| {
                held.as_str().eq_ignore_ascii_case(folded.as_str())
                    || held.word().eq_ignore_ascii_case(folded.as_str())
            })
            .or_else(|| Self::from_pattern(spelling))
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
            Self::OrderBatch => "orderbatch",
            Self::QuoteBatch => "quotebatch",
            Self::ExecutionBatch => "executionbatch",
            Self::TradeBatch => "tradebatch",
        }
    }

    /// Whether an element of this kind is sided: an order or an execution,
    /// which takes one side of the market, so its cross code is stored under
    /// that side's code - `10:1:ORD-1` - and the two sides of one identifier
    /// are two chains. The one owner of that rule: every other kind - a
    /// quote, which holds its bid and its ask in one element and states its
    /// `side` only as a tag, a trade, a book, a batch, a category the
    /// standard files no operation under - stores its cross code under side
    /// `0` whatever side it states
    /// ([`Market::stored_crosscode`](crate::graph::Market::stored_crosscode)).
    ///
    /// ```
    /// use yggdryl::MarketDataKind;
    ///
    /// assert!(MarketDataKind::Order.is_sided());
    /// assert!(MarketDataKind::Execution.is_sided());
    /// assert!(!MarketDataKind::Quotation.is_sided());
    /// assert!(!MarketDataKind::Trade.is_sided());
    /// assert!(!MarketDataKind::Book.is_sided());
    /// assert!(!MarketDataKind::OrderBatch.is_sided());
    /// assert!(!MarketDataKind::Unknown.is_sided());
    /// ```
    #[must_use]
    pub const fn is_sided(self) -> bool {
        matches!(self, Self::Order | Self::Execution)
    }

    /// The side a cross code and a chain of this kind are keyed by: the
    /// stated `side` of a sided kind ([`Self::is_sided`]), and
    /// [`Side::Unknown`] for every other kind, whatever side it states -
    /// `BOTH` included. The one owner of that reading, so the stored code,
    /// the walk's chain key and a session's key never disagree on it.
    #[must_use]
    pub(crate) const fn stored_side(self, side: Side) -> Side {
        if self.is_sided() { side } else { Side::Unknown }
    }

    /// Whether an element of this kind folds into a book's sides: an order
    /// or a quote, which rests on a side, and a book, whose snapshot
    /// replaces the depth of its instant. The one owner of that rule. An
    /// execution folds into no side - its fill moved the book through its
    /// order's or quote's own report - and is recorded among the book's
    /// events instead ([`Self::is_recorded`]); every other kind - a trade,
    /// whose fills are executions, a batch, whose items arrive as their own
    /// kinds, and every category the standard files no resting interest
    /// under - is pruned before a book walk routes it.
    ///
    /// ```
    /// use yggdryl::MarketDataKind;
    ///
    /// assert!(MarketDataKind::Order.is_booked());
    /// assert!(MarketDataKind::Quotation.is_booked());
    /// assert!(MarketDataKind::Book.is_booked());
    /// assert!(!MarketDataKind::Execution.is_booked());
    /// assert!(!MarketDataKind::Trade.is_booked());
    /// assert!(!MarketDataKind::OrderBatch.is_booked());
    /// assert!(!MarketDataKind::Session.is_booked());
    /// assert!(!MarketDataKind::Unknown.is_booked());
    /// ```
    #[must_use]
    pub const fn is_booked(self) -> bool {
        matches!(self, Self::Order | Self::Quotation | Self::Book)
    }

    /// Whether a book states an element of this kind - folded into a side,
    /// or recorded among its events: what [`Self::is_booked`] admits, a
    /// snapshot control recorded among the events of the book whose
    /// membership it replaced, and an execution, recorded among the events
    /// of its instant in the book its instrument keys and moving no side,
    /// since its fill moved the book through its order's or quote's own
    /// report. The
    /// one owner of that rule, which every pruning site of a book walk
    /// reads: a trade, whose fills are executions, a batch, whose items
    /// arrive as their own kinds, and every other category are pruned
    /// before the walk routes them.
    ///
    /// ```
    /// use yggdryl::MarketDataKind;
    ///
    /// assert!(MarketDataKind::Order.is_recorded());
    /// assert!(MarketDataKind::Execution.is_recorded());
    /// assert!(!MarketDataKind::Execution.is_booked());
    /// assert!(!MarketDataKind::Trade.is_recorded());
    /// assert!(!MarketDataKind::ExecutionBatch.is_recorded());
    /// assert!(!MarketDataKind::Unknown.is_recorded());
    /// ```
    #[must_use]
    pub const fn is_recorded(self) -> bool {
        self.is_booked() || matches!(self, Self::Execution)
    }

    /// Whether this kind files a batch: a message stating many orders,
    /// quotes, executions or trades at once, which a parse splits into one
    /// message of [`Self::item`] per entry.
    ///
    /// ```
    /// use yggdryl::MarketDataKind;
    ///
    /// assert!(MarketDataKind::QuoteBatch.is_batch());
    /// assert!(!MarketDataKind::Quotation.is_batch());
    /// ```
    #[must_use]
    pub const fn is_batch(self) -> bool {
        matches!(
            self,
            Self::OrderBatch | Self::QuoteBatch | Self::ExecutionBatch | Self::TradeBatch
        )
    }

    /// The kind one entry of a batch of this kind is - an order batch's
    /// `ORDR`, a quote batch's `QUOT`, an execution batch's `EXEC`, a trade
    /// batch's `TRAD` - and this kind itself for any other.
    ///
    /// ```
    /// use yggdryl::MarketDataKind;
    ///
    /// assert_eq!(MarketDataKind::OrderBatch.item(), MarketDataKind::Order);
    /// assert_eq!(MarketDataKind::TradeBatch.item(), MarketDataKind::Trade);
    /// assert_eq!(MarketDataKind::Book.item(), MarketDataKind::Book);
    /// ```
    #[must_use]
    pub const fn item(self) -> Self {
        match self {
            Self::OrderBatch => Self::Order,
            Self::QuoteBatch => Self::Quotation,
            Self::ExecutionBatch => Self::Execution,
            Self::TradeBatch => Self::Trade,
            other => other,
        }
    }
}

/// Each member's own word, which its spelling patterns read beside the
/// four-letter code.
fn marketdatakind_aliases() -> Vec<(&'static str, MarketDataKind)> {
    MarketDataKind::ALL
        .iter()
        .map(|member| (member.word(), *member))
        .collect()
}

/// The Arrow extension name of a market data element's kind, over `uint8`
/// storage.
pub(crate) const MARKETDATAKIND_EXTENSION_NAME: &str = "yggdryl.marketdatakind";

impl crate::DataType {
    /// Creates the market data kind datatype: FIX's MsgCat code set.
    ///
    /// ```
    /// use yggdryl::{DataType, MARKETDATAKIND_KIND};
    ///
    /// assert_eq!(DataType::marketdatakind(), MARKETDATAKIND_KIND.dtype());
    /// assert_eq!(DataType::marketdatakind().to_string(), "marketdatakind");
    /// assert!(DataType::marketdatakind().is_enum());
    /// ```
    #[must_use]
    pub const fn marketdatakind() -> Self {
        Self::Market(crate::MarketType::new(&MARKETDATAKIND_KIND))
    }
}

// /// A field declared as a market data element's kind.
define_field_types!(
    MarketDataKindType,
    MarketDataKindField,
    market = MARKETDATAKIND_KIND,
    MarketDataKind
);

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/marketdatakind.rs` pins and a caller cannot
    //! reach: the side a cross code and a chain are keyed by, which the graph
    //! and the FIX session keys read and no caller names.

    use crate::{MarketDataKind, Side};

    /// The side an element of `kind` stating `side` is keyed by.
    #[must_use]
    pub fn stored_side(kind: MarketDataKind, side: Side) -> Side {
        kind.stored_side(side)
    }
}
