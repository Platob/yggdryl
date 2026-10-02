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
use crate::securityid::{SymbolCode, embedded};
use crate::xxhash::Xxh3;
use crate::{
    Ccy, Cfi, Decimal, IdSource, IdType, Identifier, Identifiers, Mic, Result, Side, TimeInForce,
    Unit,
};

/// Free-form facts a market element carries beside its typed ones: never an
/// identifier, which has a typed home in [`Market::get_securityids`] or an
/// operation's alternate identifiers and parties.
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
///
/// # Setting: fill, or overwrite
///
/// Every setter takes `overwrite`. `true` states the value whatever the
/// element held - `None` included, which clears it; `false` fills: the value
/// lands only where the element states nothing yet - `None`, a side of
/// `UNKN`, a currency or unit of none, a type of `UNKN` - and a map fills
/// only the keys it lacks. A value equal to the held one changes nothing
/// either way.
///
/// A change then carries what it implies onto the facts that follow it. A
/// source moves a fact it implies along with it - where the element states
/// nothing there, or still holds what the source was - and never one stated
/// apart from it; a fact that states something back about its source only
/// fills the source where the element states none:
///
/// | Changed | Moves | Fills back |
/// | --- | --- | --- |
/// | price, quantity | the bid of a buyer, the ask of a seller: `bidpx`/`bidqty`, `askpx`/`askqty` | |
/// | side | the side it left stops quoting the price and quantity, the side it takes quotes them; a sided kind's cross code | the price and quantity, from what the side quotes |
/// | currency | `bidccy`/`askccy` of a bid or ask the element states | |
/// | quantity, `displayqty` | `hiddenqty`, the quantity past the shown part | |
/// | `bidpx`/`bidqty`, `askpx`/`askqty` | | the price and quantity of an element taking that side |
/// | `hiddenqty` | | `displayqty`, the quantity less it - or the quantity, the two parts together |
/// | a predecessor's `hiddenqty`, followed | a follower stating none: what it kept back less what traded since - the rise in `cumqty`, else `lastqty` | |
/// | `lastpx`, `spotrate`, `forwardpoints` | | the third, where two are stated: `lastpx` is spot plus points |
/// | an operation's `ordqty`, `cumqty`, `leavesqty` and its state | | working, the third of the three - fresh, asked for or new, all it ordered is left; filled, `leavesqty` 0 and `cumqty` all ordered; no longer active, `leavesqty` 0 and - canceled, done for the day, expired - `cxlqty` what was left ([`Operation::get_ordqty`]) |
/// | `leavesqty` | the quantity: what is still open is what the element is about | |
/// | `cumqty`, `lastqty`, `lastpx` | | `avgpx`, the last price, where all that traded is the last fill |
///
/// Each redirection writes its target directly, never through the target's
/// setter, so a change runs once and a chain of them cannot loop.
///
/// ```
/// use yggdryl::graph::{Element, Market, OrderEvent};
/// use yggdryl::Side;
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut order = OrderEvent::at(1);
/// order.set_crosscode("ORD-1".to_owned());
/// order.set_price(Some("101.5".parse()?), true);
/// order.set_quantity(Some("500".parse()?), true);
/// // A buyer bids its price and quantity, under its side's cross code.
/// order.set_side(Side::Buy, false);
/// assert_eq!(order.get_bidpx(), Some("101.5".parse()?));
/// assert_eq!(order.get_bidqty(), Some("500".parse()?));
/// assert_eq!(order.get_crosscode(), "10:1:ORD-1");
/// // Without `overwrite` a stated fact stands; with it, it moves, and the
/// // bid it quoted moves with it.
/// order.set_price(Some("99".parse()?), false);
/// assert_eq!(order.get_price(), Some("101.5".parse()?));
/// order.set_price(Some("102".parse()?), true);
/// assert_eq!(order.get_bidpx(), Some("102".parse()?));
/// // Selling instead: the bid is withdrawn and the ask quotes the order.
/// order.set_side(Side::Sell, true);
/// assert_eq!((order.get_bidpx(), order.get_askpx()), (None, Some("102".parse()?)));
/// assert_eq!(order.get_crosscode(), "10:2:ORD-1");
/// // An iceberg: the hidden part is the quantity past the shown one.
/// order.set_displayqty(Some("100".parse()?), false);
/// assert_eq!(order.get_hiddenqty(), Some("400".parse()?));
/// # Ok(())
/// # }
/// ```
pub trait Market {
    /// The price the element states; `None` where it states none. Never
    /// defaulted: a last executed price is [`Self::get_lastpx`], not this.
    fn get_price(&self) -> Option<Decimal>;
    /// Sets [`Self::get_price`].
    fn set_price(&mut self, price: Option<Decimal>, overwrite: bool);
    /// The price a stop order triggers at, where it states one.
    fn get_stoppx(&self) -> Option<Decimal>;
    /// Sets [`Self::get_stoppx`].
    fn set_stoppx(&mut self, px: Option<Decimal>, overwrite: bool);
    /// The currency the element is priced in, [`Ccy::none`] where unstated.
    fn get_currency(&self) -> &Ccy;
    /// Sets [`Self::get_currency`].
    fn set_currency(&mut self, currency: Ccy, overwrite: bool);
    /// The quantity the element states; `None` where it states none. Never
    /// defaulted: a last executed quantity is [`Self::get_lastqty`], not this.
    fn get_quantity(&self) -> Option<Decimal>;
    /// Sets [`Self::get_quantity`].
    fn set_quantity(&mut self, quantity: Option<Decimal>, overwrite: bool);
    /// The part of the quantity shown to the market, where the element
    /// shows less than all of it - an iceberg order's peak.
    fn get_displayqty(&self) -> Option<Decimal>;
    /// Sets [`Self::get_displayqty`].
    fn set_displayqty(&mut self, qty: Option<Decimal>, overwrite: bool);
    /// The part of the quantity kept from the market, where the element
    /// hides some - an iceberg order's reserve.
    fn get_hiddenqty(&self) -> Option<Decimal>;
    /// Sets [`Self::get_hiddenqty`].
    fn set_hiddenqty(&mut self, qty: Option<Decimal>, overwrite: bool);
    /// The unit the quantity is counted in, [`Unit::none`] where unstated.
    fn get_unit(&self) -> &Unit;
    /// Sets [`Self::get_unit`].
    fn set_unit(&mut self, unit: Unit, overwrite: bool);
    /// The side the element takes, [`Side::Unknown`] where it states none.
    fn get_side(&self) -> Side;
    /// Sets [`Self::get_side`]; a sided element's cross code is stored under
    /// the side taken ([`Self::stored_crosscode`]), and the side's bid or ask
    /// quotes the element's price and quantity.
    fn set_side(&mut self, side: Side, overwrite: bool);
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
    /// The type of its kind the element is - its order type (`ORDLIMIT`),
    /// its quote type, its trade type, its book entry type -
    /// [`MarketDataType::Unknown`](crate::MarketDataType::Unknown) where it
    /// states none.
    fn get_marketdatatype(&self) -> crate::MarketDataType;
    /// Sets [`Self::get_marketdatatype`].
    fn set_marketdatatype(&mut self, mdtype: crate::MarketDataType, overwrite: bool);
    /// The security's identifiers - its ISIN, its CUSIP, a venue's own
    /// instrument key - one value per key `src:type`, each held as its
    /// [`IdType`] stores it.
    fn get_securityids(&self) -> &Identifiers;
    /// Replaces the stated security identifiers, or fills only the types
    /// and sources they lack without `overwrite`.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// one of them.
    fn set_securityids(&mut self, ids: Identifiers, overwrite: bool) -> Result<()>;
    /// States one security identifier, filling an absent type and source;
    /// a derived identifier of its type is taken back, since a statement
    /// answers before it. Whether it was added.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// it.
    fn insert_securityid(&mut self, id: Identifier) -> Result<bool>;
    /// Removes the identifier keyed `src:kind`; whether one was held.
    /// Removing the ISIN takes back every derived identifier, since each
    /// hangs on it.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// the removal.
    fn remove_securityid(&mut self, src: &IdSource, kind: &IdType) -> Result<bool>;
    /// Derives one security identifier - one the element implies rather
    /// than states: the national number its ISIN carries, what its ticker's
    /// shape names, what a lifecycle learned - under the
    /// [`IdSource::Derived`] source, filling only a type
    /// the element holds none of; whether it was added. A derived
    /// identifier never reaches a store the holder is a view of.
    fn derive_securityid(&mut self, kind: &IdType, code: &str) -> bool;
    /// The detailed CFI classification, where one is known.
    fn get_cficode(&self) -> Option<&Cfi>;
    /// Sets [`Self::get_cficode`].
    fn set_cficode(&mut self, code: Option<Cfi>, overwrite: bool);
    /// The market the element trades on, where known.
    fn get_miccode(&self) -> Option<&Mic>;
    /// Sets [`Self::get_miccode`].
    fn set_miccode(&mut self, code: Option<Mic>, overwrite: bool);
    /// When the element last executed: the latest execution clock its
    /// lifecycle reached, nanoseconds since the Unix epoch, UTC, where it
    /// knows. A market fact rather than an event's: an event that is a
    /// market element and whose state reports an execution with no clock of
    /// its own dates it from its own instant, following carries the later
    /// of its own and its predecessor's, and two statements of one event
    /// keep the earliest either knows.
    fn get_execunix(&self) -> Option<i64>;
    /// Sets [`Self::get_execunix`]; `None` states it does not know.
    fn set_execunix(&mut self, unix: Option<i64>, overwrite: bool);
    /// The last executed price: what the element's last execution traded
    /// at, never the price it states.
    fn get_lastpx(&self) -> Option<Decimal>;
    /// Sets [`Self::get_lastpx`].
    fn set_lastpx(&mut self, px: Option<Decimal>, overwrite: bool);
    /// The last executed quantity: what the element's last execution
    /// traded, never the quantity it states.
    fn get_lastqty(&self) -> Option<Decimal>;
    /// Sets [`Self::get_lastqty`].
    fn set_lastqty(&mut self, qty: Option<Decimal>, overwrite: bool);
    /// The average price of what the element has traded.
    fn get_avgpx(&self) -> Option<Decimal>;
    /// Sets [`Self::get_avgpx`].
    fn set_avgpx(&mut self, px: Option<Decimal>, overwrite: bool);
    /// How much the element has traded.
    fn get_cumqty(&self) -> Option<Decimal>;
    /// Sets [`Self::get_cumqty`].
    fn set_cumqty(&mut self, qty: Option<Decimal>, overwrite: bool);
    /// How much of the element is left to trade.
    fn get_leavesqty(&self) -> Option<Decimal>;
    /// Sets [`Self::get_leavesqty`].
    fn set_leavesqty(&mut self, qty: Option<Decimal>, overwrite: bool);
    /// How much of the element was canceled.
    fn get_cxlqty(&self) -> Option<Decimal>;
    /// Sets [`Self::get_cxlqty`].
    fn set_cxlqty(&mut self, qty: Option<Decimal>, overwrite: bool);
    /// The price the step before this one settled on.
    fn get_prevpx(&self) -> Option<Decimal>;
    /// Sets [`Self::get_prevpx`].
    fn set_prevpx(&mut self, px: Option<Decimal>, overwrite: bool);
    /// The quantity the step before this one settled on.
    fn get_prevqty(&self) -> Option<Decimal>;
    /// Sets [`Self::get_prevqty`].
    fn set_prevqty(&mut self, qty: Option<Decimal>, overwrite: bool);
    /// The spot part of an FX forward price.
    fn get_spotrate(&self) -> Option<Decimal>;
    /// Sets [`Self::get_spotrate`].
    fn set_spotrate(&mut self, rate: Option<Decimal>, overwrite: bool);
    /// The forward points of an FX forward price.
    fn get_forwardpoints(&self) -> Option<Decimal>;
    /// Sets [`Self::get_forwardpoints`].
    fn set_forwardpoints(&mut self, points: Option<Decimal>, overwrite: bool);
    /// The ticker the instrument goes by, where stated.
    fn get_ticker(&self) -> Option<&str>;
    /// Sets [`Self::get_ticker`].
    fn set_ticker(&mut self, ticker: Option<SmolStr>, overwrite: bool);
    /// The free-form facts the element carries; a shared empty map where it
    /// carries none.
    fn get_metadata(&self) -> &Metadata;
    /// Sets [`Self::get_metadata`]; `None` and an empty map are the same.
    fn set_metadata(&mut self, metadata: Option<Metadata>, overwrite: bool);
    /// The FX rates the element states, target currency to the rate to
    /// divide by: an amount in [`Self::get_currency`] divided by
    /// `rates[target]` is that amount in `target`. A shared empty map where
    /// it states none.
    fn get_fxrates(&self) -> &FxRates;
    /// Replaces [`Self::get_fxrates`]; an empty map states none.
    fn set_fxrates(&mut self, rates: FxRates, overwrite: bool);
    /// The bid price the element states: a quote's `BidPx(132)`, a book's
    /// best tradable bid level, a buyer's own price; `None` where it states
    /// none. Never filled from a last executed price.
    fn get_bidpx(&self) -> Option<Decimal>;
    /// Sets [`Self::get_bidpx`].
    fn set_bidpx(&mut self, px: Option<Decimal>, overwrite: bool);
    /// The quantity bid at [`Self::get_bidpx`]: a quote's `BidSize(134)`, a
    /// book's best tradable bid level's; `None` where it states none.
    fn get_bidqty(&self) -> Option<Decimal>;
    /// Sets [`Self::get_bidqty`].
    fn set_bidqty(&mut self, qty: Option<Decimal>, overwrite: bool);
    /// The currency the bid is stated in, where the element states a bid.
    fn get_bidccy(&self) -> Option<&Ccy>;
    /// Sets [`Self::get_bidccy`].
    fn set_bidccy(&mut self, ccy: Option<Ccy>, overwrite: bool);
    /// The ask price the element states: a quote's `OfferPx(133)`, a book's
    /// best tradable ask level; `None` where it states none.
    fn get_askpx(&self) -> Option<Decimal>;
    /// Sets [`Self::get_askpx`].
    fn set_askpx(&mut self, px: Option<Decimal>, overwrite: bool);
    /// The quantity offered at [`Self::get_askpx`]: a quote's
    /// `OfferSize(135)`, a book's best tradable ask level's.
    fn get_askqty(&self) -> Option<Decimal>;
    /// Sets [`Self::get_askqty`].
    fn set_askqty(&mut self, qty: Option<Decimal>, overwrite: bool);
    /// The currency the ask is stated in, where the element states an ask.
    fn get_askccy(&self) -> Option<&Ccy>;
    /// Sets [`Self::get_askccy`].
    fn set_askccy(&mut self, ccy: Option<Ccy>, overwrite: bool);

    /// The cross code `code` is stored as for this element:
    /// `"{kind}:{side}:{code}"`, the [`MarketDataKind`](crate::MarketDataKind)
    /// code of the category it is filed under and the [`Side`] code of the
    /// side it takes - `10:1:ORD-1` for an order to buy - so each category
    /// and each side of one identifier is a chain of its own. Only a sided
    /// element ([`Self::is_sided`]: an order, a quote or an execution) states
    /// its side there; any other - a trade, a book, a snapshot control -
    /// states `0`, as one taking [`Side::Unknown`] does: `21:0:T-1`,
    /// `3:0:XNAS:ESVUFR`, `10:0:ORD-1`. Idempotent: a code already carrying
    /// this prefix is answered as it is, one carrying another has it
    /// replaced, and an empty code stays empty. The one place the prefix is
    /// decided; every market holder stores its cross code through it, so
    /// stamping the category or setting the side after the code converges
    /// on the same answer.
    ///
    /// ```
    /// use yggdryl::graph::{BookEvent, Element, Market, OrderEvent};
    /// use yggdryl::Side;
    ///
    /// let mut order = OrderEvent::at(1);
    /// order.set_side(Side::Buy, true);
    /// assert_eq!(order.stored_crosscode("ORD-1"), "10:1:ORD-1");
    /// assert_eq!(order.stored_crosscode("10:2:ORD-1"), "10:1:ORD-1");
    /// order.set_crosscode("ORD-1".to_owned());
    /// assert_eq!(order.get_crosscode(), "10:1:ORD-1");
    /// order.set_side(Side::Sell, true);
    /// assert_eq!(order.get_crosscode(), "10:2:ORD-1");
    ///
    /// // A book is not sided: it states side 0 whatever side it takes.
    /// let mut book = BookEvent::new(1, "AAPL");
    /// book.set_side(Side::Buy, true);
    /// assert_eq!(book.get_crosscode(), "3:0:AAPL");
    /// ```
    fn stored_crosscode<'code>(&self, code: &'code str) -> Cow<'code, str> {
        stored_crosscode(self.marketdatakind(), self.get_side(), code)
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
    /// order.set_currency(Ccy::new("EUR")?, true);
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
        self.set_fxrates(rates, true);
        true
    }

    /// The ISIN the element names: its `ISIN` security identifier, borrowed
    /// - a projection of [`Self::get_securityids`], never a second store.
    fn get_isincode(&self) -> Option<&str> {
        self.get_securityids().get(&IdType::Isin)
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
    /// a key the element does not state, as a derived identifier. A currency
    /// pair states its quantity in the currency dealt, so an element trading
    /// one and stating no unit takes that currency as its unit - its stated
    /// currency where that is a leg of the pair, else the pair's base.
    ///
    /// Provided, and what an implementor's [`Element::finalize`] runs before
    /// it digests. Running it twice changes nothing the first run did not.
    fn fill_market(&mut self)
    where
        Self: Sized,
    {
        // A currency pair's quantity is stated in the currency dealt: the
        // currency stated where it is a leg of the pair, else the base.
        let pair = if self.get_unit().is_none() {
            self.get_securityids()
                .get(&IdType::Forex)
                .and_then(|code| crate::Forex::new(code).ok())
        } else {
            None
        };
        if let Some(pair) = pair {
            let currency = self.get_currency();
            let dealt = if *currency == pair.base() || *currency == pair.quote() {
                currency.clone()
            } else {
                pair.base()
            };
            if let Ok(unit) = Unit::new(dealt.as_str()) {
                self.set_unit(unit, false);
            }
        }
        // A ticker that is an identifier's own shape - an ISIN, a FIGI, an
        // instrument key - names it: filled, never stated over what the
        // element states, and read by its length before any check runs.
        if let Some(code) = self.get_ticker().and_then(SymbolCode::from_symbol) {
            fill_symbol(self, code);
        }
        // The ISIN is owned - a code inline - so its national identifiers
        // are derived straight off it, with no list staged in between.
        let Some(isin) = self
            .get_securityids()
            .get(&IdType::Isin)
            .and_then(|code| crate::Isin::new(code).ok())
        else {
            return;
        };
        for id in embedded(&isin) {
            self.derive_securityid(id.kind(), id.value());
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

/// Carries an iceberg's hidden part along its chain where this statement
/// states none: what the predecessor kept back, less what traded since -
/// the rise in `cumqty`, else this statement's `lastqty` - and never below
/// nothing. Whether it moved.
fn follow_hidden<E: Market + ?Sized>(this: &mut E, previous: &E) -> bool {
    if this.get_hiddenqty().is_some() {
        return false;
    }
    let Some(kept) = previous.get_hiddenqty() else {
        return false;
    };
    let consumed = this
        .get_cumqty()
        .zip(previous.get_cumqty())
        .and_then(|(now, before)| now.checked_sub(before))
        .or_else(|| this.get_lastqty())
        .filter(|held| held.is_positive());
    let hidden = match consumed {
        Some(consumed) => kept.checked_sub(consumed).map(|left| {
            if left.is_negative() {
                Decimal::from_int(0)
            } else {
                left
            }
        }),
        None => Some(kept),
    };
    this.set_hiddenqty(hidden, false);
    this.get_hiddenqty().is_some()
}

/// The type `leading` states, else the one `other` does: a type stated as
/// none takes the other.
fn stated_type(
    leading: crate::MarketDataType,
    other: crate::MarketDataType,
) -> crate::MarketDataType {
    if leading == crate::MarketDataType::Unknown {
        other
    } else {
        leading
    }
}

/// [`Market::stored_crosscode`] for an element of `kind` taking `side`: the
/// one speller of the prefix.
pub(crate) fn stored_crosscode(
    kind: crate::MarketDataKind,
    side: Side,
    code: &str,
) -> Cow<'_, str> {
    if code.is_empty() {
        return Cow::Borrowed(code);
    }
    let side = if kind.is_sided() { side } else { Side::Unknown };
    let base = match split_crosscode(code) {
        Some((held_kind, held_side, _)) if held_kind == kind && held_side == side => {
            return Cow::Borrowed(code);
        }
        Some((.., base)) => base,
        None => code,
    };
    // The prefix is two codes of at most three digits, spelled inline, so
    // the code is one allocation at its exact length.
    let prefix = smol_str::format_smolstr!("{}:{}:", kind.code(), side.code());
    let mut stored = String::with_capacity(prefix.len() + base.len());
    stored.push_str(&prefix);
    stored.push_str(base);
    Cow::Owned(stored)
}

/// A cross code without the prefix [`stored_crosscode`] gives it: the base
/// every category and every side of one identifier shares.
pub(crate) fn base_crosscode(code: &str) -> &str {
    split_crosscode(code).map_or(code, |(.., base)| base)
}

/// The category, the side and the base a stored cross code spells, where
/// it opens with the prefix [`stored_crosscode`] writes: each code in
/// decimal without a leading zero, and one its enum holds.
fn split_crosscode(code: &str) -> Option<(crate::MarketDataKind, Side, &str)> {
    let decimal = |text: &str| {
        let canonical = !text.is_empty()
            && text.len() <= 3
            && text.bytes().all(|byte| byte.is_ascii_digit())
            && (text == "0" || !text.starts_with('0'));
        canonical.then(|| text.parse::<u8>().ok()).flatten()
    };
    let (kind, rest) = code.split_once(':')?;
    let (side, base) = rest.split_once(':')?;
    Some((
        crate::MarketDataKind::from_code(decimal(kind)?)?,
        Side::from_code(decimal(side)?)?,
        base,
    ))
}

/// A market element that is also an operation on the market: what it adds
/// to the slim facts.
pub trait Operation: Market {
    /// The quantity the operation ordered, FIX's `OrderQty(38)`, where
    /// stated. Working, what is left is what was ordered less what traded,
    /// so any two of `ordqty`, [`Market::get_cumqty`] and
    /// [`Market::get_leavesqty`] fill the third; filled, nothing is left and
    /// all of it traded; no longer active - canceled, done for the day,
    /// expired, calculated or rejected - nothing is left, and what someone
    /// ended canceled the rest ([`Market::get_cxlqty`]). What was ordered is
    /// never the quantity the element is about: [`Market::get_quantity`]
    /// stands apart from it.
    fn get_ordqty(&self) -> Option<Decimal>;
    /// Sets [`Self::get_ordqty`].
    fn set_ordqty(&mut self, qty: Option<Decimal>, overwrite: bool);
    /// How long the operation stands, where stated.
    fn get_timeinforce(&self) -> Option<&TimeInForce>;
    /// Sets [`Self::get_timeinforce`].
    fn set_timeinforce(&mut self, tif: Option<TimeInForce>, overwrite: bool);
    /// Whether the instrument can trade, where a status says.
    fn get_tradable(&self) -> Option<bool>;
    /// Sets [`Self::get_tradable`].
    fn set_tradable(&mut self, tradable: Option<bool>, overwrite: bool);
    /// The operation's own identifiers - `fix:clordid`, `fix:orderid`,
    /// `fix:execid`, a regulatory trade id - one value per key `src:type`,
    /// each base beside the parents its chain gave it.
    fn get_identifiers(&self) -> &Identifiers;
    /// Replaces [`Self::get_identifiers`], or fills only the types and sources
    /// they lack without `overwrite`.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// one of them.
    fn set_identifiers(&mut self, ids: Identifiers, overwrite: bool) -> Result<()>;
    /// States one identifier, filling only an absent type and source;
    /// whether it was added.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// it.
    fn insert_identifier(&mut self, id: Identifier) -> Result<bool>;
    /// Removes the identifier keyed `src:kind`; whether one was held.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// the removal.
    fn remove_identifier(&mut self, src: &IdSource, kind: &IdType) -> Result<bool>;
    /// The parties the operation names - its accounts, its traders, its
    /// firms, its users - each a value of a role (the type) from the source
    /// that issued it: a FIX party is its `PartyID` as its `PartyRole`'s
    /// name from its `PartyIDSource`'s, `Account(1)` an `account`.
    fn get_partyids(&self) -> &Identifiers;
    /// Replaces [`Self::get_partyids`], or fills only the roles and sources
    /// they lack without `overwrite`.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// them.
    fn set_partyids(&mut self, partyids: Identifiers, overwrite: bool) -> Result<()>;
    /// States one party, filling only an absent role and source; whether it
    /// was added.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// it.
    fn insert_partyid(&mut self, partyid: Identifier) -> Result<bool>;
    /// Removes the party keyed `src:kind`; whether one was held.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// the removal.
    fn remove_partyid(&mut self, src: &IdSource, kind: &IdType) -> Result<bool>;
    /// Whether an operation that follows another carries `id`, one of the
    /// predecessor's identifiers, where it states none of its key: every
    /// type but a book entry's [`IdType::MdEntryRefId`], the reference one
    /// statement reaches its predecessor by, unless the holder's own
    /// dictionary says otherwise - a FIX message follows its `FIX:idmap`
    /// flags, each followed type's parents with it.
    fn is_followed_identifier(&self, id: &Identifier) -> bool {
        id.kind() != &IdType::MdEntryRefId
    }
    /// The parent types of one of the operation's identifier types,
    /// nearest first ([`IdType::parents`]), unless the holder's own
    /// dictionary says otherwise: a FIX message reads the `FIX:parents` its
    /// fields state.
    fn parents_of(&self, base: &IdType) -> Cow<'_, [IdType]> {
        base.parents()
    }
    /// The base `kind` is a parent of and its place among the base's
    /// [`Self::parents_of`] ([`IdType::parent_of`]), as the same holder
    /// reads it.
    fn parent_of(&self, kind: &IdType) -> Option<(IdType, usize)> {
        kind.parent_of()
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
    moved(event.get_execunix(), unix, |unix| {
        event.set_execunix(unix, true)
    })
}

/// Folds the execution instants of two statements of the same market event:
/// the earliest either knows, an unstamped execution observation dating
/// itself from its own instant first; whether it moved. It never folds
/// between successive events in one lifecycle, which follow instead.
fn fold_execution<E: Event + Market + ?Sized>(this: &mut E, other: &E) -> bool {
    let unix = earliest(execution_unix(this), execution_unix(other));
    moved(this.get_execunix(), unix, |unix| {
        this.set_execunix(unix, true)
    })
}

fn merge_operation_event<E: Event + Operation>(
    this: &mut E,
    other: &E,
    other_is_reference: bool,
) -> bool {
    let changed = merge_market_event(this, other, other_is_reference);
    merge_operation(this, other, other_is_reference) || changed
}

/// Fills what a ticker of an identifier's shape names: the identifier
/// as a derived one, and an instrument key's market and currency where
/// the element states none; a CFI code as the classification.
fn fill_symbol<E: Market + ?Sized>(this: &mut E, code: SymbolCode) {
    if let Some((kind, id)) = code.identifier() {
        this.derive_securityid(&kind, id);
    }
    match code {
        SymbolCode::Instrument { mic, ccy, .. } => {
            if mic.is_some() {
                this.set_miccode(mic, false);
            }
            if let Some(ccy) = ccy {
                this.set_currency(ccy, false);
            }
        }
        SymbolCode::Cfi(cfi) => this.set_cficode(Some(cfi), false),
        _ => {}
    }
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
    // Fed only where stated, so an element of no type digests as it did
    // before types existed.
    let mdtype = this.get_marketdatatype();
    if mdtype != crate::MarketDataType::Unknown {
        staged.feed("marketdatatype", mdtype.as_str().as_bytes());
    }
    for id in this.get_securityids() {
        staged.feed("securityids", id.src().as_str().as_bytes());
        staged.feed("securityids", id.kind().as_str().as_bytes());
        staged.feed("securityids", id.value().as_bytes());
    }
    if let Some(code) = this.get_cficode() {
        staged.feed("cficode", code.as_str().as_bytes());
    }
    if let Some(code) = this.get_miccode() {
        staged.feed("miccode", code.as_str().as_bytes());
    }
    // Fed only where stated, so an element stating none of them digests as
    // it did before they were facts.
    for (name, held) in [
        ("stoppx", this.get_stoppx()),
        ("displayqty", this.get_displayqty()),
        ("hiddenqty", this.get_hiddenqty()),
        ("cxlqty", this.get_cxlqty()),
    ] {
        if let Some(held) = held {
            staged.feed(name, &held.units().to_le_bytes());
        }
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
    if let Some(qty) = this.get_ordqty() {
        staged.feed("ordqty", &qty.units().to_le_bytes());
    }
    if let Some(tif) = this.get_timeinforce() {
        staged.feed("tif", tif.as_str().as_bytes());
    }
    if let Some(tradable) = this.get_tradable() {
        staged.feed("tradable", &[u8::from(tradable)]);
    }
    for id in this.get_identifiers() {
        staged.feed("identifiers", id.src().as_str().as_bytes());
        staged.feed("identifiers", id.kind().as_str().as_bytes());
        staged.feed("identifiers", id.value().as_bytes());
    }
    for id in this.get_partyids() {
        staged.feed("partyids", id.src().as_str().as_bytes());
        staged.feed("partyids", id.kind().as_str().as_bytes());
        staged.feed("partyids", id.value().as_bytes());
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
        this.set_execunix(unix, true)
    });
    if this.get_prevpx().is_none() {
        let px = previous.get_price();
        changed |= moved(this.get_prevpx(), px, |px| this.set_prevpx(px, true));
    }
    if this.get_prevqty().is_none() {
        let qty = previous.get_quantity();
        changed |= moved(this.get_prevqty(), qty, |qty| this.set_prevqty(qty, true));
    }
    changed | chain_market(this, previous)
}

fn restate_market<E: Market + ?Sized>(this: &mut E, live: &E) -> bool {
    let mut changed = moved(
        this.get_prevpx(),
        stated(this.get_prevpx(), live.get_prevpx(), false),
        |px| this.set_prevpx(px, true),
    );
    changed |= moved(
        this.get_prevqty(),
        stated(this.get_prevqty(), live.get_prevqty(), false),
        |qty| this.set_prevqty(qty, true),
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
    let mut changed = follow_hidden(this, previous);
    changed |= moved(
        this.get_currency().clone(),
        better(this.get_currency().clone(), previous.get_currency(), false),
        |currency| this.set_currency(currency, true),
    );
    changed |= moved(
        this.get_side(),
        this.get_side().merge_with(previous.get_side()),
        |side| this.set_side(side, true),
    );
    changed |= moved(
        this.get_marketdatatype(),
        stated_type(this.get_marketdatatype(), previous.get_marketdatatype()),
        |mdtype| this.set_marketdatatype(mdtype, true),
    );
    changed |= moved(
        this.get_unit().clone(),
        better(this.get_unit().clone(), previous.get_unit(), false),
        |unit| this.set_unit(unit, true),
    );
    if this.get_ticker().is_none()
        && let Some(ticker) = previous.get_ticker()
    {
        this.set_ticker(Some(SmolStr::new(ticker)), true);
        changed = true;
    }
    // An element naming another ISIN than its predecessor is another
    // instrument, and takes none of the predecessor's identifiers.
    if !names_other_instrument(this, previous) {
        changed |= yield_unknown_isin(this, previous);
        // The follower takes the identifiers it lacks.
        let mut ids = this.get_securityids().clone();
        if ids.carry(previous.get_securityids(), |_| true) {
            changed |= this.set_securityids(ids, true).is_ok();
        }
    }
    changed |= moved(
        this.get_cficode().cloned(),
        better_stated(this.get_cficode().cloned(), previous.get_cficode(), false),
        |code| this.set_cficode(code, true),
    );
    changed |= moved(
        this.get_miccode().cloned(),
        better_stated(this.get_miccode().cloned(), previous.get_miccode(), false),
        |code| this.set_miccode(code, true),
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
        this.set_metadata(Some(merged), true);
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
    let (Some(mine), Some(theirs)) = (
        this.get_securityids()
            .get_identifier(&IdType::Isin)
            .cloned(),
        other.get_securityids().get_identifier(&IdType::Isin),
    ) else {
        return false;
    };
    if !is_unknown_isin(mine.value()) || is_unknown_isin(theirs.value()) {
        return false;
    }
    let theirs = theirs.clone();
    let _ = this.remove_securityid(mine.src(), mine.kind());
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
        this.set_execunix(unix, true)
    });
    if later {
        changed |= moved(this.get_price(), other.get_price(), |px| {
            this.set_price(px, true)
        });
        changed |= moved(this.get_quantity(), other.get_quantity(), |qty| {
            this.set_quantity(qty, true)
        });
        changed |= moved(this.get_unit().clone(), other.get_unit().clone(), |unit| {
            this.set_unit(unit, true)
        });
    }
    macro_rules! optional {
        ($get:ident, $set:ident) => {
            changed |= moved(
                this.$get(),
                stated(this.$get(), other.$get(), later),
                |held| this.$set(held, true),
            );
        };
    }
    optional!(get_stoppx, set_stoppx);
    optional!(get_displayqty, set_displayqty);
    optional!(get_hiddenqty, set_hiddenqty);
    optional!(get_lastpx, set_lastpx);
    optional!(get_lastqty, set_lastqty);
    optional!(get_avgpx, set_avgpx);
    optional!(get_cumqty, set_cumqty);
    optional!(get_leavesqty, set_leavesqty);
    optional!(get_cxlqty, set_cxlqty);
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
        |ccy| this.set_bidccy(ccy, true),
    );
    changed |= moved(
        this.get_askccy().cloned(),
        stated(
            this.get_askccy().cloned(),
            other.get_askccy().cloned(),
            later,
        ),
        |ccy| this.set_askccy(ccy, true),
    );
    changed |= moved(
        this.get_currency().clone(),
        better(this.get_currency().clone(), other.get_currency(), later),
        |currency| this.set_currency(currency, true),
    );
    changed |= moved(
        this.get_side(),
        if later {
            other.get_side().merge_with(this.get_side())
        } else {
            this.get_side().merge_with(other.get_side())
        },
        |side| this.set_side(side, true),
    );
    changed |= moved(
        this.get_marketdatatype(),
        if later {
            stated_type(other.get_marketdatatype(), this.get_marketdatatype())
        } else {
            stated_type(this.get_marketdatatype(), other.get_marketdatatype())
        },
        |mdtype| this.set_marketdatatype(mdtype, true),
    );
    changed |= moved(
        this.get_ticker().map(SmolStr::new),
        stated(
            this.get_ticker().map(SmolStr::new),
            other.get_ticker().map(SmolStr::new),
            later,
        ),
        |ticker| this.set_ticker(ticker, true),
    );
    // Two statements naming different ISINs name two instruments, whose
    // identifiers never mix: the leading statement's stand whole.
    if names_other_instrument(this, other) {
        if later {
            changed |= this
                .set_securityids(other.get_securityids().clone(), true)
                .is_ok();
        }
    } else {
        changed |= yield_unknown_isin(this, other);
        for id in other.get_securityids() {
            match this.get_securityids().get_from(id.src(), id.kind()) {
                Some(held) if held == id.value() => {}
                // A `ZZ` ISIN never replaces a real one, whichever leads.
                Some(held)
                    if id.kind() == &IdType::Isin
                        && is_unknown_isin(id.value())
                        && !is_unknown_isin(held) => {}
                Some(_) if later => {
                    let _ = this.remove_securityid(id.src(), id.kind());
                    changed |= this.insert_securityid(id.clone()).unwrap_or(false);
                }
                Some(_) => {}
                None if id.src() == &IdSource::Derived => {
                    changed |= this.derive_securityid(id.kind(), id.value());
                }
                None => changed |= this.insert_securityid(id.clone()).unwrap_or(false),
            }
        }
    }
    changed |= moved(
        this.get_cficode().cloned(),
        better_stated(this.get_cficode().cloned(), other.get_cficode(), later),
        |code| this.set_cficode(code, true),
    );
    changed |= moved(
        this.get_miccode().cloned(),
        better_stated(this.get_miccode().cloned(), other.get_miccode(), later),
        |code| this.set_miccode(code, true),
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
            this.set_metadata(Some(merged), true);
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
            this.set_fxrates(merged, true);
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
        this.get_ordqty(),
        stated(this.get_ordqty(), previous.get_ordqty(), false),
        |qty| this.set_ordqty(qty, true),
    );
    changed |= moved(
        this.get_timeinforce().cloned(),
        stated(
            this.get_timeinforce().cloned(),
            previous.get_timeinforce().cloned(),
            false,
        ),
        |tif| this.set_timeinforce(tif, true),
    );
    changed |= moved(
        this.get_tradable(),
        stated(this.get_tradable(), previous.get_tradable(), false),
        |tradable| this.set_tradable(tradable, true),
    );
    // Each base identifier the follower states takes its parents - the
    // value before the last change, back to the chain's first - from its
    // predecessor's statement, before anything is carried, and each
    // identifier it lacks is carried where it follows.
    let mut identifiers = this.get_identifiers().clone();
    let parented = identifiers.follow_parents(
        previous.get_identifiers(),
        |base| this.parents_of(base),
        |kind| this.parent_of(kind),
    );
    let carried = identifiers.carry(previous.get_identifiers(), |id| {
        this.is_followed_identifier(id)
    });
    if parented || carried {
        changed |= this.set_identifiers(identifiers, true).is_ok();
    }
    // The parties an operation is booked to stay with its chain: a role this
    // statement names none for is the chain's.
    let mut partyids = this.get_partyids().clone();
    if partyids.carry(previous.get_partyids(), |_| true) {
        changed |= this.set_partyids(partyids, true).is_ok();
    }
    changed
}

/// The operation facts an element takes from another statement of itself.
pub(crate) fn merge_operation<E: Operation + ?Sized>(this: &mut E, other: &E, later: bool) -> bool {
    let mut changed = moved(
        this.get_ordqty(),
        stated(this.get_ordqty(), other.get_ordqty(), later),
        |qty| this.set_ordqty(qty, true),
    );
    changed |= moved(
        this.get_timeinforce().cloned(),
        stated(
            this.get_timeinforce().copied(),
            other.get_timeinforce().copied(),
            later,
        ),
        |tif| this.set_timeinforce(tif, true),
    );
    changed |= moved(
        this.get_tradable(),
        stated(this.get_tradable(), other.get_tradable(), later),
        |tradable| this.set_tradable(tradable, true),
    );
    if !other.get_identifiers().is_empty() {
        let mut merged = if later {
            other.get_identifiers().clone()
        } else {
            this.get_identifiers().clone()
        };
        let supplement = if later {
            this.get_identifiers()
        } else {
            other.get_identifiers()
        };
        merged.merge(supplement);
        if &merged != this.get_identifiers() && this.set_identifiers(merged, true).is_ok() {
            changed = true;
        }
    }
    if !other.get_partyids().is_empty() {
        let (mut merged, supplement) = if later {
            (other.get_partyids().clone(), this.get_partyids())
        } else {
            (this.get_partyids().clone(), other.get_partyids())
        };
        merged.merge(supplement);
        if &merged != this.get_partyids() && this.set_partyids(merged, true).is_ok() {
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
