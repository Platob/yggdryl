//! The crate-private holders: the graph vocabulary as plain fields, for the
//! leaf that wants nothing more.
//!
//! [`MarketFacts`] holds every fact [`Element`] and [`Market`] name and no
//! instant: a book level, a side's summary, an undated entry.
//! [`MarketEventFacts`] adds the clocks and the state an [`Event`] answers, so
//! it is a market event: a book, a level dated at an instant.
//! [`OperationFacts`] and [`OperationEventFacts`] add the eight
//! facts an [`Operation`] states - its category, how long it stands,
//! whether it can trade, its account, user and own identifiers, and its two
//! lanes - to each, so an undated operation and a dated one embed the
//! slim holder and convert to its view by a move, never a copy.
//!
//! `Default` states nothing: a price and a quantity of nothing in no currency
//! (`XXX`), no unit, a side of `UNKNOWN`, no identifiers, a `00UNKNOWN`
//! state at the epoch, and the nil identity until [`Element::finalize`]
//! derives one from the facts.

use smol_str::SmolStr;

use super::market::{Lane, Metadata, empty_metadata, restating_operation};
use super::{Element, Event, Market, Operation};
use crate::idmap::IdMap;
use crate::securityid::{SecType, SecurityId, SecurityIds};
use crate::{Ccy, CfiCode, Decimal18, MicCode, Result, Side, State, TimeInForce, Unit, Uuid};

/// Every fact [`Element`] and [`Market`] name, as plain fields, with no
/// instant: what a book level, a side's summary or an undated entry is.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MarketFacts {
    curruuid: Uuid,
    crossuuid: Uuid,
    crosscode: String,
    currhashcode: u64,
    crosshashcode: u64,
    srcuuids: Vec<Uuid>,
    price: Option<Decimal18>,
    currency: Ccy,
    quantity: Option<Decimal18>,
    unit: Unit,
    side: Side,
    securityids: SecurityIds,
    cficode: Option<CfiCode>,
    miccode: Option<MicCode>,
    lastpx: Option<Decimal18>,
    lastqty: Option<Decimal18>,
    avgpx: Option<Decimal18>,
    cumqty: Option<Decimal18>,
    leavesqty: Option<Decimal18>,
    prevpx: Option<Decimal18>,
    prevqty: Option<Decimal18>,
    spotrate: Option<Decimal18>,
    forwardpoints: Option<Decimal18>,
    ticker: Option<SmolStr>,
    metadata: Option<Box<Metadata>>,
}

impl Default for MarketFacts {
    /// An element stating nothing.
    fn default() -> Self {
        Self {
            curruuid: Uuid::default(),
            crossuuid: Uuid::default(),
            crosscode: String::new(),
            currhashcode: 0,
            crosshashcode: 0,
            srcuuids: Vec::new(),
            price: None,
            currency: Ccy::none(),
            quantity: None,
            unit: Unit::none(),
            side: Side::Unknown,
            securityids: SecurityIds::default(),
            cficode: None,
            miccode: None,
            lastpx: None,
            lastqty: None,
            avgpx: None,
            cumqty: None,
            leavesqty: None,
            prevpx: None,
            prevqty: None,
            spotrate: None,
            forwardpoints: None,
            ticker: None,
            metadata: None,
        }
    }
}

impl Element for MarketFacts {
    fn get_curruuid(&self) -> Uuid {
        self.curruuid
    }

    fn set_curruuid(&mut self, curruuid: Uuid) {
        self.curruuid = curruuid;
    }

    fn get_crossuuid(&self) -> Uuid {
        self.crossuuid
    }

    fn set_crossuuid(&mut self, crossuuid: Uuid) {
        self.crossuuid = crossuuid;
    }

    fn get_crosscode(&self) -> &str {
        &self.crosscode
    }

    fn set_crosscode(&mut self, crosscode: String) {
        self.crosscode = crosscode;
    }

    fn get_currhashcode(&self) -> u64 {
        self.currhashcode
    }

    fn set_currhashcode(&mut self, hashcode: u64) {
        self.currhashcode = hashcode;
    }

    fn get_crosshashcode(&self) -> u64 {
        self.crosshashcode
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.crosshashcode = crosshashcode;
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        &self.srcuuids
    }

    fn set_srcuuids(&mut self, mut sources: Vec<Uuid>) {
        super::element::canonicalize_uuids(&mut sources);
        self.srcuuids = sources;
    }

    /// An element with no instant and no predecessor states no order.
    fn is_after(&self, _: &Self) -> bool {
        false
    }

    fn finalize(&mut self) {
        self.fill_market();
        self.sync_cross();
        self.currhashcode = self.digest_market().as_u64();
        self.curruuid = Uuid::from_v8(u128::from(self.currhashcode));
        self.crossuuid = self.cross_uuid();
    }

    fn with_previous(mut self, previous: &Self) -> Option<Self> {
        if previous.curruuid == self.curruuid {
            return None;
        }
        let mut changed = super::element::follow_element(&mut self, previous);
        changed |= super::market::follow_market(&mut self, previous);
        if !changed {
            return None;
        }
        self.finalize();
        Some(self)
    }

    fn merge_with(self, other: &Self) -> Option<Self> {
        self.merging_market(other)
    }
}

impl Market for MarketFacts {
    fn get_price(&self) -> Option<Decimal18> {
        self.price
    }

    fn set_price(&mut self, price: Option<Decimal18>) {
        self.price = price;
    }

    fn get_currency(&self) -> &Ccy {
        &self.currency
    }

    fn set_currency(&mut self, currency: Ccy) {
        self.currency = currency;
    }

    fn get_quantity(&self) -> Option<Decimal18> {
        self.quantity
    }

    fn set_quantity(&mut self, quantity: Option<Decimal18>) {
        self.quantity = quantity;
    }

    fn get_unit(&self) -> &Unit {
        &self.unit
    }

    fn set_unit(&mut self, unit: Unit) {
        self.unit = unit;
    }

    fn get_side(&self) -> Side {
        self.side
    }

    fn set_side(&mut self, side: Side) {
        self.side = side;
    }

    fn get_securityids(&self) -> &SecurityIds {
        &self.securityids
    }

    fn set_securityids(&mut self, ids: SecurityIds) -> Result<()> {
        self.securityids = ids;
        Ok(())
    }

    fn insert_securityid(&mut self, id: SecurityId) -> Result<bool> {
        Ok(self.securityids.insert(id))
    }

    fn remove_securityid(&mut self, key: &SecType) -> Result<bool> {
        Ok(self.securityids.remove(key).is_some())
    }

    fn derive_securityid(&mut self, id: SecurityId) -> bool {
        self.securityids.insert(id)
    }

    fn get_cficode(&self) -> Option<&CfiCode> {
        self.cficode.as_ref()
    }

    /// A market keeps only a detailed classification: a code that says
    /// nothing past its category and group is stored as none.
    fn set_cficode(&mut self, code: Option<CfiCode>) {
        self.cficode = code.filter(|held| CfiCode::is_detailed(held.as_str()));
    }

    fn get_miccode(&self) -> Option<&MicCode> {
        self.miccode.as_ref()
    }

    fn set_miccode(&mut self, code: Option<MicCode>) {
        self.miccode = code;
    }

    fn get_lastpx(&self) -> Option<Decimal18> {
        self.lastpx
    }

    fn set_lastpx(&mut self, px: Option<Decimal18>) {
        self.lastpx = px;
    }

    fn get_lastqty(&self) -> Option<Decimal18> {
        self.lastqty
    }

    fn set_lastqty(&mut self, qty: Option<Decimal18>) {
        self.lastqty = qty;
    }

    fn get_avgpx(&self) -> Option<Decimal18> {
        self.avgpx
    }

    fn set_avgpx(&mut self, px: Option<Decimal18>) {
        self.avgpx = px;
    }

    fn get_cumqty(&self) -> Option<Decimal18> {
        self.cumqty
    }

    fn set_cumqty(&mut self, qty: Option<Decimal18>) {
        self.cumqty = qty;
    }

    fn get_leavesqty(&self) -> Option<Decimal18> {
        self.leavesqty
    }

    fn set_leavesqty(&mut self, qty: Option<Decimal18>) {
        self.leavesqty = qty;
    }

    fn get_prevpx(&self) -> Option<Decimal18> {
        self.prevpx
    }

    fn set_prevpx(&mut self, px: Option<Decimal18>) {
        self.prevpx = px;
    }

    fn get_prevqty(&self) -> Option<Decimal18> {
        self.prevqty
    }

    fn set_prevqty(&mut self, qty: Option<Decimal18>) {
        self.prevqty = qty;
    }

    fn get_spotrate(&self) -> Option<Decimal18> {
        self.spotrate
    }

    fn set_spotrate(&mut self, rate: Option<Decimal18>) {
        self.spotrate = rate;
    }

    fn get_forwardpoints(&self) -> Option<Decimal18> {
        self.forwardpoints
    }

    fn set_forwardpoints(&mut self, points: Option<Decimal18>) {
        self.forwardpoints = points;
    }

    fn get_ticker(&self) -> Option<&str> {
        self.ticker.as_deref()
    }

    fn set_ticker(&mut self, ticker: Option<SmolStr>) {
        self.ticker = ticker.filter(|held| !held.is_empty());
    }

    fn get_metadata(&self) -> &Metadata {
        match &self.metadata {
            Some(held) => held,
            None => empty_metadata(),
        }
    }

    fn set_metadata(&mut self, metadata: Option<Metadata>) {
        self.metadata = metadata.filter(|held| !held.is_empty()).map(Box::new);
    }
}

/// [`MarketFacts`] with the clocks and the state an event answers: a market
/// event as plain fields.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MarketEventFacts {
    market: MarketFacts,
    currunix: i64,
    state: State,
    seqnum: u64,
    creaunix: Option<i64>,
    execunix: Option<i64>,
    recdunix: Option<i64>,
    exprtime: Option<i64>,
    prevunix: Option<i64>,
    prevuuid: Option<Uuid>,
    snapunix: Option<i64>,
}

impl MarketEventFacts {
    /// An event that happened at `unix`, nanoseconds since the Unix epoch,
    /// stating nothing else yet: the identity is what [`Element::finalize`]
    /// derives once the facts are in.
    #[must_use]
    pub(crate) fn at(unix: i64) -> Self {
        Self {
            market: MarketFacts::default(),
            currunix: unix,
            state: State::unknown(),
            seqnum: 0,
            creaunix: None,
            execunix: None,
            recdunix: None,
            exprtime: None,
            prevunix: None,
            prevuuid: None,
            snapunix: None,
        }
    }

    /// Reprojects the generic event identities after one of their inputs
    /// changes. A UUIDv7 refusal retains the current identity, as
    /// [`Event::finalized`] does; the cross identity always follows the
    /// resulting current identity and cross hash.
    fn refresh_uuids(&mut self) {
        if let Ok(uuid) = self.time_uuid() {
            self.market.curruuid = uuid;
        }
        self.market.crossuuid = self.cross_uuid();
    }
}

impl Default for MarketEventFacts {
    /// An event at the epoch, stating nothing.
    fn default() -> Self {
        Self::at(0)
    }
}

impl From<MarketFacts> for MarketEventFacts {
    /// The market facts dated at the epoch.
    fn from(market: MarketFacts) -> Self {
        Self {
            market,
            ..Self::default()
        }
    }
}

impl Element for MarketEventFacts {
    fn get_curruuid(&self) -> Uuid {
        self.market.curruuid
    }

    fn set_curruuid(&mut self, curruuid: Uuid) {
        self.market.curruuid = curruuid;
    }

    fn get_crossuuid(&self) -> Uuid {
        self.market.crossuuid
    }

    fn set_crossuuid(&mut self, crossuuid: Uuid) {
        self.market.crossuuid = crossuuid;
    }

    fn get_crosscode(&self) -> &str {
        &self.market.crosscode
    }

    fn set_crosscode(&mut self, crosscode: String) {
        self.market.crosscode = crosscode;
        self.market.crosshashcode = if self.market.crosscode.is_empty() {
            0
        } else {
            super::element::crosshash(&self.market.crosscode)
        };
        self.refresh_uuids();
    }

    fn get_currhashcode(&self) -> u64 {
        self.market.currhashcode
    }

    fn set_currhashcode(&mut self, hashcode: u64) {
        self.market.currhashcode = hashcode;
        self.refresh_uuids();
    }

    fn get_crosshashcode(&self) -> u64 {
        self.market.crosshashcode
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.market.crosshashcode = crosshashcode;
        self.refresh_uuids();
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        &self.market.srcuuids
    }

    fn set_srcuuids(&mut self, mut sources: Vec<Uuid>) {
        super::element::canonicalize_uuids(&mut sources);
        self.market.srcuuids = sources;
    }

    fn is_after(&self, other: &Self) -> bool {
        self.currunix > other.currunix
    }

    fn finalize(&mut self) {
        self.fill_market();
        self.sync_cross();
        let hashcode = self.digest_market_event().as_u64();
        self.finalized(hashcode);
    }

    fn with_previous(self, previous: &Self) -> Option<Self> {
        self.following_market(previous)
    }

    fn merge_with(self, other: &Self) -> Option<Self> {
        self.merging_market_event(other)
    }
}

impl Event for MarketEventFacts {
    fn get_currunix(&self) -> i64 {
        self.currunix
    }

    fn set_currunix(&mut self, unix: i64) {
        self.currunix = unix;
        self.refresh_uuids();
    }

    fn get_state(&self) -> &State {
        &self.state
    }

    fn set_state(&mut self, state: State) {
        self.state = state;
    }

    fn get_seqnum(&self) -> u64 {
        self.seqnum
    }

    fn set_seqnum(&mut self, seqnum: u64) {
        self.seqnum = seqnum;
        self.refresh_uuids();
    }

    fn get_creaunix(&self) -> Option<i64> {
        self.creaunix
    }

    fn set_creaunix(&mut self, unix: Option<i64>) {
        self.creaunix = unix;
    }

    fn get_execunix(&self) -> Option<i64> {
        self.execunix
    }

    fn set_execunix(&mut self, unix: Option<i64>) {
        self.execunix = unix;
    }

    fn get_recdunix(&self) -> Option<i64> {
        self.recdunix
    }

    fn set_recdunix(&mut self, unix: Option<i64>) {
        self.recdunix = unix;
    }

    fn get_exprtime(&self) -> Option<i64> {
        self.exprtime
    }

    fn set_exprtime(&mut self, unix: Option<i64>) {
        self.exprtime = unix;
    }

    fn get_prevunix(&self) -> Option<i64> {
        self.prevunix
    }

    fn set_prevunix(&mut self, unix: Option<i64>) {
        self.prevunix = unix;
    }

    fn get_prevuuid(&self) -> Option<Uuid> {
        self.prevuuid
    }

    fn set_prevuuid(&mut self, uuid: Option<Uuid>) {
        self.prevuuid = uuid;
    }

    fn get_snapunix(&self) -> Option<i64> {
        self.snapunix
    }

    fn set_snapunix(&mut self, unix: Option<i64>) {
        self.snapunix = unix;
    }

    fn restating(self, live: &Self) -> Self {
        super::market::restating_market(self, live)
    }

    fn finalized(&mut self, hashcode: u64) {
        self.market.currhashcode = hashcode;
        self.refresh_uuids();
    }
}

delegate_market!(MarketEventFacts, market);

/// The eight facts an operation adds to the market's, as plain fields.
#[derive(Clone, Debug, Default, PartialEq)]
struct OperationExtras {
    marketoperationid: Option<i32>,
    tif: Option<TimeInForce>,
    tradable: Option<bool>,
    accountids: IdMap,
    userids: IdMap,
    altids: IdMap,
    bid: Option<Box<Lane>>,
    ask: Option<Box<Lane>>,
}

/// `impl Operation` over an [`OperationExtras`] field.
macro_rules! operation_extras {
    ($type:ty, $($field:ident).+) => {
        impl Operation for $type {
            fn get_marketoperationid(&self) -> Option<i32> {
                self.$($field).+.marketoperationid
            }
            fn set_marketoperationid(&mut self, marketoperationid: Option<i32>) {
                self.$($field).+.marketoperationid = marketoperationid;
            }
            fn get_tif(&self) -> Option<&TimeInForce> {
                self.$($field).+.tif.as_ref()
            }
            fn set_tif(&mut self, tif: Option<TimeInForce>) {
                self.$($field).+.tif = tif;
            }
            fn get_tradable(&self) -> Option<bool> {
                self.$($field).+.tradable
            }
            fn set_tradable(&mut self, tradable: Option<bool>) {
                self.$($field).+.tradable = tradable;
            }
            fn get_accountids(&self) -> &IdMap {
                &self.$($field).+.accountids
            }
            fn set_accountids(&mut self, ids: IdMap) -> Result<()> {
                self.$($field).+.accountids = ids;
                Ok(())
            }
            fn insert_accountid(&mut self, key: &str, value: &str) -> Result<bool> {
                self.$($field).+.accountids.insert(key, value)
            }
            fn remove_accountid(&mut self, key: &str) -> Result<bool> {
                Ok(self.$($field).+.accountids.remove(key).is_some())
            }
            fn get_userids(&self) -> &IdMap {
                &self.$($field).+.userids
            }
            fn set_userids(&mut self, ids: IdMap) -> Result<()> {
                self.$($field).+.userids = ids;
                Ok(())
            }
            fn insert_userid(&mut self, key: &str, value: &str) -> Result<bool> {
                self.$($field).+.userids.insert(key, value)
            }
            fn remove_userid(&mut self, key: &str) -> Result<bool> {
                Ok(self.$($field).+.userids.remove(key).is_some())
            }
            fn get_altids(&self) -> &IdMap {
                &self.$($field).+.altids
            }
            fn set_altids(&mut self, ids: IdMap) -> Result<()> {
                self.$($field).+.altids = ids;
                Ok(())
            }
            fn insert_altid(&mut self, key: &str, value: &str) -> Result<bool> {
                self.$($field).+.altids.insert(key, value)
            }
            fn remove_altid(&mut self, key: &str) -> Result<bool> {
                Ok(self.$($field).+.altids.remove(key).is_some())
            }
            fn get_bid(&self) -> Option<&Lane> {
                self.$($field).+.bid.as_deref()
            }
            fn set_bid(&mut self, lane: Option<Lane>) {
                self.$($field).+.bid = lane.and_then(Lane::stated).map(Box::new);
            }
            fn get_ask(&self) -> Option<&Lane> {
                self.$($field).+.ask.as_deref()
            }
            fn set_ask(&mut self, lane: Option<Lane>) {
                self.$($field).+.ask = lane.and_then(Lane::stated).map(Box::new);
            }
        }
    };
}

/// [`MarketFacts`] with the operation's facts and no instant: an undated
/// operation entry as plain fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct OperationFacts {
    market: MarketFacts,
    operation: OperationExtras,
}

impl OperationFacts {
    /// This entry dated at `unix`, nanoseconds since the Unix epoch: a
    /// move, finalized by the caller once the clocks are in.
    #[must_use]
    pub(crate) fn at(self, unix: i64) -> OperationEventFacts {
        let mut event = MarketEventFacts::from(self.market);
        event.currunix = unix;
        OperationEventFacts {
            event,
            operation: self.operation,
        }
    }
}

impl From<MarketFacts> for OperationFacts {
    /// The market facts with no operation fact stated.
    fn from(market: MarketFacts) -> Self {
        Self {
            market,
            operation: OperationExtras::default(),
        }
    }
}

impl Element for OperationFacts {
    fn get_curruuid(&self) -> Uuid {
        self.market.curruuid
    }

    fn set_curruuid(&mut self, curruuid: Uuid) {
        self.market.curruuid = curruuid;
    }

    fn get_crossuuid(&self) -> Uuid {
        self.market.crossuuid
    }

    fn set_crossuuid(&mut self, crossuuid: Uuid) {
        self.market.crossuuid = crossuuid;
    }

    fn get_crosscode(&self) -> &str {
        &self.market.crosscode
    }

    fn set_crosscode(&mut self, crosscode: String) {
        self.market.crosscode = crosscode;
    }

    fn get_currhashcode(&self) -> u64 {
        self.market.currhashcode
    }

    fn set_currhashcode(&mut self, hashcode: u64) {
        self.market.currhashcode = hashcode;
    }

    fn get_crosshashcode(&self) -> u64 {
        self.market.crosshashcode
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.market.crosshashcode = crosshashcode;
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        &self.market.srcuuids
    }

    fn set_srcuuids(&mut self, sources: Vec<Uuid>) {
        self.market.set_srcuuids(sources);
    }

    /// An entry with no instant and no predecessor states no order.
    fn is_after(&self, _: &Self) -> bool {
        false
    }

    fn finalize(&mut self) {
        self.fill_market();
        self.fill_operation();
        self.sync_cross();
        self.market.currhashcode = self.digest_operation().as_u64();
        self.market.curruuid = Uuid::from_v8(u128::from(self.market.currhashcode));
        self.market.crossuuid = self.cross_uuid();
    }

    fn with_previous(mut self, previous: &Self) -> Option<Self> {
        if previous.market.curruuid == self.market.curruuid {
            return None;
        }
        let mut changed = super::element::follow_element(&mut self, previous);
        changed |= super::market::follow_market_of_operation(&mut self, previous);
        changed |= super::market::follow_operation(&mut self, previous);
        if !changed {
            return None;
        }
        self.finalize();
        Some(self)
    }

    fn merge_with(self, other: &Self) -> Option<Self> {
        self.merging_operation(other)
    }
}

delegate_market!(OperationFacts, market);
operation_extras!(OperationFacts, operation);

/// [`MarketEventFacts`] with the operation's facts: a dated operation as
/// plain fields, what an order, a quote, an execution and a message hold.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct OperationEventFacts {
    event: MarketEventFacts,
    operation: OperationExtras,
}

impl OperationEventFacts {
    /// An operation that happened at `unix`, nanoseconds since the Unix
    /// epoch, stating nothing else yet.
    #[must_use]
    pub(crate) fn at(unix: i64) -> Self {
        Self {
            event: MarketEventFacts::at(unix),
            operation: OperationExtras::default(),
        }
    }

    /// This operation as the event alone, the operation facts dropped: a
    /// move.
    #[must_use]
    pub(crate) fn into_event(self) -> MarketEventFacts {
        self.event
    }

    /// This operation without its clocks: a move.
    #[must_use]
    pub(crate) fn into_entry(self) -> OperationFacts {
        OperationFacts {
            market: self.event.market,
            operation: self.operation,
        }
    }
}

impl From<MarketEventFacts> for OperationEventFacts {
    /// The event with no operation fact stated.
    fn from(event: MarketEventFacts) -> Self {
        Self {
            event,
            operation: OperationExtras::default(),
        }
    }
}

impl Element for OperationEventFacts {
    fn get_curruuid(&self) -> Uuid {
        self.event.get_curruuid()
    }

    fn set_curruuid(&mut self, curruuid: Uuid) {
        self.event.set_curruuid(curruuid);
    }

    fn get_crossuuid(&self) -> Uuid {
        self.event.get_crossuuid()
    }

    fn set_crossuuid(&mut self, crossuuid: Uuid) {
        self.event.set_crossuuid(crossuuid);
    }

    fn get_crosscode(&self) -> &str {
        self.event.get_crosscode()
    }

    fn set_crosscode(&mut self, crosscode: String) {
        self.event.set_crosscode(crosscode);
    }

    fn get_currhashcode(&self) -> u64 {
        self.event.get_currhashcode()
    }

    fn set_currhashcode(&mut self, hashcode: u64) {
        self.event.set_currhashcode(hashcode);
    }

    fn get_crosshashcode(&self) -> u64 {
        self.event.get_crosshashcode()
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.event.set_crosshashcode(crosshashcode);
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        self.event.get_srcuuids()
    }

    fn set_srcuuids(&mut self, sources: Vec<Uuid>) {
        self.event.set_srcuuids(sources);
    }

    fn is_after(&self, other: &Self) -> bool {
        self.event.is_after(&other.event)
    }

    fn finalize(&mut self) {
        self.fill_market();
        self.fill_operation();
        self.sync_cross();
        let hashcode = self.digest_operation_event().as_u64();
        self.finalized(hashcode);
    }

    fn with_previous(self, previous: &Self) -> Option<Self> {
        self.following_operation(previous)
    }

    fn merge_with(self, other: &Self) -> Option<Self> {
        self.merging_operation_event(other)
    }
}

delegate_event!(
    OperationEventFacts,
    event,
    restating =
        |this: OperationEventFacts, live: &OperationEventFacts| { restating_operation(this, live) }
);
delegate_market!(OperationEventFacts, event.market);
operation_extras!(OperationEventFacts, operation);

fn copy_element<T: Element + ?Sized, E: Element + ?Sized>(this: &mut T, other: &E) {
    this.set_curruuid(other.get_curruuid());
    this.set_crossuuid(other.get_crossuuid());
    this.set_crosscode(other.get_crosscode().to_owned());
    this.set_currhashcode(other.get_currhashcode());
    this.set_crosshashcode(other.get_crosshashcode());
    this.set_srcuuids(other.get_srcuuids().to_vec());
}

fn copy_event<T: Event + ?Sized, E: Event + ?Sized>(this: &mut T, other: &E) {
    this.set_currunix(other.get_currunix());
    this.set_state(other.get_state().clone());
    this.set_seqnum(other.get_seqnum());
    this.set_creaunix(other.get_creaunix());
    this.set_execunix(other.get_execunix());
    this.set_recdunix(other.get_recdunix());
    this.set_exprtime(other.get_exprtime());
    this.set_prevunix(other.get_prevunix());
    this.set_prevuuid(other.get_prevuuid());
    this.set_snapunix(other.get_snapunix());
}

fn copy_market<T: Market + ?Sized, E: Market + ?Sized>(this: &mut T, other: &E) {
    this.set_price(other.get_price());
    this.set_currency(other.get_currency().clone());
    this.set_quantity(other.get_quantity());
    this.set_unit(other.get_unit().clone());
    this.set_side(other.get_side());
    // A plain holder refuses no identifier; a view of a store may, and a
    // copy takes what it can.
    let _ = this.set_securityids(other.get_securityids().clone());
    this.set_cficode(other.get_cficode().cloned());
    this.set_miccode(other.get_miccode().cloned());
    this.set_lastpx(other.get_lastpx());
    this.set_lastqty(other.get_lastqty());
    this.set_avgpx(other.get_avgpx());
    this.set_cumqty(other.get_cumqty());
    this.set_leavesqty(other.get_leavesqty());
    this.set_prevpx(other.get_prevpx());
    this.set_prevqty(other.get_prevqty());
    this.set_spotrate(other.get_spotrate());
    this.set_forwardpoints(other.get_forwardpoints());
    this.set_ticker(other.get_ticker().map(SmolStr::new));
    this.set_metadata(Some(other.get_metadata().clone()));
}

fn copy_operation<T: Operation + ?Sized, E: Operation + ?Sized>(this: &mut T, other: &E) {
    this.set_marketoperationid(other.get_marketoperationid());
    this.set_tif(other.get_tif().cloned());
    this.set_tradable(other.get_tradable());
    let _ = this.set_accountids(other.get_accountids().clone());
    let _ = this.set_userids(other.get_userids().clone());
    let _ = this.set_altids(other.get_altids().clone());
    this.set_bid(other.get_bid().cloned());
    this.set_ask(other.get_ask().cloned());
}

impl<E: Element + Market + ?Sized> From<&E> for MarketFacts {
    fn from(other: &E) -> Self {
        let mut this = Self::default();
        copy_element(&mut this, other);
        copy_market(&mut this, other);
        this
    }
}

impl<E: Event + Market + ?Sized> From<&E> for MarketEventFacts {
    fn from(other: &E) -> Self {
        let mut this = Self::default();
        copy_element(&mut this, other);
        copy_event(&mut this, other);
        copy_market(&mut this, other);
        // The setters above keep a derived event coherent while it is
        // mutated. Conversion copies the exact identities the source states,
        // including an assigned identity, after every dependency is in place.
        this.market.curruuid = other.get_curruuid();
        this.market.crossuuid = other.get_crossuuid();
        this
    }
}

impl<E: Element + Operation + ?Sized> From<&E> for OperationFacts {
    fn from(other: &E) -> Self {
        let mut this = Self::default();
        copy_element(&mut this, other);
        copy_market(&mut this, other);
        copy_operation(&mut this, other);
        this
    }
}

impl<E: Event + Operation + ?Sized> From<&E> for OperationEventFacts {
    fn from(other: &E) -> Self {
        let mut this = Self::default();
        copy_element(&mut this, other);
        copy_event(&mut this, other);
        copy_market(&mut this, other);
        copy_operation(&mut this, other);
        this.event.market.curruuid = other.get_curruuid();
        this.event.market.crossuuid = other.get_crossuuid();
        this
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/graph/facts.rs` pins and a caller cannot reach.
    //!
    //! The four holders are crate-private: a caller reaches their facts
    //! through a leaf that holds one. What each holder answers on its own -
    //! its size, the identity its finalize derives, the order it stands in
    //! and how it merges - is observed here, each answer in role order:
    //! market, market event, operation, operation event.
    use super::{MarketEventFacts, MarketFacts, OperationEventFacts, OperationFacts};
    use crate::Uuid;
    use crate::graph::Element;

    /// Each holder stating `crosscode` - the dated two at `unix` - and
    /// nothing else.
    fn stated(
        crosscode: &str,
        unix: i64,
    ) -> (
        MarketFacts,
        MarketEventFacts,
        OperationFacts,
        OperationEventFacts,
    ) {
        let mut market = MarketFacts::default();
        market.set_crosscode(crosscode.to_owned());
        let mut event = MarketEventFacts::at(unix);
        event.set_crosscode(crosscode.to_owned());
        let mut operation = OperationFacts::default();
        operation.set_crosscode(crosscode.to_owned());
        let mut operation_event = OperationEventFacts::at(unix);
        operation_event.set_crosscode(crosscode.to_owned());
        (market, event, operation, operation_event)
    }

    /// The four holders' sizes.
    #[must_use]
    pub fn sizes() -> [usize; 4] {
        use std::mem::size_of;
        [
            size_of::<MarketFacts>(),
            size_of::<MarketEventFacts>(),
            size_of::<OperationFacts>(),
            size_of::<OperationEventFacts>(),
        ]
    }

    /// Each holder stating `crosscode` - the dated two at `unix` -
    /// finalized: the identity it derives and the code it digests to.
    #[must_use]
    pub fn finalized(crosscode: &str, unix: i64) -> [(Uuid, u64); 4] {
        fn settle<E: Element>(mut element: E) -> (Uuid, u64) {
            element.finalize();
            (element.get_curruuid(), element.get_currhashcode())
        }
        let (market, event, operation, operation_event) = stated(crosscode, unix);
        [
            settle(market),
            settle(event),
            settle(operation),
            settle(operation_event),
        ]
    }

    /// Whether each holder stated at `later` is after the same holder
    /// stated at `earlier`, both under one cross code.
    #[must_use]
    pub fn is_after(earlier: i64, later: i64) -> [bool; 4] {
        fn after<E: Element>(mut this: E, mut other: E) -> bool {
            this.finalize();
            other.finalize();
            this.is_after(&other)
        }
        let (a, b, c, d) = stated("ORDER", later);
        let (e, f, g, h) = stated("ORDER", earlier);
        [after(a, e), after(b, f), after(c, g), after(d, h)]
    }

    /// Each holder merged with another statement of itself that adds
    /// `source`: the sources the merge answers, or `None` where it
    /// answers nothing; and whether it merges with a stranger under
    /// another cross code.
    #[must_use]
    pub fn merged(source: Uuid) -> [(Option<Vec<Uuid>>, bool); 4] {
        fn merge<E: Element + Clone>(
            mut this: E,
            mut stranger: E,
            source: Uuid,
        ) -> (Option<Vec<Uuid>>, bool) {
            this.finalize();
            stranger.finalize();
            let mut restated = this.clone();
            restated.set_srcuuids(vec![source]);
            let merged = this
                .clone()
                .merge_with(&restated)
                .map(|merged| merged.get_srcuuids().to_vec());
            (merged, this.merge_with(&stranger).is_some())
        }
        let (a, b, c, d) = stated("ORDER", 1);
        let (e, f, g, h) = stated("OTHER", 1);
        [
            merge(a, e, source),
            merge(b, f, source),
            merge(c, g, source),
            merge(d, h, source),
        ]
    }
}
