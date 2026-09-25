//! [`MarketKind`]: which leaf a [`MarketData`](super::MarketData) value is,
//! and the word every schema and digest that states one spells it under.

/// Which leaf a [`MarketData`](super::MarketData) value is: an undated
/// operation or book side, or one of the six dated leaves.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum MarketKind {
    /// An undated order: [`Order`](super::Order).
    Order,
    /// An undated quote: [`Quote`](super::Quote).
    Quote,
    /// An undated execution: [`Execution`](super::Execution).
    Execution,
    /// A book side summary: [`BookSide`](super::BookSide).
    BookSide,
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
}

impl MarketKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 10] = [
        Self::Order,
        Self::Quote,
        Self::Execution,
        Self::BookSide,
        Self::OrderEvent,
        Self::QuoteEvent,
        Self::ExecutionEvent,
        Self::TradeEvent,
        Self::BookEvent,
        Self::SnapshotEvent,
    ];

    /// The stored spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Order => "order",
            Self::Quote => "quote",
            Self::Execution => "execution",
            Self::BookSide => "book_side",
            Self::OrderEvent => "order_event",
            Self::QuoteEvent => "quote_event",
            Self::ExecutionEvent => "execution_event",
            Self::TradeEvent => "trade_event",
            Self::BookEvent => "book_event",
            Self::SnapshotEvent => "snapshot_event",
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

    /// Whether the kind is one of the six dated leaves.
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
        )
    }
}
