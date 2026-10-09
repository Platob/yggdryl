//! [`MarketData`]: one value over every leaf the graph vocabulary ships,
//! typed through at the boundary and read generically past it.

use std::any::Any;
use std::fmt;

use smol_str::{SmolStr, format_smolstr};

use super::Market;
use super::book::{BookEvent, SnapshotEvent};
use super::facts::OperationEventFacts;
use super::kind::MarketKind;
use super::operation::{
    BookOperation, BookRef, Execution, ExecutionEvent, Order, OrderEvent, Quote, QuoteEvent,
};
use super::trade::TradeEvent;
use yggdryl::graph::Element;
use yggdryl::{Error, Result};

/// One value over every leaf the graph vocabulary ships: an undated
/// operation, or one of the six dated leaves. Every operation
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
    /// A message held whole - a FIX message, every fact its dictionary
    /// reads, its fields and its capture - answered through the same
    /// traits as every leaf. It walks, merges and writes as it is; a book
    /// folds the leaves it splits into
    /// ([`MarketMessage::into_market_data`]).
    Fix(Box<dyn MarketMessage>),
}

impl MarketData {
    /// Which leaf this value is.
    #[must_use]
    pub fn kind(&self) -> MarketKind {
        match self {
            Self::Order(_) => MarketKind::Order,
            Self::Quote(_) => MarketKind::Quote,
            Self::Execution(_) => MarketKind::Execution,
            Self::OrderEvent(_) => MarketKind::OrderEvent,
            Self::QuoteEvent(_) => MarketKind::QuoteEvent,
            Self::ExecutionEvent(_) => MarketKind::ExecutionEvent,
            Self::TradeEvent(_) => MarketKind::TradeEvent,
            Self::BookEvent(_) => MarketKind::BookEvent,
            Self::SnapshotEvent(_) => MarketKind::SnapshotEvent,
            Self::Fix(_) => MarketKind::Fix,
        }
    }

    /// The market data category of this value's leaf: an order `ORDR`, a
    /// quote `QUOT`, an execution `EXEC`, a trade `TRAD`, a book or a
    /// snapshot `BOOK`, and a FIX message the category its dictionary files
    /// it under - what the `marketdatakind` column states.
    #[must_use]
    pub fn marketdatakind(&self) -> crate::MarketDataKind {
        match self {
            Self::Fix(message) => message.marketdatakind(),
            other => other.kind().marketdatakind(),
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

    /// This value as the operation-event seam a book's internals read
    /// through, where it is a dated order or quote - the only two kinds a
    /// book holds alive.
    pub(crate) fn as_operation_event(&self) -> Option<&dyn BookOperation> {
        match self {
            Self::OrderEvent(event) => Some(event),
            Self::QuoteEvent(event) => Some(event),
            Self::ExecutionEvent(event) => Some(event),
            _ => None,
        }
    }

    /// [`Self::as_operation_event`], mutably.
    pub(crate) fn as_operation_event_mut(&mut self) -> Option<&mut dyn BookOperation> {
        match self {
            Self::OrderEvent(event) => Some(event),
            Self::QuoteEvent(event) => Some(event),
            Self::ExecutionEvent(event) => Some(event),
            _ => None,
        }
    }

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

    /// This value's dated order, quote or execution: the seam a book's
    /// internals read through, unwrapped - a side and a book's delta hold
    /// orders and quotes, its events the executions beside the snapshot
    /// controls, which are no operation and are never read through it.
    pub(crate) fn operation_event(&self) -> &dyn BookOperation {
        self.as_operation_event()
            .expect("a book reads operation facts only off a dated order, quote or execution")
    }

    /// [`Self::operation_event`], mutably.
    pub(crate) fn operation_event_mut(&mut self) -> &mut dyn BookOperation {
        self.as_operation_event_mut()
            .expect("a book reads operation facts only off a dated order, quote or execution")
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
            Self::OrderEvent(v) => v.$method($($arg),*),
            Self::QuoteEvent(v) => v.$method($($arg),*),
            Self::ExecutionEvent(v) => v.$method($($arg),*),
            Self::TradeEvent(v) => v.$method($($arg),*),
            Self::BookEvent(v) => v.$method($($arg),*),
            Self::SnapshotEvent(v) => v.$method($($arg),*),
            Self::Fix(v) => v.$method($($arg),*),
        }
    };
}

impl Element for MarketData {
    fn get_uuid(&self) -> yggdryl::Uuid {
        delegate_by_variant!(self, get_uuid)
    }
    fn set_uuid(&mut self, uuid: yggdryl::Uuid) {
        delegate_by_variant!(self, set_uuid, uuid);
    }
    fn get_crossuuid(&self) -> yggdryl::Uuid {
        delegate_by_variant!(self, get_crossuuid)
    }
    fn set_crossuuid(&mut self, crossuuid: yggdryl::Uuid) {
        delegate_by_variant!(self, set_crossuuid, crossuuid);
    }
    fn get_crosscode(&self) -> &str {
        delegate_by_variant!(self, get_crosscode)
    }
    fn set_crosscode(&mut self, crosscode: String) {
        delegate_by_variant!(self, set_crosscode, crosscode);
    }
    fn get_hashcode(&self) -> u64 {
        delegate_by_variant!(self, get_hashcode)
    }
    fn set_hashcode(&mut self, hashcode: u64) {
        delegate_by_variant!(self, set_hashcode, hashcode);
    }
    fn get_crosshashcode(&self) -> u64 {
        delegate_by_variant!(self, get_crosshashcode)
    }
    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        delegate_by_variant!(self, set_crosshashcode, crosshashcode);
    }
    fn get_srcuuids(&self) -> &[yggdryl::Uuid] {
        delegate_by_variant!(self, get_srcuuids)
    }
    fn set_srcuuids(&mut self, sources: Vec<yggdryl::Uuid>) {
        delegate_by_variant!(self, set_srcuuids, sources);
    }

    /// Both dated: by instant. Else: never - an undated value, or a mix of
    /// dated and undated, states no order.
    fn is_after(&self, other: &Self) -> bool {
        match (self.as_event(), other.as_event()) {
            (Some(this), Some(other)) => this.get_transunix() > other.get_transunix(),
            _ => false,
        }
    }

    fn finalize(&mut self) {
        delegate_by_variant!(self, finalize);
    }

    /// Same variant: the leaf's own following. An operation event follows
    /// an operation event of another kind through the facts both hold - an
    /// execution follows the order it fills - keeping its own kind. Else:
    /// nothing - a book and a trade, say, do not follow one another.
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
            (Self::Fix(v), Self::Fix(previous)) => {
                v.with_previous(previous.as_ref()).map(Self::Fix)
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
            (Self::Fix(v), Self::Fix(other)) => v.merge_with(other.as_ref()).map(Self::Fix),
            _ => None,
        }
    }
}

impl Market for MarketData {
    fn get_price(&self) -> Option<yggdryl::Decimal> {
        delegate_by_variant!(self, get_price)
    }
    fn set_price(&mut self, price: Option<yggdryl::Decimal>, overwrite: bool) {
        delegate_by_variant!(self, set_price, price, overwrite);
    }

    fn get_stoppx(&self) -> Option<yggdryl::Decimal> {
        delegate_by_variant!(self, get_stoppx)
    }

    fn set_stoppx(&mut self, value: Option<yggdryl::Decimal>, overwrite: bool) {
        delegate_by_variant!(self, set_stoppx, value, overwrite);
    }

    fn get_strikepx(&self) -> Option<yggdryl::Decimal> {
        delegate_by_variant!(self, get_strikepx)
    }

    fn set_strikepx(&mut self, value: Option<yggdryl::Decimal>, overwrite: bool) {
        delegate_by_variant!(self, set_strikepx, value, overwrite);
    }
    fn get_currency(&self) -> &yggdryl::Ccy {
        delegate_by_variant!(self, get_currency)
    }
    fn set_currency(&mut self, currency: yggdryl::Ccy, overwrite: bool) {
        delegate_by_variant!(self, set_currency, currency, overwrite);
    }
    fn get_origccy(&self) -> &yggdryl::Ccy {
        delegate_by_variant!(self, get_origccy)
    }
    fn set_origccy(&mut self, ccy: yggdryl::Ccy, overwrite: bool) {
        delegate_by_variant!(self, set_origccy, ccy, overwrite);
    }
    fn get_quantity(&self) -> Option<yggdryl::Decimal> {
        delegate_by_variant!(self, get_quantity)
    }
    fn set_quantity(&mut self, quantity: Option<yggdryl::Decimal>, overwrite: bool) {
        delegate_by_variant!(self, set_quantity, quantity, overwrite);
    }

    fn get_displayqty(&self) -> Option<yggdryl::Decimal> {
        delegate_by_variant!(self, get_displayqty)
    }

    fn set_displayqty(&mut self, value: Option<yggdryl::Decimal>, overwrite: bool) {
        delegate_by_variant!(self, set_displayqty, value, overwrite);
    }

    fn get_hiddenqty(&self) -> Option<yggdryl::Decimal> {
        delegate_by_variant!(self, get_hiddenqty)
    }

    fn set_hiddenqty(&mut self, value: Option<yggdryl::Decimal>, overwrite: bool) {
        delegate_by_variant!(self, set_hiddenqty, value, overwrite);
    }
    fn get_unit(&self) -> &yggdryl::Unit {
        delegate_by_variant!(self, get_unit)
    }
    fn set_unit(&mut self, unit: yggdryl::Unit, overwrite: bool) {
        delegate_by_variant!(self, set_unit, unit, overwrite);
    }
    fn get_side(&self) -> crate::Side {
        delegate_by_variant!(self, get_side)
    }
    fn set_side(&mut self, side: crate::Side, overwrite: bool) {
        delegate_by_variant!(self, set_side, side, overwrite);
    }
    fn marketdatakind(&self) -> crate::MarketDataKind {
        MarketData::marketdatakind(self)
    }
    fn get_marketdatatype(&self) -> crate::MarketDataType {
        delegate_by_variant!(self, get_marketdatatype)
    }
    fn set_marketdatatype(&mut self, mdtype: crate::MarketDataType, overwrite: bool) {
        delegate_by_variant!(self, set_marketdatatype, mdtype, overwrite);
    }
    fn get_securityids(&self) -> &crate::Identifiers {
        delegate_by_variant!(self, get_securityids)
    }
    fn set_securityids(&mut self, ids: crate::Identifiers, overwrite: bool) -> yggdryl::Result<()> {
        delegate_by_variant!(self, set_securityids, ids, overwrite)
    }
    fn insert_securityid(&mut self, id: crate::Identifier) -> yggdryl::Result<bool> {
        delegate_by_variant!(self, insert_securityid, id)
    }
    fn remove_securityid(&mut self, key: &crate::IdKey) -> yggdryl::Result<bool> {
        delegate_by_variant!(self, remove_securityid, key)
    }
    fn derive_securityid(&mut self, kind: &crate::IdType, code: &str) -> bool {
        delegate_by_variant!(self, derive_securityid, kind, code)
    }
    fn get_cficode(&self) -> Option<&yggdryl::Cfi> {
        delegate_by_variant!(self, get_cficode)
    }
    fn set_cficode(&mut self, code: Option<yggdryl::Cfi>, overwrite: bool) {
        delegate_by_variant!(self, set_cficode, code, overwrite);
    }
    fn get_miccode(&self) -> Option<&yggdryl::Mic> {
        delegate_by_variant!(self, get_miccode)
    }
    fn set_miccode(&mut self, code: Option<yggdryl::Mic>, overwrite: bool) {
        delegate_by_variant!(self, set_miccode, code, overwrite);
    }
    fn get_execunix(&self) -> Option<i64> {
        delegate_by_variant!(self, get_execunix)
    }
    fn set_execunix(&mut self, unix: Option<i64>, overwrite: bool) {
        delegate_by_variant!(self, set_execunix, unix, overwrite);
    }
    fn get_lastpx(&self) -> Option<yggdryl::Decimal> {
        delegate_by_variant!(self, get_lastpx)
    }
    fn set_lastpx(&mut self, px: Option<yggdryl::Decimal>, overwrite: bool) {
        delegate_by_variant!(self, set_lastpx, px, overwrite);
    }
    fn get_lastqty(&self) -> Option<yggdryl::Decimal> {
        delegate_by_variant!(self, get_lastqty)
    }
    fn set_lastqty(&mut self, qty: Option<yggdryl::Decimal>, overwrite: bool) {
        delegate_by_variant!(self, set_lastqty, qty, overwrite);
    }
    fn get_avgpx(&self) -> Option<yggdryl::Decimal> {
        delegate_by_variant!(self, get_avgpx)
    }
    fn set_avgpx(&mut self, px: Option<yggdryl::Decimal>, overwrite: bool) {
        delegate_by_variant!(self, set_avgpx, px, overwrite);
    }
    fn get_cumqty(&self) -> Option<yggdryl::Decimal> {
        delegate_by_variant!(self, get_cumqty)
    }
    fn set_cumqty(&mut self, qty: Option<yggdryl::Decimal>, overwrite: bool) {
        delegate_by_variant!(self, set_cumqty, qty, overwrite);
    }
    fn get_leavesqty(&self) -> Option<yggdryl::Decimal> {
        delegate_by_variant!(self, get_leavesqty)
    }
    fn set_leavesqty(&mut self, qty: Option<yggdryl::Decimal>, overwrite: bool) {
        delegate_by_variant!(self, set_leavesqty, qty, overwrite);
    }

    fn get_cxlqty(&self) -> Option<yggdryl::Decimal> {
        delegate_by_variant!(self, get_cxlqty)
    }

    fn set_cxlqty(&mut self, value: Option<yggdryl::Decimal>, overwrite: bool) {
        delegate_by_variant!(self, set_cxlqty, value, overwrite);
    }
    fn get_prevpx(&self) -> Option<yggdryl::Decimal> {
        delegate_by_variant!(self, get_prevpx)
    }
    fn set_prevpx(&mut self, px: Option<yggdryl::Decimal>, overwrite: bool) {
        delegate_by_variant!(self, set_prevpx, px, overwrite);
    }
    fn get_prevqty(&self) -> Option<yggdryl::Decimal> {
        delegate_by_variant!(self, get_prevqty)
    }
    fn set_prevqty(&mut self, qty: Option<yggdryl::Decimal>, overwrite: bool) {
        delegate_by_variant!(self, set_prevqty, qty, overwrite);
    }
    fn get_spotrate(&self) -> Option<yggdryl::Decimal> {
        delegate_by_variant!(self, get_spotrate)
    }
    fn set_spotrate(&mut self, rate: Option<yggdryl::Decimal>, overwrite: bool) {
        delegate_by_variant!(self, set_spotrate, rate, overwrite);
    }
    fn get_forwardpoints(&self) -> Option<yggdryl::Decimal> {
        delegate_by_variant!(self, get_forwardpoints)
    }
    fn set_forwardpoints(&mut self, points: Option<yggdryl::Decimal>, overwrite: bool) {
        delegate_by_variant!(self, set_forwardpoints, points, overwrite);
    }
    fn get_ticker(&self) -> Option<&str> {
        delegate_by_variant!(self, get_ticker)
    }
    fn set_ticker(&mut self, ticker: Option<smol_str::SmolStr>, overwrite: bool) {
        delegate_by_variant!(self, set_ticker, ticker, overwrite);
    }
    fn get_metadata(&self) -> &super::market::Metadata {
        delegate_by_variant!(self, get_metadata)
    }
    fn set_metadata(&mut self, metadata: Option<super::market::Metadata>, overwrite: bool) {
        delegate_by_variant!(self, set_metadata, metadata, overwrite);
    }
    fn get_fxrates(&self) -> &super::market::FxRates {
        delegate_by_variant!(self, get_fxrates)
    }
    fn set_fxrates(&mut self, rates: super::market::FxRates, overwrite: bool) {
        delegate_by_variant!(self, set_fxrates, rates, overwrite);
    }
    fn get_bidpx(&self) -> Option<yggdryl::Decimal> {
        delegate_by_variant!(self, get_bidpx)
    }
    fn set_bidpx(&mut self, px: Option<yggdryl::Decimal>, overwrite: bool) {
        delegate_by_variant!(self, set_bidpx, px, overwrite);
    }
    fn get_bidqty(&self) -> Option<yggdryl::Decimal> {
        delegate_by_variant!(self, get_bidqty)
    }
    fn set_bidqty(&mut self, qty: Option<yggdryl::Decimal>, overwrite: bool) {
        delegate_by_variant!(self, set_bidqty, qty, overwrite);
    }
    fn get_bidccy(&self) -> Option<&yggdryl::Ccy> {
        delegate_by_variant!(self, get_bidccy)
    }
    fn set_bidccy(&mut self, ccy: Option<yggdryl::Ccy>, overwrite: bool) {
        delegate_by_variant!(self, set_bidccy, ccy, overwrite);
    }
    fn get_askpx(&self) -> Option<yggdryl::Decimal> {
        delegate_by_variant!(self, get_askpx)
    }
    fn set_askpx(&mut self, px: Option<yggdryl::Decimal>, overwrite: bool) {
        delegate_by_variant!(self, set_askpx, px, overwrite);
    }
    fn get_askqty(&self) -> Option<yggdryl::Decimal> {
        delegate_by_variant!(self, get_askqty)
    }
    fn set_askqty(&mut self, qty: Option<yggdryl::Decimal>, overwrite: bool) {
        delegate_by_variant!(self, set_askqty, qty, overwrite);
    }
    fn get_askccy(&self) -> Option<&yggdryl::Ccy> {
        delegate_by_variant!(self, get_askccy)
    }
    fn set_askccy(&mut self, ccy: Option<yggdryl::Ccy>, overwrite: bool) {
        delegate_by_variant!(self, set_askccy, ccy, overwrite);
    }
}

impl MarketData {
    /// This value as an [`Event`](yggdryl::graph::Event), where it is one of the six
    /// dated leaves.
    #[must_use]
    pub(crate) fn as_event(&self) -> Option<&dyn yggdryl::graph::Event> {
        match self {
            Self::OrderEvent(v) => Some(v),
            Self::QuoteEvent(v) => Some(v),
            Self::ExecutionEvent(v) => Some(v),
            Self::TradeEvent(v) => Some(v),
            Self::BookEvent(v) => Some(v.as_ref()),
            Self::SnapshotEvent(v) => Some(v),
            Self::Fix(v) => Some(&**v),
            _ => None,
        }
    }

    /// This value as an [`Event`] that is also an [`Operation`] - the five
    /// kinds [`super::EventIterator`] walks: [`OrderEvent`], [`QuoteEvent`],
    /// [`ExecutionEvent`], [`TradeEvent`] and a FIX message. `None` for the
    /// other five, which the walk yields unchanged, in place.
    #[must_use]
    pub(crate) fn as_event_operation(&self) -> Option<&dyn EventOperation> {
        match self {
            Self::OrderEvent(v) => Some(v),
            Self::QuoteEvent(v) => Some(v),
            Self::ExecutionEvent(v) => Some(v),
            Self::TradeEvent(v) => Some(v),
            Self::Fix(v) => Some(&**v),
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
            Self::Fix(v) => Some(&mut **v),
            _ => None,
        }
    }
}

pub(crate) use seam::EventOperation;

/// Where the walk's seam lives: a module the crate keeps to itself, so the
/// trait in it is nominally public - which is what lets [`MarketMessage`]
/// name it as a supertrait, and a held message upcast to it - while no
/// caller outside the crate can reach it.
mod seam {
    use super::super::Operation;
    use yggdryl::graph::Event;

    /// An event that is also an operation: the seam the event walk reads
    /// `MarketData`'s five walked kinds through.
    pub trait EventOperation: Event + Operation {}
    impl<T: Event + Operation + ?Sized> EventOperation for T {}
}

/// A message held whole that splits into market leaves: what
/// [`MarketData::Fix`] holds, a FIX message (`FixMsg`) being the one
/// implementation.
///
/// It answers every fact through the graph traits it extends, so a walk,
/// a merge and the Arrow writer read it as they read a leaf; it follows,
/// merges and restates as itself through the boxed forms below, and a book
/// folds the leaves it splits into ([`Self::into_market_data`]). The last
/// five methods are what a trait object owes the enum holding it - the
/// stable hash, a copy, equality - and the downcast back to its own type
/// ([`MarketData::as_message`]).
///
/// Its supertraits are the event and operation traits, the walk's own seam
/// over the two (crate-private, so a held message reads as every walked
/// leaf does), `Debug`, `Send`, `Sync` and `'static`.
pub trait MarketMessage:
    yggdryl::graph::Event + super::Operation + seam::EventOperation + fmt::Debug + Send + Sync + 'static
{
    /// Moves the message into the market data leaves it splits into: one
    /// per operation it states, none for a message with no market reading.
    ///
    /// # Errors
    ///
    /// Returns the implementation's refusal of a message it cannot split.
    fn into_market_data(self: Box<Self>) -> Result<Vec<MarketData>>;

    /// Moves the message into the one market data leaf it is.
    ///
    /// # Errors
    ///
    /// Returns an error where the message is not exactly one leaf.
    fn into_market_leaf(self: Box<Self>) -> Result<MarketData>;

    /// [`Element::with_previous`], boxed: this message stated as the one
    /// after `previous`, or nothing where it cannot follow it, where
    /// following it changes nothing, or where `previous` is a message of
    /// another type.
    fn with_previous(
        self: Box<Self>,
        previous: &dyn MarketMessage,
    ) -> Option<Box<dyn MarketMessage>>;

    /// [`Element::merge_with`], boxed: this message with another statement
    /// of itself folded in, or nothing where `other` is another message,
    /// a message of another type, or where the fold changes nothing.
    fn merge_with(self: Box<Self>, other: &dyn MarketMessage) -> Option<Box<dyn MarketMessage>>;

    /// [`Event::restating`](yggdryl::graph::Event::restating), boxed: this message
    /// restated under the live statement of its chain, unchanged where
    /// `live` is a message of another type.
    fn restating(self: Box<Self>, live: &dyn MarketMessage) -> Box<dyn MarketMessage>;

    /// The deterministic hash of the message, as its type states it.
    fn stable_hash(&self) -> u64;

    /// A boxed copy.
    fn clone_box(&self) -> Box<dyn MarketMessage>;

    /// Equality across the trait object: the same type holding an equal
    /// message.
    fn dyn_eq(&self, other: &dyn MarketMessage) -> bool;

    /// The message as `Any`, for [`MarketData::as_message`].
    fn as_any(&self) -> &dyn Any;

    /// The message as `Any`, owned, for the conversion back to its own
    /// type.
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
}

impl Clone for Box<dyn MarketMessage> {
    fn clone(&self) -> Self {
        (**self).clone_box()
    }
}

impl PartialEq for dyn MarketMessage {
    fn eq(&self, other: &Self) -> bool {
        self.dyn_eq(other)
    }
}

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
    /// Borrows the message this value holds as the type `T` it is, where
    /// it holds a message of that type: `as_message::<FixMsg>()` reaches a
    /// FIX message's own surface.
    #[must_use]
    pub fn as_message<T: MarketMessage>(&self) -> Option<&T> {
        match self {
            Self::Fix(message) => message.as_any().downcast_ref::<T>(),
            _ => None,
        }
    }
}
