//! One statement of one product, so a mixed stream is a stream of market
//! events.

use crate::graph::{Element, Event, MarketEventData};

use super::{ExecutionData, OrderData, QuoteData, TradeData};

/// One statement of one product: an order, a quote, an execution or a
/// trade, as the one value a stream of any of them holds.
///
/// A [`BookIterator`](super::BookIterator) reads makers and prints out of
/// one stream, and a walk chains what it is handed, so the four products
/// that state something to a ladder are one type here, and that type is a
/// market event exactly as each arm is: every accessor and mutator reaches
/// the arm's own event, its order is its instant, and the four readings a
/// walk takes by value - finalizing, following, merging, restating - are
/// the arm's own for two statements of one arm and refuse across arms,
/// because an order is never another statement of a quote. Two arms never
/// share an identity, since an arm's own facts are among what its identity
/// digests to, so a restatement across arms cannot arise from a walk and
/// answers the statement unchanged.
///
/// ```
/// use yggdryl::graph::{Element, Event, MarketElement};
/// use yggdryl::market::{ExecutionData, OrderData, Statement};
/// use yggdryl::Decimal18;
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut order = OrderData::at(1_700_000_000_000_000_000);
/// order.set_crosscode("O-1".to_owned());
/// order.set_qty(Decimal18::from_int(100));
/// order.finalize();
/// let mut fill = ExecutionData::at(1_700_000_000_000_000_001);
/// fill.set_crosscode("O-1".to_owned());
/// fill.set_qty(Decimal18::from_int(40));
/// fill.finalize();
/// let (maker, print) = (Statement::from(order.clone()), Statement::from(fill));
/// assert!(maker.is_maker() && print.is_print());
/// assert_eq!(maker.get_curruuid(), order.get_curruuid());
/// assert_eq!(print.get_qty(), Decimal18::from_int(40));
/// assert!(print.is_after(&maker));
/// // Across arms, following and merging refuse.
/// assert!(print.clone().with_previous(&maker).is_none());
/// assert!(maker.clone().merge_with(&print).is_none());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub enum Statement {
    /// A statement of an order: what rests on a ladder.
    Order(OrderData),
    /// A statement of a quote: two lanes that rest on a ladder.
    Quote(QuoteData),
    /// A fill: what prints against a ladder.
    Execution(ExecutionData),
    /// A trade: likewise, from the match's side.
    Trade(TradeData),
}

impl Statement {
    /// The event the statement is, whichever product.
    #[must_use]
    pub const fn event(&self) -> &MarketEventData {
        match self {
            Self::Order(held) => held.event(),
            Self::Quote(held) => held.event(),
            Self::Execution(held) => held.event(),
            Self::Trade(held) => held.event(),
        }
    }

    /// The event, to write.
    const fn event_mut(&mut self) -> &mut MarketEventData {
        match self {
            Self::Order(held) => held.event_mut(),
            Self::Quote(held) => held.event_mut(),
            Self::Execution(held) => held.event_mut(),
            Self::Trade(held) => held.event_mut(),
        }
    }

    /// An order or a quote: what rests on a ladder.
    #[must_use]
    pub const fn is_maker(&self) -> bool {
        matches!(self, Self::Order(_) | Self::Quote(_))
    }

    /// An execution or a trade: what prints against a ladder.
    #[must_use]
    pub const fn is_print(&self) -> bool {
        matches!(self, Self::Execution(_) | Self::Trade(_))
    }

    /// The arm's own settling.
    fn settle(&mut self) {
        match self {
            Self::Order(held) => held.finalize(),
            Self::Quote(held) => held.finalize(),
            Self::Execution(held) => held.finalize(),
            Self::Trade(held) => held.finalize(),
        }
    }

    /// The arm's own following for two statements of one arm; nothing
    /// across arms.
    fn following(self, previous: &Self) -> Option<Self> {
        match (self, previous) {
            (Self::Order(this), Self::Order(that)) => this.with_previous(that).map(Self::Order),
            (Self::Quote(this), Self::Quote(that)) => this.with_previous(that).map(Self::Quote),
            (Self::Execution(this), Self::Execution(that)) => {
                this.with_previous(that).map(Self::Execution)
            }
            (Self::Trade(this), Self::Trade(that)) => this.with_previous(that).map(Self::Trade),
            _ => None,
        }
    }

    /// The arm's own merge for two statements of one arm; nothing across
    /// arms.
    fn merging(self, other: &Self) -> Option<Self> {
        match (self, other) {
            (Self::Order(this), Self::Order(that)) => this.merge_with(that).map(Self::Order),
            (Self::Quote(this), Self::Quote(that)) => this.merge_with(that).map(Self::Quote),
            (Self::Execution(this), Self::Execution(that)) => {
                this.merge_with(that).map(Self::Execution)
            }
            (Self::Trade(this), Self::Trade(that)) => this.merge_with(that).map(Self::Trade),
            _ => None,
        }
    }

    /// The arm's own restatement for two statements of one arm; the
    /// statement unchanged across arms, which no walk produces.
    fn restating(self, live: &Self) -> Self {
        match (self, live) {
            (Self::Order(this), Self::Order(that)) => Self::Order(this.restating(that)),
            (Self::Quote(this), Self::Quote(that)) => Self::Quote(this.restating(that)),
            (Self::Execution(this), Self::Execution(that)) => Self::Execution(this.restating(that)),
            (Self::Trade(this), Self::Trade(that)) => Self::Trade(this.restating(that)),
            (this, _) => this,
        }
    }
}

impl From<OrderData> for Statement {
    fn from(held: OrderData) -> Self {
        Self::Order(held)
    }
}

impl From<QuoteData> for Statement {
    fn from(held: QuoteData) -> Self {
        Self::Quote(held)
    }
}

impl From<ExecutionData> for Statement {
    fn from(held: ExecutionData) -> Self {
        Self::Execution(held)
    }
}

impl From<TradeData> for Statement {
    fn from(held: TradeData) -> Self {
        Self::Trade(held)
    }
}

super::product::holds_market_event!(
    Statement via event, event_mut,
    with_previous = Self::following,
    merge_with = Self::merging,
    restating = Self::restating
);
