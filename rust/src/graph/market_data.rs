//! [`MarketData`]: one value over every leaf the graph vocabulary ships,
//! typed through at the boundary and read generically past it.

use smol_str::{SmolStr, format_smolstr};

use super::book::{BookEvent, BookSide, SnapshotEvent};
use super::facts::OperationEventFacts;
use super::kind::MarketKind;
use super::operation::{
    BookOperation, BookRef, Execution, ExecutionEvent, Order, OrderEvent, Quote, QuoteEvent,
};
use super::trade::TradeEvent;
use super::{Element, Market};
use crate::{Error, Result};

/// One value over every leaf the graph vocabulary ships: an undated
/// operation, a book side, or one of the six dated leaves. Every operation
/// application (Arrow, FIX, a book) reads or writes a `MarketData`, resolved
/// exactly once at the boundary that built it.
#[derive(Clone, Debug, PartialEq)]
pub enum MarketData {
    /// An undated order.
    Order(Order),
    /// An undated quote.
    Quote(Quote),
    /// An undated execution.
    Execution(Execution),
    /// A book side summary.
    BookSide(BookSide),
    /// A dated order.
    OrderEvent(OrderEvent),
    /// A dated quote.
    QuoteEvent(QuoteEvent),
    /// A dated execution.
    ExecutionEvent(ExecutionEvent),
    /// A composite trade.
    TradeEvent(TradeEvent),
    /// One coherent book at an instant.
    BookEvent(Box<BookEvent>),
    /// A full-snapshot control.
    SnapshotEvent(SnapshotEvent),
}

impl MarketData {
    /// Which leaf this value is.
    #[must_use]
    pub fn kind(&self) -> MarketKind {
        match self {
            Self::Order(_) => MarketKind::Order,
            Self::Quote(_) => MarketKind::Quote,
            Self::Execution(_) => MarketKind::Execution,
            Self::BookSide(_) => MarketKind::BookSide,
            Self::OrderEvent(_) => MarketKind::OrderEvent,
            Self::QuoteEvent(_) => MarketKind::QuoteEvent,
            Self::ExecutionEvent(_) => MarketKind::ExecutionEvent,
            Self::TradeEvent(_) => MarketKind::TradeEvent,
            Self::BookEvent(_) => MarketKind::BookEvent,
            Self::SnapshotEvent(_) => MarketKind::SnapshotEvent,
        }
    }

    /// Whether this value is one of the six dated leaves.
    #[must_use]
    pub fn is_event(&self) -> bool {
        self.kind().is_event()
    }

    /// The book-control facts this value states, where it is an operation
    /// event or a snapshot control.
    #[must_use]
    pub fn book(&self) -> Option<&BookRef> {
        match self {
            Self::OrderEvent(event) => event.book(),
            Self::QuoteEvent(event) => event.book(),
            Self::ExecutionEvent(event) => event.book(),
            Self::SnapshotEvent(event) => Some(event.book()),
            _ => None,
        }
    }

    /// This value as the operation-event seam book-side internals read
    /// through, where it is a dated order or quote - the only two kinds a
    /// book side ever holds.
    pub(crate) fn as_operation_event(&self) -> Option<&dyn BookOperation> {
        match self {
            Self::OrderEvent(event) => Some(event),
            Self::QuoteEvent(event) => Some(event),
            _ => None,
        }
    }

    /// [`Self::as_operation_event`], mutably.
    pub(crate) fn as_operation_event_mut(&mut self) -> Option<&mut dyn BookOperation> {
        match self {
            Self::OrderEvent(event) => Some(event),
            Self::QuoteEvent(event) => Some(event),
            _ => None,
        }
    }

    /// This value's dated order or quote, where it is one; the seam
    /// book-side internals read through, unwrapped.
    /// The facts of an operation event, whichever kind; none for any other
    /// variant.
    pub(crate) fn operation_event_facts(&self) -> Option<&OperationEventFacts> {
        match self {
            Self::OrderEvent(v) => Some(v.facts()),
            Self::QuoteEvent(v) => Some(v.facts()),
            Self::ExecutionEvent(v) => Some(v.facts()),
            _ => None,
        }
    }

    pub(crate) fn operation_event(&self) -> &dyn BookOperation {
        self.as_operation_event()
            .expect("a book side holds only dated order or quote events")
    }

    /// [`Self::operation_event`], mutably.
    pub(crate) fn operation_event_mut(&mut self) -> &mut dyn BookOperation {
        self.as_operation_event_mut()
            .expect("a book side holds only dated order or quote events")
    }
}

/// Delegates `impl Element for MarketData` and `impl Market for MarketData`
/// to whichever leaf each variant holds, keyed on the crate-owned trait
/// spelling.
macro_rules! delegate_by_variant {
    ($self:expr, $method:ident $(, $arg:expr)*) => {
        match $self {
            Self::Order(v) => v.$method($($arg),*),
            Self::Quote(v) => v.$method($($arg),*),
            Self::Execution(v) => v.$method($($arg),*),
            Self::BookSide(v) => v.$method($($arg),*),
            Self::OrderEvent(v) => v.$method($($arg),*),
            Self::QuoteEvent(v) => v.$method($($arg),*),
            Self::ExecutionEvent(v) => v.$method($($arg),*),
            Self::TradeEvent(v) => v.$method($($arg),*),
            Self::BookEvent(v) => v.$method($($arg),*),
            Self::SnapshotEvent(v) => v.$method($($arg),*),
        }
    };
}

impl Element for MarketData {
    fn get_curruuid(&self) -> crate::Uuid {
        delegate_by_variant!(self, get_curruuid)
    }
    fn set_curruuid(&mut self, curruuid: crate::Uuid) {
        delegate_by_variant!(self, set_curruuid, curruuid);
    }
    fn get_crossuuid(&self) -> crate::Uuid {
        delegate_by_variant!(self, get_crossuuid)
    }
    fn set_crossuuid(&mut self, crossuuid: crate::Uuid) {
        delegate_by_variant!(self, set_crossuuid, crossuuid);
    }
    fn get_crosscode(&self) -> &str {
        delegate_by_variant!(self, get_crosscode)
    }
    fn set_crosscode(&mut self, crosscode: String) {
        delegate_by_variant!(self, set_crosscode, crosscode);
    }
    fn get_currhashcode(&self) -> u64 {
        delegate_by_variant!(self, get_currhashcode)
    }
    fn set_currhashcode(&mut self, hashcode: u64) {
        delegate_by_variant!(self, set_currhashcode, hashcode);
    }
    fn get_crosshashcode(&self) -> u64 {
        delegate_by_variant!(self, get_crosshashcode)
    }
    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        delegate_by_variant!(self, set_crosshashcode, crosshashcode);
    }
    fn get_srcuuids(&self) -> &[crate::Uuid] {
        delegate_by_variant!(self, get_srcuuids)
    }
    fn set_srcuuids(&mut self, sources: Vec<crate::Uuid>) {
        delegate_by_variant!(self, set_srcuuids, sources);
    }

    /// Both dated: by instant. Else: never - an undated value, or a mix of
    /// dated and undated, states no order.
    fn is_after(&self, other: &Self) -> bool {
        match (self.as_event(), other.as_event()) {
            (Some(this), Some(other)) => this.get_currunix() > other.get_currunix(),
            _ => false,
        }
    }

    fn finalize(&mut self) {
        delegate_by_variant!(self, finalize);
    }

    /// Same variant: the leaf's own following. An operation event follows
    /// an operation event of another kind through the facts both hold - an
    /// execution follows the order it fills - keeping its own kind. Else:
    /// nothing - a book side and a trade, say, do not follow one another.
    fn with_previous(self, previous: &Self) -> Option<Self> {
        if std::mem::discriminant(&self) != std::mem::discriminant(previous) {
            let facts = previous.operation_event_facts()?;
            return match self {
                Self::OrderEvent(v) => v.following_facts(facts).map(Self::OrderEvent),
                Self::QuoteEvent(v) => v.following_facts(facts).map(Self::QuoteEvent),
                Self::ExecutionEvent(v) => v.following_facts(facts).map(Self::ExecutionEvent),
                _ => None,
            };
        }
        match (self, previous) {
            (Self::Order(v), Self::Order(previous)) => v.with_previous(previous).map(Self::Order),
            (Self::Quote(v), Self::Quote(previous)) => v.with_previous(previous).map(Self::Quote),
            (Self::Execution(v), Self::Execution(previous)) => {
                v.with_previous(previous).map(Self::Execution)
            }
            (Self::BookSide(v), Self::BookSide(previous)) => {
                v.with_previous(previous).map(Self::BookSide)
            }
            (Self::OrderEvent(v), Self::OrderEvent(previous)) => {
                v.with_previous(previous).map(Self::OrderEvent)
            }
            (Self::QuoteEvent(v), Self::QuoteEvent(previous)) => {
                v.with_previous(previous).map(Self::QuoteEvent)
            }
            (Self::ExecutionEvent(v), Self::ExecutionEvent(previous)) => {
                v.with_previous(previous).map(Self::ExecutionEvent)
            }
            (Self::TradeEvent(v), Self::TradeEvent(previous)) => {
                v.with_previous(previous).map(Self::TradeEvent)
            }
            (Self::BookEvent(v), Self::BookEvent(previous)) => (*v)
                .with_previous(previous)
                .map(|b| Self::BookEvent(Box::new(b))),
            (Self::SnapshotEvent(v), Self::SnapshotEvent(previous)) => {
                v.with_previous(previous).map(Self::SnapshotEvent)
            }
            _ => None,
        }
    }

    /// Same variant: the leaf's own merge. Else: nothing.
    fn merge_with(self, other: &Self) -> Option<Self> {
        match (self, other) {
            (Self::Order(v), Self::Order(other)) => v.merge_with(other).map(Self::Order),
            (Self::Quote(v), Self::Quote(other)) => v.merge_with(other).map(Self::Quote),
            (Self::Execution(v), Self::Execution(other)) => {
                v.merge_with(other).map(Self::Execution)
            }
            (Self::BookSide(v), Self::BookSide(other)) => v.merge_with(other).map(Self::BookSide),
            (Self::OrderEvent(v), Self::OrderEvent(other)) => {
                v.merge_with(other).map(Self::OrderEvent)
            }
            (Self::QuoteEvent(v), Self::QuoteEvent(other)) => {
                v.merge_with(other).map(Self::QuoteEvent)
            }
            (Self::ExecutionEvent(v), Self::ExecutionEvent(other)) => {
                v.merge_with(other).map(Self::ExecutionEvent)
            }
            (Self::TradeEvent(v), Self::TradeEvent(other)) => {
                v.merge_with(other).map(Self::TradeEvent)
            }
            (Self::BookEvent(v), Self::BookEvent(other)) => {
                (*v).merge_with(other).map(|b| Self::BookEvent(Box::new(b)))
            }
            (Self::SnapshotEvent(v), Self::SnapshotEvent(other)) => {
                v.merge_with(other).map(Self::SnapshotEvent)
            }
            _ => None,
        }
    }
}

impl Market for MarketData {
    fn get_price(&self) -> Option<crate::Decimal> {
        delegate_by_variant!(self, get_price)
    }
    fn set_price(&mut self, price: Option<crate::Decimal>) {
        delegate_by_variant!(self, set_price, price);
    }
    fn get_currency(&self) -> &crate::Ccy {
        delegate_by_variant!(self, get_currency)
    }
    fn set_currency(&mut self, currency: crate::Ccy) {
        delegate_by_variant!(self, set_currency, currency);
    }
    fn get_quantity(&self) -> Option<crate::Decimal> {
        delegate_by_variant!(self, get_quantity)
    }
    fn set_quantity(&mut self, quantity: Option<crate::Decimal>) {
        delegate_by_variant!(self, set_quantity, quantity);
    }
    fn get_unit(&self) -> &crate::Unit {
        delegate_by_variant!(self, get_unit)
    }
    fn set_unit(&mut self, unit: crate::Unit) {
        delegate_by_variant!(self, set_unit, unit);
    }
    fn get_side(&self) -> crate::Side {
        delegate_by_variant!(self, get_side)
    }
    fn set_side(&mut self, side: crate::Side) {
        delegate_by_variant!(self, set_side, side);
    }
    fn get_securityids(&self) -> &crate::securityid::SecurityIds {
        delegate_by_variant!(self, get_securityids)
    }
    fn set_securityids(&mut self, ids: crate::securityid::SecurityIds) -> Result<()> {
        delegate_by_variant!(self, set_securityids, ids)
    }
    fn insert_securityid(&mut self, id: crate::securityid::SecurityId) -> Result<bool> {
        delegate_by_variant!(self, insert_securityid, id)
    }
    fn remove_securityid(&mut self, key: &crate::securityid::SecType) -> Result<bool> {
        delegate_by_variant!(self, remove_securityid, key)
    }
    fn derive_securityid(&mut self, id: crate::securityid::SecurityId) -> bool {
        delegate_by_variant!(self, derive_securityid, id)
    }
    fn get_cficode(&self) -> Option<&crate::Cfi> {
        delegate_by_variant!(self, get_cficode)
    }
    fn set_cficode(&mut self, code: Option<crate::Cfi>) {
        delegate_by_variant!(self, set_cficode, code);
    }
    fn get_miccode(&self) -> Option<&crate::Mic> {
        delegate_by_variant!(self, get_miccode)
    }
    fn set_miccode(&mut self, code: Option<crate::Mic>) {
        delegate_by_variant!(self, set_miccode, code);
    }
    fn get_lastpx(&self) -> Option<crate::Decimal> {
        delegate_by_variant!(self, get_lastpx)
    }
    fn set_lastpx(&mut self, px: Option<crate::Decimal>) {
        delegate_by_variant!(self, set_lastpx, px);
    }
    fn get_lastqty(&self) -> Option<crate::Decimal> {
        delegate_by_variant!(self, get_lastqty)
    }
    fn set_lastqty(&mut self, qty: Option<crate::Decimal>) {
        delegate_by_variant!(self, set_lastqty, qty);
    }
    fn get_avgpx(&self) -> Option<crate::Decimal> {
        delegate_by_variant!(self, get_avgpx)
    }
    fn set_avgpx(&mut self, px: Option<crate::Decimal>) {
        delegate_by_variant!(self, set_avgpx, px);
    }
    fn get_cumqty(&self) -> Option<crate::Decimal> {
        delegate_by_variant!(self, get_cumqty)
    }
    fn set_cumqty(&mut self, qty: Option<crate::Decimal>) {
        delegate_by_variant!(self, set_cumqty, qty);
    }
    fn get_leavesqty(&self) -> Option<crate::Decimal> {
        delegate_by_variant!(self, get_leavesqty)
    }
    fn set_leavesqty(&mut self, qty: Option<crate::Decimal>) {
        delegate_by_variant!(self, set_leavesqty, qty);
    }
    fn get_prevpx(&self) -> Option<crate::Decimal> {
        delegate_by_variant!(self, get_prevpx)
    }
    fn set_prevpx(&mut self, px: Option<crate::Decimal>) {
        delegate_by_variant!(self, set_prevpx, px);
    }
    fn get_prevqty(&self) -> Option<crate::Decimal> {
        delegate_by_variant!(self, get_prevqty)
    }
    fn set_prevqty(&mut self, qty: Option<crate::Decimal>) {
        delegate_by_variant!(self, set_prevqty, qty);
    }
    fn get_spotrate(&self) -> Option<crate::Decimal> {
        delegate_by_variant!(self, get_spotrate)
    }
    fn set_spotrate(&mut self, rate: Option<crate::Decimal>) {
        delegate_by_variant!(self, set_spotrate, rate);
    }
    fn get_forwardpoints(&self) -> Option<crate::Decimal> {
        delegate_by_variant!(self, get_forwardpoints)
    }
    fn set_forwardpoints(&mut self, points: Option<crate::Decimal>) {
        delegate_by_variant!(self, set_forwardpoints, points);
    }
    fn get_ticker(&self) -> Option<&str> {
        delegate_by_variant!(self, get_ticker)
    }
    fn set_ticker(&mut self, ticker: Option<smol_str::SmolStr>) {
        delegate_by_variant!(self, set_ticker, ticker);
    }
    fn get_metadata(&self) -> &super::market::Metadata {
        delegate_by_variant!(self, get_metadata)
    }
    fn set_metadata(&mut self, metadata: Option<super::market::Metadata>) {
        delegate_by_variant!(self, set_metadata, metadata);
    }
}

impl MarketData {
    /// This value as an [`Event`](super::Event), where it is one of the six
    /// dated leaves.
    #[must_use]
    pub(crate) fn as_event(&self) -> Option<&dyn super::Event> {
        match self {
            Self::OrderEvent(v) => Some(v),
            Self::QuoteEvent(v) => Some(v),
            Self::ExecutionEvent(v) => Some(v),
            Self::TradeEvent(v) => Some(v),
            Self::BookEvent(v) => Some(v.as_ref()),
            Self::SnapshotEvent(v) => Some(v),
            _ => None,
        }
    }

    /// This value as an [`Event`] that is also an [`Operation`] - the four
    /// kinds [`super::EventIterator`] walks: [`OrderEvent`], [`QuoteEvent`],
    /// [`ExecutionEvent`] and [`TradeEvent`]. `None` for the other six,
    /// which the walk yields unchanged, in place.
    #[must_use]
    pub(crate) fn as_event_operation(&self) -> Option<&dyn EventOperation> {
        match self {
            Self::OrderEvent(v) => Some(v),
            Self::QuoteEvent(v) => Some(v),
            Self::ExecutionEvent(v) => Some(v),
            Self::TradeEvent(v) => Some(v),
            _ => None,
        }
    }

    /// [`Self::as_event_operation`], mutably.
    #[must_use]
    pub(crate) fn as_event_operation_mut(&mut self) -> Option<&mut dyn EventOperation> {
        match self {
            Self::OrderEvent(v) => Some(v),
            Self::QuoteEvent(v) => Some(v),
            Self::ExecutionEvent(v) => Some(v),
            Self::TradeEvent(v) => Some(v),
            _ => None,
        }
    }
}

/// An [`Event`](super::Event) that is also an [`Operation`](super::Operation):
/// the seam [`EventIterator`](super::EventIterator) reads `MarketData`'s
/// four walked kinds through.
pub(crate) trait EventOperation: super::Event + super::Operation {}
impl<T: super::Event + super::Operation + ?Sized> EventOperation for T {}

impl From<MarketData> for Result<MarketData> {
    /// A value as the infallible item of a fallible stream.
    fn from(value: MarketData) -> Self {
        Ok(value)
    }
}

macro_rules! from_leaf {
    ($leaf:ty, $variant:ident) => {
        impl From<$leaf> for MarketData {
            fn from(value: $leaf) -> Self {
                Self::$variant(value)
            }
        }

        impl TryFrom<MarketData> for $leaf {
            type Error = Error;

            fn try_from(value: MarketData) -> Result<Self> {
                match value {
                    MarketData::$variant(value) => Ok(value),
                    other => Err(Error::InvalidRecord {
                        path: SmolStr::new_static("$.kind"),
                        reason: format_smolstr!(
                            "expected {}, got {}",
                            MarketKind::$variant.as_str(),
                            other.kind().as_str()
                        ),
                    }),
                }
            }
        }
    };
}

from_leaf!(Order, Order);
from_leaf!(Quote, Quote);
from_leaf!(Execution, Execution);
from_leaf!(BookSide, BookSide);
from_leaf!(OrderEvent, OrderEvent);
from_leaf!(QuoteEvent, QuoteEvent);
from_leaf!(ExecutionEvent, ExecutionEvent);
from_leaf!(TradeEvent, TradeEvent);
from_leaf!(SnapshotEvent, SnapshotEvent);

impl From<BookEvent> for MarketData {
    fn from(value: BookEvent) -> Self {
        Self::BookEvent(Box::new(value))
    }
}

impl TryFrom<MarketData> for BookEvent {
    type Error = Error;

    fn try_from(value: MarketData) -> Result<Self> {
        match value {
            MarketData::BookEvent(value) => Ok(*value),
            other => Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.kind"),
                reason: format_smolstr!(
                    "expected {}, got {}",
                    MarketKind::BookEvent.as_str(),
                    other.kind().as_str()
                ),
            }),
        }
    }
}

impl MarketData {
    /// Borrows this value as an [`Order`], where it is one.
    #[must_use]
    pub fn as_order(&self) -> Option<&Order> {
        match self {
            Self::Order(value) => Some(value),
            _ => None,
        }
    }
    /// Borrows this value as a [`Quote`], where it is one.
    #[must_use]
    pub fn as_quote(&self) -> Option<&Quote> {
        match self {
            Self::Quote(value) => Some(value),
            _ => None,
        }
    }
    /// Borrows this value as an [`Execution`], where it is one.
    #[must_use]
    pub fn as_execution(&self) -> Option<&Execution> {
        match self {
            Self::Execution(value) => Some(value),
            _ => None,
        }
    }
    /// Borrows this value as a [`BookSide`], where it is one.
    #[must_use]
    pub fn as_book_side(&self) -> Option<&BookSide> {
        match self {
            Self::BookSide(value) => Some(value),
            _ => None,
        }
    }
    /// Borrows this value as an [`OrderEvent`], where it is one.
    #[must_use]
    pub fn as_order_event(&self) -> Option<&OrderEvent> {
        match self {
            Self::OrderEvent(value) => Some(value),
            _ => None,
        }
    }
    /// Borrows this value as a [`QuoteEvent`], where it is one.
    #[must_use]
    pub fn as_quote_event(&self) -> Option<&QuoteEvent> {
        match self {
            Self::QuoteEvent(value) => Some(value),
            _ => None,
        }
    }
    /// Borrows this value as an [`ExecutionEvent`], where it is one.
    #[must_use]
    pub fn as_execution_event(&self) -> Option<&ExecutionEvent> {
        match self {
            Self::ExecutionEvent(value) => Some(value),
            _ => None,
        }
    }
    /// Borrows this value as a [`TradeEvent`], where it is one.
    #[must_use]
    pub fn as_trade_event(&self) -> Option<&TradeEvent> {
        match self {
            Self::TradeEvent(value) => Some(value),
            _ => None,
        }
    }
    /// Borrows this value as a [`BookEvent`], where it is one.
    #[must_use]
    pub fn as_book_event(&self) -> Option<&BookEvent> {
        match self {
            Self::BookEvent(value) => Some(value),
            _ => None,
        }
    }
    /// Borrows this value as a [`SnapshotEvent`], where it is one.
    #[must_use]
    pub fn as_snapshot_event(&self) -> Option<&SnapshotEvent> {
        match self {
            Self::SnapshotEvent(value) => Some(value),
            _ => None,
        }
    }
}
