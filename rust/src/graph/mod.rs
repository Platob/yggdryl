//! The graph vocabulary: what an element of a graph answers about itself,
//! and one walk over elements.
//!
//! A graph is elements that know their own identity and, where they follow
//! one, the identity of the element before them. [`element`] holds the two
//! traits that state it: [`Element`] is the node - its [`Uuid`](crate::Uuid),
//! the one it has elsewhere and the UUIDs of what it was read from, read and
//! written, the order it stands in, and how it follows and merges - and
//! [`Event`] is an element that also happened at one instant and stands in
//! one state. [`market`] holds the two that stand in a market: [`Market`]
//! is the slim reading any plain struct gives cheaply - the instrument, the
//! side, the price and quantity it states, its last executed price and quantity - and
//! [`Operation`] adds what an operation states: how long
//! it stands, whether it can trade and its own identifiers.
//! What an [`Event`] that is also one of them answers is provided on
//! [`Market`] and [`Operation`] themselves, gated `where Self: Event`,
//! rather than a separate blanket trait. The traits state signatures and
//! the provided readings - no storage - so a message, a chain entry and a
//! lifecycle incarnation can each be an element without the graph owning
//! any of them; the crate-private `*Facts` holders store the facts of any
//! market element or event as plain fields, and every leaf
//! ([`OperationElement`]/[`OperationEvent`], [`TradeEvent`], [`BookEvent`],
//! [`SnapshotEvent`]) holds one as its facts. [`OperationElement`] and
//! [`OperationEvent`] are the one undated and one dated operation type - an
//! order, a quote or an execution by the sealed [`OperationKind`] they are
//! generic over - and [`MarketData`] is the one value over every leaf, read
//! generically past the boundary that resolved it. [`BookEvent`] holds its
//! `delta` - the orders and quotes its instant applied, in the order
//! applied - and its `events` - every other event the instant recorded:
//! the executions, resting on no side, and the snapshot controls - and, a
//! complete book, its alive [`OrderEvent`]/[`QuoteEvent`] entries,
//! answering each side as its price levels, best first; a [`BookIterator`]
//! emits a complete book only at a snapshot tick and a delta book between,
//! which [`Element::with_previous`] rebuilds. The one walk,
//! [`EventIterator`], reads operations in their order and states each as
//! the one after the live element it follows. [`ElementColumn`] is the six
//! columns every generated schema of an element opens with,
//! [`EventColumn`] the nine an event adds, [`MarketColumn`] the thirty-six
//! of a market and [`OperationColumn`] the five of an operation - one per
//! fact the traits answer, under one name and one datatype each, in that
//! order - so a text line's batch, a FIX row, a chained message and a
//! `marketdata` row open with the same columns and join on them without a
//! mapping.

/// `impl Event` forwarding every accessor to a field that is an `Event`,
/// with the readings a wrapper answers itself given as closures.
macro_rules! delegate_event {
    ($type:ty, $($field:ident).+, restating = $restating:expr) => {
        delegate_event!(
            $type,
            $($field).+,
            restating = $restating,
            is_execution = |this: &$type| $crate::graph::Event::is_execution(&this.$($field).+),
            set_currunix = |this: &mut $type, unix: i64| {
                $crate::graph::Event::set_currunix(&mut this.$($field).+, unix);
            }
        );
    };
    (
        $type:ty,
        $($field:ident).+,
        restating = $restating:expr,
        is_execution = $is_execution:expr,
        set_currunix = $set_currunix:expr
    ) => {
        impl $crate::graph::Event for $type {
            fn get_currunix(&self) -> i64 {
                $crate::graph::Event::get_currunix(&self.$($field).+)
            }
            fn set_currunix(&mut self, unix: i64) {
                ($set_currunix)(self, unix);
            }
            fn get_state(&self) -> &$crate::State {
                $crate::graph::Event::get_state(&self.$($field).+)
            }
            fn set_state(&mut self, state: $crate::State) {
                $crate::graph::Event::set_state(&mut self.$($field).+, state);
            }
            fn is_execution(&self) -> bool {
                ($is_execution)(self)
            }
            fn get_seqnum(&self) -> u64 {
                $crate::graph::Event::get_seqnum(&self.$($field).+)
            }
            fn set_seqnum(&mut self, seqnum: u64) {
                $crate::graph::Event::set_seqnum(&mut self.$($field).+, seqnum);
            }
            fn get_creaunix(&self) -> Option<i64> {
                $crate::graph::Event::get_creaunix(&self.$($field).+)
            }
            fn set_creaunix(&mut self, unix: Option<i64>) {
                $crate::graph::Event::set_creaunix(&mut self.$($field).+, unix);
            }
            fn get_recdunix(&self) -> Option<i64> {
                $crate::graph::Event::get_recdunix(&self.$($field).+)
            }
            fn set_recdunix(&mut self, unix: Option<i64>) {
                $crate::graph::Event::set_recdunix(&mut self.$($field).+, unix);
            }
            fn get_exprunix(&self) -> Option<i64> {
                $crate::graph::Event::get_exprunix(&self.$($field).+)
            }
            fn set_exprunix(&mut self, unix: Option<i64>) {
                $crate::graph::Event::set_exprunix(&mut self.$($field).+, unix);
            }
            fn get_prevunix(&self) -> Option<i64> {
                $crate::graph::Event::get_prevunix(&self.$($field).+)
            }
            fn set_prevunix(&mut self, unix: Option<i64>) {
                $crate::graph::Event::set_prevunix(&mut self.$($field).+, unix);
            }
            fn get_prevuuid(&self) -> Option<$crate::Uuid> {
                $crate::graph::Event::get_prevuuid(&self.$($field).+)
            }
            fn set_prevuuid(&mut self, uuid: Option<$crate::Uuid>) {
                $crate::graph::Event::set_prevuuid(&mut self.$($field).+, uuid);
            }
            fn get_snapunix(&self) -> Option<i64> {
                $crate::graph::Event::get_snapunix(&self.$($field).+)
            }
            fn set_snapunix(&mut self, unix: Option<i64>) {
                $crate::graph::Event::set_snapunix(&mut self.$($field).+, unix);
            }
            fn restating(self, live: &Self) -> Self {
                ($restating)(self, live)
            }
            fn finalized(&mut self, hashcode: u64) {
                $crate::graph::Event::finalized(&mut self.$($field).+, hashcode);
            }
        }
    };
}

/// `impl Market` forwarding every fact to a field that is a `Market`.
macro_rules! delegate_market {
    ($type:ty, $($field:ident).+) => {
        impl $crate::graph::Market for $type {
            fn get_price(&self) -> Option<$crate::Decimal> {
                $crate::graph::Market::get_price(&self.$($field).+)
            }
            fn set_price(&mut self, price: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Market::set_price(&mut self.$($field).+, price, overwrite);
            }

            fn get_stoppx(&self) -> Option<$crate::Decimal> {
                $crate::graph::Market::get_stoppx(&self.$($field).+)
            }

            fn set_stoppx(&mut self, value: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Market::set_stoppx(&mut self.$($field).+, value, overwrite);
            }

            fn get_strikepx(&self) -> Option<$crate::Decimal> {
                $crate::graph::Market::get_strikepx(&self.$($field).+)
            }

            fn set_strikepx(&mut self, value: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Market::set_strikepx(&mut self.$($field).+, value, overwrite);
            }
            fn get_currency(&self) -> &$crate::Ccy {
                $crate::graph::Market::get_currency(&self.$($field).+)
            }
            fn set_currency(&mut self, currency: $crate::Ccy, overwrite: bool) {
                $crate::graph::Market::set_currency(&mut self.$($field).+, currency, overwrite);
            }
            fn get_origccy(&self) -> &$crate::Ccy {
                $crate::graph::Market::get_origccy(&self.$($field).+)
            }
            fn set_origccy(&mut self, ccy: $crate::Ccy, overwrite: bool) {
                $crate::graph::Market::set_origccy(&mut self.$($field).+, ccy, overwrite);
            }
            fn get_quantity(&self) -> Option<$crate::Decimal> {
                $crate::graph::Market::get_quantity(&self.$($field).+)
            }
            fn set_quantity(&mut self, quantity: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Market::set_quantity(&mut self.$($field).+, quantity, overwrite);
            }

            fn get_displayqty(&self) -> Option<$crate::Decimal> {
                $crate::graph::Market::get_displayqty(&self.$($field).+)
            }

            fn set_displayqty(&mut self, value: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Market::set_displayqty(&mut self.$($field).+, value, overwrite);
            }

            fn get_hiddenqty(&self) -> Option<$crate::Decimal> {
                $crate::graph::Market::get_hiddenqty(&self.$($field).+)
            }

            fn set_hiddenqty(&mut self, value: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Market::set_hiddenqty(&mut self.$($field).+, value, overwrite);
            }
            fn get_unit(&self) -> &$crate::Unit {
                $crate::graph::Market::get_unit(&self.$($field).+)
            }
            fn set_unit(&mut self, unit: $crate::Unit, overwrite: bool) {
                $crate::graph::Market::set_unit(&mut self.$($field).+, unit, overwrite);
            }
            fn get_side(&self) -> $crate::Side {
                $crate::graph::Market::get_side(&self.$($field).+)
            }
            fn set_side(&mut self, side: $crate::Side, overwrite: bool) {
                $crate::graph::Market::set_side(&mut self.$($field).+, side, overwrite);
            }
            fn marketdatakind(&self) -> $crate::MarketDataKind {
                $crate::graph::Market::marketdatakind(&self.$($field).+)
            }
            fn get_marketdatatype(&self) -> $crate::MarketDataType {
                $crate::graph::Market::get_marketdatatype(&self.$($field).+)
            }
            fn set_marketdatatype(&mut self, mdtype: $crate::MarketDataType, overwrite: bool) {
                $crate::graph::Market::set_marketdatatype(&mut self.$($field).+, mdtype, overwrite);
            }
            fn get_securityids(&self) -> &$crate::Identifiers {
                $crate::graph::Market::get_securityids(&self.$($field).+)
            }
            fn set_securityids(&mut self, ids: $crate::Identifiers, overwrite: bool) -> $crate::Result<()> {
                $crate::graph::Market::set_securityids(&mut self.$($field).+, ids, overwrite)
            }
            fn insert_securityid(&mut self, id: $crate::Identifier) -> $crate::Result<bool> {
                $crate::graph::Market::insert_securityid(&mut self.$($field).+, id)
            }
            fn remove_securityid(&mut self, key: &$crate::IdKey) -> $crate::Result<bool> {
                $crate::graph::Market::remove_securityid(&mut self.$($field).+, key)
            }
            fn derive_securityid(&mut self, kind: &$crate::IdType, code: &str) -> bool {
                $crate::graph::Market::derive_securityid(&mut self.$($field).+, kind, code)
            }
            fn get_cficode(&self) -> Option<&$crate::Cfi> {
                $crate::graph::Market::get_cficode(&self.$($field).+)
            }
            fn set_cficode(&mut self, code: Option<$crate::Cfi>, overwrite: bool) {
                $crate::graph::Market::set_cficode(&mut self.$($field).+, code, overwrite);
            }
            fn get_miccode(&self) -> Option<&$crate::Mic> {
                $crate::graph::Market::get_miccode(&self.$($field).+)
            }
            fn set_miccode(&mut self, code: Option<$crate::Mic>, overwrite: bool) {
                $crate::graph::Market::set_miccode(&mut self.$($field).+, code, overwrite);
            }
            fn get_execunix(&self) -> Option<i64> {
                $crate::graph::Market::get_execunix(&self.$($field).+)
            }
            fn set_execunix(&mut self, unix: Option<i64>, overwrite: bool) {
                $crate::graph::Market::set_execunix(&mut self.$($field).+, unix, overwrite);
            }
            fn get_lastpx(&self) -> Option<$crate::Decimal> {
                $crate::graph::Market::get_lastpx(&self.$($field).+)
            }
            fn set_lastpx(&mut self, px: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Market::set_lastpx(&mut self.$($field).+, px, overwrite);
            }
            fn get_lastqty(&self) -> Option<$crate::Decimal> {
                $crate::graph::Market::get_lastqty(&self.$($field).+)
            }
            fn set_lastqty(&mut self, qty: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Market::set_lastqty(&mut self.$($field).+, qty, overwrite);
            }
            fn get_avgpx(&self) -> Option<$crate::Decimal> {
                $crate::graph::Market::get_avgpx(&self.$($field).+)
            }
            fn set_avgpx(&mut self, px: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Market::set_avgpx(&mut self.$($field).+, px, overwrite);
            }
            fn get_cumqty(&self) -> Option<$crate::Decimal> {
                $crate::graph::Market::get_cumqty(&self.$($field).+)
            }
            fn set_cumqty(&mut self, qty: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Market::set_cumqty(&mut self.$($field).+, qty, overwrite);
            }
            fn get_leavesqty(&self) -> Option<$crate::Decimal> {
                $crate::graph::Market::get_leavesqty(&self.$($field).+)
            }
            fn set_leavesqty(&mut self, qty: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Market::set_leavesqty(&mut self.$($field).+, qty, overwrite);
            }

            fn get_cxlqty(&self) -> Option<$crate::Decimal> {
                $crate::graph::Market::get_cxlqty(&self.$($field).+)
            }

            fn set_cxlqty(&mut self, value: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Market::set_cxlqty(&mut self.$($field).+, value, overwrite);
            }
            fn get_prevpx(&self) -> Option<$crate::Decimal> {
                $crate::graph::Market::get_prevpx(&self.$($field).+)
            }
            fn set_prevpx(&mut self, px: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Market::set_prevpx(&mut self.$($field).+, px, overwrite);
            }
            fn get_prevqty(&self) -> Option<$crate::Decimal> {
                $crate::graph::Market::get_prevqty(&self.$($field).+)
            }
            fn set_prevqty(&mut self, qty: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Market::set_prevqty(&mut self.$($field).+, qty, overwrite);
            }
            fn get_spotrate(&self) -> Option<$crate::Decimal> {
                $crate::graph::Market::get_spotrate(&self.$($field).+)
            }
            fn set_spotrate(&mut self, rate: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Market::set_spotrate(&mut self.$($field).+, rate, overwrite);
            }
            fn get_forwardpoints(&self) -> Option<$crate::Decimal> {
                $crate::graph::Market::get_forwardpoints(&self.$($field).+)
            }
            fn set_forwardpoints(&mut self, points: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Market::set_forwardpoints(&mut self.$($field).+, points, overwrite);
            }
            fn get_ticker(&self) -> Option<&str> {
                $crate::graph::Market::get_ticker(&self.$($field).+)
            }
            fn set_ticker(&mut self, ticker: Option<smol_str::SmolStr>, overwrite: bool) {
                $crate::graph::Market::set_ticker(&mut self.$($field).+, ticker, overwrite);
            }
            fn get_metadata(&self) -> &$crate::graph::Metadata {
                $crate::graph::Market::get_metadata(&self.$($field).+)
            }
            fn set_metadata(&mut self, metadata: Option<$crate::graph::Metadata>, overwrite: bool) {
                $crate::graph::Market::set_metadata(&mut self.$($field).+, metadata, overwrite);
            }
            fn get_fxrates(&self) -> &$crate::graph::FxRates {
                $crate::graph::Market::get_fxrates(&self.$($field).+)
            }
            fn set_fxrates(&mut self, rates: $crate::graph::FxRates, overwrite: bool) {
                $crate::graph::Market::set_fxrates(&mut self.$($field).+, rates, overwrite);
            }
            fn get_bidpx(&self) -> Option<$crate::Decimal> {
                $crate::graph::Market::get_bidpx(&self.$($field).+)
            }
            fn set_bidpx(&mut self, px: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Market::set_bidpx(&mut self.$($field).+, px, overwrite);
            }
            fn get_bidqty(&self) -> Option<$crate::Decimal> {
                $crate::graph::Market::get_bidqty(&self.$($field).+)
            }
            fn set_bidqty(&mut self, qty: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Market::set_bidqty(&mut self.$($field).+, qty, overwrite);
            }
            fn get_bidccy(&self) -> Option<&$crate::Ccy> {
                $crate::graph::Market::get_bidccy(&self.$($field).+)
            }
            fn set_bidccy(&mut self, ccy: Option<$crate::Ccy>, overwrite: bool) {
                $crate::graph::Market::set_bidccy(&mut self.$($field).+, ccy, overwrite);
            }
            fn get_askpx(&self) -> Option<$crate::Decimal> {
                $crate::graph::Market::get_askpx(&self.$($field).+)
            }
            fn set_askpx(&mut self, px: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Market::set_askpx(&mut self.$($field).+, px, overwrite);
            }
            fn get_askqty(&self) -> Option<$crate::Decimal> {
                $crate::graph::Market::get_askqty(&self.$($field).+)
            }
            fn set_askqty(&mut self, qty: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Market::set_askqty(&mut self.$($field).+, qty, overwrite);
            }
            fn get_askccy(&self) -> Option<&$crate::Ccy> {
                $crate::graph::Market::get_askccy(&self.$($field).+)
            }
            fn set_askccy(&mut self, ccy: Option<$crate::Ccy>, overwrite: bool) {
                $crate::graph::Market::set_askccy(&mut self.$($field).+, ccy, overwrite);
            }
        }
    };
}

/// `impl Operation` forwarding every fact to a field that is a
/// `Operation`.
macro_rules! delegate_operation {
    ($type:ty, $($field:ident).+) => {
        impl $crate::graph::Operation for $type {
            fn get_ordqty(&self) -> Option<$crate::Decimal> {
                $crate::graph::Operation::get_ordqty(&self.$($field).+)
            }
            fn set_ordqty(&mut self, qty: Option<$crate::Decimal>, overwrite: bool) {
                $crate::graph::Operation::set_ordqty(&mut self.$($field).+, qty, overwrite);
            }
            fn get_timeinforce(&self) -> Option<&$crate::TimeInForce> {
                $crate::graph::Operation::get_timeinforce(&self.$($field).+)
            }
            fn set_timeinforce(&mut self, tif: Option<$crate::TimeInForce>, overwrite: bool) {
                $crate::graph::Operation::set_timeinforce(&mut self.$($field).+, tif, overwrite);
            }
            fn get_tradable(&self) -> Option<bool> {
                $crate::graph::Operation::get_tradable(&self.$($field).+)
            }
            fn set_tradable(&mut self, tradable: Option<bool>, overwrite: bool) {
                $crate::graph::Operation::set_tradable(&mut self.$($field).+, tradable, overwrite);
            }
            fn get_identifiers(&self) -> &$crate::Identifiers {
                $crate::graph::Operation::get_identifiers(&self.$($field).+)
            }
            fn set_identifiers(&mut self, ids: $crate::Identifiers, overwrite: bool) -> $crate::Result<()> {
                $crate::graph::Operation::set_identifiers(&mut self.$($field).+, ids, overwrite)
            }
            fn insert_identifier(&mut self, id: $crate::Identifier) -> $crate::Result<bool> {
                $crate::graph::Operation::insert_identifier(&mut self.$($field).+, id)
            }
            fn remove_identifier(&mut self, key: &$crate::IdKey) -> $crate::Result<bool> {
                $crate::graph::Operation::remove_identifier(&mut self.$($field).+, key)
            }
            fn get_partyids(&self) -> &$crate::Identifiers {
                $crate::graph::Operation::get_partyids(&self.$($field).+)
            }
            fn set_partyids(&mut self, partyids: $crate::Identifiers, overwrite: bool) -> $crate::Result<()> {
                $crate::graph::Operation::set_partyids(&mut self.$($field).+, partyids, overwrite)
            }
            fn insert_partyid(&mut self, partyid: $crate::Identifier) -> $crate::Result<bool> {
                $crate::graph::Operation::insert_partyid(&mut self.$($field).+, partyid)
            }
            fn remove_partyid(&mut self, key: &$crate::IdKey) -> $crate::Result<bool> {
                $crate::graph::Operation::remove_partyid(&mut self.$($field).+, key)
            }
            fn is_followed_identifier(&self, id: &$crate::Identifier) -> bool {
                $crate::graph::Operation::is_followed_identifier(&self.$($field).+, id)
            }
            fn parents_of(&self, base: &$crate::IdType) -> ::std::borrow::Cow<'_, [$crate::IdType]> {
                $crate::graph::Operation::parents_of(&self.$($field).+, base)
            }
            fn parent_of(&self, kind: &$crate::IdType) -> Option<($crate::IdType, usize)> {
                $crate::graph::Operation::parent_of(&self.$($field).+, kind)
            }
        }
    };
}

pub(crate) use delegate_market;
pub(crate) use delegate_operation;

pub mod arrow;
pub mod book;
pub mod candle;
pub mod column;
pub mod element;
pub mod element_column;
pub(crate) mod facts;
pub mod iterator;
pub mod kind;
pub mod market;
pub mod market_column;
pub mod market_data;
pub mod operation;
pub mod operation_column;
#[cfg(feature = "http")]
pub mod serve;
pub mod trade;
pub mod view;

pub use book::{BookEvent, BookIterator, SnapshotEvent};
pub use candle::{Candle, CandleIterator, CandleOptions, Ohlc};
pub use column::EventColumn;
pub use element::{Element, Event};
pub use element_column::ElementColumn;
pub use iterator::EventIterator;
pub use kind::MarketKind;
pub use market::{FxRates, Market, Metadata, Operation, empty_fxrates, empty_metadata};
pub use market_column::MarketColumn;
pub use market_data::MarketData;
pub use operation::{
    BookRef, Execution, ExecutionEvent, ExecutionKind, MdUpdateAction, OperationElement,
    OperationEvent, OperationKind, Order, OrderEvent, OrderKind, Quote, QuoteEvent, QuoteKind,
};
pub use operation_column::OperationColumn;
#[cfg(feature = "http")]
pub use serve::{BookQuery, BookService, BookServiceOptions, BookTable};
pub use trade::TradeEvent;
pub use view::MarketView;
