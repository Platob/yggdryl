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
//! side, the price and quantity, what it last traded - and
//! [`MarketOperation`] adds what an operation states: its category, how long
//! it stands, its account, user and own identifiers, and its two lanes.
//! [`MarketEvent`] and [`MarketOperationEvent`] are the blankets over an
//! event that is one or the other. The traits state signatures and the
//! provided readings - no storage - so a message, a chain entry and a
//! lifecycle incarnation can each be an element without the graph owning
//! any of them; [`MarketData`], [`MarketEventData`], [`MarketOperationData`]
//! and [`MarketOperationEventData`] hold the facts as plain fields for the
//! holder that wants nothing more, and [`Operation`] is the one operation
//! type - an order, a quote or an execution by its [`OperationKind`] - a
//! [`Book`] holds. The one walk, [`EventIterator`], reads operations in
//! their order and states each as the one after the live element it
//! follows. [`EventColumn`] is the sixteen columns every generated schema
//! of an event states, [`MarketColumn`] the nineteen of a market and
//! [`OperationColumn`] the eight of an operation - one per fact the traits
//! answer, under one name and one datatype each - so a text line's batch,
//! a FIX row and a chained message join on them without a mapping.

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
            fn get_execunix(&self) -> Option<i64> {
                $crate::graph::Event::get_execunix(&self.$($field).+)
            }
            fn set_execunix(&mut self, unix: Option<i64>) {
                $crate::graph::Event::set_execunix(&mut self.$($field).+, unix);
            }
            fn get_recdunix(&self) -> Option<i64> {
                $crate::graph::Event::get_recdunix(&self.$($field).+)
            }
            fn set_recdunix(&mut self, unix: Option<i64>) {
                $crate::graph::Event::set_recdunix(&mut self.$($field).+, unix);
            }
            fn get_exprtime(&self) -> Option<i64> {
                $crate::graph::Event::get_exprtime(&self.$($field).+)
            }
            fn set_exprtime(&mut self, unix: Option<i64>) {
                $crate::graph::Event::set_exprtime(&mut self.$($field).+, unix);
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
            fn get_price(&self) -> $crate::Decimal18 {
                $crate::graph::Market::get_price(&self.$($field).+)
            }
            fn set_price(&mut self, price: $crate::Decimal18) {
                $crate::graph::Market::set_price(&mut self.$($field).+, price);
            }
            fn get_currency(&self) -> &$crate::Ccy {
                $crate::graph::Market::get_currency(&self.$($field).+)
            }
            fn set_currency(&mut self, currency: $crate::Ccy) {
                $crate::graph::Market::set_currency(&mut self.$($field).+, currency);
            }
            fn get_quantity(&self) -> $crate::Decimal18 {
                $crate::graph::Market::get_quantity(&self.$($field).+)
            }
            fn set_quantity(&mut self, quantity: $crate::Decimal18) {
                $crate::graph::Market::set_quantity(&mut self.$($field).+, quantity);
            }
            fn get_unit(&self) -> &$crate::Unit {
                $crate::graph::Market::get_unit(&self.$($field).+)
            }
            fn set_unit(&mut self, unit: $crate::Unit) {
                $crate::graph::Market::set_unit(&mut self.$($field).+, unit);
            }
            fn get_side(&self) -> $crate::Side {
                $crate::graph::Market::get_side(&self.$($field).+)
            }
            fn set_side(&mut self, side: $crate::Side) {
                $crate::graph::Market::set_side(&mut self.$($field).+, side);
            }
            fn get_securityids(&self) -> &$crate::securityid::SecurityIds {
                $crate::graph::Market::get_securityids(&self.$($field).+)
            }
            fn set_securityids(
                &mut self,
                ids: $crate::securityid::SecurityIds,
            ) -> $crate::Result<()> {
                $crate::graph::Market::set_securityids(&mut self.$($field).+, ids)
            }
            fn insert_securityid(
                &mut self,
                id: $crate::securityid::SecurityId,
            ) -> $crate::Result<bool> {
                $crate::graph::Market::insert_securityid(&mut self.$($field).+, id)
            }
            fn remove_securityid(
                &mut self,
                key: &$crate::securityid::SecType,
            ) -> $crate::Result<bool> {
                $crate::graph::Market::remove_securityid(&mut self.$($field).+, key)
            }
            fn derive_securityid(&mut self, id: $crate::securityid::SecurityId) -> bool {
                $crate::graph::Market::derive_securityid(&mut self.$($field).+, id)
            }
            fn get_cficode(&self) -> Option<&$crate::CfiCode> {
                $crate::graph::Market::get_cficode(&self.$($field).+)
            }
            fn set_cficode(&mut self, code: Option<$crate::CfiCode>) {
                $crate::graph::Market::set_cficode(&mut self.$($field).+, code);
            }
            fn get_miccode(&self) -> Option<&$crate::MicCode> {
                $crate::graph::Market::get_miccode(&self.$($field).+)
            }
            fn set_miccode(&mut self, code: Option<$crate::MicCode>) {
                $crate::graph::Market::set_miccode(&mut self.$($field).+, code);
            }
            fn get_lastpx(&self) -> Option<$crate::Decimal18> {
                $crate::graph::Market::get_lastpx(&self.$($field).+)
            }
            fn set_lastpx(&mut self, px: Option<$crate::Decimal18>) {
                $crate::graph::Market::set_lastpx(&mut self.$($field).+, px);
            }
            fn get_lastqty(&self) -> Option<$crate::Decimal18> {
                $crate::graph::Market::get_lastqty(&self.$($field).+)
            }
            fn set_lastqty(&mut self, qty: Option<$crate::Decimal18>) {
                $crate::graph::Market::set_lastqty(&mut self.$($field).+, qty);
            }
            fn get_avgpx(&self) -> Option<$crate::Decimal18> {
                $crate::graph::Market::get_avgpx(&self.$($field).+)
            }
            fn set_avgpx(&mut self, px: Option<$crate::Decimal18>) {
                $crate::graph::Market::set_avgpx(&mut self.$($field).+, px);
            }
            fn get_cumqty(&self) -> Option<$crate::Decimal18> {
                $crate::graph::Market::get_cumqty(&self.$($field).+)
            }
            fn set_cumqty(&mut self, qty: Option<$crate::Decimal18>) {
                $crate::graph::Market::set_cumqty(&mut self.$($field).+, qty);
            }
            fn get_leavesqty(&self) -> Option<$crate::Decimal18> {
                $crate::graph::Market::get_leavesqty(&self.$($field).+)
            }
            fn set_leavesqty(&mut self, qty: Option<$crate::Decimal18>) {
                $crate::graph::Market::set_leavesqty(&mut self.$($field).+, qty);
            }
            fn get_prevpx(&self) -> Option<$crate::Decimal18> {
                $crate::graph::Market::get_prevpx(&self.$($field).+)
            }
            fn set_prevpx(&mut self, px: Option<$crate::Decimal18>) {
                $crate::graph::Market::set_prevpx(&mut self.$($field).+, px);
            }
            fn get_prevqty(&self) -> Option<$crate::Decimal18> {
                $crate::graph::Market::get_prevqty(&self.$($field).+)
            }
            fn set_prevqty(&mut self, qty: Option<$crate::Decimal18>) {
                $crate::graph::Market::set_prevqty(&mut self.$($field).+, qty);
            }
            fn get_spotrate(&self) -> Option<$crate::Decimal18> {
                $crate::graph::Market::get_spotrate(&self.$($field).+)
            }
            fn set_spotrate(&mut self, rate: Option<$crate::Decimal18>) {
                $crate::graph::Market::set_spotrate(&mut self.$($field).+, rate);
            }
            fn get_forwardpoints(&self) -> Option<$crate::Decimal18> {
                $crate::graph::Market::get_forwardpoints(&self.$($field).+)
            }
            fn set_forwardpoints(&mut self, points: Option<$crate::Decimal18>) {
                $crate::graph::Market::set_forwardpoints(&mut self.$($field).+, points);
            }
            fn get_ticker(&self) -> Option<&str> {
                $crate::graph::Market::get_ticker(&self.$($field).+)
            }
            fn set_ticker(&mut self, ticker: Option<smol_str::SmolStr>) {
                $crate::graph::Market::set_ticker(&mut self.$($field).+, ticker);
            }
            fn get_metadata(&self) -> &$crate::graph::Metadata {
                $crate::graph::Market::get_metadata(&self.$($field).+)
            }
            fn set_metadata(&mut self, metadata: Option<$crate::graph::Metadata>) {
                $crate::graph::Market::set_metadata(&mut self.$($field).+, metadata);
            }
        }
    };
}

/// `impl MarketOperation` forwarding every fact to a field that is a
/// `MarketOperation`.
macro_rules! delegate_operation {
    ($type:ty, $($field:ident).+) => {
        impl $crate::graph::MarketOperation for $type {
            fn get_marketoperationid(&self) -> Option<i32> {
                $crate::graph::MarketOperation::get_marketoperationid(&self.$($field).+)
            }
            fn set_marketoperationid(&mut self, marketoperationid: Option<i32>) {
                $crate::graph::MarketOperation::set_marketoperationid(
                    &mut self.$($field).+,
                    marketoperationid,
                );
            }
            fn get_tif(&self) -> Option<&$crate::TimeInForce> {
                $crate::graph::MarketOperation::get_tif(&self.$($field).+)
            }
            fn set_tif(&mut self, tif: Option<$crate::TimeInForce>) {
                $crate::graph::MarketOperation::set_tif(&mut self.$($field).+, tif);
            }
            fn get_tradable(&self) -> Option<bool> {
                $crate::graph::MarketOperation::get_tradable(&self.$($field).+)
            }
            fn set_tradable(&mut self, tradable: Option<bool>) {
                $crate::graph::MarketOperation::set_tradable(&mut self.$($field).+, tradable);
            }
            fn get_accountids(&self) -> &$crate::idmap::IdMap {
                $crate::graph::MarketOperation::get_accountids(&self.$($field).+)
            }
            fn set_accountids(&mut self, ids: $crate::idmap::IdMap) -> $crate::Result<()> {
                $crate::graph::MarketOperation::set_accountids(&mut self.$($field).+, ids)
            }
            fn insert_accountid(&mut self, key: &str, value: &str) -> $crate::Result<bool> {
                $crate::graph::MarketOperation::insert_accountid(
                    &mut self.$($field).+,
                    key,
                    value,
                )
            }
            fn remove_accountid(&mut self, key: &str) -> $crate::Result<bool> {
                $crate::graph::MarketOperation::remove_accountid(&mut self.$($field).+, key)
            }
            fn get_userids(&self) -> &$crate::idmap::IdMap {
                $crate::graph::MarketOperation::get_userids(&self.$($field).+)
            }
            fn set_userids(&mut self, ids: $crate::idmap::IdMap) -> $crate::Result<()> {
                $crate::graph::MarketOperation::set_userids(&mut self.$($field).+, ids)
            }
            fn insert_userid(&mut self, key: &str, value: &str) -> $crate::Result<bool> {
                $crate::graph::MarketOperation::insert_userid(&mut self.$($field).+, key, value)
            }
            fn remove_userid(&mut self, key: &str) -> $crate::Result<bool> {
                $crate::graph::MarketOperation::remove_userid(&mut self.$($field).+, key)
            }
            fn get_altids(&self) -> &$crate::idmap::IdMap {
                $crate::graph::MarketOperation::get_altids(&self.$($field).+)
            }
            fn set_altids(&mut self, ids: $crate::idmap::IdMap) -> $crate::Result<()> {
                $crate::graph::MarketOperation::set_altids(&mut self.$($field).+, ids)
            }
            fn insert_altid(&mut self, key: &str, value: &str) -> $crate::Result<bool> {
                $crate::graph::MarketOperation::insert_altid(&mut self.$($field).+, key, value)
            }
            fn remove_altid(&mut self, key: &str) -> $crate::Result<bool> {
                $crate::graph::MarketOperation::remove_altid(&mut self.$($field).+, key)
            }
            fn get_bid(&self) -> Option<&$crate::graph::Lane> {
                $crate::graph::MarketOperation::get_bid(&self.$($field).+)
            }
            fn set_bid(&mut self, lane: Option<$crate::graph::Lane>) {
                $crate::graph::MarketOperation::set_bid(&mut self.$($field).+, lane);
            }
            fn get_ask(&self) -> Option<&$crate::graph::Lane> {
                $crate::graph::MarketOperation::get_ask(&self.$($field).+)
            }
            fn set_ask(&mut self, lane: Option<$crate::graph::Lane>) {
                $crate::graph::MarketOperation::set_ask(&mut self.$($field).+, lane);
            }
        }
    };
}

pub(crate) use delegate_market;
pub(crate) use delegate_operation;

pub mod arrow;
pub mod book;
pub mod column;
pub mod element;
pub mod event;
pub mod iterator;
pub mod market;
pub mod market_column;
pub mod operation;
pub mod operation_column;
pub mod trade;

pub use book::{Book, BookControl, BookInput, BookIterator, BookSide, GLOBAL_SYMBOL};
pub use column::EventColumn;
pub use element::{Element, Event};
pub use event::{MarketData, MarketEventData, MarketOperationData, MarketOperationEventData};
pub use iterator::EventIterator;
pub use market::{
    FOLLOWED_ALTIDS, Lane, Market, MarketEvent, MarketOperation, MarketOperationEvent, Metadata,
    empty_metadata,
};
pub use market_column::MarketColumn;
pub use operation::{BookRef, MdUpdateAction, Operation, OperationEntry, OperationKind};
pub use operation_column::OperationColumn;
pub use trade::Trade;
