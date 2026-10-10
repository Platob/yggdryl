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
//! (`XXX`), no unit, a side of `UKNW`, no identifiers, a `UNKNOWN`
//! state at the epoch, and the nil identity until [`Element::finalize`]
//! derives one from the facts.
//!
//! Each holder also carries the [`MarketDataKind`] of the leaf that holds
//! it, stamped by that leaf - `UKNW` until one does. The kind is what
//! decides whether the cross code carries the side ([`MarketDataKind::is_sided`]):
//! only an order's and an execution's does - a quote holds both its legs
//! and states its side as a tag - and so whether a side moved withdraws the
//! leg it left; and whether the element reports fills - an execution, a
//! trade - so that its quantity is its own rather than what an order has
//! left. It is never digested: a leaf feeds its own kind where its identity
//! needs one.

use smol_str::SmolStr;

use super::market::{FxRates, Metadata, empty_fxrates, empty_metadata, restating_operation};
use super::{Market, Operation};
use crate::{
    IdKey, IdSource, IdType, Identifier, Identifiers, MarketDataKind, MarketDataType, Side,
    TimeInForce,
};
use yggdryl::graph::{Element, Event};
use yggdryl::{Ccy, Cfi, Decimal, Mic, Result, State, Str, Unit, Uuid};

/// Every fact [`Element`] and [`Market`] name, as plain fields, with no
/// instant: what an undated entry is.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MarketFacts {
    /// The category of the leaf holding these facts: whether the cross code
    /// is stored under the side.
    kind: MarketDataKind,
    /// The type of its kind the element is: its order, quote, trade or book
    /// entry type.
    mdtype: MarketDataType,
    /// Where the state of the event holding these facts leaves an order's
    /// quantities, stamped by that event: what the ordered, filled and left
    /// quantities imply of one another.
    standing: Standing,
    uuid: Uuid,
    crossuuid: Uuid,
    crosscode: String,
    hashcode: u64,
    crosshashcode: u64,
    srcuuids: Vec<Uuid>,
    price: Option<Decimal>,
    currency: Ccy,
    quantity: Option<Decimal>,
    unit: Unit,
    side: Side,
    securityids: Identifiers,
    cficode: Option<Cfi>,
    miccode: Option<Mic>,
    execunix: Option<i64>,
    lastpx: Option<Decimal>,
    lastqty: Option<Decimal>,
    avgpx: Option<Decimal>,
    cumqty: Option<Decimal>,
    leavesqty: Option<Decimal>,
    /// The stop price, the shown and hidden parts of the quantity, what was
    /// canceled, an option's strike and the currency the instrument
    /// originates in, boxed: most elements state none of them and pay one
    /// pointer.
    terms: Option<Box<Terms>>,
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
    /// The resolved instrument's cross code: inline to twenty-three bytes,
    /// one shared `Arc<str>` beyond, so a copy along a chain is a byte
    /// copy or a reference count.
    instcode: Option<Str>,
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
    /// The price and the quantity of the leg `side` takes: the bid's for a
    /// buyer, the ask's for a seller, none for a side taking neither.
    fn leg(&self, side: Side) -> (Option<Decimal>, Option<Decimal>) {
        if side.is_bid() {
            (self.bidpx, self.bidqty)
        } else if side.is_ask() {
            (self.askpx, self.askqty)
        } else {
            (None, None)
        }
    }

    /// The price `side` quotes: the bid's for a buyer, the ask's for a
    /// seller, none for a side taking neither.
    fn px_mut(&mut self, side: Side) -> Option<&mut Option<Decimal>> {
        if side.is_bid() {
            Some(&mut self.bidpx)
        } else if side.is_ask() {
            Some(&mut self.askpx)
        } else {
            None
        }
    }

    /// The quantity `side` quotes, as [`Self::px_mut`] picks it.
    fn qty_mut(&mut self, side: Side) -> Option<&mut Option<Decimal>> {
        if side.is_bid() {
            Some(&mut self.bidqty)
        } else if side.is_ask() {
            Some(&mut self.askqty)
        } else {
            None
        }
    }

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

/// The order terms a market element may state beside its price and
/// quantity, the strike of the option it is about and the currency the
/// instrument originates in, held together because most elements state
/// none of them.
#[derive(Clone, Debug, Default, PartialEq)]
struct Terms {
    stoppx: Option<Decimal>,
    strikepx: Option<Decimal>,
    displayqty: Option<Decimal>,
    hiddenqty: Option<Decimal>,
    cxlqty: Option<Decimal>,
    /// An operation's ordered quantity: held here, beside the terms, so an
    /// element stating none pays nothing for it.
    ordqty: Option<Decimal>,
    /// The currency the instrument originates in, held only where stated
    /// or filled - never [`Ccy::none`], which is none held.
    origccy: Option<Ccy>,
}

impl Terms {
    /// Whether every term is unstated, which is when the holder drops it.
    fn is_empty(&self) -> bool {
        self.stoppx.is_none()
            && self.strikepx.is_none()
            && self.displayqty.is_none()
            && self.hiddenqty.is_none()
            && self.cxlqty.is_none()
            && self.ordqty.is_none()
            && self.origccy.is_none()
    }
}

/// Where a state leaves an order's quantities, one byte: unstated, where no
/// state says whether the order still works; still working - fresh where
/// nothing can have traded yet, asked for or acknowledged - all filled, or
/// no longer active with nothing left - ended by someone or by the clock,
/// the rest of what was ordered canceled, or ended any other way.
///
/// A stamp of the state the event holding the facts states, never content:
/// two holders equal in every fact are equal whatever their stamps say, as
/// they are whichever identifiers they hold by derivation.
#[derive(Clone, Copy, Debug, Default, Eq)]
#[repr(u8)]
enum Standing {
    #[default]
    Unstated,
    Working,
    Fresh,
    Filled,
    Ended,
    Canceled,
}

impl PartialEq for Standing {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Standing {
    /// The standing a state leaves an order's quantities in, read off the
    /// state's own bands as FIX reads `OrdStatus(39)`: `LeavesQty(151)` is
    /// `OrderQty(38)` less `CumQty(14)` while the order is live
    /// ([`State::is_live`]) and nothing once it ended. Filled; ended by
    /// someone or by the clock - the cancellation band, done for the day,
    /// expired; ended any other way; asked for or acknowledged and not yet
    /// working - the pending band and rank `20`; any other live state
    /// working. [`State::Unknown`] says none of it: an order whose state is
    /// unstated implies nothing of its quantities until one is, so stating
    /// the state after them lands what stating it first does.
    fn of(state: State) -> Self {
        if state == State::Unknown {
            Self::Unstated
        } else if state == State::Filled {
            Self::Filled
        } else if state.is_cancelled() || matches!(state, State::DoneForDay | State::Expired) {
            Self::Canceled
        } else if !state.is_live() {
            Self::Ended
        } else if state.is_pending() || state.rank() == 20 {
            Self::Fresh
        } else {
            Self::Working
        }
    }
}

/// Whether an element of `kind` reports fills - an execution, a trade or a
/// batch of either: its state says what it reports, never where an order it
/// fills stands, so its order quantities imply one another as a working
/// order's do, and its quantity is the fill's own, never what the order has
/// left. The one reading the standing, the quantity following what is left
/// and the settle of an order's quantities share.
pub(super) fn reports_fills(kind: MarketDataKind) -> bool {
    matches!(
        kind,
        MarketDataKind::Execution
            | MarketDataKind::ExecutionBatch
            | MarketDataKind::Trade
            | MarketDataKind::TradeBatch
    )
}

/// The fact a setter states, which nothing it implies writes back: a
/// statement - a clear included - stands over every derivation, so a fact
/// stated as none stays none whatever the others imply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fact {
    Quantity,
    Displayqty,
    Hiddenqty,
    Ordqty,
    Cumqty,
    Leavesqty,
    Cxlqty,
    Lastpx,
    Lastqty,
    Avgpx,
    Spotrate,
    Forwardpoints,
}

impl Default for MarketFacts {
    /// An element stating nothing.
    fn default() -> Self {
        Self {
            kind: MarketDataKind::Unknown,
            mdtype: MarketDataType::Unknown,
            standing: Standing::default(),
            uuid: Uuid::default(),
            crossuuid: Uuid::default(),
            crosscode: String::new(),
            hashcode: 0,
            crosshashcode: 0,
            srcuuids: Vec::new(),
            price: None,
            currency: Ccy::none(),
            quantity: None,
            unit: Unit::none(),
            side: Side::Unknown,
            securityids: Identifiers::new(),
            cficode: None,
            miccode: None,
            execunix: None,
            lastpx: None,
            lastqty: None,
            avgpx: None,
            cumqty: None,
            leavesqty: None,
            terms: None,
            prevpx: None,
            prevqty: None,
            spotrate: None,
            forwardpoints: None,
            fxrates: None,
            bidask: None,
            ticker: None,
            instcode: None,
            metadata: None,
        }
    }
}

impl MarketFacts {
    /// Stores `crosscode` under this element's category and side
    /// ([`Market::stored_crosscode`]): the one place a holder's cross code is
    /// written.
    fn state_crosscode(&mut self, crosscode: String) {
        self.crosscode = match self.stored_crosscode(&crosscode) {
            std::borrow::Cow::Borrowed(_) => crosscode,
            std::borrow::Cow::Owned(stored) => stored,
        };
    }

    /// Stores the held cross code under the category and the side again,
    /// and the cross hash and element with it, where the prefix moved: what
    /// a side taken and a kind stamped each run.
    fn reprefix(&mut self) {
        if let std::borrow::Cow::Owned(stored) = self.stored_crosscode(&self.crosscode) {
            self.crosscode = stored;
            self.crosshashcode = yggdryl::implementer::crosshash(&self.crosscode);
            self.crossuuid = self.cross_uuid();
        }
    }

    /// Stamps the category of the leaf that holds these facts, storing the
    /// cross code under it.
    pub(crate) fn set_marketdatakind(&mut self, kind: MarketDataKind) {
        let before = self.kind;
        self.kind = kind;
        self.reprefix();
        // The quantity by the kind's rule: an execution's or a trade's is
        // the fill it reports, every other kind's what is still available,
        // so a holder refiled from one rule to the other moves it.
        if reports_fills(before) != reports_fills(kind) {
            let (was, now) = if reports_fills(kind) {
                (self.leavesqty, self.lastqty)
            } else {
                (self.lastqty, self.leavesqty)
            };
            let held = self.quantity;
            if follow(&mut self.quantity, was.as_ref(), now.as_ref()) {
                self.quantity_moved(held, None);
            }
        }
    }

    /// Writes one bid or ask fact, allocating the holder on the first stated
    /// one and dropping it when the last is cleared.
    fn set_bidask(&mut self, write: impl FnOnce(&mut BidAsk)) {
        if let Some(held) = self.bidask.as_deref_mut() {
            write(held);
            if held.is_empty() {
                self.bidask = None;
            }
            return;
        }
        let mut held = BidAsk::default();
        write(&mut held);
        if !held.is_empty() {
            self.bidask = Some(Box::new(held));
        }
    }

    /// The bid and ask facts, or none.
    fn bidask(&self) -> Option<&BidAsk> {
        self.bidask.as_deref()
    }

    /// Writes one order term, allocating the holder on the first stated
    /// one and dropping it when the last is cleared.
    fn set_terms(&mut self, write: impl FnOnce(&mut Terms)) {
        if let Some(held) = self.terms.as_deref_mut() {
            write(held);
            if held.is_empty() {
                self.terms = None;
            }
            return;
        }
        let mut held = Terms::default();
        write(&mut held);
        if !held.is_empty() {
            self.terms = Some(Box::new(held));
        }
    }

    /// The order terms, or none.
    fn terms(&self) -> Option<&Terms> {
        self.terms.as_deref()
    }
}

impl Element for MarketFacts {
    fn get_uuid(&self) -> Uuid {
        self.uuid
    }

    fn set_uuid(&mut self, uuid: Uuid) {
        self.uuid = uuid;
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

    fn get_srcuuids(&self) -> &[Uuid] {
        &self.srcuuids
    }

    fn set_srcuuids(&mut self, mut sources: Vec<Uuid>) {
        yggdryl::implementer::canonicalize_uuids(&mut sources);
        self.srcuuids = sources;
    }

    /// An element with no instant and no predecessor states no order.
    fn is_after(&self, _: &Self) -> bool {
        false
    }

    fn finalize(&mut self) {
        self.fill_market();
        self.sync_cross();
        self.hashcode = self.digest_market().as_u64();
        self.uuid = Uuid::from_v8(u128::from(self.hashcode));
        self.crossuuid = self.cross_uuid();
    }

    fn with_previous(mut self, previous: &Self) -> Option<Self> {
        if previous.uuid == self.uuid {
            return None;
        }
        let mut changed = yggdryl::implementer::follow_element(&mut self, previous);
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

/// Whether a value lands on a fact holding `held`: always under
/// `overwrite`, else only on an `unstated` fact - and never when the two are
/// already equal, so a setter that changes nothing redirects nothing.
fn lands<T: PartialEq>(held: &T, value: &T, unstated: bool, overwrite: bool) -> bool {
    held != value && (overwrite || unstated)
}

/// Fills `target` with `value` where the element states nothing under it;
/// whether it filled. What a derived fact states back about its source - a
/// buyer's bid about its price - only ever fills: a source stated on its
/// own stands.
fn fill<T: Clone>(target: &mut Option<T>, value: Option<&T>) -> bool {
    match (target.is_none(), value) {
        (true, Some(value)) => {
            *target = Some(value.clone());
            true
        }
        _ => false,
    }
}

/// Moves `target` along with a source that went from `before` to `after`:
/// a target the element states nothing under, or one still holding what
/// the source was, follows it; one stated apart from it stands. Whether it
/// moved. What a source implies is kept in step through this, so a price
/// overwritten moves the bid it quoted rather than leaving it stale.
fn follow<T: PartialEq + Clone>(
    target: &mut Option<T>,
    before: Option<&T>,
    after: Option<&T>,
) -> bool {
    if (target.is_none() || target.as_ref() == before) && target.as_ref() != after {
        *target = after.cloned();
        return true;
    }
    false
}

/// The part of `quantity` an iceberg keeps back when it shows `display`:
/// the difference where it is positive, else none.
fn hidden_of(quantity: Option<Decimal>, display: Option<Decimal>) -> Option<Decimal> {
    quantity
        .zip(display)
        .and_then(|(total, shown)| total.checked_sub(shown))
        .filter(|held| held.is_positive())
}

/// A currency as a fact: [`Ccy::none`] is none.
fn stated_ccy(ccy: &Ccy) -> Option<&Ccy> {
    (!ccy.is_none()).then_some(ccy)
}

/// The origin a holder holding none answers: one shared [`Ccy::none`].
static NO_ORIGCCY: std::sync::LazyLock<Ccy> = std::sync::LazyLock::new(Ccy::none);

/// Every redirection is written directly on the fields - never through a
/// setter - so one change runs once and a chain of them cannot loop.
impl MarketFacts {
    /// The price moved from `before`: the side the element takes quotes
    /// it, so the bid of a buyer and the ask of a seller move with it.
    fn price_moved(&mut self, before: Option<Decimal>) {
        let (after, side) = (self.price, self.side);
        if side.is_bid() || side.is_ask() {
            self.set_bidask(|held| {
                if let Some(slot) = held.px_mut(side) {
                    follow(slot, before.as_ref(), after.as_ref());
                }
            });
            self.quote_currency();
        }
    }

    /// The quantity moved from `before`: the side's bid or ask quantity
    /// moves with it, and so does the iceberg it is the whole of.
    fn quantity_moved(&mut self, before: Option<Decimal>, stated: Option<Fact>) {
        let (after, side) = (self.quantity, self.side);
        if side.is_bid() || side.is_ask() {
            self.set_bidask(|held| {
                if let Some(slot) = held.qty_mut(side) {
                    follow(slot, before.as_ref(), after.as_ref());
                }
            });
            self.quote_currency();
        }
        let display = self.get_displayqty();
        self.iceberg_moved(hidden_of(before, display), stated);
    }

    /// One of an iceberg's three quantities moved - the whole, the part it
    /// shows, the part it keeps back - the hidden part having been `before`:
    /// the hidden part follows the whole past the shown one, then any two
    /// fill the third - the shown part the whole less the hidden one, the
    /// whole the two parts together - never the one a setter stated.
    fn iceberg_moved(&mut self, before: Option<Decimal>, stated: Option<Fact>) {
        if stated != Some(Fact::Hiddenqty) {
            let after = hidden_of(self.quantity, self.get_displayqty());
            if before.is_some() || after.is_some() {
                self.set_terms(|held| {
                    follow(&mut held.hiddenqty, before.as_ref(), after.as_ref());
                });
            }
        }
        match (self.quantity, self.get_displayqty(), self.get_hiddenqty()) {
            (Some(total), None, Some(kept)) if stated != Some(Fact::Displayqty) => {
                if let Some(shown) = total.checked_sub(kept).filter(|held| !held.is_negative()) {
                    self.set_terms(|held| held.displayqty = Some(shown));
                }
            }
            (None, Some(shown), Some(kept)) if stated != Some(Fact::Quantity) => {
                if let Some(total) = shown.checked_add(kept) {
                    self.quantity = Some(total);
                    self.quantity_moved(None, stated);
                }
            }
            _ => {}
        }
    }

    /// The currency moved from `before`: a bid or an ask the element states
    /// in it, or in none, moves with it.
    fn currency_moved(&mut self, before: &Ccy) {
        let before = stated_ccy(before).cloned();
        let after = stated_ccy(&self.currency).cloned();
        if self.bidask.is_none() {
            return;
        }
        self.set_bidask(|held| {
            if held.bidpx.is_some() || held.bidqty.is_some() {
                follow(&mut held.bidccy, before.as_ref(), after.as_ref());
            }
            if held.askpx.is_some() || held.askqty.is_some() {
                follow(&mut held.askccy, before.as_ref(), after.as_ref());
            }
        });
    }

    /// Fills the currency of a bid or an ask the element states in none of
    /// its own with the element's currency.
    fn quote_currency(&mut self) {
        let Some(currency) = stated_ccy(&self.currency).cloned() else {
            return;
        };
        if self.bidask.is_none() {
            return;
        }
        self.set_bidask(|held| {
            if held.bidpx.is_some() || held.bidqty.is_some() {
                fill(&mut held.bidccy, Some(&currency));
            }
            if held.askpx.is_some() || held.askqty.is_some() {
                fill(&mut held.askccy, Some(&currency));
            }
        });
    }

    /// The side moved from `before`, and a sided kind's cross code with it.
    /// A sided element - an order, an execution - takes one side, so the
    /// side it left withdraws what it quoted ([`Self::side_left`]); any other
    /// element states its side as a tag over the legs it holds, so moving
    /// the tag withdraws no leg ([`Self::tag_moved`]). Then the side it
    /// takes quotes the price and quantity where it states none, and they
    /// fill from what that side quotes. A book's side moves nothing but its
    /// cross code's prefix: its legs are its sides' best levels, which the
    /// book settles.
    fn side_moved(&mut self, before: Side) {
        // A book's legs are its sides' best levels, which the book settles:
        // its side, `BOTH`, quotes and moves nothing. A book message's entry
        // passes through this kind before it is refiled as the quote it is,
        // and the side it tags quotes its price onto that leg as any tag
        // does.
        if self.kind == MarketDataKind::Book && self.side == Side::Both {
            self.reprefix();
            return;
        }
        if self.kind.is_sided() {
            self.side_left(before);
        } else {
            self.tag_moved(before);
        }
        let (side, price, quantity) = (self.side, self.price, self.quantity);
        if side.is_bid() || side.is_ask() {
            let mut quoted = (None, None);
            self.set_bidask(|held| {
                if let Some(slot) = held.px_mut(side) {
                    fill(slot, price.as_ref());
                    quoted.0 = *slot;
                }
                if let Some(slot) = held.qty_mut(side) {
                    fill(slot, quantity.as_ref());
                    quoted.1 = *slot;
                }
            });
            fill(&mut self.price, quoted.0.as_ref());
            if fill(&mut self.quantity, quoted.1.as_ref()) {
                self.iceberg_moved(None, None);
            }
            self.quote_currency();
        }
        self.reprefix();
    }

    /// A sided element left `before`: that side no longer quotes its price
    /// and quantity, and a leg left with neither states no currency.
    fn side_left(&mut self, before: Side) {
        let (price, quantity) = (self.price, self.quantity);
        let currency = stated_ccy(&self.currency).cloned();
        if self.bidask.is_some() && (before.is_bid() || before.is_ask()) {
            self.set_bidask(|held| {
                if let Some(slot) = held.px_mut(before) {
                    follow(slot, price.as_ref(), None);
                }
                if let Some(slot) = held.qty_mut(before) {
                    follow(slot, quantity.as_ref(), None);
                }
                // A quote left with neither a price nor a quantity states
                // no currency either.
                if held.bidpx.is_none() && held.bidqty.is_none() {
                    follow(&mut held.bidccy, currency.as_ref(), None);
                }
                if held.askpx.is_none() && held.askqty.is_none() {
                    follow(&mut held.askccy, currency.as_ref(), None);
                }
            });
        }
    }

    /// An unsided element's tag moved from `before`: a quote holds its bid
    /// and its ask whatever side it tags, so no leg is withdrawn, and the
    /// price and quantity it read off the leg the old tag took move to the
    /// leg the new one takes - none where it takes neither or that leg
    /// states nothing. A price or quantity stated apart from the old leg
    /// stands.
    fn tag_moved(&mut self, before: Side) {
        if !(before.is_bid() || before.is_ask()) {
            return;
        }
        let ((left_px, left_qty), (taken_px, taken_qty)) =
            self.bidask().map_or(((None, None), (None, None)), |held| {
                (held.leg(before), held.leg(self.side))
            });
        follow(&mut self.price, left_px.as_ref(), taken_px.as_ref());
        let hidden = hidden_of(self.quantity, self.get_displayqty());
        if follow(&mut self.quantity, left_qty.as_ref(), taken_qty.as_ref()) {
            self.iceberg_moved(hidden, None);
        }
    }

    /// A bid or ask price is stated: where the element takes that side, its
    /// own price fills from it.
    fn quoted_px_moved(&mut self, bid: bool) {
        let quoted = if bid {
            self.get_bidpx()
        } else {
            self.get_askpx()
        };
        if (bid && self.side.is_bid()) || (!bid && self.side.is_ask()) {
            fill(&mut self.price, quoted.as_ref());
        }
        self.quote_currency();
    }

    /// A bid or ask quantity is stated: where the element takes that side,
    /// its own quantity fills from it.
    fn quoted_qty_moved(&mut self, bid: bool) {
        let quoted = if bid {
            self.get_bidqty()
        } else {
            self.get_askqty()
        };
        if ((bid && self.side.is_bid()) || (!bid && self.side.is_ask()))
            && fill(&mut self.quantity, quoted.as_ref())
        {
            self.iceberg_moved(None, None);
        }
        self.quote_currency();
    }

    /// Stamps the standing a state leaves the order's quantities in, and
    /// fills what it implies.
    pub(crate) fn set_standing(&mut self, state: State) {
        let standing = Standing::of(state);
        if self.standing as u8 != standing as u8 {
            self.standing = standing;
            self.orders_moved(None);
        }
    }

    /// The ordered quantity, where an operation states one.
    pub(crate) fn ordqty(&self) -> Option<Decimal> {
        self.terms().and_then(|held| held.ordqty)
    }

    /// Sets the ordered quantity, filling or overwriting as a [`Market`]
    /// setter does, and fills what it implies.
    pub(crate) fn set_ordqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        let before = self.ordqty();
        if lands(&before, &qty, before.is_none(), overwrite) {
            self.set_terms(|held| held.ordqty = qty);
            self.orders_moved(Some(Fact::Ordqty));
        }
    }

    /// An order's quantities moved - what it ordered, filled, canceled and
    /// has left, its last fill or the standing its state leaves it in: each
    /// fills what the others imply, never over the fact a setter `stated`.
    ///
    /// Working, `LeavesQty` is `OrderQty` less `CumQty`, so any two of the
    /// three fill the third, and fresh - asked for or acknowledged - all it
    /// ordered is left. Filled, nothing is left and all it ordered traded.
    /// Ended, nothing is left and what it ordered is what traded plus what
    /// was canceled - and where someone or the clock ended it - canceled,
    /// done for the day, expired - the rest of what was ordered is what was
    /// canceled. One fill is its own average: where all that traded is the
    /// last fill, the average price is the last one.
    fn orders_moved(&mut self, stated: Option<Fact>) {
        let left = self.leavesqty;
        self.orders_filled(stated);
        self.leaves_moved(left);
    }

    /// Fills what the order's quantities imply, then the quantity from what
    /// is left where the element states none: what a reader stating each
    /// fact in turn runs once they are all stated.
    pub(crate) fn settle_orders(&mut self) {
        self.orders_moved(None);
        let own = if reports_fills(self.kind) {
            self.lastqty
        } else {
            self.leavesqty
        };
        if fill(&mut self.quantity, own.as_ref()) {
            self.quantity_moved(None, None);
        }
    }

    /// What is left moved from `before`: the quantity an order is about is
    /// what it still has open, so it moves with it - never an execution's
    /// or a trade's, whose quantity is the fill it reports.
    fn leaves_moved(&mut self, before: Option<Decimal>) {
        let after = self.leavesqty;
        if before != after && !reports_fills(self.kind) {
            let was = self.quantity;
            if follow(&mut self.quantity, before.as_ref(), after.as_ref()) {
                self.quantity_moved(was, None);
            }
        }
    }

    /// The last fill moved from `before`: the quantity an execution or a
    /// trade is about is what executed, so it moves with it - never an
    /// order's, whose quantity is what it has left.
    fn fill_moved(&mut self, before: Option<Decimal>) {
        let after = self.lastqty;
        if before != after && reports_fills(self.kind) {
            let was = self.quantity;
            if follow(&mut self.quantity, before.as_ref(), after.as_ref()) {
                self.quantity_moved(was, None);
            }
        }
    }

    /// The fills [`Self::orders_moved`] makes among the order's quantities,
    /// never writing the fact a setter `stated`.
    fn orders_filled(&mut self, stated: Option<Fact>) {
        let free = |fact: Fact| stated != Some(fact);
        let (ordered, filled) = (self.ordqty(), self.cumqty);
        // An execution's or a trade's state says what it reports - `FILLED`
        // is the fill it is - never where an order it fills stands.
        let standing = if reports_fills(self.kind) {
            Standing::Working
        } else {
            self.standing
        };
        let less = |whole: Option<Decimal>, part: Option<Decimal>| {
            whole
                .zip(part)
                .and_then(|(whole, part)| whole.checked_sub(part))
                .filter(|held| !held.is_negative())
        };
        match standing {
            Standing::Unstated => {}
            // Nothing traded yet and nothing said to be left: all of what
            // was ordered is left.
            Standing::Fresh if filled.is_none() && self.leavesqty.is_none() => {
                if free(Fact::Leavesqty) {
                    self.leavesqty = ordered;
                }
            }
            Standing::Working | Standing::Fresh => match (ordered, filled, self.leavesqty) {
                // What is left moves off all of what was ordered once
                // something traded.
                (Some(_), Some(_), left)
                    if free(Fact::Leavesqty) && (left.is_none() || left == ordered) =>
                {
                    self.leavesqty = less(ordered, filled);
                }
                // What traded is what was ordered less what is left - on a
                // fresh order only where that is something, since nothing
                // traded yet is stated as none whichever fact came first.
                (Some(_), None, Some(_)) if free(Fact::Cumqty) => {
                    let traded = less(ordered, self.leavesqty);
                    if standing as u8 == Standing::Working as u8
                        || traded.is_some_and(|held| held.is_positive())
                    {
                        self.cumqty = traded;
                    }
                }
                (None, Some(cum), Some(left)) if free(Fact::Ordqty) => {
                    if let Some(total) = cum.checked_add(left) {
                        self.set_terms(|held| held.ordqty = Some(total));
                    }
                }
                // Fresh, nothing traded: what is left is all that was
                // ordered.
                (None, None, Some(left))
                    if standing as u8 == Standing::Fresh as u8 && free(Fact::Ordqty) =>
                {
                    self.set_terms(|held| held.ordqty = Some(left));
                }
                _ => {}
            },
            Standing::Filled | Standing::Ended | Standing::Canceled => {
                // An order that ended has nothing left, whatever else it
                // states; any other element only where it states what it
                // ordered or traded. What was left while it worked - all it
                // ordered, or that less what traded - moves to nothing.
                if free(Fact::Leavesqty)
                    && (self.kind == MarketDataKind::Order || ordered.is_some() || filled.is_some())
                {
                    let zero = Decimal::from_int(0);
                    let was = less(ordered, filled).or(ordered);
                    follow(&mut self.leavesqty, was.as_ref(), Some(&zero));
                }
                if standing as u8 == Standing::Filled as u8 {
                    // All it ordered traded.
                    if free(Fact::Cumqty) {
                        fill(&mut self.cumqty, ordered.as_ref());
                    }
                    if free(Fact::Ordqty) && ordered.is_none() && filled.is_some() {
                        self.set_terms(|held| held.ordqty = filled);
                    }
                } else {
                    // What it ordered is what traded plus what was canceled,
                    // and where someone or the clock ended it, the rest was.
                    let canceled = self.get_cxlqty();
                    if standing as u8 == Standing::Canceled as u8
                        && free(Fact::Cxlqty)
                        && canceled.is_none()
                        && let Some(rest) = less(ordered, filled).filter(|held| held.is_positive())
                    {
                        self.set_terms(|held| held.cxlqty = Some(rest));
                    }
                    let canceled = self.get_cxlqty();
                    if free(Fact::Ordqty)
                        && ordered.is_none()
                        && let Some(total) = filled
                            .zip(canceled)
                            .and_then(|(done, canceled)| done.checked_add(canceled))
                    {
                        self.set_terms(|held| held.ordqty = Some(total));
                    }
                    if free(Fact::Cumqty) && filled.is_none() {
                        self.cumqty = less(ordered, canceled);
                    }
                }
            }
        }
        if free(Fact::Avgpx)
            && self.avgpx.is_none()
            && self.cumqty.is_some_and(|held| held.is_positive())
            && self.cumqty == self.lastqty
        {
            self.avgpx = self.lastpx;
        }
    }

    /// One part of an FX forward price moved: `LastPx` is the spot rate
    /// plus the forward points, so where two of the three are stated the
    /// third fills from them - never the part a setter `stated`. Whether the
    /// last price filled, which the average of one fill then reads.
    fn fx_moved(&mut self, stated: Fact) -> bool {
        match (self.lastpx, self.spotrate, self.forwardpoints) {
            (None, Some(spot), Some(points)) if stated != Fact::Lastpx => {
                self.lastpx = spot.checked_add(points);
                return self.lastpx.is_some();
            }
            (Some(last), None, Some(points)) if stated != Fact::Spotrate => {
                self.spotrate = last.checked_sub(points);
            }
            (Some(last), Some(spot), None) if stated != Fact::Forwardpoints => {
                self.forwardpoints = last.checked_sub(spot);
            }
            _ => {}
        }
        false
    }

    /// The rates held, a map allocated on the first one.
    fn fxrates_mut(&mut self) -> &mut FxRates {
        self.fxrates.get_or_insert_with(Box::default)
    }
}

impl Market for MarketFacts {
    fn get_price(&self) -> Option<Decimal> {
        self.price
    }

    fn set_price(&mut self, price: Option<Decimal>, overwrite: bool) {
        if lands(&self.price, &price, self.price.is_none(), overwrite) {
            let before = std::mem::replace(&mut self.price, price);
            self.price_moved(before);
        }
    }

    fn get_stoppx(&self) -> Option<Decimal> {
        self.terms().and_then(|held| held.stoppx)
    }

    fn set_stoppx(&mut self, value: Option<Decimal>, overwrite: bool) {
        let held = self.get_stoppx();
        if lands(&held, &value, held.is_none(), overwrite) {
            self.set_terms(|held| held.stoppx = value);
        }
    }

    fn get_strikepx(&self) -> Option<Decimal> {
        self.terms().and_then(|held| held.strikepx)
    }

    fn set_strikepx(&mut self, value: Option<Decimal>, overwrite: bool) {
        let held = self.get_strikepx();
        if lands(&held, &value, held.is_none(), overwrite) {
            self.set_terms(|held| held.strikepx = value);
        }
    }

    fn get_currency(&self) -> &Ccy {
        &self.currency
    }

    fn set_currency(&mut self, currency: Ccy, overwrite: bool) {
        let unstated = stated_ccy(&self.currency).is_none();
        if lands(&self.currency, &currency, unstated, overwrite) {
            let before = std::mem::replace(&mut self.currency, currency);
            self.currency_moved(&before);
            self.quote_currency();
        }
    }

    fn get_origccy(&self) -> &Ccy {
        self.terms()
            .and_then(|held| held.origccy.as_ref())
            .unwrap_or(&*NO_ORIGCCY)
    }

    /// Lands as the currency does - a fill only where none is held - and
    /// moves nothing else: the origin implies no other fact.
    fn set_origccy(&mut self, ccy: Ccy, overwrite: bool) {
        let held = self.get_origccy();
        if lands(held, &ccy, held.is_none(), overwrite) {
            let ccy = stated_ccy(&ccy).is_some().then_some(ccy);
            self.set_terms(|held| held.origccy = ccy);
        }
    }

    fn get_quantity(&self) -> Option<Decimal> {
        self.quantity
    }

    fn set_quantity(&mut self, quantity: Option<Decimal>, overwrite: bool) {
        if lands(
            &self.quantity,
            &quantity,
            self.quantity.is_none(),
            overwrite,
        ) {
            let before = std::mem::replace(&mut self.quantity, quantity);
            self.quantity_moved(before, Some(Fact::Quantity));
        }
    }

    fn get_displayqty(&self) -> Option<Decimal> {
        self.terms().and_then(|held| held.displayqty)
    }

    fn set_displayqty(&mut self, value: Option<Decimal>, overwrite: bool) {
        let before = self.get_displayqty();
        if lands(&before, &value, before.is_none(), overwrite) {
            let hidden = hidden_of(self.quantity, before);
            self.set_terms(|held| held.displayqty = value);
            self.iceberg_moved(hidden, Some(Fact::Displayqty));
        }
    }

    fn get_hiddenqty(&self) -> Option<Decimal> {
        self.terms().and_then(|held| held.hiddenqty)
    }

    fn set_hiddenqty(&mut self, value: Option<Decimal>, overwrite: bool) {
        let before = self.get_hiddenqty();
        if lands(&before, &value, before.is_none(), overwrite) {
            self.set_terms(|held| held.hiddenqty = value);
            self.iceberg_moved(before, Some(Fact::Hiddenqty));
        }
    }

    fn get_unit(&self) -> &Unit {
        &self.unit
    }

    fn set_unit(&mut self, unit: Unit, overwrite: bool) {
        let unstated = self.unit.as_str() == Unit::none().as_str();
        if lands(&self.unit, &unit, unstated, overwrite) {
            self.unit = unit;
        }
    }

    fn get_side(&self) -> Side {
        self.side
    }

    /// A side taken quotes the element's price and quantity on that side
    /// and re-prefixes the cross code of a sided kind, so setting the side
    /// after the code converges on the code set after the side.
    fn set_side(&mut self, side: Side, overwrite: bool) {
        if lands(&self.side, &side, self.side == Side::Unknown, overwrite) {
            let before = std::mem::replace(&mut self.side, side);
            self.side_moved(before);
        }
    }

    fn marketdatakind(&self) -> MarketDataKind {
        self.kind
    }

    fn get_marketdatatype(&self) -> MarketDataType {
        self.mdtype
    }

    fn set_marketdatatype(&mut self, mdtype: MarketDataType, overwrite: bool) {
        if lands(
            &self.mdtype,
            &mdtype,
            self.mdtype == MarketDataType::Unknown,
            overwrite,
        ) {
            self.mdtype = mdtype;
        }
    }

    fn get_securityids(&self) -> &Identifiers {
        &self.securityids
    }

    /// Under `overwrite` the set replaces every identifier held, derived
    /// ones included; else it is merged in, filling only the keys held none
    /// of ([`Identifiers::merge`]). Every type it holds is a security type,
    /// or nothing moves.
    fn set_securityids(&mut self, ids: Identifiers, overwrite: bool) -> Result<()> {
        if overwrite {
            self.securityids = ids;
        } else {
            for id in &ids {
                id.kind().check_security()?;
            }
            self.securityids.merge(&ids, false);
        }
        Ok(())
    }

    /// A security identifier through [`Identifiers::insert`], which keeps
    /// the base rule: it fills an absent key and its type's base key, and a
    /// statement takes back a derived one of its type.
    fn insert_securityid(&mut self, id: Identifier) -> Result<bool> {
        id.kind().check_security()?;
        Ok(self.securityids.insert(id))
    }

    fn remove_securityid(&mut self, key: &IdKey) -> Result<bool> {
        if self.securityids.remove(key).is_none() {
            return Ok(false);
        }
        // Every derived identifier hangs on the ISIN.
        if self.securityids.get(&IdType::Isin).is_none() {
            let derived: Vec<IdKey> = self
                .securityids
                .iter()
                .filter(|held| held.src() == &IdSource::Derived)
                .map(|held| held.key().clone())
                .collect();
            for key in derived {
                self.securityids.remove(&key);
            }
        }
        Ok(true)
    }

    fn derive_securityid(&mut self, kind: &IdType, code: &str) -> bool {
        // A derived code its type refuses names nothing.
        kind.check_security().is_ok()
            && Identifier::new(IdKey::new(IdSource::Derived, kind.clone()), code)
                .is_ok_and(|id| self.securityids.insert(id))
    }

    fn get_cficode(&self) -> Option<&Cfi> {
        self.cficode.as_ref()
    }

    /// A market keeps only a detailed classification: a code that says
    /// nothing past its category and group states nothing, so it neither
    /// fills nor clears; none under `overwrite` clears. A detailed code
    /// stated over another describing the same instrument takes what that
    /// one says where it says nothing ([`Cfi::refined`]); over another
    /// instrument's it stands whole.
    fn set_cficode(&mut self, code: Option<Cfi>, overwrite: bool) {
        if code
            .as_ref()
            .is_some_and(|held| !Cfi::is_detailed(held.as_str()))
        {
            return;
        }
        let code = match (code, &self.cficode) {
            (Some(stated), Some(held)) if overwrite => Cfi::refined(stated.as_str(), held.as_str())
                .and_then(|refined| Cfi::new(&refined).ok())
                .or(Some(stated)),
            (code, _) => code,
        };
        if lands(&self.cficode, &code, self.cficode.is_none(), overwrite) {
            self.cficode = code;
        }
    }

    fn get_miccode(&self) -> Option<&Mic> {
        self.miccode.as_ref()
    }

    /// A market stated as none - `None`, or `XXXX` - is unstated, so a
    /// stated market fills over either.
    fn set_miccode(&mut self, code: Option<Mic>, overwrite: bool) {
        let unstated = self.miccode.as_ref().is_none_or(Mic::is_none);
        if lands(&self.miccode, &code, unstated, overwrite) {
            self.miccode = code;
        }
    }

    fn get_execunix(&self) -> Option<i64> {
        self.execunix
    }

    fn set_execunix(&mut self, unix: Option<i64>, overwrite: bool) {
        if lands(&self.execunix, &unix, self.execunix.is_none(), overwrite) {
            self.execunix = unix;
        }
    }

    fn get_lastpx(&self) -> Option<Decimal> {
        self.lastpx
    }

    fn set_lastpx(&mut self, px: Option<Decimal>, overwrite: bool) {
        if lands(&self.lastpx, &px, self.lastpx.is_none(), overwrite) {
            self.lastpx = px;
            self.fx_moved(Fact::Lastpx);
            self.orders_moved(Some(Fact::Lastpx));
        }
    }

    fn get_lastqty(&self) -> Option<Decimal> {
        self.lastqty
    }

    fn set_lastqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        if lands(&self.lastqty, &qty, self.lastqty.is_none(), overwrite) {
            let before = std::mem::replace(&mut self.lastqty, qty);
            self.fill_moved(before);
            self.orders_moved(Some(Fact::Lastqty));
        }
    }

    fn get_avgpx(&self) -> Option<Decimal> {
        self.avgpx
    }

    fn set_avgpx(&mut self, px: Option<Decimal>, overwrite: bool) {
        if lands(&self.avgpx, &px, self.avgpx.is_none(), overwrite) {
            self.avgpx = px;
            self.orders_moved(Some(Fact::Avgpx));
        }
    }

    fn get_cumqty(&self) -> Option<Decimal> {
        self.cumqty
    }

    fn set_cumqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        if lands(&self.cumqty, &qty, self.cumqty.is_none(), overwrite) {
            self.cumqty = qty;
            self.orders_moved(Some(Fact::Cumqty));
        }
    }

    fn get_leavesqty(&self) -> Option<Decimal> {
        self.leavesqty
    }

    /// What is left is what an order is about: its quantity moves with it
    /// ([`Market`]).
    fn set_leavesqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        if lands(&self.leavesqty, &qty, self.leavesqty.is_none(), overwrite) {
            let before = std::mem::replace(&mut self.leavesqty, qty);
            self.orders_filled(Some(Fact::Leavesqty));
            self.leaves_moved(before);
        }
    }

    fn get_cxlqty(&self) -> Option<Decimal> {
        self.terms().and_then(|held| held.cxlqty)
    }

    fn set_cxlqty(&mut self, value: Option<Decimal>, overwrite: bool) {
        let held = self.get_cxlqty();
        if lands(&held, &value, held.is_none(), overwrite) {
            self.set_terms(|held| held.cxlqty = value);
            self.orders_moved(Some(Fact::Cxlqty));
        }
    }

    fn get_prevpx(&self) -> Option<Decimal> {
        self.prevpx
    }

    fn set_prevpx(&mut self, px: Option<Decimal>, overwrite: bool) {
        if lands(&self.prevpx, &px, self.prevpx.is_none(), overwrite) {
            self.prevpx = px;
        }
    }

    fn get_prevqty(&self) -> Option<Decimal> {
        self.prevqty
    }

    fn set_prevqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        if lands(&self.prevqty, &qty, self.prevqty.is_none(), overwrite) {
            self.prevqty = qty;
        }
    }

    fn get_spotrate(&self) -> Option<Decimal> {
        self.spotrate
    }

    fn set_spotrate(&mut self, rate: Option<Decimal>, overwrite: bool) {
        if lands(&self.spotrate, &rate, self.spotrate.is_none(), overwrite) {
            self.spotrate = rate;
            if self.fx_moved(Fact::Spotrate) {
                self.orders_moved(Some(Fact::Spotrate));
            }
        }
    }

    fn get_forwardpoints(&self) -> Option<Decimal> {
        self.forwardpoints
    }

    fn set_forwardpoints(&mut self, points: Option<Decimal>, overwrite: bool) {
        if lands(
            &self.forwardpoints,
            &points,
            self.forwardpoints.is_none(),
            overwrite,
        ) {
            self.forwardpoints = points;
            if self.fx_moved(Fact::Forwardpoints) {
                self.orders_moved(Some(Fact::Forwardpoints));
            }
        }
    }

    fn get_ticker(&self) -> Option<&str> {
        self.ticker.as_deref()
    }

    fn set_ticker(&mut self, ticker: Option<SmolStr>, overwrite: bool) {
        let ticker = ticker.filter(|held| !held.is_empty());
        if lands(&self.ticker, &ticker, self.ticker.is_none(), overwrite) {
            self.ticker = ticker;
        }
    }

    fn get_instcode(&self) -> Option<&str> {
        self.instcode.as_ref().map(Str::as_str)
    }

    fn set_instcode(&mut self, code: Option<Str>, overwrite: bool) {
        let code = code.filter(|held| !held.is_empty());
        if lands(&self.instcode, &code, self.instcode.is_none(), overwrite) {
            self.instcode = code;
        }
    }

    fn get_metadata(&self) -> &Metadata {
        match &self.metadata {
            Some(held) => held,
            None => empty_metadata(),
        }
    }

    /// Under `overwrite` the map replaces what is held; else only its keys
    /// the element holds none under fill.
    fn set_metadata(&mut self, metadata: Option<Metadata>, overwrite: bool) {
        let metadata = metadata.filter(|held| !held.is_empty());
        if overwrite {
            self.metadata = metadata.map(Box::new);
        } else if let Some(stated) = metadata {
            let held = self.metadata.get_or_insert_with(Box::default);
            for (key, value) in stated {
                held.entry(key).or_insert(value);
            }
        }
    }

    fn get_fxrates(&self) -> &FxRates {
        match &self.fxrates {
            Some(held) => held,
            None => empty_fxrates(),
        }
    }

    /// Under `overwrite` the rates replace what is held; else only the
    /// targets the element states no rate for fill.
    fn set_fxrates(&mut self, rates: FxRates, overwrite: bool) {
        if overwrite {
            self.fxrates = (!rates.is_empty()).then(|| Box::new(rates));
        } else if !rates.is_empty() {
            let held = self.fxrates_mut();
            for (target, rate) in rates {
                held.entry(target).or_insert(rate);
            }
        }
    }

    fn get_bidpx(&self) -> Option<Decimal> {
        self.bidask().and_then(|held| held.bidpx)
    }

    fn set_bidpx(&mut self, px: Option<Decimal>, overwrite: bool) {
        let before = self.get_bidpx();
        if lands(&before, &px, before.is_none(), overwrite) {
            self.set_bidask(|held| held.bidpx = px);
            self.quoted_px_moved(true);
        }
    }

    fn get_bidqty(&self) -> Option<Decimal> {
        self.bidask().and_then(|held| held.bidqty)
    }

    fn set_bidqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        let before = self.get_bidqty();
        if lands(&before, &qty, before.is_none(), overwrite) {
            self.set_bidask(|held| held.bidqty = qty);
            self.quoted_qty_moved(true);
        }
    }

    fn get_bidccy(&self) -> Option<&Ccy> {
        self.bidask().and_then(|held| held.bidccy.as_ref())
    }

    fn set_bidccy(&mut self, ccy: Option<Ccy>, overwrite: bool) {
        let held = self.get_bidccy().cloned();
        if lands(&held, &ccy, held.is_none(), overwrite) {
            self.set_bidask(|held| held.bidccy = ccy);
        }
    }

    fn get_askpx(&self) -> Option<Decimal> {
        self.bidask().and_then(|held| held.askpx)
    }

    fn set_askpx(&mut self, px: Option<Decimal>, overwrite: bool) {
        let before = self.get_askpx();
        if lands(&before, &px, before.is_none(), overwrite) {
            self.set_bidask(|held| held.askpx = px);
            self.quoted_px_moved(false);
        }
    }

    fn get_askqty(&self) -> Option<Decimal> {
        self.bidask().and_then(|held| held.askqty)
    }

    fn set_askqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        let before = self.get_askqty();
        if lands(&before, &qty, before.is_none(), overwrite) {
            self.set_bidask(|held| held.askqty = qty);
            self.quoted_qty_moved(false);
        }
    }

    fn get_askccy(&self) -> Option<&Ccy> {
        self.bidask().and_then(|held| held.askccy.as_ref())
    }

    fn set_askccy(&mut self, ccy: Option<Ccy>, overwrite: bool) {
        let held = self.get_askccy().cloned();
        if lands(&held, &ccy, held.is_none(), overwrite) {
            self.set_bidask(|held| held.askccy = ccy);
        }
    }
}

/// [`MarketFacts`] with the clocks and the state an event answers: a market
/// event as plain fields.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MarketEventFacts {
    market: MarketFacts,
    transunix: i64,
    state: State,
    seqnum: u64,
    creaunix: Option<i64>,
    sendunix: Option<i64>,
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
            transunix: unix,
            state: State::unknown(),
            seqnum: 0,
            creaunix: None,
            sendunix: None,
            exprunix: None,
            prevunix: None,
            prevuuid: None,
            snapunix: None,
        }
    }

    /// Reprojects the generic event identities after one of their inputs
    /// changes. A UUIDv7 refusal retains the element's `uuid`, as
    /// [`Event::finalized`] does; the cross identity always follows the
    /// resulting `uuid` and cross hash.
    fn refresh_uuids(&mut self) {
        if let Ok(uuid) = self.time_uuid() {
            self.market.uuid = uuid;
        }
        self.market.crossuuid = self.cross_uuid();
    }
}

impl MarketEventFacts {
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
    fn get_uuid(&self) -> Uuid {
        self.market.uuid
    }

    fn set_uuid(&mut self, uuid: Uuid) {
        self.market.uuid = uuid;
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
            yggdryl::implementer::crosshash(&self.market.crosscode)
        };
        self.refresh_uuids();
    }

    fn get_hashcode(&self) -> u64 {
        self.market.hashcode
    }

    fn set_hashcode(&mut self, hashcode: u64) {
        self.market.hashcode = hashcode;
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
        yggdryl::implementer::canonicalize_uuids(&mut sources);
        self.market.srcuuids = sources;
    }

    fn is_after(&self, other: &Self) -> bool {
        self.transunix > other.transunix
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
    fn get_transunix(&self) -> i64 {
        self.transunix
    }

    fn set_transunix(&mut self, unix: i64) {
        self.transunix = unix;
        self.refresh_uuids();
    }

    fn get_state(&self) -> &State {
        &self.state
    }

    /// The state stamps the standing it leaves the order's quantities in.
    fn set_state(&mut self, state: State) {
        self.market.set_standing(state);
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

    fn get_sendunix(&self) -> Option<i64> {
        self.sendunix
    }

    fn set_sendunix(&mut self, unix: Option<i64>) {
        self.sendunix = unix;
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
        self.market.hashcode = hashcode;
        self.refresh_uuids();
    }
}

delegate_market!(MarketEventFacts, market);

/// Four of the five facts an operation adds to the market's, as plain
/// fields; the ordered quantity rides the market facts' boxed [`Terms`],
/// beside the quantities it implies.
#[derive(Clone, Debug, Default, PartialEq)]
struct OperationExtras {
    timeinforce: Option<TimeInForce>,
    tradable: Option<bool>,
    identifiers: Identifiers,
    partyids: Identifiers,
}

impl OperationExtras {
    /// Each base a parent identifier names and the operation does not
    /// state, filled from it once an element finalizes, after every
    /// enrichment ([`Identifiers::fill_parents`]).
    fn fill_parents(&mut self) {
        self.identifiers.fill_parents(IdType::parent_of);
    }
}

/// `impl Operation` over an [`OperationExtras`] field.
macro_rules! operation_extras {
    ($type:ty, $($field:ident).+; $($market:ident).+) => {
        impl Operation for $type {
            fn get_ordqty(&self) -> Option<Decimal> {
                self.$($market).+.ordqty()
            }
            fn set_ordqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
                self.$($market).+.set_ordqty(qty, overwrite);
            }
            fn get_timeinforce(&self) -> Option<&TimeInForce> {
                self.$($field).+.timeinforce.as_ref()
            }
            fn set_timeinforce(&mut self, tif: Option<TimeInForce>, overwrite: bool) {
                let held = &mut self.$($field).+.timeinforce;
                if lands(held, &tif, held.is_none(), overwrite) {
                    *held = tif;
                }
            }
            fn get_tradable(&self) -> Option<bool> {
                self.$($field).+.tradable
            }
            fn set_tradable(&mut self, tradable: Option<bool>, overwrite: bool) {
                let held = &mut self.$($field).+.tradable;
                if lands(held, &tradable, held.is_none(), overwrite) {
                    *held = tradable;
                }
            }
            fn get_identifiers(&self) -> &Identifiers {
                &self.$($field).+.identifiers
            }
            fn set_identifiers(&mut self, ids: Identifiers, overwrite: bool) -> Result<()> {
                if overwrite {
                    self.$($field).+.identifiers = ids;
                } else {
                    self.$($field).+.identifiers.merge(&ids, false);
                }
                Ok(())
            }
            fn insert_identifier(&mut self, id: Identifier) -> Result<bool> {
                Ok(self.$($field).+.identifiers.insert(id))
            }
            fn remove_identifier(&mut self, key: &IdKey) -> Result<bool> {
                Ok(self.$($field).+.identifiers.remove(key).is_some())
            }
            fn get_partyids(&self) -> &Identifiers {
                &self.$($field).+.partyids
            }
            fn set_partyids(&mut self, partyids: Identifiers, overwrite: bool) -> Result<()> {
                if overwrite {
                    self.$($field).+.partyids = partyids;
                } else {
                    self.$($field).+.partyids.merge(&partyids, false);
                }
                Ok(())
            }
            fn insert_partyid(&mut self, partyid: Identifier) -> Result<bool> {
                Ok(self.$($field).+.partyids.insert(partyid))
            }
            fn remove_partyid(&mut self, key: &IdKey) -> Result<bool> {
                Ok(self.$($field).+.partyids.remove(key).is_some())
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
    /// Each base a parent identifier names and the operation does not
    /// state, filled from it once it finalizes, after every enrichment.
    pub(crate) fn fill_parents(&mut self) {
        self.operation.fill_parents();
    }

    /// Stamps the category of the leaf that holds these facts.
    pub(crate) fn set_marketdatakind(&mut self, kind: MarketDataKind) {
        self.market.set_marketdatakind(kind);
    }

    /// This entry dated at `unix`, nanoseconds since the Unix epoch: a
    /// move, finalized by the caller once the clocks are in.
    #[must_use]
    pub(crate) fn at(self, unix: i64) -> OperationEventFacts {
        let mut event = MarketEventFacts::from(self.market);
        event.transunix = unix;
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
    fn get_uuid(&self) -> Uuid {
        self.market.uuid
    }

    fn set_uuid(&mut self, uuid: Uuid) {
        self.market.uuid = uuid;
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

    fn get_hashcode(&self) -> u64 {
        self.market.hashcode
    }

    fn set_hashcode(&mut self, hashcode: u64) {
        self.market.hashcode = hashcode;
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
        self.fill_parents();
        self.sync_cross();
        self.market.hashcode = self.digest_operation().as_u64();
        self.market.uuid = Uuid::from_v8(u128::from(self.market.hashcode));
        self.market.crossuuid = self.cross_uuid();
    }

    fn with_previous(mut self, previous: &Self) -> Option<Self> {
        if previous.market.uuid == self.market.uuid {
            return None;
        }
        let mut changed = yggdryl::implementer::follow_element(&mut self, previous);
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
operation_extras!(OperationFacts, operation; market);

/// `MarketEventFacts` with the operation's facts: a dated operation as
/// plain fields, what an order, a quote, an execution and a message hold.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OperationEventFacts {
    event: MarketEventFacts,
    operation: OperationExtras,
}

impl OperationEventFacts {
    /// Each base a parent identifier names and the operation does not
    /// state, filled from it once it finalizes, after every enrichment.
    pub(crate) fn fill_parents(&mut self) {
        self.operation.fill_parents();
    }

    /// An operation that happened at `unix`, nanoseconds since the Unix
    /// epoch, stating nothing else yet.
    #[must_use]
    pub(crate) fn at(unix: i64) -> Self {
        Self {
            event: MarketEventFacts::at(unix),
            operation: OperationExtras::default(),
        }
    }

    /// Stamps the category of the leaf that holds these facts.
    pub fn set_marketdatakind(&mut self, kind: MarketDataKind) {
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
    fn get_uuid(&self) -> Uuid {
        self.event.get_uuid()
    }

    fn set_uuid(&mut self, uuid: Uuid) {
        self.event.set_uuid(uuid);
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

    fn get_hashcode(&self) -> u64 {
        self.event.get_hashcode()
    }

    fn set_hashcode(&mut self, hashcode: u64) {
        self.event.set_hashcode(hashcode);
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
        self.fill_parents();
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
operation_extras!(OperationEventFacts, operation; event.market);

impl OperationEventFacts {
    /// Fills what the order's quantities imply of one another, against the
    /// standing its state leaves them in: what a reader stating each of
    /// them in turn runs once they are all stated.
    pub fn settle_orders(&mut self) {
        self.event.market.settle_orders();
    }

    /// Whether `other` states what this operation states: every fact of the
    /// statement compared field by field - its kind and type, its cross
    /// code, every market and operation fact, its state, when it was
    /// created and when it expires - and nothing about where it stands in
    /// a chain or a stream: its identities and digests, its sources, the
    /// standing its state stamps, when it last executed, the previous price
    /// and quantity its chain gave it, its instant, place, wire clock,
    /// predecessor and snapshot. What a book reads to tell a repeat from a
    /// change, one comparison per field and nothing built.
    ///
    /// Each holder is destructured whole, so a fact added to one does not
    /// compile until it is named here, compared or not.
    pub(crate) fn same_statement(&self, other: &Self) -> bool {
        let Self {
            event:
                MarketEventFacts {
                    market:
                        MarketFacts {
                            kind,
                            mdtype,
                            standing: _,
                            uuid: _,
                            crossuuid: _,
                            crosscode,
                            hashcode: _,
                            crosshashcode: _,
                            srcuuids: _,
                            price,
                            currency,
                            quantity,
                            unit,
                            side,
                            securityids,
                            cficode,
                            miccode,
                            execunix: _,
                            lastpx,
                            lastqty,
                            avgpx,
                            cumqty,
                            leavesqty,
                            terms,
                            prevpx: _,
                            prevqty: _,
                            spotrate,
                            forwardpoints,
                            fxrates,
                            bidask,
                            ticker,
                            // A reading of the facts compared above, as the
                            // identities are: never a statement of its own.
                            instcode: _,
                            metadata,
                        },
                    transunix: _,
                    state,
                    seqnum: _,
                    creaunix,
                    sendunix: _,
                    exprunix,
                    prevunix: _,
                    prevuuid: _,
                    snapunix: _,
                },
            operation:
                OperationExtras {
                    timeinforce,
                    tradable,
                    identifiers,
                    partyids,
                },
        } = self;
        let (market, event, operation) = (&other.event.market, &other.event, &other.operation);
        *kind == market.kind
            && *mdtype == market.mdtype
            && *crosscode == market.crosscode
            && *price == market.price
            && *currency == market.currency
            && *quantity == market.quantity
            && *unit == market.unit
            && *side == market.side
            && *securityids == market.securityids
            && *cficode == market.cficode
            && *miccode == market.miccode
            && *lastpx == market.lastpx
            && *lastqty == market.lastqty
            && *avgpx == market.avgpx
            && *cumqty == market.cumqty
            && *leavesqty == market.leavesqty
            && *terms == market.terms
            && *spotrate == market.spotrate
            && *forwardpoints == market.forwardpoints
            && *fxrates == market.fxrates
            && *bidask == market.bidask
            && *ticker == market.ticker
            && *metadata == market.metadata
            && *state == event.state
            && *creaunix == event.creaunix
            && *exprunix == event.exprunix
            && *timeinforce == operation.timeinforce
            && *tradable == operation.tradable
            && *identifiers == operation.identifiers
            && *partyids == operation.partyids
    }
}

/// The cross code a copy of `other` takes: the source's without the
/// prefix its holder gave it, which the holder of the copy gives again
/// under its own category and side.
fn copied_crosscode<E: Element + Market + ?Sized>(other: &E) -> &str {
    super::market::base_crosscode(other.get_crosscode())
}

/// Brings the copy's cross hash and element in step with the code it took,
/// where its holder stores it under another prefix than the source's;
/// whether it did.
fn resync_copied<T: Element + ?Sized, E: Element + Market + ?Sized>(
    this: &mut T,
    other: &E,
) -> bool {
    let resynced = this.get_crosscode() != other.get_crosscode();
    if resynced {
        this.sync_cross();
    }
    resynced
}

fn copy_element<T: Element + ?Sized, E: Element + Market + ?Sized>(this: &mut T, other: &E) {
    this.set_uuid(other.get_uuid());
    this.set_crossuuid(other.get_crossuuid());
    this.set_crosscode(copied_crosscode(other).to_owned());
    this.set_hashcode(other.get_hashcode());
    this.set_crosshashcode(other.get_crosshashcode());
    this.set_srcuuids(other.get_srcuuids().to_vec());
}

fn copy_event<T: Event + ?Sized, E: Event + ?Sized>(this: &mut T, other: &E) {
    this.set_transunix(other.get_transunix());
    this.set_state(*other.get_state());
    this.set_seqnum(other.get_seqnum());
    this.set_creaunix(other.get_creaunix());
    this.set_sendunix(other.get_sendunix());
    this.set_exprunix(other.get_exprunix());
    this.set_prevunix(other.get_prevunix());
    this.set_prevuuid(other.get_prevuuid());
    this.set_snapunix(other.get_snapunix());
}

/// Copies every market fact `other` states onto `this`, each through its
/// setter. The copy holds `other`'s kind while it is made - each `From`
/// below stamps it first - so what the facts imply of one another under it
/// is what they implied of `other`, and a leaf of another kind restamps its
/// own afterwards.
fn copy_market<T: Market + ?Sized, E: Market + ?Sized>(this: &mut T, other: &E) {
    this.set_price(other.get_price(), true);
    this.set_stoppx(other.get_stoppx(), true);
    this.set_strikepx(other.get_strikepx(), true);
    this.set_currency(other.get_currency().clone(), true);
    this.set_origccy(other.get_origccy().clone(), true);
    this.set_quantity(other.get_quantity(), true);
    this.set_displayqty(other.get_displayqty(), true);
    this.set_hiddenqty(other.get_hiddenqty(), true);
    this.set_unit(other.get_unit().clone(), true);
    this.set_side(other.get_side(), true);
    // A plain holder refuses no identifier; a view of a store may, and a
    // copy takes what it can.
    let _ = this.set_securityids(other.get_securityids().clone(), true);
    this.set_cficode(other.get_cficode().cloned(), true);
    this.set_miccode(other.get_miccode().cloned(), true);
    this.set_execunix(other.get_execunix(), true);
    this.set_lastpx(other.get_lastpx(), true);
    this.set_lastqty(other.get_lastqty(), true);
    this.set_avgpx(other.get_avgpx(), true);
    this.set_cumqty(other.get_cumqty(), true);
    this.set_leavesqty(other.get_leavesqty(), true);
    this.set_cxlqty(other.get_cxlqty(), true);
    this.set_prevpx(other.get_prevpx(), true);
    this.set_prevqty(other.get_prevqty(), true);
    this.set_spotrate(other.get_spotrate(), true);
    this.set_forwardpoints(other.get_forwardpoints(), true);
    this.set_fxrates(other.get_fxrates().clone(), true);
    this.set_bidpx(other.get_bidpx(), true);
    this.set_bidqty(other.get_bidqty(), true);
    this.set_bidccy(other.get_bidccy().cloned(), true);
    this.set_askpx(other.get_askpx(), true);
    this.set_askqty(other.get_askqty(), true);
    this.set_askccy(other.get_askccy().cloned(), true);
    this.set_ticker(other.get_ticker().map(SmolStr::new), true);
    this.set_instcode(other.get_instcode().map(Str::new), true);
    this.set_metadata(Some(other.get_metadata().clone()), true);
}

fn copy_operation<T: Operation + ?Sized, E: Operation + ?Sized>(this: &mut T, other: &E) {
    this.set_ordqty(other.get_ordqty(), true);
    this.set_timeinforce(other.get_timeinforce().cloned(), true);
    this.set_tradable(other.get_tradable(), true);
    let _ = this.set_identifiers(other.get_identifiers().clone(), true);
    let _ = this.set_partyids(other.get_partyids().clone(), true);
}

impl<E: Element + Market + ?Sized> From<&E> for MarketFacts {
    fn from(other: &E) -> Self {
        let mut this = Self::default();
        this.set_marketdatakind(other.marketdatakind());
        copy_element(&mut this, other);
        copy_market(&mut this, other);
        let _ = resync_copied(&mut this, other);
        this
    }
}

impl<E: Event + Market + ?Sized> From<&E> for MarketEventFacts {
    fn from(other: &E) -> Self {
        let mut this = Self::default();
        this.set_marketdatakind(other.marketdatakind());
        copy_element(&mut this, other);
        copy_event(&mut this, other);
        copy_market(&mut this, other);
        // The setters above keep a derived event coherent while it is
        // mutated. Conversion copies the exact identities the source states,
        // including an assigned identity, after every dependency is in place.
        let resynced = resync_copied(&mut this, other);
        this.market.uuid = other.get_uuid();
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
        this.set_marketdatakind(other.marketdatakind());
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
        this.set_marketdatakind(other.marketdatakind());
        copy_element(&mut this, other);
        copy_event(&mut this, other);
        copy_market(&mut this, other);
        copy_operation(&mut this, other);
        let resynced = resync_copied(&mut this, other);
        this.event.market.uuid = other.get_uuid();
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
    //! What `rust/market/tests/graph/facts.rs` pins and a caller cannot reach.
    //!
    //! The four holders are crate-private: a caller reaches their facts
    //! through a leaf that holds one. What each holder answers on its own -
    //! its size, the identity its finalize derives, the order it stands in
    //! and how it merges - is observed here, each answer in role order:
    //! market, market event, operation, operation event.
    use super::{MarketEventFacts, MarketFacts, OperationEventFacts, OperationFacts};
    use yggdryl::Uuid;
    use yggdryl::graph::Element;

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
            (element.get_uuid(), element.get_hashcode())
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
