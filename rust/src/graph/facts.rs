//! The crate-private holders: the graph vocabulary as plain fields, for the
//! leaf that wants nothing more.
//!
//! [`MarketFacts`] holds every fact [`Element`] and [`Market`] name and no
//! instant: an undated entry.
//! [`MarketEventFacts`] adds the clocks and the state an [`Event`] answers, so
//! it is a market event: a book, a snapshot control dated at an instant.
//! [`OperationFacts`] and [`OperationEventFacts`] add the three
//! facts an [`Operation`] states - how long it stands,
//! whether it can trade and its own identifiers - to each, so an
//! undated operation and a dated one embed the
//! slim holder and convert to its view by a move, never a copy.
//!
//! `Default` states nothing: a price and a quantity of nothing in no currency
//! (`XXX`), no unit, a side of `UNKNOWN`, no identifiers, a `UNKNOWN`
//! state at the epoch, and the nil identity until [`Element::finalize`]
//! derives one from the facts.
//!
//! Each holder also carries the [`MarketDataKind`] of the leaf that holds
//! it, stamped by that leaf - `UNKN` until one does. The kind is what
//! decides whether the cross code carries the side ([`MarketDataKind::is_sided`]):
//! only an order's, a quote's and an execution's does. It is never digested:
//! a leaf feeds its own kind where its identity needs one.

use smol_str::SmolStr;

use super::market::{FxRates, Metadata, empty_fxrates, empty_metadata, restating_operation};
use super::{Element, Event, Market, Operation};
use crate::idmap::IdMap;
use crate::securityid::{SecType, SecurityId, SecurityIds};
use crate::{Ccy, Cfi, Decimal, MarketDataKind, Mic, Result, Side, State, TimeInForce, Unit, Uuid};

/// Every fact [`Element`] and [`Market`] name, as plain fields, with no
/// instant: what an undated entry is.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MarketFacts {
    /// The category of the leaf holding these facts: whether the cross code
    /// is stored under the side.
    kind: MarketDataKind,
    curruuid: Uuid,
    crossuuid: Uuid,
    crosscode: String,
    currhashcode: u64,
    crosshashcode: u64,
    srcuuids: Vec<Uuid>,
    price: Option<Decimal>,
    currency: Ccy,
    quantity: Option<Decimal>,
    unit: Unit,
    side: Side,
    securityids: SecurityIds,
    /// The sources `securityids` holds by derivation alone: provenance,
    /// never content.
    derived: Derived,
    cficode: Option<Cfi>,
    miccode: Option<Mic>,
    execunix: Option<i64>,
    lastpx: Option<Decimal>,
    lastqty: Option<Decimal>,
    avgpx: Option<Decimal>,
    cumqty: Option<Decimal>,
    leavesqty: Option<Decimal>,
    prevpx: Option<Decimal>,
    prevqty: Option<Decimal>,
    spotrate: Option<Decimal>,
    forwardpoints: Option<Decimal>,
    /// Target currency to the rate to divide by; `None` for none, so a
    /// holder stating no rate pays one pointer and no allocation.
    fxrates: Option<Box<FxRates>>,
    /// The bid and the ask, boxed: most elements state neither and pay one
    /// pointer.
    bidask: Option<Box<BidAsk>>,
    ticker: Option<SmolStr>,
    metadata: Option<Box<Metadata>>,
}

/// The six bid and ask facts of a market element, held together because
/// an element states them together or not at all.
#[derive(Clone, Debug, Default, PartialEq)]
struct BidAsk {
    bidpx: Option<Decimal>,
    bidqty: Option<Decimal>,
    bidccy: Option<Ccy>,
    askpx: Option<Decimal>,
    askqty: Option<Decimal>,
    askccy: Option<Ccy>,
}

impl BidAsk {
    /// Whether every fact is unstated, which is when the holder drops it.
    fn is_empty(&self) -> bool {
        self.bidpx.is_none()
            && self.bidqty.is_none()
            && self.bidccy.is_none()
            && self.askpx.is_none()
            && self.askqty.is_none()
            && self.askccy.is_none()
    }
}

impl Default for MarketFacts {
    /// An element stating nothing.
    fn default() -> Self {
        Self {
            kind: MarketDataKind::Unknown,
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
            derived: Derived::default(),
            cficode: None,
            miccode: None,
            execunix: None,
            lastpx: None,
            lastqty: None,
            avgpx: None,
            cumqty: None,
            leavesqty: None,
            prevpx: None,
            prevqty: None,
            spotrate: None,
            forwardpoints: None,
            fxrates: None,
            bidask: None,
            ticker: None,
            metadata: None,
        }
    }
}

/// Which of an element's identifiers it holds by derivation alone - what
/// its ISIN implies, what a lifecycle learned under it - and never states:
/// one bit per position of the sorted set, so tracking allocates nothing. A
/// stated identifier replaces a derived one, and removing the ISIN, which
/// every one hangs on, takes them all back. A position past the 64 a mask
/// counts holds as stated; the set has 33 known sources.
///
/// Provenance rather than content: a row does not carry it, so two elements
/// holding the same identifiers are equal whichever of them were derived.
#[derive(Clone, Copy, Debug, Default)]
struct Derived(u64);

impl PartialEq for Derived {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Derived {
    fn bit(position: usize) -> u64 {
        1_u64.checked_shl(position as u32).unwrap_or(0)
    }

    /// Every position before `position`.
    fn below(position: usize) -> u64 {
        Self::bit(position).wrapping_sub(1)
    }

    fn contains(self, position: usize) -> bool {
        self.0 & Self::bit(position) != 0
    }

    /// An identifier entered at `position`: the ones after it move up.
    fn entered(&mut self, position: usize, derived: bool) {
        let below = Self::below(position);
        self.0 = (self.0 & below) | ((self.0 & !below) << 1);
        if derived {
            self.0 |= Self::bit(position);
        }
    }

    /// The identifier at `position` is stated now.
    fn stated(&mut self, position: usize) {
        self.0 &= !Self::bit(position);
    }

    /// The identifier at `position` left: the ones after it move down.
    fn left(&mut self, position: usize) {
        let below = Self::below(position);
        self.0 = (self.0 & below) | ((self.0 >> 1) & !below);
    }
}

impl MarketFacts {
    /// Stores `crosscode`, under this element's side where its kind is
    /// sided and as given where it is not: the one place a holder's cross
    /// code is written.
    fn state_crosscode(&mut self, crosscode: String) {
        self.crosscode = match self.sided_crosscode(&crosscode) {
            std::borrow::Cow::Borrowed(_) => crosscode,
            std::borrow::Cow::Owned(sided) => sided,
        };
    }

    /// Stores the held cross code under the side again, and the cross hash
    /// and element with it, where the kind is sided and the prefix moved:
    /// what a side taken and a sided kind stamped each run. Never strips a
    /// prefix: an unsided kind keeps its code as given.
    fn reprefix(&mut self) {
        if let std::borrow::Cow::Owned(sided) = self.sided_crosscode(&self.crosscode) {
            self.crosscode = sided;
            self.crosshashcode = super::element::crosshash(&self.crosscode);
            self.crossuuid = self.cross_uuid();
        }
    }

    /// The category the holding leaf stamped.
    pub(crate) fn marketdatakind(&self) -> MarketDataKind {
        self.kind
    }

    /// Stamps the category of the leaf that holds these facts, storing the
    /// cross code under the side where the new kind is sided.
    pub(crate) fn set_marketdatakind(&mut self, kind: MarketDataKind) {
        self.kind = kind;
        self.reprefix();
    }

    /// Writes one bid or ask fact, allocating the holder on the first stated
    /// one and dropping it when the last is cleared.
    fn set_bidask(&mut self, write: impl FnOnce(&mut BidAsk)) {
        let mut held = self.bidask.take().map(|held| *held).unwrap_or_default();
        write(&mut held);
        if !held.is_empty() {
            self.bidask = Some(Box::new(held));
        }
    }

    /// The bid and ask facts, or none.
    fn bidask(&self) -> Option<&BidAsk> {
        self.bidask.as_deref()
    }

    /// Where `key` stands in the sorted identifiers, or where it would.
    fn slot(&self, key: &str) -> std::result::Result<usize, usize> {
        self.securityids
            .binary_search_by(|held| held.key_str().cmp(key))
    }

    /// Whether `key`'s identifier is held by derivation alone.
    pub(crate) fn is_derived_securityid(&self, key: &SecType) -> bool {
        self.slot(key.as_str())
            .is_ok_and(|position| self.derived.contains(position))
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
        self.state_crosscode(crosscode);
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
    fn get_price(&self) -> Option<Decimal> {
        self.price
    }

    fn set_price(&mut self, price: Option<Decimal>) {
        self.price = price;
    }

    fn get_currency(&self) -> &Ccy {
        &self.currency
    }

    fn set_currency(&mut self, currency: Ccy) {
        self.currency = currency;
    }

    fn get_quantity(&self) -> Option<Decimal> {
        self.quantity
    }

    fn set_quantity(&mut self, quantity: Option<Decimal>) {
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

    /// A side taken re-prefixes the cross code of a sided kind, so setting
    /// the side after the code converges on the code set after the side.
    fn set_side(&mut self, side: Side) {
        self.side = side;
        self.reprefix();
    }

    fn is_sided(&self) -> bool {
        self.kind.is_sided()
    }

    fn get_securityids(&self) -> &SecurityIds {
        &self.securityids
    }

    fn set_securityids(&mut self, ids: SecurityIds) -> Result<()> {
        self.securityids = ids;
        self.derived = Derived::default();
        Ok(())
    }

    /// A stated identifier fills an absent source or replaces a derived one.
    fn insert_securityid(&mut self, id: SecurityId) -> Result<bool> {
        match self.slot(id.key_str()) {
            Ok(position) if self.derived.contains(position) => {
                self.derived.stated(position);
                self.securityids.set(id);
                Ok(true)
            }
            Ok(_) => Ok(false),
            Err(position) => {
                self.derived.entered(position, false);
                Ok(self.securityids.insert(id))
            }
        }
    }

    fn remove_securityid(&mut self, key: &SecType) -> Result<bool> {
        let Ok(position) = self.slot(key.as_str()) else {
            return Ok(false);
        };
        self.securityids.remove(key);
        self.derived.left(position);
        if key.as_str() == "ISIN" {
            for position in (0..self.securityids.len()).rev() {
                if self.derived.contains(position) {
                    let derived = self.securityids[position].sectype();
                    self.securityids.remove(&derived);
                    self.derived.left(position);
                }
            }
        }
        Ok(true)
    }

    fn derive_securityid(&mut self, id: SecurityId) -> bool {
        match self.slot(id.key_str()) {
            Ok(_) => false,
            Err(position) => {
                self.derived.entered(position, true);
                self.securityids.insert(id)
            }
        }
    }

    fn get_cficode(&self) -> Option<&Cfi> {
        self.cficode.as_ref()
    }

    /// A market keeps only a detailed classification: a code that says
    /// nothing past its category and group is stored as none.
    fn set_cficode(&mut self, code: Option<Cfi>) {
        self.cficode = code.filter(|held| Cfi::is_detailed(held.as_str()));
    }

    fn get_miccode(&self) -> Option<&Mic> {
        self.miccode.as_ref()
    }

    fn set_miccode(&mut self, code: Option<Mic>) {
        self.miccode = code;
    }

    fn get_execunix(&self) -> Option<i64> {
        self.execunix
    }

    fn set_execunix(&mut self, unix: Option<i64>) {
        self.execunix = unix;
    }

    fn get_lastpx(&self) -> Option<Decimal> {
        self.lastpx
    }

    fn set_lastpx(&mut self, px: Option<Decimal>) {
        self.lastpx = px;
    }

    fn get_lastqty(&self) -> Option<Decimal> {
        self.lastqty
    }

    fn set_lastqty(&mut self, qty: Option<Decimal>) {
        self.lastqty = qty;
    }

    fn get_avgpx(&self) -> Option<Decimal> {
        self.avgpx
    }

    fn set_avgpx(&mut self, px: Option<Decimal>) {
        self.avgpx = px;
    }

    fn get_cumqty(&self) -> Option<Decimal> {
        self.cumqty
    }

    fn set_cumqty(&mut self, qty: Option<Decimal>) {
        self.cumqty = qty;
    }

    fn get_leavesqty(&self) -> Option<Decimal> {
        self.leavesqty
    }

    fn set_leavesqty(&mut self, qty: Option<Decimal>) {
        self.leavesqty = qty;
    }

    fn get_prevpx(&self) -> Option<Decimal> {
        self.prevpx
    }

    fn set_prevpx(&mut self, px: Option<Decimal>) {
        self.prevpx = px;
    }

    fn get_prevqty(&self) -> Option<Decimal> {
        self.prevqty
    }

    fn set_prevqty(&mut self, qty: Option<Decimal>) {
        self.prevqty = qty;
    }

    fn get_spotrate(&self) -> Option<Decimal> {
        self.spotrate
    }

    fn set_spotrate(&mut self, rate: Option<Decimal>) {
        self.spotrate = rate;
    }

    fn get_forwardpoints(&self) -> Option<Decimal> {
        self.forwardpoints
    }

    fn set_forwardpoints(&mut self, points: Option<Decimal>) {
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

    fn get_fxrates(&self) -> &FxRates {
        match &self.fxrates {
            Some(held) => held,
            None => empty_fxrates(),
        }
    }

    fn set_fxrates(&mut self, rates: FxRates) {
        self.fxrates = (!rates.is_empty()).then(|| Box::new(rates));
    }

    fn get_bidpx(&self) -> Option<Decimal> {
        self.bidask().and_then(|held| held.bidpx)
    }

    fn set_bidpx(&mut self, px: Option<Decimal>) {
        self.set_bidask(|held| held.bidpx = px);
    }

    fn get_bidqty(&self) -> Option<Decimal> {
        self.bidask().and_then(|held| held.bidqty)
    }

    fn set_bidqty(&mut self, qty: Option<Decimal>) {
        self.set_bidask(|held| held.bidqty = qty);
    }

    fn get_bidccy(&self) -> Option<&Ccy> {
        self.bidask().and_then(|held| held.bidccy.as_ref())
    }

    fn set_bidccy(&mut self, ccy: Option<Ccy>) {
        self.set_bidask(|held| held.bidccy = ccy);
    }

    fn get_askpx(&self) -> Option<Decimal> {
        self.bidask().and_then(|held| held.askpx)
    }

    fn set_askpx(&mut self, px: Option<Decimal>) {
        self.set_bidask(|held| held.askpx = px);
    }

    fn get_askqty(&self) -> Option<Decimal> {
        self.bidask().and_then(|held| held.askqty)
    }

    fn set_askqty(&mut self, qty: Option<Decimal>) {
        self.set_bidask(|held| held.askqty = qty);
    }

    fn get_askccy(&self) -> Option<&Ccy> {
        self.bidask().and_then(|held| held.askccy.as_ref())
    }

    fn set_askccy(&mut self, ccy: Option<Ccy>) {
        self.set_bidask(|held| held.askccy = ccy);
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
    recdunix: Option<i64>,
    exprunix: Option<i64>,
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
            recdunix: None,
            exprunix: None,
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

impl MarketEventFacts {
    /// The category the holding leaf stamped.
    pub(crate) fn marketdatakind(&self) -> MarketDataKind {
        self.market.marketdatakind()
    }

    /// Stamps the category of the leaf that holds these facts.
    pub(crate) fn set_marketdatakind(&mut self, kind: MarketDataKind) {
        self.market.set_marketdatakind(kind);
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
        self.market.state_crosscode(crosscode);
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

    fn get_recdunix(&self) -> Option<i64> {
        self.recdunix
    }

    fn set_recdunix(&mut self, unix: Option<i64>) {
        self.recdunix = unix;
    }

    fn get_exprunix(&self) -> Option<i64> {
        self.exprunix
    }

    fn set_exprunix(&mut self, unix: Option<i64>) {
        self.exprunix = unix;
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

/// The four facts an operation adds to the market's, as plain fields.
#[derive(Clone, Debug, Default, PartialEq)]
struct OperationExtras {
    tif: Option<TimeInForce>,
    tradable: Option<bool>,
    altids: IdMap,
    accountids: IdMap,
}

/// `impl Operation` over an [`OperationExtras`] field.
macro_rules! operation_extras {
    ($type:ty, $($field:ident).+) => {
        impl Operation for $type {
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
    /// Stamps the category of the leaf that holds these facts.
    pub(crate) fn set_marketdatakind(&mut self, kind: MarketDataKind) {
        self.market.set_marketdatakind(kind);
    }

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
        self.market.state_crosscode(crosscode);
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

    /// The market filled, then digested with the operation's facts.
    fn finalize(&mut self) {
        self.fill_market();
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
        changed |= super::market::follow_market(&mut self, previous);
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

    /// Whether `key`'s identifier is held by derivation alone.
    pub(crate) fn is_derived_securityid(&self, key: &SecType) -> bool {
        self.event.market.is_derived_securityid(key)
    }

    /// The category the holding leaf stamped.
    pub(crate) fn marketdatakind(&self) -> MarketDataKind {
        self.event.marketdatakind()
    }

    /// Stamps the category of the leaf that holds these facts.
    pub(crate) fn set_marketdatakind(&mut self, kind: MarketDataKind) {
        self.event.set_marketdatakind(kind);
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

    /// The market filled, then digested with the event's and the
    /// operation's facts.
    fn finalize(&mut self) {
        self.fill_market();
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

/// The cross code a copy of `other` takes: a sided source's without the
/// side prefix its holder gave it - which a sided leaf built over the copy
/// gives again, and an unsided one never states - and an unsided source's
/// as it stands.
fn copied_crosscode<E: Element + Market + ?Sized>(other: &E) -> &str {
    if other.is_sided() {
        super::market::unsided_crosscode(other.get_crosscode())
    } else {
        other.get_crosscode()
    }
}

/// Brings the copy's cross hash and element in step with the code it took,
/// where that is a sided source's base rather than the source's own code;
/// whether it did.
fn resync_copied<T: Element + ?Sized, E: Element + Market + ?Sized>(
    this: &mut T,
    other: &E,
) -> bool {
    let resynced = this.get_crosscode().len() != other.get_crosscode().len();
    if resynced {
        this.sync_cross();
    }
    resynced
}

fn copy_element<T: Element + ?Sized, E: Element + Market + ?Sized>(this: &mut T, other: &E) {
    this.set_curruuid(other.get_curruuid());
    this.set_crossuuid(other.get_crossuuid());
    this.set_crosscode(copied_crosscode(other).to_owned());
    this.set_currhashcode(other.get_currhashcode());
    this.set_crosshashcode(other.get_crosshashcode());
    this.set_srcuuids(other.get_srcuuids().to_vec());
}

fn copy_event<T: Event + ?Sized, E: Event + ?Sized>(this: &mut T, other: &E) {
    this.set_currunix(other.get_currunix());
    this.set_state(*other.get_state());
    this.set_seqnum(other.get_seqnum());
    this.set_creaunix(other.get_creaunix());
    this.set_recdunix(other.get_recdunix());
    this.set_exprunix(other.get_exprunix());
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
    this.set_execunix(other.get_execunix());
    this.set_lastpx(other.get_lastpx());
    this.set_lastqty(other.get_lastqty());
    this.set_avgpx(other.get_avgpx());
    this.set_cumqty(other.get_cumqty());
    this.set_leavesqty(other.get_leavesqty());
    this.set_prevpx(other.get_prevpx());
    this.set_prevqty(other.get_prevqty());
    this.set_spotrate(other.get_spotrate());
    this.set_forwardpoints(other.get_forwardpoints());
    this.set_fxrates(other.get_fxrates().clone());
    this.set_bidpx(other.get_bidpx());
    this.set_bidqty(other.get_bidqty());
    this.set_bidccy(other.get_bidccy().cloned());
    this.set_askpx(other.get_askpx());
    this.set_askqty(other.get_askqty());
    this.set_askccy(other.get_askccy().cloned());
    this.set_ticker(other.get_ticker().map(SmolStr::new));
    this.set_metadata(Some(other.get_metadata().clone()));
}

fn copy_operation<T: Operation + ?Sized, E: Operation + ?Sized>(this: &mut T, other: &E) {
    this.set_tif(other.get_tif().cloned());
    this.set_tradable(other.get_tradable());
    let _ = this.set_altids(other.get_altids().clone());
    let _ = this.set_accountids(other.get_accountids().clone());
}

impl<E: Element + Market + ?Sized> From<&E> for MarketFacts {
    fn from(other: &E) -> Self {
        let mut this = Self::default();
        copy_element(&mut this, other);
        copy_market(&mut this, other);
        let _ = resync_copied(&mut this, other);
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
        let resynced = resync_copied(&mut this, other);
        this.market.curruuid = other.get_curruuid();
        this.market.crossuuid = if resynced {
            this.cross_uuid()
        } else {
            other.get_crossuuid()
        };
        this
    }
}

impl<E: Element + Operation + ?Sized> From<&E> for OperationFacts {
    fn from(other: &E) -> Self {
        let mut this = Self::default();
        copy_element(&mut this, other);
        copy_market(&mut this, other);
        copy_operation(&mut this, other);
        let _ = resync_copied(&mut this, other);
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
        let resynced = resync_copied(&mut this, other);
        this.event.market.curruuid = other.get_curruuid();
        this.event.market.crossuuid = if resynced {
            this.cross_uuid()
        } else {
            other.get_crossuuid()
        };
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
