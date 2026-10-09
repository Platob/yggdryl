//! [`MarketKind`]: which leaf a [`MarketData`](super::MarketData) value is,
//! and the word every schema and digest that states one spells it under.

use crate::MarketDataKind;

/// Which leaf a [`MarketData`](super::MarketData) value is: an undated
/// operation, one of the six dated leaves, or a FIX message held whole.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum MarketKind {
    /// An undated order: [`Order`](super::Order).
    Order,
    /// An undated quote: [`Quote`](super::Quote).
    Quote,
    /// An undated execution: [`Execution`](super::Execution).
    Execution,
    /// A dated order: [`OrderEvent`](super::OrderEvent).
    OrderEvent,
    /// A dated quote: [`QuoteEvent`](super::QuoteEvent).
    QuoteEvent,
    /// A dated execution: [`ExecutionEvent`](super::ExecutionEvent).
    ExecutionEvent,
    /// A composite trade: [`TradeEvent`](super::TradeEvent).
    TradeEvent,
    /// One coherent book at an instant: [`BookEvent`](super::BookEvent).
    BookEvent,
    /// A full-snapshot control: [`SnapshotEvent`](super::SnapshotEvent).
    SnapshotEvent,
    /// A message held whole: a
    /// [`MarketMessage`](super::market_data::MarketMessage) - a FIX
    /// message - filed under the category its dictionary files its type
    /// under.
    Fix,
}

impl MarketKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 10] = [
        Self::Order,
        Self::Quote,
        Self::Execution,
        Self::OrderEvent,
        Self::QuoteEvent,
        Self::ExecutionEvent,
        Self::TradeEvent,
        Self::BookEvent,
        Self::SnapshotEvent,
        Self::Fix,
    ];

    /// The stored spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Order => "order",
            Self::Quote => "quote",
            Self::Execution => "execution",
            Self::OrderEvent => "order_event",
            Self::QuoteEvent => "quote_event",
            Self::ExecutionEvent => "execution_event",
            Self::TradeEvent => "trade_event",
            Self::BookEvent => "book_event",
            Self::SnapshotEvent => "snapshot_event",
            Self::Fix => "fix",
        }
    }

    /// The kind a stored spelling names, ignoring ASCII case; `None` for
    /// any other text.
    #[must_use]
    pub fn read(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str().eq_ignore_ascii_case(text))
    }

    /// The market data category this leaf stands under, the member its
    /// `marketdatakind` column states: an order `ORDR`, a quote `QUOT`, an
    /// execution `EXEC`, a trade `TRAD`, and a book and a snapshot
    /// control `BOOK`. A FIX message states its own - the category its
    /// dictionary files it under ([`Market::marketdatakind`](crate::graph::Market::marketdatakind)) -
    /// so the kind alone answers `UKNW` for it.
    ///
    /// ```
    /// use yggdryl::MarketDataKind;
    /// use yggdryl::graph::MarketKind;
    ///
    /// assert_eq!(MarketKind::OrderEvent.marketdatakind(), MarketDataKind::Order);
    /// assert_eq!(MarketKind::SnapshotEvent.marketdatakind(), MarketDataKind::Book);
    /// ```
    #[must_use]
    pub const fn marketdatakind(self) -> MarketDataKind {
        match self {
            Self::Order | Self::OrderEvent => MarketDataKind::Order,
            Self::Quote | Self::QuoteEvent => MarketDataKind::Quotation,
            Self::Execution | Self::ExecutionEvent => MarketDataKind::Execution,
            Self::TradeEvent => MarketDataKind::Trade,
            Self::BookEvent | Self::SnapshotEvent => MarketDataKind::Book,
            Self::Fix => MarketDataKind::Unknown,
        }
    }

    /// Whether the kind is dated: one of the six dated leaves, or a FIX
    /// message.
    #[must_use]
    pub const fn is_event(self) -> bool {
        matches!(
            self,
            Self::OrderEvent
                | Self::QuoteEvent
                | Self::ExecutionEvent
                | Self::TradeEvent
                | Self::BookEvent
                | Self::SnapshotEvent
                | Self::Fix
        )
    }
}
