//! The market element and the market event held as plain fields: every
//! fact the traits name, and nothing else.

use std::collections::BTreeMap;

use super::{Element, Event, MarketElement, MarketEvent};
use crate::{
    BloombergCode, CfiCode, Currency, CusipCode, Decimal18, FIGICode, IsinCode, MicCode, SedolCode,
    Side, State, Uuid,
};

/// The concrete market element: every fact [`Element`] and
/// [`MarketElement`] name, held as one field each, with no instant of its
/// own.
///
/// Its identity is derived: [`Element::finalize`] digests the facts the
/// traits know through [`MarketElement::digest_market`], records the code
/// and sets the identity that code derives, RFC 9562 UUIDv8 over it, so
/// two elements stating the same things are one identity. Its order is
/// lineage, as a node's is: it is after the elements it descends from, and
/// it follows another by descending from its whole lineage.
///
/// A new element states nothing: no identity, no cross code, no names, no
/// parents, no sources, a price and a quantity of nothing in no currency
/// (`XXX`) and no unit, a side of `UNKNOWN`, no instrument named, no lane
/// stated. It is what any [`MarketElement`] converts into, dropping
/// whatever else that element states, and what a [`MarketEventData`] is
/// without its instants.
///
/// ```
/// use yggdryl::graph::{Element, Event, MarketElement, MarketElementData, MarketEventData};
/// use yggdryl::{Decimal18, Side, Uuid};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut element = MarketElementData::default();
/// element.set_crosscode("O-100".to_owned());
/// // Where the element was read from: provenance, beside its lineage.
/// element.set_srcuuids(vec![Uuid::from_v8(7)]);
/// element.set_price("82.5".parse()?);
/// element.set_quantity(Decimal18::from_int(1_000));
/// element.set_side(Side::read("Buy")?);
/// element.finalize();
/// assert_ne!(element.get_currhashcode(), 0);
/// assert_eq!(element.get_crossuuid(), element.cross_uuid());
/// // The same facts at an instant: the event states them and its own.
/// let mut event = MarketEventData::from(element.clone());
/// event.set_currunix(1_700_000_000_000_000_000);
/// event.finalize();
/// assert_eq!(event.get_price(), element.get_price());
/// assert_eq!(event.get_crosscode(), "O-100");
/// assert_eq!(event.get_srcuuids(), [Uuid::from_v8(7)], "the source survives");
/// // And back, through the signatures the two share: the event's instants
/// // drop, and the identity is what the shared facts derive - a source
/// // among them not, because where an element was read from is not what it
/// // states.
/// let mut again = MarketElementData::from(&event);
/// assert_eq!(again.get_srcuuids(), [Uuid::from_v8(7)]);
/// again.set_srcuuids(vec![Uuid::from_v8(8)]);
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
    currhashcode: u64,
    crosshashcode: u64,
    identifiers: BTreeMap<String, String>,
    parentuuids: Vec<Uuid>,
    srcuuids: Vec<Uuid>,
    marketoperationid: Option<i32>,
    price: Decimal18,
    currency: Currency,
    quantity: Decimal18,
    unit: String,
    side: Side,
    isincode: Option<IsinCode>,
    cusipcode: Option<CusipCode>,
    sedolcode: Option<SedolCode>,
    bloombergcode: Option<BloombergCode>,
    figicode: Option<FIGICode>,
    cficode: Option<CfiCode>,
    miccode: Option<MicCode>,
    lastpx: Option<Decimal18>,
    lastqty: Option<Decimal18>,
    avgpx: Option<Decimal18>,
    cumqty: Option<Decimal18>,
    leavesqty: Option<Decimal18>,
    tif: Option<String>,
    tradable: Option<bool>,
    symbolticker: Option<String>,
    prevpx: Option<Decimal18>,
    prevqty: Option<Decimal18>,
    bidpx: Option<Decimal18>,
    bidcurrency: Option<Currency>,
    bidqty: Option<Decimal18>,
    bidunit: Option<String>,
    askpx: Option<Decimal18>,
    askcurrency: Option<Currency>,
    askqty: Option<Decimal18>,
    askunit: Option<String>,
}

impl Default for MarketElementData {
    /// An element stating nothing.
    fn default() -> Self {
        Self {
            curruuid: Uuid::default(),
            crossuuid: Uuid::default(),
            crosscode: String::new(),
            currhashcode: 0,
            crosshashcode: 0,
            identifiers: BTreeMap::new(),
            parentuuids: Vec::new(),
            srcuuids: Vec::new(),
            marketoperationid: None,
            price: Decimal18::ZERO,
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
            quantity: Decimal18::ZERO,
            unit: String::new(),
            side: Side::unknown(),
            isincode: None,
            cusipcode: None,
            sedolcode: None,
            bloombergcode: None,
            figicode: None,
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

    fn get_identifiers(&self) -> &BTreeMap<String, String> {
        &self.identifiers
    }

    fn set_identifiers(&mut self, identifiers: BTreeMap<String, String>) {
        self.identifiers = identifiers;
    }

    fn get_parentuuids(&self) -> &[Uuid] {
        &self.parentuuids
    }

    fn set_parentuuids(&mut self, mut parents: Vec<Uuid>) {
        super::element::canonicalize_uuids(&mut parents);
        self.parentuuids = parents;
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        &self.srcuuids
    }

    fn set_srcuuids(&mut self, mut sources: Vec<Uuid>) {
        super::element::canonicalize_uuids(&mut sources);
        self.srcuuids = sources;
    }

    fn is_after(&self, other: &Self) -> bool {
        self.parentuuids.binary_search(&other.curruuid).is_ok()
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
        changed |= super::element::follow_market(&mut self, previous);
        changed |= super::element::descend_from(&mut self, previous);
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
    fn get_marketoperationid(&self) -> Option<i32> {
        self.marketoperationid
    }

    fn set_marketoperationid(&mut self, marketoperationid: Option<i32>) {
        self.marketoperationid = marketoperationid;
    }

    fn get_price(&self) -> Decimal18 {
        self.price
    }

    fn set_price(&mut self, price: Decimal18) {
        self.price = price;
    }

    fn get_currency(&self) -> &Currency {
        &self.currency
    }

    fn set_currency(&mut self, currency: Currency) {
        self.currency = currency;
    }

    fn get_quantity(&self) -> Decimal18 {
        self.quantity
    }

    fn set_quantity(&mut self, quantity: Decimal18) {
        self.quantity = quantity;
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

    fn get_isincode(&self) -> Option<&IsinCode> {
        self.isincode.as_ref()
    }

    fn set_isincode(&mut self, isincode: Option<IsinCode>) {
        self.isincode = isincode;
        if self.cusipcode.is_none() {
            self.cusipcode = self
                .isincode
                .as_ref()
                .and_then(super::instrument::embedded_cusip);
        }
    }

    fn get_cusipcode(&self) -> Option<&CusipCode> {
        self.cusipcode.as_ref()
    }

    fn set_cusipcode(&mut self, cusipcode: Option<CusipCode>) {
        self.cusipcode = cusipcode;
    }

    fn get_sedolcode(&self) -> Option<&SedolCode> {
        self.sedolcode.as_ref()
    }

    fn set_sedolcode(&mut self, sedolcode: Option<SedolCode>) {
        self.sedolcode = sedolcode;
    }

    fn get_bloombergcode(&self) -> Option<&BloombergCode> {
        self.bloombergcode.as_ref()
    }

    fn set_bloombergcode(&mut self, bloombergcode: Option<BloombergCode>) {
        self.bloombergcode = bloombergcode;
    }

    fn get_figicode(&self) -> Option<&FIGICode> {
        self.figicode.as_ref()
    }

    fn set_figicode(&mut self, figicode: Option<FIGICode>) {
        self.figicode = figicode;
    }

    fn get_cficode(&self) -> Option<&CfiCode> {
        self.cficode.as_ref()
    }

    fn set_cficode(&mut self, cficode: Option<CfiCode>) {
        self.cficode = cficode;
    }

    fn get_miccode(&self) -> Option<&MicCode> {
        self.miccode.as_ref()
    }

    fn set_miccode(&mut self, miccode: Option<MicCode>) {
        self.miccode = miccode;
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

    fn get_bidpx(&self) -> Option<Decimal18> {
        self.bidpx
    }

    fn set_bidpx(&mut self, px: Option<Decimal18>) {
        self.bidpx = px;
    }

    fn get_bidcurrency(&self) -> Option<&Currency> {
        self.bidcurrency.as_ref()
    }

    fn set_bidcurrency(&mut self, currency: Option<Currency>) {
        self.bidcurrency = currency;
    }

    fn get_bidqty(&self) -> Option<Decimal18> {
        self.bidqty
    }

    fn set_bidqty(&mut self, qty: Option<Decimal18>) {
        self.bidqty = qty;
    }

    fn get_bidunit(&self) -> Option<&str> {
        self.bidunit.as_deref()
    }

    fn set_bidunit(&mut self, unit: Option<String>) {
        self.bidunit = unit;
    }

    fn get_askpx(&self) -> Option<Decimal18> {
        self.askpx
    }

    fn set_askpx(&mut self, px: Option<Decimal18>) {
        self.askpx = px;
    }

    fn get_askcurrency(&self) -> Option<&Currency> {
        self.askcurrency.as_ref()
    }

    fn set_askcurrency(&mut self, currency: Option<Currency>) {
        self.askcurrency = currency;
    }

    fn get_askqty(&self) -> Option<Decimal18> {
        self.askqty
    }

    fn set_askqty(&mut self, qty: Option<Decimal18>) {
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
/// chain, no creation, execution, recording, expiration, predecessor or
/// snapshot. It is what any
/// [`MarketEvent`] converts into, and what a [`MarketElementData`] becomes
/// at the epoch, for a caller to date.
///
/// ```
/// use yggdryl::graph::{Element, Event, MarketElement, MarketElementData, MarketEventData};
/// use yggdryl::{Decimal18, Side, Uuid};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut event = MarketEventData::at(1_700_000_000_000_000_000);
/// event.set_price("82.5".parse()?);
/// event.set_quantity(Decimal18::from_int(1_000));
/// event.set_side(Side::read("Buy")?);
/// event.set_srcuuids(vec![Uuid::from_v8(7)]);
/// // The lane the side implies fills from the event's own facts.
/// event.fill_lanes();
/// assert_eq!(event.get_bidpx(), Some("82.5".parse()?));
/// event.finalize();
/// assert_eq!(event.get_curruuid(), event.time_uuid()?);
/// assert_ne!(event.get_currhashcode(), 0);
/// // The source survives both conversions, and is never part of the code.
/// assert_eq!(MarketEventData::from(&event).get_srcuuids(), [Uuid::from_v8(7)]);
/// assert_eq!(MarketElementData::from(event.clone()).get_srcuuids(), [Uuid::from_v8(7)]);
/// // Restating the same facts is the same identity; a new price is not.
/// let mut same = MarketEventData::at(1_700_000_000_000_000_000);
/// same.set_price("82.5".parse()?);
/// same.set_quantity(Decimal18::from_int(1_000));
/// same.set_side(Side::read("Buy")?);
/// same.set_srcuuids(vec![Uuid::from_v8(8)]);
/// same.fill_lanes();
/// same.finalize();
/// assert_eq!(same.get_curruuid(), event.get_curruuid());
/// same.set_price("83".parse()?);
/// same.finalize();
/// assert_ne!(same.get_curruuid(), event.get_curruuid());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct MarketEventData {
    element: MarketElementData,
    currunix: i64,
    state: State,
    seqnum: u64,
    creaunix: Option<i64>,
    execunix: Option<i64>,
    recdunix: Option<i64>,
    refrecdunix: Option<i64>,
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
            element: MarketElementData::default(),
            currunix: unix,
            state: State::unknown(),
            seqnum: 0,
            creaunix: None,
            execunix: None,
            recdunix: None,
            refrecdunix: None,
            exprtime: None,
            prevunix: None,
            prevuuid: None,
            snapunix: None,
        }
    }

    /// The names this event goes by, for a holder synchronizing one derived
    /// name before it finalizes the event.
    pub(crate) fn identifiers_mut(&mut self) -> &mut BTreeMap<String, String> {
        &mut self.element.identifiers
    }

    /// Reprojects the generic event identities after one of their inputs
    /// changes. A UUIDv7 refusal retains the current identity, as
    /// [`Event::finalized`] does; the cross identity always follows the
    /// resulting current identity and cross hash.
    fn refresh_uuids(&mut self) {
        if let Ok(uuid) = self.time_uuid() {
            self.element.curruuid = uuid;
        }
        self.element.crossuuid = self.cross_uuid();
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
        self.element.crosshashcode = if self.element.crosscode.is_empty() {
            0
        } else {
            super::element::crosshash(&self.element.crosscode)
        };
        self.refresh_uuids();
    }

    fn get_currhashcode(&self) -> u64 {
        self.element.currhashcode
    }

    fn set_currhashcode(&mut self, hashcode: u64) {
        self.element.currhashcode = hashcode;
        self.refresh_uuids();
    }

    fn get_crosshashcode(&self) -> u64 {
        self.element.crosshashcode
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.element.crosshashcode = crosshashcode;
        self.refresh_uuids();
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

    fn set_parentuuids(&mut self, mut parents: Vec<Uuid>) {
        super::element::canonicalize_uuids(&mut parents);
        self.element.parentuuids = parents;
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        &self.element.srcuuids
    }

    fn set_srcuuids(&mut self, mut sources: Vec<Uuid>) {
        super::element::canonicalize_uuids(&mut sources);
        self.element.srcuuids = sources;
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
    /// The timed restatement, and then the market's: a twin takes the live
    /// event's place in its chain, which is the step before it as well as
    /// the predecessor and the position, and what that chain is about where
    /// this reading of the message said nothing of it.
    fn restating(self, live: &Self) -> Self {
        super::element::restating_market(self, live)
    }

    /// Records a finalized content code and projects both identities once.
    ///
    /// The public identity-input setters refresh eagerly. Finalization already
    /// owns the complete new input set, so writing the code directly avoids
    /// the setter's projection followed by the provided finalizer's identical
    /// projection. A [`crate::FixMsg`] finalizes through this holder and takes
    /// the same single projection.
    fn finalized(&mut self, hashcode: u64) {
        self.element.currhashcode = hashcode;
        self.refresh_uuids();
    }

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
        let tracks_recording = self.refrecdunix.is_none() || self.refrecdunix == self.recdunix;
        self.recdunix = unix;
        if tracks_recording {
            self.refrecdunix = unix;
        }
    }

    fn get_refrecdunix(&self) -> Option<i64> {
        self.refrecdunix
    }

    fn set_refrecdunix(&mut self, unix: Option<i64>) {
        self.refrecdunix = unix;
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
}

impl MarketElement for MarketEventData {
    fn get_marketoperationid(&self) -> Option<i32> {
        self.element.marketoperationid
    }

    fn set_marketoperationid(&mut self, marketoperationid: Option<i32>) {
        self.element.marketoperationid = marketoperationid;
    }

    fn get_price(&self) -> Decimal18 {
        self.element.price
    }

    fn set_price(&mut self, price: Decimal18) {
        self.element.price = price;
    }

    fn get_currency(&self) -> &Currency {
        &self.element.currency
    }

    fn set_currency(&mut self, currency: Currency) {
        self.element.currency = currency;
    }

    fn get_quantity(&self) -> Decimal18 {
        self.element.quantity
    }

    fn set_quantity(&mut self, quantity: Decimal18) {
        self.element.quantity = quantity;
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

    fn get_isincode(&self) -> Option<&IsinCode> {
        self.element.isincode.as_ref()
    }

    fn set_isincode(&mut self, isincode: Option<IsinCode>) {
        self.element.set_isincode(isincode);
    }

    fn get_cusipcode(&self) -> Option<&CusipCode> {
        self.element.cusipcode.as_ref()
    }

    fn set_cusipcode(&mut self, cusipcode: Option<CusipCode>) {
        self.element.set_cusipcode(cusipcode);
    }

    fn get_sedolcode(&self) -> Option<&SedolCode> {
        self.element.sedolcode.as_ref()
    }

    fn set_sedolcode(&mut self, sedolcode: Option<SedolCode>) {
        self.element.set_sedolcode(sedolcode);
    }

    fn get_bloombergcode(&self) -> Option<&BloombergCode> {
        self.element.bloombergcode.as_ref()
    }

    fn set_bloombergcode(&mut self, bloombergcode: Option<BloombergCode>) {
        self.element.set_bloombergcode(bloombergcode);
    }

    fn get_figicode(&self) -> Option<&FIGICode> {
        self.element.figicode.as_ref()
    }

    fn set_figicode(&mut self, figicode: Option<FIGICode>) {
        self.element.set_figicode(figicode);
    }

    fn get_cficode(&self) -> Option<&CfiCode> {
        self.element.cficode.as_ref()
    }

    fn set_cficode(&mut self, cficode: Option<CfiCode>) {
        self.element.set_cficode(cficode);
    }

    fn get_miccode(&self) -> Option<&MicCode> {
        self.element.miccode.as_ref()
    }

    fn set_miccode(&mut self, miccode: Option<MicCode>) {
        self.element.set_miccode(miccode);
    }

    fn get_lastpx(&self) -> Option<Decimal18> {
        self.element.lastpx
    }

    fn set_lastpx(&mut self, px: Option<Decimal18>) {
        self.element.lastpx = px;
    }

    fn get_lastqty(&self) -> Option<Decimal18> {
        self.element.lastqty
    }

    fn set_lastqty(&mut self, qty: Option<Decimal18>) {
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

    fn get_avgpx(&self) -> Option<Decimal18> {
        self.element.avgpx
    }

    fn set_avgpx(&mut self, px: Option<Decimal18>) {
        self.element.avgpx = px;
    }

    fn get_cumqty(&self) -> Option<Decimal18> {
        self.element.cumqty
    }

    fn set_cumqty(&mut self, qty: Option<Decimal18>) {
        self.element.cumqty = qty;
    }

    fn get_leavesqty(&self) -> Option<Decimal18> {
        self.element.leavesqty
    }

    fn set_leavesqty(&mut self, qty: Option<Decimal18>) {
        self.element.leavesqty = qty;
    }

    fn get_prevpx(&self) -> Option<Decimal18> {
        self.element.prevpx
    }

    fn set_prevpx(&mut self, px: Option<Decimal18>) {
        self.element.prevpx = px;
    }

    fn get_prevqty(&self) -> Option<Decimal18> {
        self.element.prevqty
    }

    fn set_prevqty(&mut self, qty: Option<Decimal18>) {
        self.element.prevqty = qty;
    }

    fn get_bidpx(&self) -> Option<Decimal18> {
        self.element.bidpx
    }

    fn set_bidpx(&mut self, px: Option<Decimal18>) {
        self.element.bidpx = px;
    }

    fn get_bidcurrency(&self) -> Option<&Currency> {
        self.element.bidcurrency.as_ref()
    }

    fn set_bidcurrency(&mut self, currency: Option<Currency>) {
        self.element.bidcurrency = currency;
    }

    fn get_bidqty(&self) -> Option<Decimal18> {
        self.element.bidqty
    }

    fn set_bidqty(&mut self, qty: Option<Decimal18>) {
        self.element.bidqty = qty;
    }

    fn get_bidunit(&self) -> Option<&str> {
        self.element.bidunit.as_deref()
    }

    fn set_bidunit(&mut self, unit: Option<String>) {
        self.element.bidunit = unit;
    }

    fn get_askpx(&self) -> Option<Decimal18> {
        self.element.askpx
    }

    fn set_askpx(&mut self, px: Option<Decimal18>) {
        self.element.askpx = px;
    }

    fn get_askcurrency(&self) -> Option<&Currency> {
        self.element.askcurrency.as_ref()
    }

    fn set_askcurrency(&mut self, currency: Option<Currency>) {
        self.element.askcurrency = currency;
    }

    fn get_askqty(&self) -> Option<Decimal18> {
        self.element.askqty
    }

    fn set_askqty(&mut self, qty: Option<Decimal18>) {
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
    this.set_currhashcode(other.get_currhashcode());
    this.set_crosshashcode(other.get_crosshashcode());
    this.set_identifiers(other.get_identifiers().clone());
    this.set_parentuuids(other.get_parentuuids().to_vec());
    this.set_srcuuids(other.get_srcuuids().to_vec());
}

/// Every fact [`Event`] names, copied from `other` into `this`.
fn copy_event<T: Event + ?Sized, E: Event + ?Sized>(this: &mut T, other: &E) {
    this.set_currunix(other.get_currunix());
    this.set_state(other.get_state().clone());
    this.set_seqnum(other.get_seqnum());
    this.set_creaunix(other.get_creaunix());
    this.set_execunix(other.get_execunix());
    this.set_recdunix(other.get_recdunix());
    this.set_refrecdunix(other.get_refrecdunix());
    this.set_exprtime(other.get_exprtime());
    this.set_prevunix(other.get_prevunix());
    this.set_prevuuid(other.get_prevuuid());
    this.set_snapunix(other.get_snapunix());
}

/// Every fact [`MarketElement`] names, copied from `other` into `this`.
fn copy_market<T: MarketElement + ?Sized, E: MarketElement + ?Sized>(this: &mut T, other: &E) {
    this.set_marketoperationid(other.get_marketoperationid());
    this.set_price(other.get_price());
    this.set_currency(other.get_currency().clone());
    this.set_quantity(other.get_quantity());
    this.set_unit(other.get_unit().to_owned());
    this.set_side(other.get_side().clone());
    this.set_isincode(other.get_isincode().cloned());
    this.set_cusipcode(other.get_cusipcode().cloned());
    this.set_sedolcode(other.get_sedolcode().cloned());
    this.set_bloombergcode(other.get_bloombergcode().cloned());
    this.set_figicode(other.get_figicode().cloned());
    this.set_cficode(other.get_cficode().cloned());
    this.set_miccode(other.get_miccode().cloned());
    this.set_lastpx(other.get_lastpx());
    this.set_lastqty(other.get_lastqty());
    this.set_tif(other.get_tif().map(str::to_owned));
    this.set_tradable(other.get_tradable());
    this.set_symbolticker(other.get_symbolticker().map(str::to_owned));
    this.set_avgpx(other.get_avgpx());
    this.set_cumqty(other.get_cumqty());
    this.set_leavesqty(other.get_leavesqty());
    this.set_prevpx(other.get_prevpx());
    this.set_prevqty(other.get_prevqty());
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
        // The setters above keep a derived event coherent while it is
        // mutated. Conversion copies the exact identities the source states,
        // including an assigned identity, after every dependency is in place.
        this.element.curruuid = other.get_curruuid();
        this.element.crossuuid = other.get_crossuuid();
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
    /// [`Event::set_currunix`] dates it, and [`Element::finalize`] then derives
    /// the identity the instant and the facts couple to.
    fn from(element: MarketElementData) -> Self {
        Self {
            element,
            ..Self::default()
        }
    }
}
