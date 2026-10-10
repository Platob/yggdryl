//! The market vocabulary: what an element that stands in a market answers,
//! and what an operation on that market adds.
//!
//! [`Market`] is the slim reading a book level or any plain struct gives
//! cheaply: the instrument it is about - its security identifiers, its
//! classification, the market it trades on, the ticker it goes by and an
//! option's strike - the
//! side it takes, what it is priced and counted in and the currency it
//! originates in, the price and quantity
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

use crate::securityid::{SymbolCode, embedded};
use crate::{IdKey, IdType, Identifier, Identifiers, Side, TimeInForce};
use yggdryl::CodeValue;
use yggdryl::graph::{Element, Event};
use yggdryl::implementer::warned;
use yggdryl::implementer::{
    Staged, earliest, fold_event_instants, follow_timed, latest, merge_element,
    merge_event_element, merge_timed, moved, restate_event, right_is_reference, stated,
};
use yggdryl::xxhash::Xxh3;
use yggdryl::{Ccy, Cfi, Decimal, Isin, Mic, Result, State, Str, Unit, Uuid};

use super::facts::reports_fills;

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
/// `UKNW`, a currency or unit of none, a type of `UKNW` - and a map fills
/// only the keys it lacks. A value equal to the held one changes nothing
/// either way.
///
/// A change then carries what it implies onto the facts that follow it. A
/// source moves a fact it implies along with it - where the element states
/// nothing there, or still holds what the source was - and never one stated
/// apart from it; a fact that states something back about its source only
/// fills the source where the element states none; and nothing a setter's
/// fact implies writes that fact back, so a fact stated - as none included -
/// stands. Facts that do not contradict one another land alike in whatever
/// order they are stated:
///
/// | Changed | Moves | Fills back |
/// | --- | --- | --- |
/// | price, quantity | the bid of a buyer, the ask of a seller: `bidpx`/`bidqty`, `askpx`/`askqty` | |
/// | side | a sided element's - an order's, an execution's: the side it left stops quoting the price and quantity, and its cross code. Any other's is a tag over the legs it holds, withdrawing none: the price and quantity move from the leg the old tag took to the leg the new one takes. Either way the side it takes quotes them | the price and quantity, from what the side quotes |
/// | currency | `bidccy`/`askccy` of a bid or ask the element states | |
/// | quantity, `displayqty`, `hiddenqty` | `hiddenqty`, the quantity past the shown part | the third of an iceberg's three where two are stated: `displayqty` the quantity less the hidden part, the quantity the two parts together |
/// | `bidpx`/`bidqty`, `askpx`/`askqty` | | the price and quantity of an element taking that side |
/// | a predecessor's `hiddenqty`, followed | a follower stating none: what it kept back less what traded since - the rise in `cumqty`, else `lastqty` | |
/// | a predecessor's side, followed | a sided follower stating none takes it: the side is part of its identity | |
/// | a predecessor's `strikepx`, followed | a follower of the same instrument stating none takes it: the strike is the option's | |
/// | a predecessor's `origccy`, followed | a follower of the same instrument holding none takes it: the origin is the instrument's | |
/// | a predecessor's bid or ask, followed | an unsided follower tagging no side and stating neither the price nor the quantity of that leg: the leg whole, its currency with it | |
/// | a predecessor's `ordqty`, `cumqty`, `avgpx`, followed | an operation's follower stating none of them: what its chain ordered, traded and at what average - never a last fill, which no rise in `cumqty` invents | |
/// | `lastpx`, `spotrate`, `forwardpoints` | | the third, where two are stated: `lastpx` is spot plus points |
/// | an operation's `ordqty`, `cumqty`, `leavesqty`, `cxlqty` and its state | | the standing its state leaves them in ([`Operation::get_ordqty`]): working, the third of `ordqty`, `cumqty`, `leavesqty` - fresh, asked for or acknowledged, `leavesqty` and `ordqty` each other while nothing traded; filled, `leavesqty` 0 and `cumqty` and `ordqty` each other; ended any other way, `leavesqty` 0 and the third of `ordqty`, `cumqty`, `cxlqty` - canceled, done for the day, expired, `cxlqty` the rest of what was ordered. A state unstated implies none of it, and an execution or a trade reads as working whatever its state |
/// | `leavesqty` | the quantity of an order, a quote or a book entry: what is still available is what it is about | |
/// | `lastqty` | the quantity of an execution or a trade ([`MarketDataKind::is_recorded`](crate::MarketDataKind) less a book): what executed is what it is about, so one definition holds per kind - available on an order, executed on a fill | |
/// | a chain's fills, walked | a lifecycle walk counts an order chain's fills once by execution identifier over the chain's first stated total ([`Operation::fill_of`]): a repeated identifier adds nothing, `cumqty` is the count, `leavesqty` the accepted order quantity less it, and a partial state whose count reaches that quantity reads `FILLED` | |
/// | `cumqty`, `lastqty`, `lastpx` | | `avgpx`, the last price, where all that traded is the last, positive fill |
/// | `cficode` | | a detailed code over another describing one instrument, what that one says where it says nothing; a coarse code states nothing |
///
/// Each redirection writes its target directly, never through the target's
/// setter, so a change runs once and a chain of them cannot loop.
///
/// ```
/// use yggdryl::graph::Element;
/// use yggdryl_market::graph::{Market, OrderEvent};
/// use yggdryl_market::Side;
///
/// # fn main() -> yggdryl::Result<()> {
/// #     yggdryl_market::install().unwrap();
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
    /// The strike price of the option the element is about, where it states
    /// one: an instrument fact, which a follower of the same instrument
    /// stating none takes along its chain.
    fn get_strikepx(&self) -> Option<Decimal>;
    /// Sets [`Self::get_strikepx`].
    fn set_strikepx(&mut self, px: Option<Decimal>, overwrite: bool);
    /// The currency the element is priced in, [`Ccy::none`] where unstated.
    fn get_currency(&self) -> &Ccy;
    /// Sets [`Self::get_currency`].
    fn set_currency(&mut self, currency: Ccy, overwrite: bool);
    /// The currency the instrument originates in - the one it was issued
    /// in, which a depositary receipt or a share class listed in another
    /// currency trades apart from - where the element states it or the
    /// lifecycle's registry filled it, [`Ccy::none`] where neither did.
    /// Never defaulted, so a fill lands wherever nothing is held:
    /// [`Self::origin_currency`] is the reading that answers the currency
    /// where it is unheld.
    fn get_origccy(&self) -> &Ccy;
    /// Sets [`Self::get_origccy`]. Nothing else moves: the currency is
    /// never filled from it, nor it from the currency.
    fn set_origccy(&mut self, ccy: Ccy, overwrite: bool);
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
    /// The side the element takes, [`Side::Unknown`] where it states none:
    /// a sided element's one side ([`Self::is_sided`]), any other's tag - a
    /// one-sided quote names the leg it states, a two-sided quote states
    /// [`Side::Both`] ([`Self::fill_market`]), and a book is always `BOTH`.
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
    /// its kind is an order or an execution
    /// ([`MarketDataKind::is_sided`](crate::MarketDataKind::is_sided)).
    /// Every other element - a quote, which holds its bid and its ask and
    /// tags a side, a trade, a book, a snapshot control, a message of any
    /// other category - stores its cross code under side `0`, whatever side
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
    /// instrument key - one value per [`IdKey`], each held as its
    /// [`IdType`] stores it, and the base key of each type its answer
    /// ([`Identifiers`]): `isin` is the ISIN whichever source stated it.
    fn get_securityids(&self) -> &Identifiers;
    /// Replaces the stated security identifiers, or fills only the keys
    /// they lack without `overwrite`.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// one of them.
    fn set_securityids(&mut self, ids: Identifiers, overwrite: bool) -> Result<()>;
    /// States one security identifier, filling an absent key and the base
    /// key of its type where that is empty; a derived identifier of its type
    /// is taken back, since a statement answers before it
    /// ([`Identifiers::insert`]). Whether it was added.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// it.
    fn insert_securityid(&mut self, id: Identifier) -> Result<bool>;
    /// Removes what `key` holds ([`Identifiers::remove`]): a named
    /// source's identifier alone, or, under a base key, every identifier of
    /// its type; whether one was held. Once no ISIN is left, every derived
    /// identifier is taken back, since each hangs on it.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// the removal.
    fn remove_securityid(&mut self, key: &IdKey) -> Result<bool>;
    /// Derives one security identifier - one the element implies rather
    /// than states: the national number its ISIN carries, what its ticker's
    /// shape names, what a lifecycle learned - under the
    /// [`IdSource::Derived`](crate::IdSource::Derived) source, filling only a type
    /// the element holds none of; whether it was added. A derived
    /// identifier never reaches a store the holder is a view of.
    fn derive_securityid(&mut self, kind: &IdType, code: &str) -> bool;
    /// The detailed CFI classification, where one is known.
    fn get_cficode(&self) -> Option<&Cfi>;
    /// Sets [`Self::get_cficode`]: a code saying nothing past its category
    /// and group states nothing, and a detailed one stated over another
    /// describing the same instrument takes what that one says where it
    /// says nothing ([`Cfi::refined`]).
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
    /// The cross code of the instrument this element is about
    /// ([`Instrument::get_crosscode`](crate::Instrument)): a real ISIN for a
    /// security an agency numbered, a `class:body` for an FX pair or a
    /// derivative - the instruments table's own key, so a reader joins a
    /// market table to it on `instcode = crosscode` with no lookup. Written
    /// by a parse where the code is a function of the element's own facts
    /// and by a lifecycle's fill from the resolved instrument, followed
    /// along a chain, fed to no digest: a reading of facts the row already
    /// feeds, as `crossuuid` is.
    fn get_instcode(&self) -> Option<&str>;
    /// Records the cross code of the instrument this element is about;
    /// `None` clears it.
    fn set_instcode(&mut self, code: Option<Str>, overwrite: bool);
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
    /// element ([`Self::is_sided`]: an order or an execution) states its
    /// side there; any other - a quote, a trade, a book, a snapshot control -
    /// states `0` whatever side it states, `BOTH` included, as one taking
    /// [`Side::Unknown`] does: `14:0:Q-1`,
    /// `21:0:T-1`, `3:0:XNAS:ESVUFR`, `10:0:ORD-1`. Idempotent: a code
    /// already carrying this prefix is answered as it is, one carrying
    /// another has it replaced, and an empty code stays empty. The one place
    /// the prefix is decided; every market holder stores its cross code
    /// through it, so stamping the category or setting the side after the
    /// code converges on the same answer.
    ///
    /// ```
    /// # yggdryl_market::install().unwrap();
    /// use yggdryl_market::graph::{BookEvent, Market, OrderEvent};
    /// use yggdryl::graph::Element;
    /// use yggdryl_market::Side;
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
    /// let mut book = BookEvent::keyed(1, "AAPL");
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
    /// use yggdryl_market::graph::{Market, OrderEvent};
    /// use yggdryl::Ccy;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// #     yggdryl_market::install().unwrap();
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

    /// The currency an amount the element states converts from: its
    /// [`Self::get_origccy`] where it holds one, else its
    /// [`Self::get_currency`] - the origin defaulted by the currency at
    /// read, never stored, so it answers [`Ccy::none`] only where the
    /// element states neither. One direction only: the currency is never
    /// read off the origin. Borrows, so no input allocates.
    ///
    /// ```
    /// use yggdryl_market::graph::{Market, OrderEvent};
    /// use yggdryl::Ccy;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// #     yggdryl_market::install().unwrap();
    /// let mut order = OrderEvent::at(1);
    /// order.set_currency(Ccy::new("EUR")?, true);
    /// // No origin held: the currency answers, and nothing is stored.
    /// assert!(order.get_origccy().is_none());
    /// assert_eq!(order.origin_currency().as_str(), "EUR");
    /// // A USD-issued share listed in EUR states its origin apart.
    /// order.set_origccy(Ccy::new("USD")?, true);
    /// assert_eq!(order.origin_currency().as_str(), "USD");
    /// assert_eq!(order.get_currency().as_str(), "EUR");
    /// # Ok(())
    /// # }
    /// ```
    fn origin_currency(&self) -> &Ccy {
        let held = self.get_origccy();
        if held.is_none() {
            self.get_currency()
        } else {
            held
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
    /// currency where that is a leg of the pair, else the pair's base. A
    /// quote tagging no side that quotes both its legs states
    /// [`Side::Both`]; a tag it states stands.
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
                .and_then(|code| yggdryl::Forex::new(code).ok())
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
        // A quote tagging no side that quotes both legs holds both sides.
        if self.marketdatakind() == crate::MarketDataKind::Quotation
            && self.get_side() == Side::Unknown
            && quotes_leg(self, true)
            && quotes_leg(self, false)
        {
            self.set_side(Side::Both, false);
        }
        // The ISIN is owned - a code inline - so its national identifiers
        // are derived straight off it, with no list staged in between.
        let Some(isin) = self
            .get_securityids()
            .get(&IdType::Isin)
            .and_then(|code| Isin::new(code).ok())
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
        if other.get_uuid() != self.get_uuid() {
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
        if previous.get_uuid() == self.get_uuid() || previous.get_transunix() > self.get_transunix()
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
    /// chosen by its wire clock; `None` where they differ or nothing
    /// moved.
    fn merging_market_event(mut self, other: &Self) -> Option<Self>
    where
        Self: Event + Sized,
    {
        if other.get_uuid() != self.get_uuid() {
            return None;
        }
        let other_is_reference = right_is_reference(
            self.get_sendunix(),
            self.get_transunix(),
            other.get_sendunix(),
            other.get_transunix(),
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

/// What a statement reports of a fill, as its holder reads it off its own
/// words ([`Operation::fill_of`]): the one classification the lifecycle
/// walk's fill accounting trusts. The walk counts each fill of an order's
/// chain once by its execution identifier, over the chain's first stated
/// total, and reads the order's `cumqty` and `leavesqty` off the count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fill {
    /// A fill of `qty` under the execution identifier `execid`, the venue's
    /// first statement of it as far as the report says; a `qty` of zero says
    /// the fill is the rise in the stated `cumqty` over the chain's count,
    /// which the accounting reads where the report states no `lastqty`.
    New { execid: Str, qty: Decimal },
    /// A bust of the fill `refid` names - FIX `ExecType(150)` `H`, a
    /// `TRADE_CANCEL`: that fill's quantity is taken back off the count.
    Bust { refid: Str },
    /// A correction of the fill `refid` names - `ExecType` `G`, a
    /// `TRADE_CORRECT`: that fill's quantity becomes `qty`.
    Correct { refid: Str, qty: Decimal },
    /// A fill the report states a quantity for but no identifier: counted
    /// nowhere, since the walk cannot tell it from a resend, and warned.
    Unidentified { qty: Decimal },
    /// Not a fill: an acknowledgement, a status reply, a restatement, a
    /// replace, a cancel, a done-for-day, a pending report, a leg of a
    /// multi-leg report - whatever `lastqty` it repeats.
    NotAFill,
}

/// What a report states of its own fill before anything is followed - the
/// venue's words alone, read before a walk's `with_previous` carries the
/// chain's `cumqty` and identifiers onto it.
#[derive(Clone, Debug)]
pub(crate) struct OwnFills {
    cumqty: Option<Decimal>,
    leavesqty: Option<Decimal>,
    ordqty: Option<Decimal>,
    hiddenqty: Option<Decimal>,
    execid: Option<Str>,
    secondaryexecid: Option<Str>,
    /// Whether the ended state the report reads is its own word
    /// ([`Operation::states_end`]).
    states_end: bool,
    fill: Fill,
}

impl OwnFills {
    /// The report's own words.
    pub(crate) fn read<E: Event + Operation + ?Sized>(this: &E) -> Self {
        let ids = this.get_identifiers();
        Self {
            cumqty: this.get_cumqty(),
            leavesqty: this.get_leavesqty(),
            ordqty: this.get_ordqty(),
            hiddenqty: this.get_hiddenqty(),
            execid: ids.get(&IdType::ExecId).map(Str::from),
            secondaryexecid: ids.get(&IdType::SecondaryExecId).map(Str::from),
            states_end: this.states_end(),
            fill: this.fill_of(),
        }
    }

    /// Whether the report says of its fill what `other` says.
    pub(crate) fn repeats(&self, other: &Self) -> bool {
        self.fill == other.fill
    }

    /// The execution identifiers the report states, its own then the
    /// exchange's.
    pub(crate) fn ids(&self) -> impl Iterator<Item = &Str> {
        self.execid.iter().chain(self.secondaryexecid.iter())
    }
}

/// One fill a chain counted: its identifier, the quantity counted under it
/// and the identity of the statement that counted it - stamped once that
/// statement is settled, so a later statement of the same fill is another
/// statement of that one.
#[derive(Clone, Debug)]
struct Counted {
    id: Str,
    qty: Decimal,
    uuid: Uuid,
}

/// What a chain remembers of its fills: each fill counted by its execution
/// identifier with the quantity counted under it, what they add up to over
/// the chain's anchor - the total the chain's first stated `cumqty` opened
/// on, what traded before the walk saw the chain - and the order quantity
/// the venue accepted. The first fill is held inline and the rest sorted by
/// identifier, so a chain of one fill - an order filled by its first report,
/// an execution, which is the one fill it reports - costs no allocation and a
/// report's lookup is one comparison and one binary search. Bounded at
/// [`Self::MAX_COUNTED`] fills: past it a fill is counted into the total and
/// not remembered, warned once per kind, so a later copy of it would count
/// again - said in the warning.
#[derive(Clone, Debug)]
pub(crate) struct Fills {
    first: Option<Counted>,
    more: Vec<Counted>,
    /// The sum of every counted quantity.
    sum: Decimal,
    /// Whether a fill was ever counted: the count is a total only then or
    /// once an anchor stands.
    filled: bool,
    anchor: Option<Decimal>,
    ordqty: Option<Decimal>,
    /// The fills counted since the last stamp, their statement's identity
    /// not yet settled.
    unstamped: u8,
}

impl Default for Fills {
    fn default() -> Self {
        Self {
            first: None,
            more: Vec::new(),
            sum: Decimal::ZERO,
            filled: false,
            anchor: None,
            ordqty: None,
            unstamped: 0,
        }
    }
}

impl Fills {
    /// The fills one chain remembers at most: one venue's fills of one order
    /// in a day, with room.
    pub(crate) const MAX_COUNTED: usize = 4096;

    /// Whether the chain counted nothing and anchored on nothing.
    pub(crate) fn is_empty(&self) -> bool {
        self.first.is_none() && self.anchor.is_none()
    }

    /// The fill counted under `id`.
    fn get(&self, id: &str) -> Option<&Counted> {
        if let Some(first) = &self.first
            && first.id == id
        {
            return Some(first);
        }
        self.more
            .binary_search_by(|held| held.id.as_str().cmp(id))
            .ok()
            .map(|at| &self.more[at])
    }

    fn get_mut(&mut self, id: &str) -> Option<&mut Counted> {
        if self.first.as_ref().is_some_and(|first| first.id == id) {
            return self.first.as_mut();
        }
        self.more
            .binary_search_by(|held| held.id.as_str().cmp(id))
            .ok()
            .map(|at| &mut self.more[at])
    }

    /// The identity of the statement that counted a fill under `id`, where
    /// one did and the statement is settled.
    pub(crate) fn counted_by(&self, id: &str) -> Option<Uuid> {
        self.get(id)
            .map(|held| held.uuid)
            .filter(|uuid| !uuid.is_nil())
    }

    /// Remembers `id` as counted at `qty`, where the ledger has room: whether
    /// it was remembered. Appended where it sorts last, inserted otherwise.
    fn remember(&mut self, id: &Str, qty: Decimal) -> bool {
        let counted = Counted {
            id: id.clone(),
            qty,
            uuid: Uuid::default(),
        };
        if self.first.is_none() {
            self.first = Some(counted);
        } else {
            if self.more.len() + 1 >= Self::MAX_COUNTED {
                return false;
            }
            match self
                .more
                .binary_search_by(|held| held.id.as_str().cmp(id.as_str()))
            {
                Ok(_) => return true,
                Err(at) => self.more.insert(at, counted),
            }
        }
        self.unstamped += 1;
        true
    }

    /// Counts a fill of `qty` under `own`'s identifiers, warned under `kind`
    /// where the ledger is full.
    fn count(&mut self, own: &OwnFills, qty: Decimal, kind: &str, code: &str) {
        self.sum = self.sum.checked_add(qty).unwrap_or(self.sum);
        self.filled = true;
        for id in own.ids() {
            if !self.remember(id, qty) {
                warned!(
                    "lifecycle ledger full: a fill counted and not remembered",
                    kind,
                    "{code}: fill {id} of {qty} counted past {} remembered fills; a later copy of it would count again",
                    Self::MAX_COUNTED
                );
            }
        }
    }

    /// Moves the quantity counted under `id` by `delta`.
    fn move_counted(&mut self, id: &str, delta: Decimal) {
        if let Some(held) = self.get_mut(id) {
            held.qty = held.qty.checked_add(delta).unwrap_or(held.qty);
        }
        self.sum = self.sum.checked_add(delta).unwrap_or(self.sum);
    }

    /// What the chain has traded: the anchor plus every counted quantity,
    /// nothing before the first fill and the first stated total.
    pub(crate) fn consumed(&self) -> Option<Decimal> {
        (self.filled || self.anchor.is_some()).then(|| {
            self.anchor
                .unwrap_or(Decimal::ZERO)
                .checked_add(self.sum)
                .unwrap_or(self.sum)
        })
    }

    /// Adopts `stated` as the chain's first stated total where none stands
    /// and the chain counted no fill yet: the anchor is what traded before
    /// the count. Adoption runs only upward from a count of nothing - a
    /// total stated once fills were counted is the count's to agree with,
    /// warned where it does not, never adopted - and nothing where the
    /// count already exceeds it; whether an anchor stands.
    fn anchor_on(&mut self, stated: Decimal) -> bool {
        if self.anchor.is_some() {
            return true;
        }
        if self.filled {
            return false;
        }
        match stated
            .checked_sub(self.sum)
            .filter(|held| !held.is_negative())
        {
            Some(before) => {
                self.anchor = Some(before);
                true
            }
            None => false,
        }
    }

    /// Stamps the statement `uuid` onto every fill counted since the last
    /// stamp.
    pub(crate) fn stamp(&mut self, uuid: Uuid) {
        if self.unstamped == 0 {
            return;
        }
        self.unstamped = 0;
        if let Some(first) = &mut self.first
            && first.uuid.is_nil()
        {
            first.uuid = uuid;
        }
        for held in &mut self.more {
            if held.uuid.is_nil() {
                held.uuid = uuid;
            }
        }
    }

    /// Takes every fill `other` remembers into this ledger: what a chain
    /// ended again under the same base cross code, category and side leaves
    /// behind.
    pub(crate) fn absorb(&mut self, other: Self) {
        for held in other.first.into_iter().chain(other.more) {
            if self.get(&held.id).is_some() {
                continue;
            }
            if self.first.is_none() {
                self.first = Some(held);
            } else if self.more.len() + 1 < Self::MAX_COUNTED
                && let Err(at) = self
                    .more
                    .binary_search_by(|kept| kept.id.as_str().cmp(held.id.as_str()))
            {
                self.more.insert(at, held);
            }
        }
    }
}

/// What the accounting of one statement answered: whether it moved
/// anything on the statement, and the settled statement that already
/// counted the fill this one reports, where one did.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Accounted {
    pub(crate) moved: bool,
    pub(crate) repeated: Option<Uuid>,
}

/// The states a count reaching the order quantity turns into `FILLED`: the
/// ones that say some of the order traded and it is still working, and the
/// carrying-on states the fold leaves a fill's report in over a chain that
/// was replaced, updated, restated or amended - following keeps the higher
/// rank, so a partial fill after a `REPLACED` reads `REPLACED`.
fn is_partial(state: State) -> bool {
    matches!(
        state,
        State::InProgress
            | State::PartiallyFilled
            | State::Trade
            | State::Updated
            | State::Replaced
            | State::Restated
            | State::Amended
    )
}

/// Whether `state` asks for something the venue has not answered - the
/// pending band, or a cancel, a replace or a reversal awaiting its answer -
/// so what it states of the order quantity is not yet the accepted one.
fn awaits_answer(state: State) -> bool {
    state.is_pending() || state.rank() == 60
}

/// The one fill accounting of a lifecycle walk: `this` as the statement
/// after what the chain `fills` remembers, its own words `own` read before
/// anything was followed, `kept` the live element's hidden part before this
/// statement.
///
/// An order's statement counts the fill it reports once by its execution
/// identifier: a fill the chain already counted - under either identifier
/// of either statement - adds nothing, moves no quantity of the chain and
/// promotes nothing; a new one adds its quantity, a bust takes the fill it
/// names back, a correction replaces it, a fill naming no identifier is
/// counted nowhere and warned. The chain's first stated total is adopted
/// once as its anchor while it counted no fill - what traded before the
/// walk saw the chain - and from
/// there the count is the order's `cumqty`, its `leavesqty` the accepted
/// order quantity less it, floored at nothing: a stated total that
/// disagrees is warned under the kind and does not move the ledger, since a
/// cumulative quantity only rises but for a bust or a correction, which the
/// ledger does itself. A count reaching the accepted order quantity turns a
/// partial state into `FILLED`, never the reverse: a stated `FILLED` whose
/// count falls short stays `FILLED` and is warned, while an ended state the
/// holder reads off a remainder rather than states as its own word - a FIX
/// trade report's `FILLED` that its `LeavesQty(151)` of nothing reads - reads as
/// the partial fill the count says it is, over a chain the walk counted
/// before it; a chain's first statement stands as its holder reads it. An overfill - a count past the
/// order quantity - leaves nothing and promotes nothing, warned. An
/// execution or a trade is the fill it reports: its chain remembers its
/// identifier alone, so a second statement of it under another instant is
/// another statement of the first; any other kind reports no fill. Every
/// quantity is written through the statement's own setter with `overwrite`,
/// so a holder marks it stated.
pub(crate) fn account_fills<E: Event + Operation + ?Sized>(
    this: &mut E,
    own: &OwnFills,
    kept: Option<Decimal>,
    fills: &mut Fills,
) -> Accounted {
    let kind = this.marketdatakind();
    let mut accounted = Accounted::default();
    if kind != crate::MarketDataKind::Order {
        if reports_fills(kind) {
            for id in own.ids() {
                match fills.get(id) {
                    Some(counted) if !counted.uuid.is_nil() => {
                        accounted.repeated = Some(counted.uuid);
                    }
                    Some(_) => {}
                    None => {
                        fills.remember(id, Decimal::ZERO);
                    }
                }
            }
        }
        return accounted;
    }
    let subject = kind.as_str();
    let mut state = *this.get_state();
    // Whether the chain stated anything of its fills before this statement:
    // a count is the order's only over what the walk saw of it.
    let known = fills.anchor.is_some() || fills.filled;
    // 2. The accepted order quantity: a request's or a pending report's
    // moves it only where the chain states none yet.
    if let Some(ordered) = own.ordqty
        && (fills.ordqty.is_none() || !awaits_answer(state))
    {
        fills.ordqty = Some(ordered);
    }
    // The total the venue states: a report whose ended state is its
    // parse's reading of the quantities states none of its own, its
    // `cumqty` being that reading's.
    let total = own.cumqty.filter(|_| own.states_end);
    // 3. The fill, against the ledger: `delta` is what this statement
    // moved the count by.
    let mut delta: Option<Decimal> = None;
    match &own.fill {
        Fill::New { execid, qty } => {
            if let Some(counted) = own.ids().find_map(|id| fills.get(id)) {
                accounted.repeated = Some(counted.uuid);
            } else {
                let qty = if qty.is_zero() {
                    total
                        .zip(fills.consumed())
                        .and_then(|(stated, held)| stated.checked_sub(held))
                        .filter(|rise| !rise.is_negative())
                } else {
                    Some(*qty)
                };
                match qty {
                    Some(qty) => {
                        // A capture opened mid-life: the first stated total
                        // less this fill is what traded before.
                        if let Some(stated) = total
                            && fills.anchor.is_none()
                            && let Some(before) = stated.checked_sub(qty)
                        {
                            fills.anchor_on(before);
                        }
                        fills.count(own, qty, subject, this.get_crosscode());
                        delta = Some(qty);
                    }
                    None => warned!(
                        "lifecycle fill states no quantity: counted nowhere",
                        subject,
                        "{code}: fill {execid} states no last quantity and no total the chain's count reads a rise from",
                        code = this.get_crosscode()
                    ),
                }
            }
        }
        Fill::Bust { refid } => {
            match fills.get(refid).map(|held| held.qty) {
                Some(busted) => {
                    let back = Decimal::ZERO.checked_sub(busted).unwrap_or(Decimal::ZERO);
                    fills.move_counted(refid, back);
                    delta = Some(back);
                }
                None => warned!(
                    "lifecycle bust names no counted fill: nothing moves",
                    subject,
                    "{code}: a bust of fill {refid}, which the chain never counted",
                    code = this.get_crosscode()
                ),
            }
            for id in own.ids() {
                fills.remember(id, Decimal::ZERO);
            }
        }
        Fill::Correct { refid, qty } => {
            match fills.get(refid).map(|held| held.qty) {
                Some(was) => {
                    let diff = qty.checked_sub(was).unwrap_or(Decimal::ZERO);
                    fills.move_counted(refid, diff);
                    delta = Some(diff);
                }
                None => {
                    warned!(
                        "lifecycle correction names no counted fill: counted as the fill",
                        subject,
                        "{code}: a correction of fill {refid} to {qty}, which the chain never counted",
                        code = this.get_crosscode()
                    );
                    fills.sum = fills.sum.checked_add(*qty).unwrap_or(fills.sum);
                    fills.filled = true;
                    fills.remember(refid, *qty);
                    delta = Some(*qty);
                }
            }
            for id in own.ids() {
                fills.remember(id, Decimal::ZERO);
            }
        }
        Fill::Unidentified { qty } => warned!(
            "lifecycle fill states no execution identifier: counted nowhere",
            subject,
            "{code}: a fill of {qty} naming no execution identifier, which the chain cannot tell from a resend",
            code = this.get_crosscode()
        ),
        Fill::NotAFill => {}
    }
    // 4. The anchor: the chain's first stated total, adopted once while it
    // counted no fill - a statement reporting no fill anchors on it.
    if delta.is_none()
        && let Some(stated) = total
        && fills.anchor.is_none()
    {
        fills.anchor_on(stated);
    }
    let consumed = fills.consumed();
    let disagrees = |stated: Option<Decimal>| {
        stated
            .zip(consumed)
            .filter(|(stated, count)| stated != count)
    };
    // An ended state stands where the holder states it, and on a chain's
    // first statement, which the walk has nothing to count against; one its
    // parse derived over a chain the walk counted reads as the partial fill
    // the count says it is.
    let mut corrected = false;
    if !state.is_live() {
        let short = consumed
            .zip(fills.ordqty)
            .is_some_and(|(count, ordered)| count < ordered);
        if !own.states_end && short && known {
            this.set_state(State::PartiallyFilled);
            state = State::PartiallyFilled;
            corrected = true;
            accounted.moved = true;
        } else {
            if let Some((stated, count)) = disagrees(total) {
                warned!(
                    "lifecycle fill total disagrees with the count: the count stands",
                    subject,
                    "{code}: an ended report states {stated} traded where the chain counted {count}",
                    code = this.get_crosscode()
                );
            }
            return accounted;
        }
    }
    let takes_count = delta.is_some() || corrected;
    // 5. The cumulative quantity: the count's where this statement moved it
    // or states none, the venue's own number where it disagrees on a
    // statement that counted nothing.
    if let Some(count) = consumed {
        if let Some((stated, _)) = disagrees(total) {
            warned!(
                "lifecycle fill total disagrees with the count: the count stands",
                subject,
                "{code}: a report states {stated} traded where the chain counted {count}",
                code = this.get_crosscode()
            );
        }
        if (total.is_none() || takes_count) && this.get_cumqty() != Some(count) {
            this.set_cumqty(Some(count), true);
            accounted.moved = true;
        }
    }
    // 6. What is left: the accepted order quantity less the count, floored
    // at nothing - an overfill is warned and promotes nothing.
    if let Some(ordered) = fills.ordqty
        && let Some(traded) = this.get_cumqty()
    {
        let left = ordered.checked_sub(traded);
        let overfilled = left.is_some_and(|left| left.is_negative());
        let left = left
            .filter(|left| !left.is_negative())
            .unwrap_or(Decimal::ZERO);
        if takes_count {
            if overfilled {
                warned!(
                    "lifecycle fills exceed the order quantity: nothing left, the state stands",
                    subject,
                    "{code}: the chain counted {traded} against {ordered} ordered",
                    code = this.get_crosscode()
                );
            }
            if this.get_leavesqty() != Some(left) {
                this.set_leavesqty(Some(left), true);
                accounted.moved = true;
            }
        } else if let Some(stated) = own.leavesqty {
            if stated != left {
                warned!(
                    "lifecycle stated leaves disagree with the order quantity less the count: left as stated",
                    subject,
                    "{code}: a report states {stated} left where {ordered} ordered less {traded} traded leaves {left}",
                    code = this.get_crosscode()
                );
            }
        } else if this.get_leavesqty() != Some(left) {
            this.set_leavesqty(Some(left), true);
            accounted.moved = true;
        }
        // 7. The state: a partial fill with nothing left is filled - after
        // a fill, a bust or a correction, never a restatement.
        if delta.is_some() && ordered.is_positive() && traded == ordered && is_partial(state) {
            this.set_state(State::Filled);
            accounted.moved = true;
        }
    }
    // 8. The hidden part: what the live element kept back, less what this
    // statement counted - a repeated fill takes nothing off it twice.
    if own.hiddenqty.is_none()
        && let Some(kept) = kept
    {
        let hidden = kept
            .checked_sub(delta.unwrap_or(Decimal::ZERO))
            .filter(|left| !left.is_negative())
            .unwrap_or(Decimal::ZERO);
        if this.get_hiddenqty() != Some(hidden) {
            this.set_hiddenqty(Some(hidden), true);
            accounted.moved = true;
        }
    }
    accounted
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
    let side = kind.stored_side(side);
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
    /// [`Market::get_leavesqty`] fill the third, and while nothing traded
    /// what is left is all that was ordered; filled, nothing is left and
    /// what was ordered all traded; no longer active - canceled, done for the
    /// day, expired, calculated, rejected, failed - nothing is left, what was
    /// ordered is what traded plus what was canceled
    /// ([`Market::get_cxlqty`]), and what someone or the clock ended
    /// canceled the rest. An order whose state is unstated implies none of
    /// it, and an execution or a trade, whose state says what it reports,
    /// reads as working whatever its state. What was ordered is never the
    /// quantity the element is about: [`Market::get_quantity`] stands apart
    /// from it.
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
    /// States one identifier, filling only an absent key and the base key
    /// of its type where that is empty; whether it was added.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// it.
    fn insert_identifier(&mut self, id: Identifier) -> Result<bool>;
    /// Removes what `key` holds - a base key every identifier of its type -
    /// ([`Identifiers::remove`]); whether one was held.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// the removal.
    fn remove_identifier(&mut self, key: &IdKey) -> Result<bool>;
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
    /// States one party, filling only an absent key and the base key of its
    /// role where that is empty; whether it was added.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// it.
    fn insert_partyid(&mut self, partyid: Identifier) -> Result<bool>;
    /// Removes what `key` holds - a base key every party of its role -
    /// ([`Identifiers::remove`]); whether one was held.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder is a view of a store that refuses
    /// the removal.
    fn remove_partyid(&mut self, key: &IdKey) -> Result<bool>;
    /// The set an identifier of `kind` belongs to: a security type's
    /// [`Market::get_securityids`], a party's [`Self::get_partyids`], any
    /// other type's [`Self::get_identifiers`] - where a key naming one lands,
    /// and so where a reader asks whether a key is held.
    fn identifier_set(&self, kind: &IdType) -> &Identifiers {
        if kind.is_security() {
            self.get_securityids()
        } else if kind.is_party() {
            self.get_partyids()
        } else {
            self.get_identifiers()
        }
    }
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
    /// What this statement reports of a fill ([`Fill`]), read off the
    /// holder's own words and never off the lifecycle state, which cannot
    /// tell a fill from a status reply that reads `PARTIALLY_FILLED`: a
    /// positive `lastqty` under an `execid` is a new fill, one under no
    /// identifier an unidentified one, anything else no fill. A holder
    /// whose wire says more overrides it - a FIX message reads its
    /// `ExecType(150)`, `ExecRefID(19)` and `MultiLegReportingType(442)`.
    /// What a lifecycle walk counts an order chain's fills by.
    fn fill_of(&self) -> Fill {
        match self.get_lastqty().filter(|qty| qty.is_positive()) {
            Some(qty) => match self.get_identifiers().get(&IdType::ExecId) {
                Some(execid) if execid != "0" => Fill::New {
                    execid: Str::from(execid),
                    qty,
                },
                _ => Fill::Unidentified { qty },
            },
            None => Fill::NotAFill,
        }
    }
    /// Whether the ended state this statement reads is the holder's own
    /// word: always, for a plain event, whose state is its own fact; a FIX
    /// message but where it is the `FILLED` a trade report's
    /// `LeavesQty(151)` of nothing reads - the venue's remainder, which a
    /// lifecycle walk's count may contradict.
    fn states_end(&self) -> bool {
        true
    }
    /// Stands this operation under the identity of `live`, the live
    /// statement of the chain a lifecycle states it in: a sided operation
    /// stating no side takes the live one's, as a follower does, and then
    /// the live statement's stored cross code where that states one and
    /// this one's differs, its cross hash code and cross element brought in
    /// step - the side first, because a code is stored under the side its
    /// holder takes. Whether anything moved; the caller finalizes where it
    /// did. A holder whose side is content of its own - a FIX message
    /// writing `Side(54)` - overrides it to state the side there.
    fn follow_identity(&mut self, live: &Self) -> bool
    where
        Self: Element + Sized,
    {
        follow_identity(self, live)
    }
    /// Says that a lifecycle found this operation citing two live chains -
    /// `cited` naming its own stored cross code and each chain's with the
    /// name that cited it - and stood it under its own identity. Nothing by
    /// default; a FIX message records it as an anomaly of its own.
    fn note_conflict(&mut self, cited: &str) {
        let _ = cited;
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
        if other.get_uuid() != self.get_uuid() {
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
        if previous.get_uuid() == self.get_uuid() || previous.get_transunix() > self.get_transunix()
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
    /// reference chosen by its wire clock; `None` where they differ or
    /// nothing moved.
    fn merging_operation_event(mut self, other: &Self) -> Option<Self>
    where
        Self: Event + Sized,
    {
        if other.get_uuid() != self.get_uuid() {
            return None;
        }
        let other_is_reference = right_is_reference(
            self.get_sendunix(),
            self.get_transunix(),
            other.get_sendunix(),
            other.get_transunix(),
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
/// unstamped lifecycle input that itself reports an execution
/// ([`Event::is_execution`]), its own instant. A predecessor marks a
/// lifecycle output: its state may have been inherited, so replaying that
/// output must not reinterpret the folded state as this event's own
/// execution report.
fn execution_unix<E: Event + Market + ?Sized>(event: &E) -> Option<i64> {
    event.get_execunix().or_else(|| {
        (event.get_prevuuid().is_none() && event.is_execution()).then_some(event.get_transunix())
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

/// The facts an operation on the market takes from another statement of
/// itself, finalizing nothing; whether any moved. What a holder folding
/// several statements reads, settling its own identity once after the last.
pub(crate) fn merge_operation_event<E: Event + Operation>(
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
    // Fed only where held, so an element stating no origin digests as it
    // did before the origin was a fact.
    let origccy = this.get_origccy();
    if !origccy.is_none() {
        staged.feed("origccy", origccy.as_str().as_bytes());
    }
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
        ("strikepx", this.get_strikepx()),
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

/// [`Operation::follow_identity`] over any element of a market, the live
/// one of any holder the same type reads: what a walk over
/// [`MarketData`](super::MarketData) states a follower under where its
/// live statement is another variant. A sided element stating no side takes
/// the live one's as [`chain_market`] does, then the live stored cross code
/// is forced as [`follow_element`](yggdryl::graph::element::follow_element) forces it.
pub(crate) fn follow_identity<E: Element + Market + ?Sized>(this: &mut E, live: &E) -> bool {
    let mut changed = false;
    if this.is_sided() {
        changed |= moved(
            this.get_side(),
            this.get_side().merge_with(live.get_side().tagged()),
            |side| this.set_side(side, true),
        );
    }
    changed | yggdryl::implementer::follow_element(this, live)
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
/// statement always leads, and nothing here is about a step. A sided
/// element's side - an order's, an execution's - is this statement's own
/// where it states one and the chain's where it states none, the side being
/// part of what the chain is; any other element's side is its own tag, and
/// it takes each leg of its quote it states nothing of where it updates
/// the quote rather than restating a level ([`carry_legs`]). The metadata
/// is this statement's, every key of the
/// chain's it does not state beside it.
fn chain_market<E: Market + ?Sized>(this: &mut E, previous: &E) -> bool {
    let mut changed = follow_hidden(this, previous);
    changed |= moved(
        this.get_currency().clone(),
        better(this.get_currency().clone(), previous.get_currency(), false),
        |currency| this.set_currency(currency, true),
    );
    if this.is_sided() {
        changed |= moved(
            this.get_side(),
            this.get_side().merge_with(previous.get_side().tagged()),
            |side| this.set_side(side, true),
        );
    }
    changed |= carry_legs(this, previous);
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
        changed |= yield_lower_isin(this, previous);
        // The follower takes the identifiers it lacks.
        let mut ids = this.get_securityids().clone();
        if ids.carry(previous.get_securityids(), |_| true) {
            changed |= this.set_securityids(ids, true).is_ok();
        }
        // And the option's strike, an instrument fact like its codes.
        changed |= moved(
            this.get_strikepx(),
            stated(this.get_strikepx(), previous.get_strikepx(), false),
            |px| this.set_strikepx(px, true),
        );
        // And the currency the instrument originates in, another.
        changed |= moved(
            this.get_origccy().clone(),
            better(this.get_origccy().clone(), previous.get_origccy(), false),
            |ccy| this.set_origccy(ccy, true),
        );
        // And the instrument's code, the instrument fact every row carries.
        if this.get_instcode().is_none()
            && let Some(code) = previous.get_instcode()
        {
            this.set_instcode(Some(Str::new(code)), true);
            changed = true;
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

/// Carries the legs of a quote along its chain: an unsided follower takes
/// each leg of the statement it follows - the bid, the ask - that it states
/// neither the price nor the quantity of, whole: its price, its quantity
/// and its currency as one, the currency only where the follower states
/// none for that leg. A statement updating one leg so keeps the other, an
/// acknowledgement quoting nothing keeps the quote, and a leg stated - a
/// zero quantity withdrawing it included - is the follower's own; the leg a
/// follower's tag takes, stated by a quantity alone, keeps the chain's
/// price. A tagged follower of a tagged entry quoting one leg - a book
/// level - restates that entry whole, moving between sides included; one
/// following a quote that holds both legs or tags no one leg - none, or
/// `BOTH` - a fill reported on the leg that traded - keeps the other. A
/// sided follower quotes its own side alone, and no leg crosses to another
/// instrument. Whether anything moved.
fn carry_legs<E: Market + ?Sized>(this: &mut E, previous: &E) -> bool {
    if this.is_sided()
        || names_other_instrument(this, previous)
        || names_other_ticker(this, previous)
    {
        return false;
    }
    let tag = this.get_side().tagged();
    if tag != Side::Unknown
        && previous.get_side().tagged() != Side::Unknown
        && !(quotes_leg(previous, true) && quotes_leg(previous, false))
    {
        return false;
    }
    let mut changed = false;
    if this.get_bidpx().is_none() && this.get_bidqty().is_none() {
        if quotes_leg(previous, true) {
            if this.get_bidccy().is_none() && previous.get_bidccy().is_some() {
                this.set_bidccy(previous.get_bidccy().cloned(), true);
            }
            this.set_bidpx(previous.get_bidpx(), true);
            this.set_bidqty(previous.get_bidqty(), true);
            changed = true;
        }
    } else if tag.is_bid()
        && this.get_bidpx().is_none()
        && this.get_bidqty().is_some_and(|qty| !qty.is_zero())
        && previous.get_bidpx().is_some()
    {
        this.set_bidpx(previous.get_bidpx(), true);
        changed = true;
    }
    if this.get_askpx().is_none() && this.get_askqty().is_none() {
        if quotes_leg(previous, false) {
            if this.get_askccy().is_none() && previous.get_askccy().is_some() {
                this.set_askccy(previous.get_askccy().cloned(), true);
            }
            this.set_askpx(previous.get_askpx(), true);
            this.set_askqty(previous.get_askqty(), true);
            changed = true;
        }
    } else if tag.is_ask()
        && this.get_askpx().is_none()
        && this.get_askqty().is_some_and(|qty| !qty.is_zero())
        && previous.get_askpx().is_some()
    {
        this.set_askpx(previous.get_askpx(), true);
        changed = true;
    }
    changed
}

/// Whether `market` quotes one leg - the bid where `bid`, else the ask -
/// stating its price or its quantity.
fn quotes_leg<E: Market + ?Sized>(market: &E, bid: bool) -> bool {
    if bid {
        market.get_bidpx().is_some() || market.get_bidqty().is_some()
    } else {
        market.get_askpx().is_some() || market.get_askqty().is_some()
    }
}

/// Whether `this` and `other` each state a ticker, and not the same one:
/// two instruments, whose quotes never mix.
fn names_other_ticker<E: Market + ?Sized>(this: &E, other: &E) -> bool {
    matches!(
        (this.get_ticker(), other.get_ticker()),
        (Some(mine), Some(theirs)) if !mine.is_empty() && !theirs.is_empty() && mine != theirs
    )
}

/// Whether `this` and `other` each state a real ISIN - closing under a
/// listed prefix ([`Isin::rank_of`]) - and not the same one: two
/// instruments. A `ZZ` number, a masked one or a typo names no country's
/// instrument, so it is never the other one.
fn names_other_instrument<E: Market + ?Sized>(this: &E, other: &E) -> bool {
    matches!(
        (this.get_isincode(), other.get_isincode()),
        (Some(mine), Some(theirs))
            if mine != theirs
                && IdType::Isin.is_real(mine)
                && IdType::Isin.is_real(theirs)
    )
}

/// An ISIN `this` states yields to one `other` states that outranks it
/// ([`Isin::rank_of`]) - a `ZZ` number, a masked one or a typo to a real
/// one: the type taken out - and every identifier derived from it with it -
/// and replaced by `other`'s statements of it, its base key first, so what
/// the higher-ranked one carries derives afresh. Whether it moved.
fn yield_lower_isin<E: Market + ?Sized>(this: &mut E, other: &E) -> bool {
    let (Some(mine), Some(theirs)) = (this.get_isincode(), other.get_isincode()) else {
        return false;
    };
    if Isin::rank_of(mine) >= Isin::rank_of(theirs) {
        return false;
    }
    let ids = other.get_securityids();
    let echoed = ids.is_derived(&IdType::Isin);
    let mut theirs: Vec<Identifier> = ids
        .of_kind(&IdType::Isin)
        .filter(|id| !(echoed && id.key().is_base()))
        .cloned()
        .collect();
    theirs.sort_by_key(|id| !id.key().is_base());
    let mut moved = this
        .remove_securityid(&IdKey::base(IdType::Isin))
        .unwrap_or(false);
    for id in theirs {
        moved |= this.insert_securityid(id).unwrap_or(false);
    }
    moved
}

/// The market facts an element takes from another statement of itself:
/// each optional fact - the price and the quantity as every other - the
/// leading statement's where it states one and the other's where it does
/// not, and each code - the unit, the currency - the better of the two.
/// `later` says whether `other` is the leading statement.
pub(crate) fn merge_market<E: Market + ?Sized>(this: &mut E, other: &E, later: bool) -> bool {
    // Two statements of one element keep the earliest execution either knows.
    let execunix = earliest(this.get_execunix(), other.get_execunix());
    let mut changed = moved(this.get_execunix(), execunix, |unix| {
        this.set_execunix(unix, true)
    });
    changed |= moved(
        this.get_unit().clone(),
        better(this.get_unit().clone(), other.get_unit(), later),
        |unit| this.set_unit(unit, true),
    );
    macro_rules! optional {
        ($get:ident, $set:ident) => {
            changed |= moved(
                this.$get(),
                stated(this.$get(), other.$get(), later),
                |held| this.$set(held, true),
            );
        };
    }
    // The legs and the side first, leg by leg: a price or a quantity the
    // side reads off a leg then follows the merged leg, whichever statement
    // leads, rather than dragging the other statement's leg along.
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
        this.get_origccy().clone(),
        better(this.get_origccy().clone(), other.get_origccy(), later),
        |ccy| this.set_origccy(ccy, true),
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
    optional!(get_price, set_price);
    optional!(get_quantity, set_quantity);
    optional!(get_stoppx, set_stoppx);
    optional!(get_strikepx, set_strikepx);
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
    changed |= moved(
        this.get_instcode().map(Str::new),
        stated(
            this.get_instcode().map(Str::new),
            other.get_instcode().map(Str::new),
            later,
        ),
        |code| this.set_instcode(code, true),
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
        changed |= yield_lower_isin(this, other);
        // Every key either statement holds, the leading one's value where
        // both hold one of a rank; a lower-ranked ISIN never replaces a
        // higher one, whichever leads ([`Identifiers::merge`]).
        let mut ids = this.get_securityids().clone();
        if ids.merge(other.get_securityids(), later) {
            changed |= this.set_securityids(ids, true).is_ok();
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
    chain_operation(this, previous, true)
}

fn restate_operation<E: Operation + ?Sized>(this: &mut E, live: &E) -> bool {
    chain_operation(this, live, false)
}

/// What an operation's chain is about, taken from another statement of it
/// where this one says nothing of it: what was ordered, then what has
/// traded and at what average - so an acknowledgement stating neither ends
/// with what its chain filled, and a cancel cancels the rest and no more -
/// how long it stands, whether it trades, its identifiers with the parents
/// its chain gave them, and its parties. No fill is carried and none is
/// invented: a rise in what traded is no `lastqty`.
///
/// A `step` - the statement before this one, rather than another statement
/// of this one - says what had traded before this statement: its
/// cumulative fill is this one's only where this one reports no fill of its
/// own, and its average only where this one's cumulative fill is the
/// chain's, so neither is ever a stale value standing for an unknown one.
fn chain_operation<E: Operation + ?Sized>(this: &mut E, previous: &E, step: bool) -> bool {
    let mut changed = moved(
        this.get_ordqty(),
        stated(this.get_ordqty(), previous.get_ordqty(), false),
        |qty| this.set_ordqty(qty, true),
    );
    if !step || this.get_lastqty().is_none_or(Decimal::is_zero) {
        changed |= moved(
            this.get_cumqty(),
            stated(this.get_cumqty(), previous.get_cumqty(), false),
            |qty| this.set_cumqty(qty, true),
        );
    }
    if !step || this.get_cumqty() == previous.get_cumqty() {
        changed |= moved(
            this.get_avgpx(),
            stated(this.get_avgpx(), previous.get_avgpx(), false),
            |px| this.set_avgpx(px, true),
        );
    }
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
    // Every key either statement holds, the leading one's value where both
    // hold one.
    let mut identifiers = this.get_identifiers().clone();
    if identifiers.merge(other.get_identifiers(), later)
        && this.set_identifiers(identifiers, true).is_ok()
    {
        changed = true;
    }
    let mut partyids = this.get_partyids().clone();
    if partyids.merge(other.get_partyids(), later) && this.set_partyids(partyids, true).is_ok() {
        changed = true;
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
