//! The market element and the market event held as plain fields: every
//! fact the traits name, and nothing else.

use std::collections::BTreeMap;

use super::{Element, Event, MarketElement, MarketEvent};
use crate::types::{Bloomberg, Cfi, Currency, Cusip, Decimal, Isin, Mic, Sedol, Side, State, Uuid};

/// The concrete market element: every fact [`Element`] and
/// [`MarketElement`] name, held as one field each, with no instant of its
/// own.
///
/// Its identity is derived: [`Element::finalize`] digests the facts the
/// traits know through [`MarketElement::digest_market`], records the code
/// and sets the identity that code derives, RFC 9562 UUIDv8 over it, so
/// two elements stating the same things are one identity. Its order is
/// lineage, as a node's is: it is after the elements it descends from, and
/// it follows another by descending from it.
///
/// A new element states nothing: no identity, no cross code, no names, no
/// parents, a price and a quantity of nothing in no currency
/// (`XXX`) and no unit, a side of `UNKNOWN`, no instrument named, no lane
/// stated. It is what any [`MarketElement`] converts into, dropping
/// whatever else that element states, and what a [`MarketEventData`] is
/// without its instants.
///
/// ```
/// use yggdryl::graph::{Element, Event, MarketElement, MarketElementData, MarketEventData};
/// use yggdryl::types::{Decimal, Side};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut element = MarketElementData::default();
/// element.set_crosscode("O-100".to_owned());
/// element.set_px("82.5".parse()?);
/// element.set_qty(Decimal::from_int(1_000));
/// element.set_side(Side::read("Buy")?);
/// element.finalize();
/// assert_ne!(element.get_hashcode(), 0);
/// assert_eq!(element.get_crossuuid(), element.cross_uuid());
/// // The same facts at an instant: the event states them and its own.
/// let mut event = MarketEventData::from(element.clone());
/// event.set_unix(1_700_000_000_000_000_000);
/// event.finalize();
/// assert_eq!(event.get_px(), element.get_px());
/// assert_eq!(event.get_crosscode(), "O-100");
/// // And back, through the signatures the two share: the event's instants
/// // drop, and the identity is what the shared facts derive.
/// let mut again = MarketElementData::from(&event);
/// again.finalize();
/// assert_eq!(again.get_curruuid(), element.get_curruuid());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct MarketElementData {
    curruuid: Uuid,
    crossuuid: Uuid,
    crosscode: String,
    hashcode: u64,
    crosshashcode: u64,
    identifiers: BTreeMap<String, String>,
    parentuuids: Vec<Uuid>,
    px: Decimal,
    currency: Currency,
    qty: Decimal,
    unit: String,
    side: Side,
    isincode: Option<Isin>,
    cusipcode: Option<Cusip>,
    sedolcode: Option<Sedol>,
    bloombergcode: Option<Bloomberg>,
    cficode: Option<Cfi>,
    miccode: Option<Mic>,
    lastpx: Option<Decimal>,
    lastqty: Option<Decimal>,
    avgpx: Option<Decimal>,
    cumqty: Option<Decimal>,
    leavesqty: Option<Decimal>,
    tif: Option<String>,
    tradable: Option<bool>,
    symbolticker: Option<String>,
    prevpx: Option<Decimal>,
    prevqty: Option<Decimal>,
    bidpx: Option<Decimal>,
    bidcurrency: Option<Currency>,
    bidqty: Option<Decimal>,
    bidunit: Option<String>,
    askpx: Option<Decimal>,
    askcurrency: Option<Currency>,
    askqty: Option<Decimal>,
    askunit: Option<String>,
}

impl Default for MarketElementData {
    /// An element stating nothing.
    fn default() -> Self {
        Self {
            curruuid: Uuid::default(),
            crossuuid: Uuid::default(),
            crosscode: String::new(),
            hashcode: 0,
            crosshashcode: 0,
            identifiers: BTreeMap::new(),
            parentuuids: Vec::new(),
            px: Decimal::ZERO,
            lastpx: None,
            lastqty: None,
            avgpx: None,
            cumqty: None,
            leavesqty: None,
            tif: None,
            tradable: None,
            symbolticker: None,
            prevpx: None,
            prevqty: None,
            currency: Currency::none(),
            qty: Decimal::ZERO,
            unit: String::new(),
            side: Side::unknown(),
            isincode: None,
            cusipcode: None,
            sedolcode: None,
            bloombergcode: None,
            cficode: None,
            miccode: None,
            bidpx: None,
            bidcurrency: None,
            bidqty: None,
            bidunit: None,
            askpx: None,
            askcurrency: None,
            askqty: None,
            askunit: None,
        }
    }
}

impl Element for MarketElementData {
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

    fn get_hashcode(&self) -> u64 {
        self.hashcode
    }

    fn set_hashcode(&mut self, hashcode: u64) {
        self.hashcode = hashcode;
    }

    fn get_crosshashcode(&self) -> u64 {
        self.crosshashcode
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.crosshashcode = crosshashcode;
    }

    fn get_identifiers(&self) -> &BTreeMap<String, String> {
        &self.identifiers
    }

    fn set_identifiers(&mut self, identifiers: BTreeMap<String, String>) {
        self.identifiers = identifiers;
    }

    fn get_parentuuids(&self) -> &[Uuid] {
        &self.parentuuids
    }

    fn set_parentuuids(&mut self, parents: Vec<Uuid>) {
        self.parentuuids = parents;
    }

    fn is_after(&self, other: &Self) -> bool {
        self.parentuuids.contains(&other.curruuid)
    }

    fn finalize(&mut self) {
        self.fill_market();
        self.sync_cross();
        self.hashcode = self.digest_market().as_u64();
        self.curruuid = Uuid::from_v8(u128::from(self.hashcode));
        self.crossuuid = self.cross_uuid();
    }

    fn with_previous(mut self, previous: &Self) -> Option<Self> {
        if previous.curruuid == self.curruuid {
            return None;
        }
        let mut changed = super::element::follow_element(&mut self, previous);
        changed |= super::element::follow_market(&mut self, previous);
        if !self.parentuuids.contains(&previous.curruuid) {
            self.parentuuids.push(previous.curruuid);
            changed = true;
        }
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

impl MarketElement for MarketElementData {
    fn get_px(&self) -> Decimal {
        self.px
    }

    fn set_px(&mut self, px: Decimal) {
        self.px = px;
    }

    fn get_currency(&self) -> &Currency {
        &self.currency
    }

    fn set_currency(&mut self, currency: Currency) {
        self.currency = currency;
    }

    fn get_qty(&self) -> Decimal {
        self.qty
    }

    fn set_qty(&mut self, qty: Decimal) {
        self.qty = qty;
    }

    fn get_unit(&self) -> &str {
        &self.unit
    }

    fn set_unit(&mut self, unit: String) {
        self.unit = unit;
    }

    fn get_side(&self) -> &Side {
        &self.side
    }

    fn set_side(&mut self, side: Side) {
        self.side = side;
    }

    fn get_isincode(&self) -> Option<&Isin> {
        self.isincode.as_ref()
    }

    fn set_isincode(&mut self, isincode: Option<Isin>) {
        self.isincode = isincode;
    }

    fn get_cusipcode(&self) -> Option<&Cusip> {
        self.cusipcode.as_ref()
    }

    fn set_cusipcode(&mut self, cusipcode: Option<Cusip>) {
        self.cusipcode = cusipcode;
    }

    fn get_sedolcode(&self) -> Option<&Sedol> {
        self.sedolcode.as_ref()
    }

    fn set_sedolcode(&mut self, sedolcode: Option<Sedol>) {
        self.sedolcode = sedolcode;
    }

    fn get_bloombergcode(&self) -> Option<&Bloomberg> {
        self.bloombergcode.as_ref()
    }

    fn set_bloombergcode(&mut self, bloombergcode: Option<Bloomberg>) {
        self.bloombergcode = bloombergcode;
    }

    fn get_cficode(&self) -> Option<&Cfi> {
        self.cficode.as_ref()
    }

    fn set_cficode(&mut self, cficode: Option<Cfi>) {
        self.cficode = cficode;
    }

    fn get_miccode(&self) -> Option<&Mic> {
        self.miccode.as_ref()
    }

    fn set_miccode(&mut self, miccode: Option<Mic>) {
        self.miccode = miccode;
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

    fn get_tif(&self) -> Option<&str> {
        self.tif.as_deref()
    }

    fn set_tif(&mut self, tif: Option<String>) {
        self.tif = tif;
    }

    fn get_tradable(&self) -> Option<bool> {
        self.tradable
    }

    fn set_tradable(&mut self, tradable: Option<bool>) {
        self.tradable = tradable;
    }

    fn get_symbolticker(&self) -> Option<&str> {
        self.symbolticker.as_deref()
    }

    fn set_symbolticker(&mut self, ticker: Option<String>) {
        self.symbolticker = ticker;
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

    fn get_bidpx(&self) -> Option<Decimal> {
        self.bidpx
    }

    fn set_bidpx(&mut self, px: Option<Decimal>) {
        self.bidpx = px;
    }

    fn get_bidcurrency(&self) -> Option<&Currency> {
        self.bidcurrency.as_ref()
    }

    fn set_bidcurrency(&mut self, currency: Option<Currency>) {
        self.bidcurrency = currency;
    }

    fn get_bidqty(&self) -> Option<Decimal> {
        self.bidqty
    }

    fn set_bidqty(&mut self, qty: Option<Decimal>) {
        self.bidqty = qty;
    }

    fn get_bidunit(&self) -> Option<&str> {
        self.bidunit.as_deref()
    }

    fn set_bidunit(&mut self, unit: Option<String>) {
        self.bidunit = unit;
    }

    fn get_askpx(&self) -> Option<Decimal> {
        self.askpx
    }

    fn set_askpx(&mut self, px: Option<Decimal>) {
        self.askpx = px;
    }

    fn get_askcurrency(&self) -> Option<&Currency> {
        self.askcurrency.as_ref()
    }

    fn set_askcurrency(&mut self, currency: Option<Currency>) {
        self.askcurrency = currency;
    }

    fn get_askqty(&self) -> Option<Decimal> {
        self.askqty
    }

    fn set_askqty(&mut self, qty: Option<Decimal>) {
        self.askqty = qty;
    }

    fn get_askunit(&self) -> Option<&str> {
        self.askunit.as_deref()
    }

    fn set_askunit(&mut self, unit: Option<String>) {
        self.askunit = unit;
    }
}

/// The concrete market event: every fact [`Element`], [`Event`] and
/// [`MarketElement`] name, held as one field each, so a type that is a
/// market event and more - a FIX message, a trade record - holds one of
/// these and delegates the traits to it rather than restating forty
/// accessors.
///
/// Its identity is derived: [`Element::finalize`] digests the facts the
/// traits know through [`MarketEvent::digest_market_event`] and resets the
/// code and the current identity from them, so two events stating the same
/// things at the same instant are one identity. A holder that has more
/// content to say - a message's body - finalizes itself by feeding that
/// content behind [`MarketEvent::digest_market_event`] and handing the code
/// to [`Event::finalized`], and leaves this event's own `finalize` for the
/// bare case.
///
/// A new event is what its instant says and nothing more: the facts of a
/// default [`MarketElementData`], a state of `00UNKNOWN`, no place in a
/// chain, no creation, expiration, predecessor or snapshot. It is what any
/// [`MarketEvent`] converts into, and what a [`MarketElementData`] becomes
/// at the epoch, for a caller to date.
///
/// ```
/// use yggdryl::graph::{Element, Event, MarketElement, MarketEventData};
/// use yggdryl::types::{Decimal, Side};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut event = MarketEventData::at(1_700_000_000_000_000_000);
/// event.set_px("82.5".parse()?);
/// event.set_qty(Decimal::from_int(1_000));
/// event.set_side(Side::read("Buy")?);
/// // The lane the side implies fills from the event's own facts.
/// event.fill_lanes();
/// assert_eq!(event.get_bidpx(), Some("82.5".parse()?));
/// event.finalize();
/// assert_eq!(event.get_curruuid(), event.time_uuid()?);
/// assert_ne!(event.get_hashcode(), 0);
/// // Restating the same facts is the same identity; a new price is not.
/// let mut same = MarketEventData::at(1_700_000_000_000_000_000);
/// same.set_px("82.5".parse()?);
/// same.set_qty(Decimal::from_int(1_000));
/// same.set_side(Side::read("Buy")?);
/// same.fill_lanes();
/// same.finalize();
/// assert_eq!(same.get_curruuid(), event.get_curruuid());
/// same.set_px("83".parse()?);
/// same.finalize();
/// assert_ne!(same.get_curruuid(), event.get_curruuid());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct MarketEventData {
    element: MarketElementData,
    unix: i64,
    state: State,
    seqnum: u64,
    creatunix: Option<i64>,
    expirunix: Option<i64>,
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
            element: MarketElementData::default(),
            unix,
            state: State::unknown(),
            seqnum: 0,
            creatunix: None,
            expirunix: None,
            prevunix: None,
            prevuuid: None,
            snapunix: None,
        }
    }
}

impl Default for MarketEventData {
    /// An event at the epoch, stating nothing.
    fn default() -> Self {
        Self::at(0)
    }
}

impl Element for MarketEventData {
    fn get_curruuid(&self) -> Uuid {
        self.element.curruuid
    }

    fn set_curruuid(&mut self, curruuid: Uuid) {
        self.element.curruuid = curruuid;
    }

    fn get_crossuuid(&self) -> Uuid {
        self.element.crossuuid
    }

    fn set_crossuuid(&mut self, crossuuid: Uuid) {
        self.element.crossuuid = crossuuid;
    }

    fn get_crosscode(&self) -> &str {
        &self.element.crosscode
    }

    fn set_crosscode(&mut self, crosscode: String) {
        self.element.crosscode = crosscode;
    }

    fn get_hashcode(&self) -> u64 {
        self.element.hashcode
    }

    fn set_hashcode(&mut self, hashcode: u64) {
        self.element.hashcode = hashcode;
    }

    fn get_crosshashcode(&self) -> u64 {
        self.element.crosshashcode
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.element.crosshashcode = crosshashcode;
    }

    fn get_identifiers(&self) -> &BTreeMap<String, String> {
        &self.element.identifiers
    }

    fn set_identifiers(&mut self, identifiers: BTreeMap<String, String>) {
        self.element.identifiers = identifiers;
    }

    fn get_parentuuids(&self) -> &[Uuid] {
        &self.element.parentuuids
    }

    fn set_parentuuids(&mut self, parents: Vec<Uuid>) {
        self.element.parentuuids = parents;
    }

    fn is_after(&self, other: &Self) -> bool {
        self.unix > other.unix
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
    /// The timed restatement, and then the market's: a twin takes the live
    /// event's place in its chain, which is the step before it as well as
    /// the predecessor and the position, and what that chain is about where
    /// this reading of the message said nothing of it.
    fn restating(self, live: &Self) -> Self {
        super::element::restating_market(self, live)
    }

    fn get_unix(&self) -> i64 {
        self.unix
    }

    fn set_unix(&mut self, unix: i64) {
        self.unix = unix;
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
    }

    fn get_creatunix(&self) -> Option<i64> {
        self.creatunix
    }

    fn set_creatunix(&mut self, unix: Option<i64>) {
        self.creatunix = unix;
    }

    fn get_expirunix(&self) -> Option<i64> {
        self.expirunix
    }

    fn set_expirunix(&mut self, unix: Option<i64>) {
        self.expirunix = unix;
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
}

impl MarketElement for MarketEventData {
    fn get_px(&self) -> Decimal {
        self.element.px
    }

    fn set_px(&mut self, px: Decimal) {
        self.element.px = px;
    }

    fn get_currency(&self) -> &Currency {
        &self.element.currency
    }

    fn set_currency(&mut self, currency: Currency) {
        self.element.currency = currency;
    }

    fn get_qty(&self) -> Decimal {
        self.element.qty
    }

    fn set_qty(&mut self, qty: Decimal) {
        self.element.qty = qty;
    }

    fn get_unit(&self) -> &str {
        &self.element.unit
    }

    fn set_unit(&mut self, unit: String) {
        self.element.unit = unit;
    }

    fn get_side(&self) -> &Side {
        &self.element.side
    }

    fn set_side(&mut self, side: Side) {
        self.element.side = side;
    }

    fn get_isincode(&self) -> Option<&Isin> {
        self.element.isincode.as_ref()
    }

    fn set_isincode(&mut self, isincode: Option<Isin>) {
        self.element.isincode = isincode;
    }

    fn get_cusipcode(&self) -> Option<&Cusip> {
        self.element.cusipcode.as_ref()
    }

    fn set_cusipcode(&mut self, cusipcode: Option<Cusip>) {
        self.element.cusipcode = cusipcode;
    }

    fn get_sedolcode(&self) -> Option<&Sedol> {
        self.element.sedolcode.as_ref()
    }

    fn set_sedolcode(&mut self, sedolcode: Option<Sedol>) {
        self.element.sedolcode = sedolcode;
    }

    fn get_bloombergcode(&self) -> Option<&Bloomberg> {
        self.element.bloombergcode.as_ref()
    }

    fn set_bloombergcode(&mut self, bloombergcode: Option<Bloomberg>) {
        self.element.bloombergcode = bloombergcode;
    }

    fn get_cficode(&self) -> Option<&Cfi> {
        self.element.cficode.as_ref()
    }

    fn set_cficode(&mut self, cficode: Option<Cfi>) {
        self.element.cficode = cficode;
    }

    fn get_miccode(&self) -> Option<&Mic> {
        self.element.miccode.as_ref()
    }

    fn set_miccode(&mut self, miccode: Option<Mic>) {
        self.element.miccode = miccode;
    }

    fn get_lastpx(&self) -> Option<Decimal> {
        self.element.lastpx
    }

    fn set_lastpx(&mut self, px: Option<Decimal>) {
        self.element.lastpx = px;
    }

    fn get_lastqty(&self) -> Option<Decimal> {
        self.element.lastqty
    }

    fn set_lastqty(&mut self, qty: Option<Decimal>) {
        self.element.lastqty = qty;
    }

    fn get_tif(&self) -> Option<&str> {
        self.element.tif.as_deref()
    }

    fn set_tif(&mut self, tif: Option<String>) {
        self.element.tif = tif;
    }

    fn get_tradable(&self) -> Option<bool> {
        self.element.tradable
    }

    fn set_tradable(&mut self, tradable: Option<bool>) {
        self.element.tradable = tradable;
    }

    fn get_symbolticker(&self) -> Option<&str> {
        self.element.symbolticker.as_deref()
    }

    fn set_symbolticker(&mut self, ticker: Option<String>) {
        self.element.symbolticker = ticker;
    }

    fn get_avgpx(&self) -> Option<Decimal> {
        self.element.avgpx
    }

    fn set_avgpx(&mut self, px: Option<Decimal>) {
        self.element.avgpx = px;
    }

    fn get_cumqty(&self) -> Option<Decimal> {
        self.element.cumqty
    }

    fn set_cumqty(&mut self, qty: Option<Decimal>) {
        self.element.cumqty = qty;
    }

    fn get_leavesqty(&self) -> Option<Decimal> {
        self.element.leavesqty
    }

    fn set_leavesqty(&mut self, qty: Option<Decimal>) {
        self.element.leavesqty = qty;
    }

    fn get_prevpx(&self) -> Option<Decimal> {
        self.element.prevpx
    }

    fn set_prevpx(&mut self, px: Option<Decimal>) {
        self.element.prevpx = px;
    }

    fn get_prevqty(&self) -> Option<Decimal> {
        self.element.prevqty
    }

    fn set_prevqty(&mut self, qty: Option<Decimal>) {
        self.element.prevqty = qty;
    }

    fn get_bidpx(&self) -> Option<Decimal> {
        self.element.bidpx
    }

    fn set_bidpx(&mut self, px: Option<Decimal>) {
        self.element.bidpx = px;
    }

    fn get_bidcurrency(&self) -> Option<&Currency> {
        self.element.bidcurrency.as_ref()
    }

    fn set_bidcurrency(&mut self, currency: Option<Currency>) {
        self.element.bidcurrency = currency;
    }

    fn get_bidqty(&self) -> Option<Decimal> {
        self.element.bidqty
    }

    fn set_bidqty(&mut self, qty: Option<Decimal>) {
        self.element.bidqty = qty;
    }

    fn get_bidunit(&self) -> Option<&str> {
        self.element.bidunit.as_deref()
    }

    fn set_bidunit(&mut self, unit: Option<String>) {
        self.element.bidunit = unit;
    }

    fn get_askpx(&self) -> Option<Decimal> {
        self.element.askpx
    }

    fn set_askpx(&mut self, px: Option<Decimal>) {
        self.element.askpx = px;
    }

    fn get_askcurrency(&self) -> Option<&Currency> {
        self.element.askcurrency.as_ref()
    }

    fn set_askcurrency(&mut self, currency: Option<Currency>) {
        self.element.askcurrency = currency;
    }

    fn get_askqty(&self) -> Option<Decimal> {
        self.element.askqty
    }

    fn set_askqty(&mut self, qty: Option<Decimal>) {
        self.element.askqty = qty;
    }

    fn get_askunit(&self) -> Option<&str> {
        self.element.askunit.as_deref()
    }

    fn set_askunit(&mut self, unit: Option<String>) {
        self.element.askunit = unit;
    }
}

/// Every fact [`Element`] names, copied from `other` into `this` through
/// the signatures the two share.
fn copy_element<T: Element + ?Sized, E: Element + ?Sized>(this: &mut T, other: &E) {
    this.set_curruuid(other.get_curruuid());
    this.set_crossuuid(other.get_crossuuid());
    this.set_crosscode(other.get_crosscode().to_owned());
    this.set_hashcode(other.get_hashcode());
    this.set_crosshashcode(other.get_crosshashcode());
    this.set_identifiers(other.get_identifiers().clone());
    this.set_parentuuids(other.get_parentuuids().to_vec());
}

/// Every fact [`Event`] names, copied from `other` into `this`.
fn copy_event<T: Event + ?Sized, E: Event + ?Sized>(this: &mut T, other: &E) {
    this.set_unix(other.get_unix());
    this.set_state(other.get_state().clone());
    this.set_seqnum(other.get_seqnum());
    this.set_creatunix(other.get_creatunix());
    this.set_expirunix(other.get_expirunix());
    this.set_prevunix(other.get_prevunix());
    this.set_prevuuid(other.get_prevuuid());
    this.set_snapunix(other.get_snapunix());
}

/// Every fact [`MarketElement`] names, copied from `other` into `this`.
fn copy_market<T: MarketElement + ?Sized, E: MarketElement + ?Sized>(this: &mut T, other: &E) {
    this.set_px(other.get_px());
    this.set_currency(other.get_currency().clone());
    this.set_qty(other.get_qty());
    this.set_unit(other.get_unit().to_owned());
    this.set_side(other.get_side().clone());
    this.set_isincode(other.get_isincode().cloned());
    this.set_cusipcode(other.get_cusipcode().cloned());
    this.set_sedolcode(other.get_sedolcode().cloned());
    this.set_bloombergcode(other.get_bloombergcode().cloned());
    this.set_cficode(other.get_cficode().cloned());
    this.set_miccode(other.get_miccode().cloned());
    this.set_bidpx(other.get_bidpx());
    this.set_bidcurrency(other.get_bidcurrency().cloned());
    this.set_bidqty(other.get_bidqty());
    this.set_bidunit(other.get_bidunit().map(str::to_owned));
    this.set_askpx(other.get_askpx());
    this.set_askcurrency(other.get_askcurrency().cloned());
    this.set_askqty(other.get_askqty());
    this.set_askunit(other.get_askunit().map(str::to_owned));
}

impl<E: MarketElement + ?Sized> From<&E> for MarketElementData {
    /// The facts any market element states, through the signatures the two
    /// share; whatever else `other` states - an event's instants, a
    /// message's body - is left behind.
    fn from(other: &E) -> Self {
        let mut this = Self::default();
        copy_element(&mut this, other);
        copy_market(&mut this, other);
        this
    }
}

impl<E: MarketEvent + ?Sized> From<&E> for MarketEventData {
    /// The facts any market event states, through the signatures the two
    /// share; whatever else `other` states is left behind.
    fn from(other: &E) -> Self {
        let mut this = Self::default();
        copy_element(&mut this, other);
        copy_event(&mut this, other);
        copy_market(&mut this, other);
        this
    }
}

impl From<MarketEventData> for MarketElementData {
    /// The event without its instants: the identity, the codes, the names,
    /// the parents and the market's facts, moved.
    fn from(event: MarketEventData) -> Self {
        event.element
    }
}

impl From<MarketElementData> for MarketEventData {
    /// The element at the epoch, stating what it states and no instant:
    /// [`Event::set_unix`] dates it, and [`Element::finalize`] then derives
    /// the identity the instant and the facts couple to.
    fn from(element: MarketElementData) -> Self {
        Self {
            element,
            ..Self::default()
        }
    }
}
