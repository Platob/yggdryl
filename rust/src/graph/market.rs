//! The market vocabulary: what an element that stands in a market answers,
//! and what an operation on that market adds.
//!
//! [`Market`] is the slim reading a book level or any plain struct gives
//! cheaply: the instrument it is about - its security identifiers, its
//! classification, the market it trades on and the ticker it goes by - the
//! side it takes, what it is priced and counted in, the price and quantity
//! it is about, its last executed price and quantity and its average, how far it has got, the
//! step before it, the two FX parts of a price, the FX rates to other
//! currencies, and free-form metadata.
//! [`Operation`] is a market element that is also an operation: how long
//! it stands, whether it can trade and the alternate identifiers it is known
//! by. What an [`Event`] that is also one of them answers -
//! [`Market::digest_market_event`], [`Market::following_market`],
//! [`Market::merging_market_event`] and their [`Operation`] counterparts -
//! is provided on the trait itself, gated `where Self: Event`, rather than
//! a separate blanket trait. The traits state signatures
//! and the provided readings - fill, digest, follow, restate, merge - so a
//! message, a book entry and a lifecycle incarnation can each be a market
//! without the graph owning any of them.

use std::borrow::Cow;
use std::collections::BTreeMap;

use smol_str::SmolStr;

use super::element::{
    Staged, earliest, fold_event_instants, follow_timed, latest, merge_element,
    merge_event_element, merge_timed, moved, restate_event, right_is_reference, stated,
};
use super::{Element, Event};
use crate::CodeValue;
use crate::idmap::IdMap;
use crate::securityid::{SecType, SecurityId, SecurityIds, embedded};
use crate::xxhash::Xxh3;
use crate::{Ccy, Cfi, Decimal, Mic, Result, Side, TimeInForce, Unit};

/// Free-form facts a market element carries beside its typed ones: never an
/// identifier, which has a typed home in [`Market::get_securityids`] or an
/// operation's alternate identifiers.
pub type Metadata = BTreeMap<SmolStr, SmolStr>;

static EMPTY_METADATA: Metadata = BTreeMap::new();

/// The metadata an element that holds none answers: one shared empty map.
#[must_use]
pub fn empty_metadata() -> &'static Metadata {
    &EMPTY_METADATA
}

/// The FX rates a market element states, sorted by target currency: an
/// amount in the element's currency divided by the rate under a target is
/// that amount in the target.
pub type FxRates = BTreeMap<Ccy, Decimal>;

static EMPTY_FXRATES: FxRates = BTreeMap::new();

/// The rates an element that states none answers: one shared empty map.
#[must_use]
pub fn empty_fxrates() -> &'static FxRates {
    &EMPTY_FXRATES
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
    /// Sets [`Self::get_side`]; a sided element's cross code is stored under
    /// the side taken ([`Self::sided_crosscode`]).
    fn set_side(&mut self, side: Side);
    /// The category the element is filed under - an order `ORDR`, a quote
    /// `QUOT`, an execution `EXEC`, a trade `TRAD`, a book `BOOK` - which
    /// its holder stamps: a lifecycle chains elements of one category only,
    /// so an order and an execution under one cross code are two chains.
    fn marketdatakind(&self) -> crate::MarketDataKind;
    /// Whether the element's cross code is stored under its side: whether
    /// its kind is an order, a quote or an execution
    /// ([`MarketDataKind::is_sided`](crate::MarketDataKind::is_sided)).
    /// Every other element - a trade, a book, a snapshot control, a message
    /// of any other category - keeps its cross code as given, whatever side
    /// it states.
    fn is_sided(&self) -> bool {
        self.marketdatakind().is_sided()
    }
    /// The security identifiers the element names, one code per key.
    fn get_securityids(&self) -> &SecurityIds;
    /// Replaces the stated security identifiers.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// one of the keys.
    fn set_securityids(&mut self, ids: SecurityIds) -> Result<()>;
    /// States one security identifier, filling an absent key or replacing
    /// a derived identifier, never a stated one; whether it was added.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// the key.
    fn insert_securityid(&mut self, id: SecurityId) -> Result<bool>;
    /// Removes the identifier under one key; whether one was held. Removing
    /// the ISIN takes back every derived identifier, since each hangs on it.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// the key.
    fn remove_securityid(&mut self, key: &SecType) -> Result<bool>;
    /// Derives one security identifier - one the element implies rather
    /// than states: the national number its ISIN carries, what a lifecycle
    /// learned under it - filling only an absent key; whether it was added.
    /// A derived identifier never reaches a store the holder is a view of.
    fn derive_securityid(&mut self, id: SecurityId) -> bool;
    /// The detailed CFI classification, where one is known.
    fn get_cficode(&self) -> Option<&Cfi>;
    /// Sets [`Self::get_cficode`].
    fn set_cficode(&mut self, code: Option<Cfi>);
    /// The market the element trades on, where known.
    fn get_miccode(&self) -> Option<&Mic>;
    /// Sets [`Self::get_miccode`].
    fn set_miccode(&mut self, code: Option<Mic>);
    /// When the element last executed: the latest execution clock its
    /// lifecycle reached, nanoseconds since the Unix epoch, UTC, where it
    /// knows. A market fact rather than an event's: an event that is a
    /// market element and whose state reports an execution with no clock of
    /// its own dates it from its own instant, following carries the later
    /// of its own and its predecessor's, and two statements of one event
    /// keep the earliest either knows.
    fn get_execunix(&self) -> Option<i64>;
    /// Sets [`Self::get_execunix`]; `None` states it does not know.
    fn set_execunix(&mut self, unix: Option<i64>);
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
    /// The FX rates the element states, target currency to the rate to
    /// divide by: an amount in [`Self::get_currency`] divided by
    /// `rates[target]` is that amount in `target`. A shared empty map where
    /// it states none.
    fn get_fxrates(&self) -> &FxRates;
    /// Replaces [`Self::get_fxrates`]; an empty map states none.
    fn set_fxrates(&mut self, rates: FxRates);
    /// The bid price the element states: a quote's `BidPx(132)`, a book's
    /// best tradable bid level; `None` where it states none. Never filled
    /// from the element's own price or its last executed price.
    fn get_bidpx(&self) -> Option<Decimal>;
    /// Sets [`Self::get_bidpx`].
    fn set_bidpx(&mut self, px: Option<Decimal>);
    /// The quantity bid at [`Self::get_bidpx`]: a quote's `BidSize(134)`, a
    /// book's best tradable bid level's; `None` where it states none.
    fn get_bidqty(&self) -> Option<Decimal>;
    /// Sets [`Self::get_bidqty`].
    fn set_bidqty(&mut self, qty: Option<Decimal>);
    /// The currency the bid is stated in, where the element states a bid.
    fn get_bidccy(&self) -> Option<&Ccy>;
    /// Sets [`Self::get_bidccy`].
    fn set_bidccy(&mut self, ccy: Option<Ccy>);
    /// The ask price the element states: a quote's `OfferPx(133)`, a book's
    /// best tradable ask level; `None` where it states none.
    fn get_askpx(&self) -> Option<Decimal>;
    /// Sets [`Self::get_askpx`].
    fn set_askpx(&mut self, px: Option<Decimal>);
    /// The quantity offered at [`Self::get_askpx`]: a quote's
    /// `OfferSize(135)`, a book's best tradable ask level's.
    fn get_askqty(&self) -> Option<Decimal>;
    /// Sets [`Self::get_askqty`].
    fn set_askqty(&mut self, qty: Option<Decimal>);
    /// The currency the ask is stated in, where the element states an ask.
    fn get_askccy(&self) -> Option<&Ccy>;
    /// Sets [`Self::get_askccy`].
    fn set_askccy(&mut self, ccy: Option<Ccy>);

    /// The cross code `code` is stored as for this element. A sided element
    /// ([`Self::is_sided`]: an order, a quote or an execution) stores it as
    /// `"{SIDE}:{code}"` under the four-letter code of the side it takes -
    /// `BUYS:ORD-1` - so the two sides of one identifier are two chains, and
    /// as `code` itself where it takes [`Side::Unknown`]. Idempotent: a code
    /// already carrying this side's prefix is answered as it is, and one
    /// carrying another side's has that prefix replaced. Any other element -
    /// a trade, a book, a snapshot control, a message of another category -
    /// is not strictly sided and answers `code` as given. The one place
    /// the prefix is decided; every market holder stores its cross code
    /// through it, so setting the side after the code converges on the same
    /// answer.
    ///
    /// ```
    /// use yggdryl::graph::{BookEvent, Element, Market, OrderEvent};
    /// use yggdryl::Side;
    ///
    /// let mut order = OrderEvent::at(1);
    /// order.set_side(Side::Buy);
    /// assert_eq!(order.sided_crosscode("ORD-1"), "BUYS:ORD-1");
    /// assert_eq!(order.sided_crosscode("SELL:ORD-1"), "BUYS:ORD-1");
    /// order.set_crosscode("ORD-1".to_owned());
    /// assert_eq!(order.get_crosscode(), "BUYS:ORD-1");
    /// order.set_side(Side::Sell);
    /// assert_eq!(order.get_crosscode(), "SELL:ORD-1");
    ///
    /// // A book is not sided: its code stays as given whatever side it states.
    /// let mut book = BookEvent::new(1, "AAPL");
    /// book.set_side(Side::Buy);
    /// assert_eq!((book.get_crosscode(), book.sided_crosscode("AAPL")), ("AAPL", "AAPL".into()));
    /// ```
    fn sided_crosscode<'code>(&self, code: &'code str) -> Cow<'code, str> {
        if self.is_sided() {
            sided_crosscode(self.get_side(), code)
        } else {
            Cow::Borrowed(code)
        }
    }

    /// States the rate to one target currency, filling a target the element
    /// states no rate for, never replacing one it does; whether it was
    /// added.
    ///
    /// ```
    /// use yggdryl::graph::{Market, OrderEvent};
    /// use yggdryl::Ccy;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut order = OrderEvent::at(1);
    /// order.set_currency(Ccy::new("EUR")?);
    /// assert!(order.insert_fxrate(Ccy::new("USD")?, "0.8".parse()?));
    /// assert!(!order.insert_fxrate(Ccy::new("USD")?, "0.9".parse()?));
    /// assert_eq!(order.get_fxrates()[&Ccy::new("USD")?], "0.8".parse()?);
    /// # Ok(())
    /// # }
    /// ```
    fn insert_fxrate(&mut self, target: Ccy, rate: Decimal) -> bool {
        if self.get_fxrates().contains_key(&target) {
            return false;
        }
        let mut rates = self.get_fxrates().clone();
        rates.insert(target, rate);
        self.set_fxrates(rates);
        true
    }

    /// The ISIN the element names: its `ISIN` security identifier, borrowed
    /// - a projection of [`Self::get_securityids`], never a second store.
    fn get_isincode(&self) -> Option<&str> {
        self.get_securityids().get("ISIN")
    }

    /// The key of the book the element stands in: its ticker where it
    /// states one, else its category - `{miccode}:{cficode}`, `XXXX` for a
    /// market it names none of and `XXXXXX` for a classification it knows
    /// none of. Borrowed where the ticker answers, so a ticker input
    /// allocates nothing.
    fn book_crosscode(&self) -> Cow<'_, str> {
        match self.get_ticker().filter(|ticker| !ticker.is_empty()) {
            Some(ticker) => Cow::Borrowed(ticker),
            None => Cow::Owned(format!(
                "{}:{}",
                self.get_miccode().map_or(Mic::NONE, |code| code.as_str()),
                self.get_cficode()
                    .map_or(Cfi::UNCLASSIFIED, |code| code.as_str())
            )),
        }
    }

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
        // The ISIN is owned - a code inline - so its national identifiers
        // are derived straight off it, with no list staged in between.
        let Some(isin) = self
            .get_securityids()
            .get_id("ISIN")
            .and_then(|id| crate::Isin::new(id.code()).ok())
        else {
            return;
        };
        for id in embedded(&isin) {
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
        // Dated before following names the predecessor, which marks a
        // lifecycle output whose state may be inherited.
        let mut changed = fill_execution(&mut self);
        changed |= follow_timed(&mut self, previous);
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

/// [`Market::sided_crosscode`] for a sided element taking `side`: the one
/// speller of the side prefix.
pub(crate) fn sided_crosscode(side: Side, code: &str) -> Cow<'_, str> {
    if side == Side::Unknown || code.is_empty() {
        return Cow::Borrowed(code);
    }
    let name = side.as_str();
    if code.len() > name.len() && code.starts_with(name) && code.as_bytes()[name.len()] == b':' {
        return Cow::Borrowed(code);
    }
    let base = unsided_crosscode(code);
    let mut sided = String::with_capacity(name.len() + 1 + base.len());
    sided.push_str(name);
    sided.push(':');
    sided.push_str(base);
    Cow::Owned(sided)
}

/// A cross code without the side prefix [`sided_crosscode`] gives it: the
/// base every side of one identifier shares.
pub(crate) fn unsided_crosscode(code: &str) -> &str {
    match code.split_once(':') {
        Some((prefix, base))
            if Side::from_name(prefix).is_some_and(|side| side != Side::Unknown) =>
        {
            base
        }
        _ => code,
    }
}

/// A market element that is also an operation on the market: what it adds
/// to the slim facts.
pub trait Operation: Market {
    /// How long the operation stands, where stated.
    fn get_tif(&self) -> Option<&TimeInForce>;
    /// Sets [`Self::get_tif`].
    fn set_tif(&mut self, tif: Option<TimeInForce>);
    /// Whether the instrument can trade, where a status says.
    fn get_tradable(&self) -> Option<bool>;
    /// Sets [`Self::get_tradable`].
    fn set_tradable(&mut self, tradable: Option<bool>);
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
    /// The accounts and parties the operation names, party role to
    /// identifier - `CUSTOMERACCOUNT`, `EXECUTINGTRADER`, `CLIENTID` - one
    /// identifier per role. A FIX message's are its parties and its
    /// `Account(1)`, under `ACCOUNT`.
    fn get_accountids(&self) -> &IdMap;
    /// Replaces [`Self::get_accountids`].
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// the map - a FIX message, whose accounts are its parties and its
    /// `Account(1)`.
    fn set_accountids(&mut self, ids: IdMap) -> Result<()>;
    /// States one account, filling only an absent role; whether it was
    /// added.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// it, or the key or value breaks [`IdMap`]'s rules.
    fn insert_accountid(&mut self, key: &str, value: &str) -> Result<bool>;
    /// Removes one account; whether one was held.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// it.
    fn remove_accountid(&mut self, key: &str) -> Result<bool>;
    /// Whether an operation that follows another carries the alternate
    /// identifier under `key` where it states none: every key but a book
    /// entry's [`MDENTRYREFID`](super::book::ENTRY_REF_ID), the reference one
    /// statement reaches its predecessor by, unless the holder's own
    /// dictionary says otherwise - a FIX message follows its `FIX:idmap`
    /// flags.
    fn is_followed_altid(&self, key: &str) -> bool {
        key != super::book::ENTRY_REF_ID
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
        let mut changed = fill_execution(&mut self);
        changed |= follow_timed(&mut self, previous);
        changed |= follow_market(&mut self, previous);
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
    fold_execution(&mut this, live);
    this.fold_lifecycle(live);
    restate_market(&mut this, live);
    this.finalize();
    this
}

/// What restating means for an operation on the market: [`restating_market`]
/// continued with the operation's facts.
pub(crate) fn restating_operation<E: Event + Operation>(mut this: E, live: &E) -> E {
    restate_event(&mut this, live);
    fold_event_instants(&mut this, live);
    fold_execution(&mut this, live);
    this.fold_lifecycle(live);
    restate_market(&mut this, live);
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

/// Merges an operation on the market into the reference statement it supplements,
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
    let executed = fold_execution(this, other);
    merge_market(this, other, other_is_reference) || executed || timed || changed
}

/// The execution instant a market event states or, while it is still an
/// unstamped lifecycle input whose state itself reports an execution, its
/// own instant. A predecessor marks a lifecycle output: its state may have
/// been inherited, so replaying that output must not reinterpret the folded
/// state as this event's own execution report.
fn execution_unix<E: Event + Market + ?Sized>(event: &E) -> Option<i64> {
    event.get_execunix().or_else(|| {
        (event.get_prevuuid().is_none() && event.is_execution()).then_some(event.get_currunix())
    })
}

/// Fills an execution event's unstated execution instant from its own
/// instant; whether it moved.
pub(crate) fn fill_execution<E: Event + Market + ?Sized>(event: &mut E) -> bool {
    let unix = execution_unix(event);
    moved(event.get_execunix(), unix, |unix| event.set_execunix(unix))
}

/// Folds the execution instants of two statements of the same market event:
/// the earliest either knows, an unstamped execution observation dating
/// itself from its own instant first; whether it moved. It never folds
/// between successive events in one lifecycle, which follow instead.
fn fold_execution<E: Event + Market + ?Sized>(this: &mut E, other: &E) -> bool {
    let unix = earliest(execution_unix(this), execution_unix(other));
    moved(this.get_execunix(), unix, |unix| this.set_execunix(unix))
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
    // Fed only where stated, so an element stating no bid or ask digests as
    // it did before they were facts.
    for (name, held) in [
        ("bidpx", this.get_bidpx()),
        ("bidqty", this.get_bidqty()),
        ("askpx", this.get_askpx()),
        ("askqty", this.get_askqty()),
    ] {
        if let Some(held) = held {
            staged.feed(name, &held.units().to_le_bytes());
        }
    }
    for (name, held) in [("bidccy", this.get_bidccy()), ("askccy", this.get_askccy())] {
        if let Some(held) = held {
            staged.feed(name, held.as_str().as_bytes());
        }
    }
    // Fed only where stated, so an element stating no rate digests as it
    // did before rates were a fact.
    for (target, rate) in this.get_fxrates() {
        staged.feed("fxrates", target.as_str().as_bytes());
        staged.feed("fxrates", &rate.units().to_le_bytes());
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
    if let Some(tif) = this.get_tif() {
        staged.feed("tif", tif.as_str().as_bytes());
    }
    if let Some(tradable) = this.get_tradable() {
        staged.feed("tradable", &[u8::from(tradable)]);
    }
    for (key, value) in this.get_altids().iter() {
        staged.feed("altids", key.as_bytes());
        staged.feed("altids", value.as_bytes());
    }
    for (key, value) in this.get_accountids().iter() {
        staged.feed("accountids", key.as_bytes());
        staged.feed("accountids", value.as_bytes());
    }
}

/// The market facts an event takes from the statement it follows: the
/// price and the quantity that statement settled on as the step before this
/// one, and what the chain is about where this statement says nothing.
pub(crate) fn follow_market<E: Market + ?Sized>(this: &mut E, previous: &E) -> bool {
    // The latest execution the chain has reached, so a delayed report
    // cannot regress it.
    let execunix = latest(this.get_execunix(), previous.get_execunix());
    let mut changed = moved(this.get_execunix(), execunix, |unix| {
        this.set_execunix(unix)
    });
    if this.get_prevpx().is_none() {
        let px = previous.get_price();
        changed |= moved(this.get_prevpx(), px, |px| this.set_prevpx(px));
    }
    if this.get_prevqty().is_none() {
        let qty = previous.get_quantity();
        changed |= moved(this.get_prevqty(), qty, |qty| this.set_prevqty(qty));
    }
    changed | chain_market(this, previous)
}

fn restate_market<E: Market + ?Sized>(this: &mut E, live: &E) -> bool {
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
    changed | chain_market(this, live)
}

/// What the chain an element stands in is about, taken from another
/// statement of that chain where this one says nothing of it. This
/// statement always leads, and nothing here is about a step. The side is
/// this statement's own where it states one, and the chain's where it
/// states none, for an operation as for any other market element; the
/// metadata is this statement's, every key of the chain's it does not state
/// beside it.
fn chain_market<E: Market + ?Sized>(this: &mut E, previous: &E) -> bool {
    let mut changed = moved(
        this.get_currency().clone(),
        better(this.get_currency().clone(), previous.get_currency(), false),
        |currency| this.set_currency(currency),
    );
    changed |= moved(
        this.get_side(),
        this.get_side().merge_with(previous.get_side()),
        |side| this.set_side(side),
    );
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
    // An element naming another ISIN than its predecessor is another
    // instrument, and takes none of the predecessor's identifiers.
    if !names_other_instrument(this, previous) {
        changed |= yield_unknown_isin(this, previous);
        for id in previous.get_securityids() {
            if !this.get_securityids().contains_key(id.sectype().as_str()) {
                changed |= this.insert_securityid(id.clone()).unwrap_or(false);
            }
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
    let own = this.get_metadata();
    if previous
        .get_metadata()
        .keys()
        .any(|key| !own.contains_key(key))
    {
        let mut merged = own.clone();
        for (key, value) in previous.get_metadata() {
            merged.entry(key.clone()).or_insert_with(|| value.clone());
        }
        this.set_metadata(Some(merged));
        changed = true;
    }
    changed
}

/// Whether an ISIN is filed under `ZZ`, the prefix of no country: an
/// identifier a better one replaces.
fn is_unknown_isin(code: &str) -> bool {
    code.starts_with("ZZ")
}

/// Whether `this` and `other` each state an ISIN, and not the same one - a
/// `ZZ` ISIN names no country's instrument, so it is never the other one.
fn names_other_instrument<E: Market + ?Sized>(this: &E, other: &E) -> bool {
    matches!(
        (this.get_isincode(), other.get_isincode()),
        (Some(mine), Some(theirs))
            if mine != theirs && !is_unknown_isin(mine) && !is_unknown_isin(theirs)
    )
}

/// A `ZZ` ISIN `this` states yields to a real one `other` states: taken
/// out - and every identifier derived from it with it - and replaced, so
/// what the real one carries derives afresh. Whether it moved.
fn yield_unknown_isin<E: Market + ?Sized>(this: &mut E, other: &E) -> bool {
    let (Some(mine), Some(theirs)) = (this.get_isincode(), other.get_securityids().get_id("ISIN"))
    else {
        return false;
    };
    if !is_unknown_isin(mine) || is_unknown_isin(theirs.code()) {
        return false;
    }
    let theirs = theirs.clone();
    let _ = this.remove_securityid(&theirs.sectype());
    this.insert_securityid(theirs).unwrap_or(false)
}

/// The market facts an element takes from another statement of itself:
/// the leading statement's price, quantity and unit, each optional fact the
/// selected statement states, and each code the better of the two. `later`
/// says whether `other` is the leading statement.
pub(crate) fn merge_market<E: Market + ?Sized>(this: &mut E, other: &E, later: bool) -> bool {
    // Two statements of one element keep the earliest execution either knows.
    let execunix = earliest(this.get_execunix(), other.get_execunix());
    let mut changed = moved(this.get_execunix(), execunix, |unix| {
        this.set_execunix(unix)
    });
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
    // The bid and the ask: the leading statement's where it states one.
    optional!(get_bidpx, set_bidpx);
    optional!(get_bidqty, set_bidqty);
    optional!(get_askpx, set_askpx);
    optional!(get_askqty, set_askqty);
    changed |= moved(
        this.get_bidccy().cloned(),
        stated(
            this.get_bidccy().cloned(),
            other.get_bidccy().cloned(),
            later,
        ),
        |ccy| this.set_bidccy(ccy),
    );
    changed |= moved(
        this.get_askccy().cloned(),
        stated(
            this.get_askccy().cloned(),
            other.get_askccy().cloned(),
            later,
        ),
        |ccy| this.set_askccy(ccy),
    );
    changed |= moved(
        this.get_currency().clone(),
        better(this.get_currency().clone(), other.get_currency(), later),
        |currency| this.set_currency(currency),
    );
    changed |= moved(
        this.get_side(),
        if later {
            other.get_side().merge_with(this.get_side())
        } else {
            this.get_side().merge_with(other.get_side())
        },
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
    // Two statements naming different ISINs name two instruments, whose
    // identifiers never mix: the leading statement's stand whole.
    if names_other_instrument(this, other) {
        if later {
            changed |= this
                .set_securityids(other.get_securityids().clone())
                .is_ok();
        }
    } else {
        changed |= yield_unknown_isin(this, other);
        for id in other.get_securityids() {
            let key = id.sectype();
            match this.get_securityids().get(key.as_str()) {
                Some(held) if held == id.code() => {}
                // A `ZZ` ISIN never replaces a real one, whichever leads.
                Some(held)
                    if key.as_str() == "ISIN"
                        && is_unknown_isin(id.code())
                        && !is_unknown_isin(held) => {}
                Some(_) if later => {
                    let _ = this.remove_securityid(&key);
                    changed |= this.insert_securityid(id.clone()).unwrap_or(false);
                }
                Some(_) => {}
                None => changed |= this.insert_securityid(id.clone()).unwrap_or(false),
            }
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
    // The union by target, the leading statement's rate where both state
    // one.
    if !other.get_fxrates().is_empty() {
        let (lead, supplement) = if later {
            (other.get_fxrates(), this.get_fxrates())
        } else {
            (this.get_fxrates(), other.get_fxrates())
        };
        let mut merged = lead.clone();
        for (target, rate) in supplement {
            merged.entry(target.clone()).or_insert(*rate);
        }
        if &merged != this.get_fxrates() {
            this.set_fxrates(merged);
            changed = true;
        }
    }
    changed
}

/// The operation facts an event takes from the statement it follows: the
/// order's own identifiers, how long it stands and whether it can trade
/// where this statement says nothing.
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
    for (key, value) in previous.get_altids().iter() {
        if this.is_followed_altid(key) && !this.get_altids().contains_key(key) {
            changed |= this.insert_altid(key, value).unwrap_or(false);
        }
    }
    // The accounts an operation is booked to stay with its chain: a role
    // this statement names none for is the chain's.
    for (key, value) in previous.get_accountids().iter() {
        if !this.get_accountids().contains_key(key) {
            changed |= this.insert_accountid(key, value).unwrap_or(false);
        }
    }
    changed
}

/// The operation facts an element takes from another statement of itself.
pub(crate) fn merge_operation<E: Operation + ?Sized>(this: &mut E, other: &E, later: bool) -> bool {
    let mut changed = moved(
        this.get_tif().cloned(),
        better_stated(this.get_tif().cloned(), other.get_tif(), later),
        |tif| this.set_tif(tif),
    );
    changed |= moved(
        this.get_tradable(),
        stated(this.get_tradable(), other.get_tradable(), later),
        |tradable| this.set_tradable(tradable),
    );
    if !other.get_altids().is_empty() {
        let mut merged = if later {
            other.get_altids().clone()
        } else {
            this.get_altids().clone()
        };
        let supplement = if later {
            this.get_altids()
        } else {
            other.get_altids()
        };
        merged.merge(supplement);
        if &merged != this.get_altids() && this.set_altids(merged).is_ok() {
            changed = true;
        }
    }
    if !other.get_accountids().is_empty() {
        let (mut merged, supplement) = if later {
            (other.get_accountids().clone(), this.get_accountids())
        } else {
            (this.get_accountids().clone(), other.get_accountids())
        };
        merged.merge(supplement);
        if &merged != this.get_accountids() && this.set_accountids(merged).is_ok() {
            changed = true;
        }
    }
    changed
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
