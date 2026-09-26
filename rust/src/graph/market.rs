//! The market vocabulary: what an element that stands in a market answers,
//! and what an operation on that market adds.
//!
//! [`Market`] is the slim reading a book level or any plain struct gives
//! cheaply: the instrument it is about - its security identifiers, its
//! classification, the market it trades on and the ticker it goes by - the
//! side it takes, what it is priced and counted in, the price and quantity
//! it is about, its last executed price and quantity and its average, how far it has got, the
//! step before it, the two FX parts of a price, and free-form metadata.
//! [`Operation`] is a market element that is also an operation: its
//! category, how long it stands, whether it can trade, the account, user
//! and alternate identifiers it is known by, and the two lanes a quote is
//! made of. What an [`Event`] that is also one of them answers -
//! [`Market::digest_market_event`], [`Market::following_market`],
//! [`Market::merging_market_event`] and their [`Operation`] counterparts -
//! is provided on the trait itself, gated `where Self: Event`, rather than
//! a separate blanket trait. The traits state signatures
//! and the provided readings - fill, digest, follow, restate, merge - so a
//! message, a book entry and a lifecycle incarnation can each be a market
//! without the graph owning any of them.

use std::collections::BTreeMap;

use smol_str::SmolStr;

use super::element::{
    Staged, fold_event_instants, follow_timed, merge_element, merge_event_element, merge_timed,
    moved, restate_event, right_is_reference, stated,
};
use super::{Element, Event};
use crate::CodeValue;
use crate::idmap::IdMap;
use crate::securityid::{SecType, SecurityId, SecurityIds, embedded};
use crate::xxhash::Xxh3;
use crate::{Ccy, CfiCode, Decimal, MicCode, Result, Side, TimeInForce, Unit};

/// Free-form facts a market element carries beside its typed ones: never an
/// identifier, which has a typed home in [`Market::get_securityids`] or an
/// operation's maps.
pub type Metadata = BTreeMap<SmolStr, SmolStr>;

static EMPTY_METADATA: Metadata = BTreeMap::new();

/// The metadata an element that holds none answers: one shared empty map.
#[must_use]
pub fn empty_metadata() -> &'static Metadata {
    &EMPTY_METADATA
}

/// The alternate-identifier keys a following operation carries from the
/// one it follows: the order's own identities, never an execution's or a
/// quote's. What [`Operation::is_followed_altid`] answers for a holder with
/// no dictionary behind it; a FIX message reads its registry's `FIX:idmap`
/// follow flags instead, which a test pins against this list.
pub const FOLLOWED_ALTIDS: [&str; 7] = [
    "ORDERID",
    "SECONDARYORDERID",
    "PARENTORDERID",
    "PARENTCLORDID",
    "OMSDEALERPARENTORDERID",
    "EXCHANGECLIENTORDERID",
    "TRANSVERSALKEY",
];

/// One lane of a quote: what a party is willing to pay or be paid, in the
/// currency and unit the lane states, with the FX parts of its price where
/// it quotes a forward.
///
/// Every slot is what the lane states; a lane states nothing of a slot it
/// leaves `None`, and [`Lane::is_stated`] is whether it states anything.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Lane {
    /// The lane's price.
    pub price: Option<Decimal>,
    /// The spot part of an FX forward price.
    pub spotrate: Option<Decimal>,
    /// The forward points of an FX forward price.
    pub forwardpoints: Option<Decimal>,
    /// The currency the lane is priced in.
    pub currency: Option<Ccy>,
    /// The lane's quantity.
    pub quantity: Option<Decimal>,
    /// The unit the lane's quantity is counted in.
    pub unit: Option<Unit>,
}

impl Lane {
    /// Whether the lane states any slot.
    #[must_use]
    pub fn is_stated(&self) -> bool {
        self.price.is_some()
            || self.spotrate.is_some()
            || self.forwardpoints.is_some()
            || self.currency.is_some()
            || self.quantity.is_some()
            || self.unit.is_some()
    }

    /// The lane as `Some` where it states anything, else `None`.
    #[must_use]
    pub fn stated(self) -> Option<Self> {
        self.is_stated().then_some(self)
    }

    /// This lane folded with another statement of it: each slot the
    /// selected statement states, else the other's. `later` selects
    /// `other`.
    #[must_use]
    fn merged(&self, other: &Self, later: bool) -> Self {
        Self {
            price: stated(self.price, other.price, later),
            spotrate: stated(self.spotrate, other.spotrate, later),
            forwardpoints: stated(self.forwardpoints, other.forwardpoints, later),
            currency: stated(self.currency.clone(), other.currency.clone(), later),
            quantity: stated(self.quantity, other.quantity, later),
            unit: stated(self.unit.clone(), other.unit.clone(), later),
        }
    }

    fn feed(&self, staged: &mut Staged<'_>, lane: &str) {
        for held in [self.price, self.spotrate, self.forwardpoints]
            .into_iter()
            .flatten()
        {
            staged.feed(lane, &held.units().to_le_bytes());
        }
        if let Some(currency) = &self.currency {
            staged.feed(lane, currency.as_str().as_bytes());
        }
        if let Some(quantity) = self.quantity {
            staged.feed(lane, &quantity.units().to_le_bytes());
        }
        if let Some(unit) = &self.unit {
            staged.feed(lane, unit.as_str().as_bytes());
        }
    }
}

/// An element that stands in a market: the slim facts a book level or any
/// plain struct answers cheaply.
///
/// Every accessor is `get_` and every mutator `set_`, except the security
/// identifiers, which a holder answers as a set and changes through
/// fallible verbs: a holder that is a view of another store - a FIX message
/// over its own fields - writes the store through and may refuse. A plain
/// holder always answers `Ok`.
pub trait Market {
    /// The price the element states; `None` where it states none. Never
    /// defaulted: a last executed price is [`Self::get_lastpx`], not this.
    fn get_price(&self) -> Option<Decimal>;
    /// Sets [`Self::get_price`].
    fn set_price(&mut self, price: Option<Decimal>);
    /// The currency the element is priced in, [`Ccy::none`] where unstated.
    fn get_currency(&self) -> &Ccy;
    /// Sets [`Self::get_currency`].
    fn set_currency(&mut self, currency: Ccy);
    /// The quantity the element states; `None` where it states none. Never
    /// defaulted: a last executed quantity is [`Self::get_lastqty`], not this.
    fn get_quantity(&self) -> Option<Decimal>;
    /// Sets [`Self::get_quantity`].
    fn set_quantity(&mut self, quantity: Option<Decimal>);
    /// The unit the quantity is counted in, [`Unit::none`] where unstated.
    fn get_unit(&self) -> &Unit;
    /// Sets [`Self::get_unit`].
    fn set_unit(&mut self, unit: Unit);
    /// The side the element takes, [`Side::Unknown`] where it states none.
    fn get_side(&self) -> Side;
    /// Sets [`Self::get_side`].
    fn set_side(&mut self, side: Side);
    /// The security identifiers the element names, one code per key.
    fn get_securityids(&self) -> &SecurityIds;
    /// Replaces the stated security identifiers.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// one of the keys.
    fn set_securityids(&mut self, ids: SecurityIds) -> Result<()>;
    /// States one security identifier, filling only an absent key; whether
    /// it was added.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// the key.
    fn insert_securityid(&mut self, id: SecurityId) -> Result<bool>;
    /// Removes the identifier under one key; whether one was held.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// the key.
    fn remove_securityid(&mut self, key: &SecType) -> Result<bool>;
    /// Derives one security identifier - one the element implies rather
    /// than states - filling only an absent key; whether it was added. A
    /// derived identifier never reaches a store the holder is a view of.
    fn derive_securityid(&mut self, id: SecurityId) -> bool;
    /// The detailed CFI classification, where one is known.
    fn get_cficode(&self) -> Option<&CfiCode>;
    /// Sets [`Self::get_cficode`].
    fn set_cficode(&mut self, code: Option<CfiCode>);
    /// The market the element trades on, where known.
    fn get_miccode(&self) -> Option<&MicCode>;
    /// Sets [`Self::get_miccode`].
    fn set_miccode(&mut self, code: Option<MicCode>);
    /// The last executed price: what the element's last execution traded
    /// at, never the price it states.
    fn get_lastpx(&self) -> Option<Decimal>;
    /// Sets [`Self::get_lastpx`].
    fn set_lastpx(&mut self, px: Option<Decimal>);
    /// The last executed quantity: what the element's last execution
    /// traded, never the quantity it states.
    fn get_lastqty(&self) -> Option<Decimal>;
    /// Sets [`Self::get_lastqty`].
    fn set_lastqty(&mut self, qty: Option<Decimal>);
    /// The average price of what the element has traded.
    fn get_avgpx(&self) -> Option<Decimal>;
    /// Sets [`Self::get_avgpx`].
    fn set_avgpx(&mut self, px: Option<Decimal>);
    /// How much the element has traded.
    fn get_cumqty(&self) -> Option<Decimal>;
    /// Sets [`Self::get_cumqty`].
    fn set_cumqty(&mut self, qty: Option<Decimal>);
    /// How much of the element is left to trade.
    fn get_leavesqty(&self) -> Option<Decimal>;
    /// Sets [`Self::get_leavesqty`].
    fn set_leavesqty(&mut self, qty: Option<Decimal>);
    /// The price the step before this one settled on.
    fn get_prevpx(&self) -> Option<Decimal>;
    /// Sets [`Self::get_prevpx`].
    fn set_prevpx(&mut self, px: Option<Decimal>);
    /// The quantity the step before this one settled on.
    fn get_prevqty(&self) -> Option<Decimal>;
    /// Sets [`Self::get_prevqty`].
    fn set_prevqty(&mut self, qty: Option<Decimal>);
    /// The spot part of an FX forward price.
    fn get_spotrate(&self) -> Option<Decimal>;
    /// Sets [`Self::get_spotrate`].
    fn set_spotrate(&mut self, rate: Option<Decimal>);
    /// The forward points of an FX forward price.
    fn get_forwardpoints(&self) -> Option<Decimal>;
    /// Sets [`Self::get_forwardpoints`].
    fn set_forwardpoints(&mut self, points: Option<Decimal>);
    /// The ticker the instrument goes by, where stated.
    fn get_ticker(&self) -> Option<&str>;
    /// Sets [`Self::get_ticker`].
    fn set_ticker(&mut self, ticker: Option<SmolStr>);
    /// The free-form facts the element carries; a shared empty map where it
    /// carries none.
    fn get_metadata(&self) -> &Metadata;
    /// Sets [`Self::get_metadata`]; `None` and an empty map are the same.
    fn set_metadata(&mut self, metadata: Option<Metadata>);

    /// Fills every market fact this element implies from the ones it
    /// states, and stops where it would be inventing.
    ///
    /// The price and the quantity are what the element states and nothing
    /// else: never a last executed price or quantity, which `lastpx` and
    /// `lastqty` answer, and never an average. How much is done and how
    /// much is left stay beside them, because together they *are* the
    /// quantity ordered and the dictionary already says so. An ISIN carries
    /// the national identifier
    /// of its country - a CUSIP, a SEDOL, a WKN, a Valor - which fills only
    /// a key the element does not state, as a derived identifier.
    ///
    /// Provided, and what an implementor's [`Element::finalize`] runs before
    /// it digests. Running it twice changes nothing the first run did not.
    fn fill_market(&mut self)
    where
        Self: Sized,
    {
        let embedded: Vec<SecurityId> = self
            .get_securityids()
            .get_id("ISIN")
            .and_then(|id| crate::IsinCode::new(id.code()).ok())
            .map(|isin| embedded(&isin).collect())
            .unwrap_or_default();
        for id in embedded {
            self.derive_securityid(id);
        }
    }

    /// Continues [`Element::digest`] with the market's facts.
    ///
    /// Provided, for an implementor's [`Element::finalize`] to feed its own
    /// content behind.
    fn digest_market(&self) -> Xxh3
    where
        Self: Element + Sized,
    {
        let mut state = self.digest();
        feed_market(&mut state, self);
        state
    }

    /// This element merged with another statement of itself, where the two
    /// are the same element; `None` where they are not or nothing moved.
    fn merging_market(mut self, other: &Self) -> Option<Self>
    where
        Self: Element + Sized,
    {
        if other.get_curruuid() != self.get_curruuid() {
            return None;
        }
        let changed = merge_element(&mut self, other);
        if !(merge_market(&mut self, other, false) || changed) {
            return None;
        }
        self.finalize();
        Some(self)
    }

    /// [`Event::digest_event`] continued with the market's facts: what a
    /// market event that is also an [`Event`] digests to.
    fn digest_market_event(&self) -> Xxh3
    where
        Self: Event + Sized,
    {
        let mut state = self.digest_event();
        feed_market(&mut state, self);
        state
    }

    /// This event as the one after `previous` in its chain, with the market
    /// facts the chain carries; `None` where it cannot follow it.
    fn following_market(mut self, previous: &Self) -> Option<Self>
    where
        Self: Event + Sized,
    {
        if previous.get_curruuid() == self.get_curruuid()
            || previous.get_currunix() > self.get_currunix()
        {
            return None;
        }
        let changed = follow_timed(&mut self, previous);
        if !(follow_market(&mut self, previous) || changed) {
            return None;
        }
        self.finalize();
        Some(self)
    }

    /// This event merged with another statement of itself, the reference
    /// chosen by its recording clock; `None` where they differ or nothing
    /// moved.
    fn merging_market_event(mut self, other: &Self) -> Option<Self>
    where
        Self: Event + Sized,
    {
        if other.get_curruuid() != self.get_curruuid() {
            return None;
        }
        let other_is_reference = right_is_reference(
            self.get_recdunix(),
            self.get_currunix(),
            other.get_recdunix(),
            other.get_currunix(),
        );
        if !merge_market_event(&mut self, other, other_is_reference) {
            return None;
        }
        self.finalize();
        Some(self)
    }
}

/// A market element that is also an operation on the market: what it adds
/// to the slim facts.
pub trait Operation: Market {
    /// The stable numeric market-operation category, where known.
    fn get_marketoperationid(&self) -> Option<i32>;
    /// Sets [`Self::get_marketoperationid`].
    fn set_marketoperationid(&mut self, marketoperationid: Option<i32>);
    /// How long the operation stands, where stated.
    fn get_tif(&self) -> Option<&TimeInForce>;
    /// Sets [`Self::get_tif`].
    fn set_tif(&mut self, tif: Option<TimeInForce>);
    /// Whether the instrument can trade, where a status says.
    fn get_tradable(&self) -> Option<bool>;
    /// Sets [`Self::get_tradable`].
    fn set_tradable(&mut self, tradable: Option<bool>);
    /// The accounts the operation is for, source key to account.
    fn get_accountids(&self) -> &IdMap;
    /// Replaces [`Self::get_accountids`].
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// one of the keys.
    fn set_accountids(&mut self, ids: IdMap) -> Result<()>;
    /// States one account, filling only an absent key; whether it was added.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// the key, or the key or value breaks [`IdMap`]'s rules.
    fn insert_accountid(&mut self, key: &str, value: &str) -> Result<bool>;
    /// Removes one account; whether one was held.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// the key.
    fn remove_accountid(&mut self, key: &str) -> Result<bool>;
    /// The users the operation is by, source key to user.
    fn get_userids(&self) -> &IdMap;
    /// Replaces [`Self::get_userids`].
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// one of the keys.
    fn set_userids(&mut self, ids: IdMap) -> Result<()>;
    /// States one user, filling only an absent key; whether it was added.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// the key, or the key or value breaks [`IdMap`]'s rules.
    fn insert_userid(&mut self, key: &str, value: &str) -> Result<bool>;
    /// Removes one user; whether one was held.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// the key.
    fn remove_userid(&mut self, key: &str) -> Result<bool>;
    /// The operation's own identifiers, source key to identifier.
    fn get_altids(&self) -> &IdMap;
    /// Replaces [`Self::get_altids`].
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// one of the keys.
    fn set_altids(&mut self, ids: IdMap) -> Result<()>;
    /// States one identifier, filling only an absent key; whether it was
    /// added.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// the key, or the key or value breaks [`IdMap`]'s rules.
    fn insert_altid(&mut self, key: &str, value: &str) -> Result<bool>;
    /// Removes one identifier; whether one was held.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// the key.
    fn remove_altid(&mut self, key: &str) -> Result<bool>;
    /// Whether an operation that follows another carries the alternate
    /// identifier under `key`: [`FOLLOWED_ALTIDS`] unless the holder's own
    /// dictionary says otherwise.
    fn is_followed_altid(&self, key: &str) -> bool {
        FOLLOWED_ALTIDS.contains(&key)
    }
    /// The bid lane a quote states, where it states one.
    fn get_bid(&self) -> Option<&Lane>;
    /// Sets [`Self::get_bid`]; a lane stating nothing is `None`.
    fn set_bid(&mut self, lane: Option<Lane>);
    /// The ask lane a quote states, where it states one.
    fn get_ask(&self) -> Option<&Lane>;
    /// Sets [`Self::get_ask`]; a lane stating nothing is `None`.
    fn set_ask(&mut self, lane: Option<Lane>);

    /// Continues [`Market::fill_market`] with what an operation implies.
    ///
    /// 0. A quote stating one lane and no side is that lane's side: a bid
    ///    alone is a party willing to pay, so the element is a buy, and an
    ///    ask alone one willing to be paid, a sell. A quote is an element
    ///    the lane is the only price of: one stating a price, a last trade
    ///    or an average of its own is about that, and a lane beside it is
    ///    context. A side the element states stands, and a quote stating
    ///    both lanes or neither names no side.
    /// 1. The price, the quantity, the currency, the unit and the FX parts
    ///    are the element's own, else what its own side's lane quotes.
    /// 2. The side's lane is then filled from all of that, by
    ///    [`Self::fill_lanes`].
    ///
    /// Nothing is invented and a stated fact is never overwritten. Running
    /// it twice changes nothing the first run did not.
    fn fill_operation(&mut self)
    where
        Self: Sized,
    {
        if self.get_side() == Side::Unknown
            && self.get_price().is_none()
            && self.get_lastpx().is_none()
            && self.get_avgpx().is_none()
        {
            let named = match lanes_stated(self) {
                (true, false) => Some(Side::Buy),
                (false, true) => Some(Side::Sell),
                _ => None,
            };
            if let Some(side) = named {
                self.set_side(side);
            }
        }
        let side = self.get_side();
        let lane = if side.is_bid() {
            self.get_bid().cloned()
        } else if side.is_ask() {
            self.get_ask().cloned()
        } else {
            None
        };
        if let Some(lane) = lane {
            if self.get_price().is_none() {
                self.set_price(lane.price);
            }
            if self.get_quantity().is_none() {
                self.set_quantity(lane.quantity);
            }
            if *self.get_currency() == Ccy::none() {
                if let Some(currency) = lane.currency {
                    self.set_currency(currency);
                }
            }
            if self.get_unit().is_none() {
                if let Some(unit) = lane.unit {
                    self.set_unit(unit);
                }
            }
            if self.get_spotrate().is_none() {
                self.set_spotrate(lane.spotrate);
            }
            if self.get_forwardpoints().is_none() {
                self.set_forwardpoints(lane.forwardpoints);
            }
        }
        self.fill_lanes();
    }

    /// Fills the lane the side implies from the element's own facts, where
    /// the lane states nothing of its own: a buy at a price is a party
    /// willing to pay it, and a sell at one a party willing to be paid it.
    /// Only a stated fact fills a lane - no price or quantity, no currency,
    /// no unit, is nothing to state on the lane either - and a
    /// side taking neither lane fills nothing.
    fn fill_lanes(&mut self)
    where
        Self: Sized,
    {
        let side = self.get_side();
        if !side.is_bid() && !side.is_ask() {
            return;
        }
        let own = Lane {
            price: self.get_price(),
            spotrate: self.get_spotrate(),
            forwardpoints: self.get_forwardpoints(),
            currency: Some(self.get_currency().clone()).filter(|held| *held != Ccy::none()),
            quantity: self.get_quantity(),
            unit: Some(self.get_unit().clone()).filter(|unit| !unit.is_none()),
        };
        let held = if side.is_bid() {
            self.get_bid()
        } else {
            self.get_ask()
        };
        let filled = held.map_or_else(|| own.clone(), |held| held.merged(&own, false));
        if Some(&filled) == held || (!filled.is_stated() && held.is_none()) {
            return;
        }
        if side.is_bid() {
            self.set_bid(filled.stated());
        } else {
            self.set_ask(filled.stated());
        }
    }

    /// Continues [`Market::digest_market`] with the operation's facts.
    fn digest_operation(&self) -> Xxh3
    where
        Self: Element + Sized,
    {
        let mut state = self.digest_market();
        feed_operation(&mut state, self);
        state
    }

    /// This operation merged with another statement of itself, where the
    /// two are the same element; `None` where they are not or nothing moved.
    fn merging_operation(mut self, other: &Self) -> Option<Self>
    where
        Self: Element + Sized,
    {
        if other.get_curruuid() != self.get_curruuid() {
            return None;
        }
        let mut changed = merge_element(&mut self, other);
        changed |= merge_market(&mut self, other, false);
        if !(merge_operation(&mut self, other, false) || changed) {
            return None;
        }
        self.finalize();
        Some(self)
    }

    /// [`Event::digest_event`] continued with the market's and the
    /// operation's facts.
    fn digest_operation_event(&self) -> Xxh3
    where
        Self: Event + Sized,
    {
        let mut state = self.digest_event();
        feed_market(&mut state, self);
        feed_operation(&mut state, self);
        state
    }

    /// This operation as the one after `previous` in its chain, with the
    /// market and operation facts the chain carries; `None` where it cannot
    /// follow it.
    fn following_operation(mut self, previous: &Self) -> Option<Self>
    where
        Self: Event + Sized,
    {
        if previous.get_curruuid() == self.get_curruuid()
            || previous.get_currunix() > self.get_currunix()
        {
            return None;
        }
        let mut changed = follow_timed(&mut self, previous);
        changed |= follow_market_of_operation(&mut self, previous);
        if !(follow_operation(&mut self, previous) || changed) {
            return None;
        }
        self.finalize();
        Some(self)
    }

    /// This operation merged with another statement of itself, the
    /// reference chosen by its recording clock; `None` where they differ or
    /// nothing moved.
    fn merging_operation_event(mut self, other: &Self) -> Option<Self>
    where
        Self: Event + Sized,
    {
        if other.get_curruuid() != self.get_curruuid() {
            return None;
        }
        let other_is_reference = right_is_reference(
            self.get_recdunix(),
            self.get_currunix(),
            other.get_recdunix(),
            other.get_currunix(),
        );
        if !merge_operation_event(&mut self, other, other_is_reference) {
            return None;
        }
        self.finalize();
        Some(self)
    }
}

/// What restating means for a market event that states no operation of its
/// own: the timed restatement, then the market's facts.
pub(crate) fn restating_market<E: Event + Market>(mut this: E, live: &E) -> E {
    restate_event(&mut this, live);
    fold_event_instants(&mut this, live);
    this.fold_lifecycle(live);
    restate_market(&mut this, live);
    this.finalize();
    this
}

/// What restating means for a market operation: [`restating_market`]
/// continued with the operation's facts.
pub(crate) fn restating_operation<E: Event + Operation>(mut this: E, live: &E) -> E {
    restate_event(&mut this, live);
    fold_event_instants(&mut this, live);
    this.fold_lifecycle(live);
    restate_market_of_operation(&mut this, live);
    restate_operation(&mut this, live);
    this.finalize();
    this
}

/// Merges a market event into the reference statement it supplements,
/// finalizing where anything moved.
pub(crate) fn merge_market_event_into_reference<E: Event + Market>(this: &mut E, supplement: &E) {
    if merge_market_event(this, supplement, false) {
        this.finalize();
    }
}

/// Merges a market operation into the reference statement it supplements,
/// finalizing where anything moved.
pub(crate) fn merge_operation_event_into_reference<E: Event + Operation>(
    this: &mut E,
    supplement: &E,
) {
    if merge_operation_event(this, supplement, false) {
        this.finalize();
    }
}

fn merge_market_event<E: Event + Market>(
    this: &mut E,
    other: &E,
    other_is_reference: bool,
) -> bool {
    let changed = merge_event_element(this, other, other_is_reference);
    let timed = merge_timed(this, other, other_is_reference);
    merge_market(this, other, other_is_reference) || timed || changed
}

fn merge_operation_event<E: Event + Operation>(
    this: &mut E,
    other: &E,
    other_is_reference: bool,
) -> bool {
    let changed = merge_market_event(this, other, other_is_reference);
    merge_operation(this, other, other_is_reference) || changed
}

/// Feeds the market's facts to a digest, each under its name: what the
/// element states, never an identity, an instant or the step before it.
pub(crate) fn feed_market<E: Market + ?Sized>(state: &mut Xxh3, this: &E) {
    let mut staged = Staged::new(state);
    if let Some(price) = this.get_price() {
        staged.feed("price", &price.units().to_le_bytes());
    }
    staged.feed("currency", this.get_currency().as_str().as_bytes());
    if let Some(quantity) = this.get_quantity() {
        staged.feed("quantity", &quantity.units().to_le_bytes());
    }
    staged.feed("unit", this.get_unit().as_str().as_bytes());
    staged.feed("side", this.get_side().as_str().as_bytes());
    for id in this.get_securityids() {
        staged.feed("securityids", id.sectype().as_str().as_bytes());
        staged.feed("securityids", id.code().as_bytes());
    }
    if let Some(code) = this.get_cficode() {
        staged.feed("cficode", code.as_str().as_bytes());
    }
    if let Some(code) = this.get_miccode() {
        staged.feed("miccode", code.as_str().as_bytes());
    }
    for (name, held) in [
        ("lastpx", this.get_lastpx()),
        ("lastqty", this.get_lastqty()),
        ("avgpx", this.get_avgpx()),
        ("cumqty", this.get_cumqty()),
        ("leavesqty", this.get_leavesqty()),
        ("spotrate", this.get_spotrate()),
        ("forwardpoints", this.get_forwardpoints()),
    ] {
        if let Some(held) = held {
            staged.feed(name, &held.units().to_le_bytes());
        }
    }
    if let Some(ticker) = this.get_ticker() {
        staged.feed("ticker", ticker.as_bytes());
    }
    for (key, value) in this.get_metadata() {
        staged.feed(key, value.as_bytes());
    }
}

/// Feeds the operation's facts to a digest, each under its name.
pub(crate) fn feed_operation<E: Operation + ?Sized>(state: &mut Xxh3, this: &E) {
    let mut staged = Staged::new(state);
    if let Some(marketoperationid) = this.get_marketoperationid() {
        staged.feed("marketoperationid", &marketoperationid.to_le_bytes());
    }
    if let Some(tif) = this.get_tif() {
        staged.feed("tif", tif.as_str().as_bytes());
    }
    if let Some(tradable) = this.get_tradable() {
        staged.feed("tradable", &[u8::from(tradable)]);
    }
    for (label, map) in [
        ("accountids", this.get_accountids()),
        ("userids", this.get_userids()),
        ("altids", this.get_altids()),
    ] {
        for (key, value) in map.iter() {
            staged.feed(label, key.as_bytes());
            staged.feed(label, value.as_bytes());
        }
    }
    if let Some(lane) = this.get_bid() {
        lane.feed(&mut staged, "bid");
    }
    if let Some(lane) = this.get_ask() {
        lane.feed(&mut staged, "ask");
    }
}

/// The market facts an event takes from the statement it follows: the
/// price and the quantity that statement settled on as the step before this
/// one, and what the chain is about where this statement says nothing.
pub(crate) fn follow_market<E: Market + ?Sized>(this: &mut E, previous: &E) -> bool {
    follow_market_facts(this, previous, true)
}

/// [`follow_market`] for an operation: one quoting a lane of its own says
/// its side itself - one lane names it and two name none - so the chain's
/// side is not its to take.
pub(crate) fn follow_market_of_operation<E: Operation + ?Sized>(
    this: &mut E,
    previous: &E,
) -> bool {
    let side_from_chain = lanes_stated(this) == (false, false);
    follow_market_facts(this, previous, side_from_chain)
}

fn follow_market_facts<E: Market + ?Sized>(
    this: &mut E,
    previous: &E,
    side_from_chain: bool,
) -> bool {
    let mut changed = false;
    if this.get_prevpx().is_none() {
        let px = previous.get_price();
        changed |= moved(this.get_prevpx(), px, |px| this.set_prevpx(px));
    }
    if this.get_prevqty().is_none() {
        let qty = previous.get_quantity();
        changed |= moved(this.get_prevqty(), qty, |qty| this.set_prevqty(qty));
    }
    changed | chain_market(this, previous, side_from_chain)
}

fn restate_market<E: Market + ?Sized>(this: &mut E, live: &E) -> bool {
    restate_market_facts(this, live, true)
}

/// [`restate_market`] for an operation, its side under the rule of
/// [`follow_market_of_operation`].
fn restate_market_of_operation<E: Operation + ?Sized>(this: &mut E, live: &E) -> bool {
    let side_from_chain = lanes_stated(this) == (false, false);
    restate_market_facts(this, live, side_from_chain)
}

fn restate_market_facts<E: Market + ?Sized>(this: &mut E, live: &E, side_from_chain: bool) -> bool {
    let mut changed = moved(
        this.get_prevpx(),
        stated(this.get_prevpx(), live.get_prevpx(), false),
        |px| this.set_prevpx(px),
    );
    changed |= moved(
        this.get_prevqty(),
        stated(this.get_prevqty(), live.get_prevqty(), false),
        |qty| this.set_prevqty(qty),
    );
    changed | chain_market(this, live, side_from_chain)
}

/// What the chain an element stands in is about, taken from another
/// statement of that chain where this one says nothing of it. This
/// statement always leads, and nothing here is about a step. The side is
/// the chain's to give only where `side_from_chain` says so: an operation
/// quoting a lane of its own names its side itself.
fn chain_market<E: Market + ?Sized>(this: &mut E, previous: &E, side_from_chain: bool) -> bool {
    let mut changed = moved(
        this.get_currency().clone(),
        better(this.get_currency().clone(), previous.get_currency(), false),
        |currency| this.set_currency(currency),
    );
    if side_from_chain {
        changed |= moved(
            this.get_side(),
            this.get_side().merge_with(&previous.get_side()),
            |side| this.set_side(side),
        );
    }
    changed |= moved(
        this.get_unit().clone(),
        better(this.get_unit().clone(), previous.get_unit(), false),
        |unit| this.set_unit(unit),
    );
    if this.get_ticker().is_none() {
        if let Some(ticker) = previous.get_ticker() {
            this.set_ticker(Some(SmolStr::new(ticker)));
            changed = true;
        }
    }
    for id in previous.get_securityids() {
        if !this.get_securityids().contains_key(id.sectype().as_str()) {
            changed |= this.insert_securityid(id.clone()).unwrap_or(false);
        }
    }
    changed |= moved(
        this.get_cficode().cloned(),
        better_stated(this.get_cficode().cloned(), previous.get_cficode(), false),
        |code| this.set_cficode(code),
    );
    changed |= moved(
        this.get_miccode().cloned(),
        better_stated(this.get_miccode().cloned(), previous.get_miccode(), false),
        |code| this.set_miccode(code),
    );
    changed
}

/// The market facts an element takes from another statement of itself:
/// the leading statement's price, quantity and unit, each optional fact the
/// selected statement states, and each code the better of the two. `later`
/// says whether `other` is the leading statement.
pub(crate) fn merge_market<E: Market + ?Sized>(this: &mut E, other: &E, later: bool) -> bool {
    let mut changed = false;
    if later {
        changed |= moved(this.get_price(), other.get_price(), |px| this.set_price(px));
        changed |= moved(this.get_quantity(), other.get_quantity(), |qty| {
            this.set_quantity(qty)
        });
        changed |= moved(this.get_unit().clone(), other.get_unit().clone(), |unit| {
            this.set_unit(unit)
        });
    }
    macro_rules! optional {
        ($get:ident, $set:ident) => {
            changed |= moved(
                this.$get(),
                stated(this.$get(), other.$get(), later),
                |held| this.$set(held),
            );
        };
    }
    optional!(get_lastpx, set_lastpx);
    optional!(get_lastqty, set_lastqty);
    optional!(get_avgpx, set_avgpx);
    optional!(get_cumqty, set_cumqty);
    optional!(get_leavesqty, set_leavesqty);
    optional!(get_prevpx, set_prevpx);
    optional!(get_prevqty, set_prevqty);
    optional!(get_spotrate, set_spotrate);
    optional!(get_forwardpoints, set_forwardpoints);
    changed |= moved(
        this.get_currency().clone(),
        better(this.get_currency().clone(), other.get_currency(), later),
        |currency| this.set_currency(currency),
    );
    changed |= moved(
        this.get_side(),
        better(this.get_side(), &other.get_side(), later),
        |side| this.set_side(side),
    );
    changed |= moved(
        this.get_ticker().map(SmolStr::new),
        stated(
            this.get_ticker().map(SmolStr::new),
            other.get_ticker().map(SmolStr::new),
            later,
        ),
        |ticker| this.set_ticker(ticker),
    );
    for id in other.get_securityids() {
        let key = id.sectype();
        match this.get_securityids().get(key.as_str()) {
            Some(held) if held == id.code() => {}
            Some(_) if later => {
                let _ = this.remove_securityid(&key);
                changed |= this.insert_securityid(id.clone()).unwrap_or(false);
            }
            Some(_) => {}
            None => changed |= this.insert_securityid(id.clone()).unwrap_or(false),
        }
    }
    changed |= moved(
        this.get_cficode().cloned(),
        better_stated(this.get_cficode().cloned(), other.get_cficode(), later),
        |code| this.set_cficode(code),
    );
    changed |= moved(
        this.get_miccode().cloned(),
        better_stated(this.get_miccode().cloned(), other.get_miccode(), later),
        |code| this.set_miccode(code),
    );
    if !other.get_metadata().is_empty() {
        let mut merged = if later {
            other.get_metadata().clone()
        } else {
            this.get_metadata().clone()
        };
        let supplement = if later {
            this.get_metadata()
        } else {
            other.get_metadata()
        };
        for (key, value) in supplement {
            merged.entry(key.clone()).or_insert_with(|| value.clone());
        }
        if &merged != this.get_metadata() {
            this.set_metadata(Some(merged));
            changed = true;
        }
    }
    changed
}

/// The operation facts an event takes from the statement it follows: the
/// accounts and users whole, the order's own identifiers, and how long it
/// stands and whether it can trade where this statement says nothing.
pub(crate) fn follow_operation<E: Operation + ?Sized>(this: &mut E, previous: &E) -> bool {
    chain_operation(this, previous)
}

fn restate_operation<E: Operation + ?Sized>(this: &mut E, live: &E) -> bool {
    chain_operation(this, live)
}

fn chain_operation<E: Operation + ?Sized>(this: &mut E, previous: &E) -> bool {
    let mut changed = moved(
        this.get_tif().cloned(),
        stated(this.get_tif().cloned(), previous.get_tif().cloned(), false),
        |tif| this.set_tif(tif),
    );
    changed |= moved(
        this.get_tradable(),
        stated(this.get_tradable(), previous.get_tradable(), false),
        |tradable| this.set_tradable(tradable),
    );
    for (key, value) in previous.get_accountids().iter() {
        if !this.get_accountids().contains_key(key) {
            changed |= this.insert_accountid(key, value).unwrap_or(false);
        }
    }
    for (key, value) in previous.get_userids().iter() {
        if !this.get_userids().contains_key(key) {
            changed |= this.insert_userid(key, value).unwrap_or(false);
        }
    }
    for (key, value) in previous.get_altids().iter() {
        if this.is_followed_altid(key) && !this.get_altids().contains_key(key) {
            changed |= this.insert_altid(key, value).unwrap_or(false);
        }
    }
    changed
}

/// The operation facts an element takes from another statement of itself.
pub(crate) fn merge_operation<E: Operation + ?Sized>(this: &mut E, other: &E, later: bool) -> bool {
    let mut changed = moved(
        this.get_marketoperationid(),
        stated(
            this.get_marketoperationid(),
            other.get_marketoperationid(),
            later,
        ),
        |marketoperationid| this.set_marketoperationid(marketoperationid),
    );
    changed |= moved(
        this.get_tif().cloned(),
        better_stated(this.get_tif().cloned(), other.get_tif(), later),
        |tif| this.set_tif(tif),
    );
    changed |= moved(
        this.get_tradable(),
        stated(this.get_tradable(), other.get_tradable(), later),
        |tradable| this.set_tradable(tradable),
    );
    macro_rules! map {
        ($get:ident, $set:ident) => {
            if !other.$get().is_empty() {
                let mut merged = if later {
                    other.$get().clone()
                } else {
                    this.$get().clone()
                };
                let supplement = if later { this.$get() } else { other.$get() };
                merged.merge(supplement);
                if &merged != this.$get() && this.$set(merged).is_ok() {
                    changed = true;
                }
            }
        };
    }
    map!(get_accountids, set_accountids);
    map!(get_userids, set_userids);
    map!(get_altids, set_altids);
    let bid = merged_lane(this.get_bid(), other.get_bid(), later);
    if bid.as_ref() != this.get_bid() {
        this.set_bid(bid);
        changed = true;
    }
    let ask = merged_lane(this.get_ask(), other.get_ask(), later);
    if ask.as_ref() != this.get_ask() {
        this.set_ask(ask);
        changed = true;
    }
    changed
}

fn merged_lane(this: Option<&Lane>, other: Option<&Lane>, later: bool) -> Option<Lane> {
    match (this, other) {
        (Some(held), Some(next)) => Some(held.merged(next, later)),
        (Some(held), None) => Some(held.clone()),
        (None, Some(next)) => Some(next.clone()),
        (None, None) => None,
    }
}

/// Whether an operation's bid lane and its ask lane each state anything.
fn lanes_stated<E: Operation + ?Sized>(this: &E) -> (bool, bool) {
    (
        this.get_bid().is_some_and(Lane::is_stated),
        this.get_ask().is_some_and(Lane::is_stated),
    )
}

/// The better of two statements of one code: the selected statement
/// leading, the other filling what it leaves unknown.
fn better<C: CodeValue>(this: C, other: &C, later: bool) -> C {
    if later {
        other.clone().merge_with(&this)
    } else {
        this.merge_with(other)
    }
}

/// [`better`] where each statement may name no code at all: the one that
/// names one, or nothing where neither does.
fn better_stated<C: CodeValue>(this: Option<C>, other: Option<&C>, later: bool) -> Option<C> {
    match (this, other) {
        (Some(this), Some(other)) => Some(better(this, other, later)),
        (Some(this), None) => Some(this),
        (None, other) => other.cloned(),
    }
}
