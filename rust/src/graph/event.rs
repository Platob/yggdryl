//! The holders: the graph vocabulary as plain fields, for the holder that
//! wants nothing more.
//!
//! [`MarketData`] holds every fact [`Element`] and [`Market`] name and no
//! instant: a book level, a side's summary, an undated entry.
//! [`MarketEventData`] adds the clocks and the state an [`Event`] answers, so
//! it is a [`MarketEvent`]: a book, a level dated at an instant.
//! [`MarketOperationData`] and [`MarketOperationEventData`] add the eight
//! facts a [`MarketOperation`] states - its category, how long it stands,
//! whether it can trade, its account, user and own identifiers, and its two
//! lanes - to each, so an operation entry and an operation event embed the
//! slim holder and convert to its view by a move, never a copy.
//!
//! `Default` states nothing: a price and a quantity of nothing in no currency
//! (`XXX`), no unit, a side of `UNKNOWN`, no identifiers, a `00UNKNOWN`
//! state at the epoch, and the nil identity until [`Element::finalize`]
//! derives one from the facts.
//!
//! ```
//! use yggdryl::graph::{Element, Event, Market, MarketEvent, MarketEventData};
//! use yggdryl::{Ccy, Decimal18, Side};
//!
//! let mut event = MarketEventData::at(1_700_000_000_000_000_000);
//! event.set_crosscode("O-1".to_owned());
//! event.set_side(Side::Buy);
//! event.set_price(Some(Decimal18::from_int(101)));
//! event.set_quantity(Some(Decimal18::from_int(5)));
//! event.set_currency(Ccy::new("USD")?);
//! event.finalize();
//! assert_ne!(event.get_currhashcode(), 0);
//! assert_eq!(event.get_crossuuid(), event.cross_uuid());
//! // The same facts stated again digest to the same code, whatever holds them.
//! let mut again = MarketEventData::at(1_700_000_000_000_000_000);
//! again.set_crosscode("O-1".to_owned());
//! again.set_side(Side::Buy);
//! again.set_price(Some(Decimal18::from_int(101)));
//! again.set_quantity(Some(Decimal18::from_int(5)));
//! again.set_currency(Ccy::new("USD")?);
//! again.finalize();
//! assert_eq!(again.get_curruuid(), event.get_curruuid());
//! assert_eq!(event.digest_market_event().as_u64(), event.get_currhashcode());
//! # Ok::<(), yggdryl::Error>(())
//! ```

use smol_str::SmolStr;

use super::market::{Lane, Metadata, empty_metadata, restating_market, restating_operation};
use super::{Element, Event, Market, MarketEvent, MarketOperation, MarketOperationEvent};
use crate::idmap::IdMap;
use crate::securityid::{SecType, SecurityId, SecurityIds};
use crate::{Ccy, CfiCode, Decimal18, MicCode, Result, Side, State, TimeInForce, Unit, Uuid};

/// Every fact [`Element`] and [`Market`] name, as plain fields, with no
/// instant: what a book level, a side's summary or an undated entry is.
#[derive(Clone, Debug, PartialEq)]
pub struct MarketData {
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

impl Default for MarketData {
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

impl Element for MarketData {
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

impl Market for MarketData {
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

/// [`MarketData`] with the clocks and the state an event answers: a
/// [`MarketEvent`] as plain fields.
#[derive(Clone, Debug, PartialEq)]
pub struct MarketEventData {
    market: MarketData,
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

impl MarketEventData {
    /// An event that happened at `unix`, nanoseconds since the Unix epoch,
    /// stating nothing else yet: the identity is what [`Element::finalize`]
    /// derives once the facts are in.
    #[must_use]
    pub fn at(unix: i64) -> Self {
        Self {
            market: MarketData::default(),
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

    /// The undated market facts this event holds.
    #[must_use]
    pub fn market(&self) -> &MarketData {
        &self.market
    }

    /// This event's market facts alone, the clocks dropped: a move.
    #[must_use]
    pub fn into_market(self) -> MarketData {
        self.market
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

impl Default for MarketEventData {
    /// An event at the epoch, stating nothing.
    fn default() -> Self {
        Self::at(0)
    }
}

impl From<MarketData> for MarketEventData {
    /// The market facts dated at the epoch.
    fn from(market: MarketData) -> Self {
        Self {
            market,
            ..Self::default()
        }
    }
}

impl Element for MarketEventData {
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

impl Event for MarketEventData {
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
        restating_market(self, live)
    }

    fn finalized(&mut self, hashcode: u64) {
        self.market.currhashcode = hashcode;
        self.refresh_uuids();
    }
}

delegate_market!(MarketEventData, market);

/// The eight facts an operation adds to the market's, as plain fields.
#[derive(Clone, Debug, Default, PartialEq)]
struct OperationFacts {
    marketoperationid: Option<i32>,
    tif: Option<TimeInForce>,
    tradable: Option<bool>,
    accountids: IdMap,
    userids: IdMap,
    altids: IdMap,
    bid: Option<Box<Lane>>,
    ask: Option<Box<Lane>>,
}

/// `impl MarketOperation` over an [`OperationFacts`] field.
macro_rules! operation_facts {
    ($type:ty, $($field:ident).+) => {
        impl MarketOperation for $type {
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

/// [`MarketData`] with the operation's facts and no instant: an undated
/// operation entry as plain fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarketOperationData {
    market: MarketData,
    operation: OperationFacts,
}

impl MarketOperationData {
    /// The slim market facts this entry holds.
    #[must_use]
    pub fn market(&self) -> &MarketData {
        &self.market
    }

    /// This entry's market facts alone: a move.
    #[must_use]
    pub fn into_market(self) -> MarketData {
        self.market
    }

    /// This entry dated at `unix`, nanoseconds since the Unix epoch: a
    /// move, finalized by the caller once the clocks are in.
    #[must_use]
    pub fn at(self, unix: i64) -> MarketOperationEventData {
        let mut event = MarketEventData::from(self.market);
        event.currunix = unix;
        MarketOperationEventData {
            event,
            operation: self.operation,
        }
    }
}

impl From<MarketData> for MarketOperationData {
    /// The market facts with no operation fact stated.
    fn from(market: MarketData) -> Self {
        Self {
            market,
            operation: OperationFacts::default(),
        }
    }
}

impl Element for MarketOperationData {
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

delegate_market!(MarketOperationData, market);
operation_facts!(MarketOperationData, operation);

/// [`MarketEventData`] with the operation's facts: a
/// [`MarketOperationEvent`] as plain fields, what an order, a quote, an
/// execution and a message hold.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarketOperationEventData {
    event: MarketEventData,
    operation: OperationFacts,
}

impl MarketOperationEventData {
    /// An operation that happened at `unix`, nanoseconds since the Unix
    /// epoch, stating nothing else yet.
    #[must_use]
    pub fn at(unix: i64) -> Self {
        Self {
            event: MarketEventData::at(unix),
            operation: OperationFacts::default(),
        }
    }

    /// The event this operation is, without its operation facts.
    #[must_use]
    pub fn event(&self) -> &MarketEventData {
        &self.event
    }

    /// The slim market facts this operation holds.
    #[must_use]
    pub fn market(&self) -> &MarketData {
        &self.event.market
    }

    /// This operation as the event alone, the operation facts dropped: a
    /// move.
    #[must_use]
    pub fn into_event(self) -> MarketEventData {
        self.event
    }

    /// This operation without its clocks: a move.
    #[must_use]
    pub fn into_entry(self) -> MarketOperationData {
        MarketOperationData {
            market: self.event.market,
            operation: self.operation,
        }
    }
}

impl From<MarketEventData> for MarketOperationEventData {
    /// The event with no operation fact stated.
    fn from(event: MarketEventData) -> Self {
        Self {
            event,
            operation: OperationFacts::default(),
        }
    }
}

impl Element for MarketOperationEventData {
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
    MarketOperationEventData,
    event,
    restating = |this: MarketOperationEventData, live: &MarketOperationEventData| {
        restating_operation(this, live)
    }
);
delegate_market!(MarketOperationEventData, event.market);
operation_facts!(MarketOperationEventData, operation);

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

fn copy_operation<T: MarketOperation + ?Sized, E: MarketOperation + ?Sized>(
    this: &mut T,
    other: &E,
) {
    this.set_marketoperationid(other.get_marketoperationid());
    this.set_tif(other.get_tif().cloned());
    this.set_tradable(other.get_tradable());
    let _ = this.set_accountids(other.get_accountids().clone());
    let _ = this.set_userids(other.get_userids().clone());
    let _ = this.set_altids(other.get_altids().clone());
    this.set_bid(other.get_bid().cloned());
    this.set_ask(other.get_ask().cloned());
}

impl<E: Element + Market + ?Sized> From<&E> for MarketData {
    fn from(other: &E) -> Self {
        let mut this = Self::default();
        copy_element(&mut this, other);
        copy_market(&mut this, other);
        this
    }
}

impl<E: MarketEvent + ?Sized> From<&E> for MarketEventData {
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

impl<E: Element + MarketOperation + ?Sized> From<&E> for MarketOperationData {
    fn from(other: &E) -> Self {
        let mut this = Self::default();
        copy_element(&mut this, other);
        copy_market(&mut this, other);
        copy_operation(&mut this, other);
        this
    }
}

impl<E: MarketOperationEvent + ?Sized> From<&E> for MarketOperationEventData {
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
