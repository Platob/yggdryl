//! A walk over timed elements that states each one as the element after the
//! live one it follows.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::iter::FusedIterator;
use std::vec;

use super::Market;
use super::facts::reports_fills;
use super::market::{Accounted, Fills, OwnFills, base_crosscode};
use crate::{IdType, Identifiers};
use crate::{MarketDataKind, Side};
use yggdryl::graph::Element;
use yggdryl::implementer::InstantSequence;
use yggdryl::implementer::warned;
use yggdryl::{State, Str, Uuid};

/// States a market's fill as none - its last price and quantity and the
/// parts of a fill's price - each a statement, which nothing the others
/// imply writes back: what an event the walk makes of a live one - an
/// expiration, a withdrawal from a book - reports of a fill, which is no
/// fill ([`Walked::walked_clear_fill`]).
pub(super) fn clear_fill<E: Market + ?Sized>(market: &mut E) {
    market.set_lastpx(None, true);
    market.set_spotrate(None, true);
    market.set_forwardpoints(None, true);
    market.set_lastqty(None, true);
}

pub(crate) mod sealed {
    use super::super::market::{Accounted, Fills, OwnFills, account_fills};
    use super::super::{MarketData, Operation};
    use super::clear_fill;
    use crate::{IdType, Identifiers};
    use crate::{MarketDataKind, Side};
    use yggdryl::graph::{Element, Event};
    use yggdryl::{Decimal, State};

    /// [`Walked::walked_origin_of`] over one holder's own parentage: the
    /// base `kind` is a parent of, where that base is a chain identity and
    /// `kind` stands last among its parents - the chain's first value.
    fn origin_of<O: Operation + ?Sized>(operation: &O, kind: &IdType) -> Option<IdType> {
        let (base, at) = operation.parent_of(kind)?;
        (base.is_chain_identity() && at + 1 == operation.parents_of(&base).len()).then_some(base)
    }

    /// [`Walked::walked_made`] over an event's own facts.
    fn made<E: Event + ?Sized>(event: &E) -> bool {
        event.get_snapunix().is_some()
            || (*event.get_state() == State::Expired
                && event.get_prevuuid().is_some()
                && event.get_exprunix() == Some(event.get_transunix()))
    }

    /// What a walk needs of an element beyond [`Element`]: sealed, so only
    /// `E: Event + Operation + Clone` and [`MarketData`] can name it. Every
    /// `E: Event + Operation + Clone` answers through its own traits; a
    /// `MarketData` answers through its four operation-event variants and a
    /// FIX message, and is not walked otherwise, so any other variant is
    /// yielded as it came and never enters the live map. The fill ledger
    /// it reads and writes is the walk's own, so the trait names
    /// crate-private types a caller never holds.
    #[allow(private_interfaces)]
    pub trait Walked: Element + Clone {
        /// [`Event::get_transunix`]; `None` for an element that states no
        /// instant, which the walk yields where it reads it.
        fn walked_transunix(&self) -> Option<i64>;
        /// The order a walk opened over unsorted elements sorts them by.
        fn walked_order(&self, other: &Self) -> std::cmp::Ordering;
        /// [`Event::set_transunix`].
        fn walked_set_transunix(&mut self, unix: i64);
        /// [`Event::get_state`].
        fn walked_state(&self) -> Option<&State>;
        /// [`Event::set_state`].
        fn walked_set_state(&mut self, state: State);
        /// [`Event::get_creaunix`].
        fn walked_creaunix(&self) -> Option<i64>;
        /// [`Event::set_creaunix`].
        fn walked_set_creaunix(&mut self, unix: Option<i64>);
        /// [`Event::get_exprunix`].
        fn walked_exprunix(&self) -> Option<i64>;
        /// [`Market::set_execunix`](super::super::Market::set_execunix).
        fn walked_set_execunix(&mut self, unix: Option<i64>);
        /// [`Event::set_sendunix`].
        fn walked_set_sendunix(&mut self, unix: Option<i64>);
        /// [`Event::get_snapunix`].
        fn walked_snapunix(&self) -> Option<i64>;
        /// [`Event::set_snapunix`].
        fn walked_set_snapunix(&mut self, unix: Option<i64>);
        /// [`Event::set_seqnum`], restating only where the place moves.
        fn walked_set_seqnum(&mut self, seqnum: u64);
        /// Whether a walk made the element rather than read it: a grid
        /// view, stating the instant it was read as the view of, or the
        /// expiration a walk emitted - an `EXPIRED` event following the
        /// live one, dated at its own deadline.
        fn walked_made(&self) -> bool;
        /// [`Operation::get_identifiers`]; `None` for an element the walk does
        /// not chain.
        fn walked_identifiers(&self) -> Option<&Identifiers>;
        /// The chain identity ([`IdType::is_chain_identity`]) whose first
        /// value an identifier of `kind` names: the base of a parent standing
        /// last among that base's [`Operation::parents_of`] - FIX's own
        /// lineage fields, `OrigClOrdID(41)`, `OrigTradeID(1126)`,
        /// `TradeReportRefID(572)`, and `origorderid` - as the holder reads
        /// its parentage ([`Operation::parent_of`]); none for any other
        /// type, the previous-value slot `parent{type}` included, which the
        /// walk itself writes from a value the chain holds and a bridge
        /// spells a hierarchy parent by.
        fn walked_origin_of(&self, kind: &IdType) -> Option<IdType>;
        /// [`Operation::follow_identity`]: the live statement's side where
        /// this one states none and its stored cross code, the cross codes
        /// brought in step; whether anything moved.
        fn walked_follow_identity(&mut self, live: &Self) -> bool;
        /// [`Operation::note_conflict`].
        fn walked_note_conflict(&mut self, cited: &str);
        /// [`Event::restating`].
        fn walked_restating(self, live: &Self) -> Self;
        /// [`Event::finalized`] over the code the element already holds:
        /// the identity its instant, place and cross hash derive, stamped
        /// again without digesting what it states. A grid view moves only
        /// instants, which feed no code, off a live element the walk keeps
        /// settled, so its code stands and only the identity moves.
        fn walked_restamp(&mut self);
        /// [`super::super::market::fill_execution`].
        fn walked_fill_execution(&mut self);
        /// The element's own words of its fill, read before anything is
        /// followed ([`OwnFills::read`]); `None` for an element the walk
        /// does not chain.
        fn walked_own_fills(&self) -> Option<OwnFills>;
        /// The fill accounting of this statement against its chain's
        /// ledger ([`account_fills`]); nothing for an element the walk does
        /// not chain.
        fn walked_account_fills(
            &mut self,
            own: &OwnFills,
            kept: Option<Decimal>,
            fills: &mut Fills,
        ) -> Accounted;
        /// [`Market::get_hiddenqty`](super::super::Market::get_hiddenqty).
        fn walked_hiddenqty(&self) -> Option<Decimal>;
        /// Clears the fill the element last reported - its last price, the
        /// two FX parts of it and its last quantity - leaving what it
        /// traded in all: what an expiration, which reports no fill, keeps
        /// of the live element it is made from.
        fn walked_clear_fill(&mut self);
        /// Whether the walk chains this element at all.
        fn is_walked(&self) -> bool;
        /// The side the element's chain is keyed by: the
        /// [`Market::get_side`](super::super::Market::get_side) of a sided
        /// kind ([`MarketDataKind::is_sided`]), whose cross code carries it,
        /// so its base code is what an element stating no side joins it by;
        /// `Side::Unknown` for any other kind - a quote's side is a tag -
        /// and for an element the walk does not chain.
        fn walked_side(&self) -> Side;
        /// The side a name the element goes by is alive on: the one leg
        /// its own statement tags, whatever its kind - a sided chain's side,
        /// a quote's or a book entry's tag, so a bid and an offer going by
        /// one `MDEntryID(278)` are two entries ([`Side::tagged`]) -
        /// `Side::Unknown` where it tags none or both legs ([`Side::Both`]),
        /// and for an element the walk does not chain. A name is matched on
        /// its side whenever both the element and the holder state one.
        fn walked_slot(&self) -> Side;
        /// [`Market::get_instcode`](super::super::Market::get_instcode): the
        /// instrument the element states, which a name is matched on
        /// whenever both the element and the holder state one; `None` for
        /// an element the walk does not chain.
        fn walked_instcode(&self) -> Option<&str>;
        /// [`Market::marketdatakind`](super::super::Market::marketdatakind):
        /// the category a chain holds elements of only.
        fn walked_kind(&self) -> MarketDataKind;
    }

    #[allow(private_interfaces)]
    impl<E: Event + Operation + Clone> Walked for E {
        fn walked_transunix(&self) -> Option<i64> {
            Some(self.get_transunix())
        }
        fn walked_order(&self, other: &Self) -> std::cmp::Ordering {
            super::order(self, other)
        }
        fn walked_set_transunix(&mut self, unix: i64) {
            self.set_transunix(unix);
        }
        fn walked_state(&self) -> Option<&State> {
            Some(self.get_state())
        }
        fn walked_set_state(&mut self, state: State) {
            self.set_state(state);
        }
        fn walked_creaunix(&self) -> Option<i64> {
            self.get_creaunix()
        }
        fn walked_set_creaunix(&mut self, unix: Option<i64>) {
            self.set_creaunix(unix);
        }
        fn walked_exprunix(&self) -> Option<i64> {
            self.get_exprunix()
        }
        fn walked_set_execunix(&mut self, unix: Option<i64>) {
            self.set_execunix(unix, true);
        }
        fn walked_set_sendunix(&mut self, unix: Option<i64>) {
            self.set_sendunix(unix);
        }
        fn walked_snapunix(&self) -> Option<i64> {
            self.get_snapunix()
        }
        fn walked_set_snapunix(&mut self, unix: Option<i64>) {
            self.set_snapunix(unix);
        }
        fn walked_set_seqnum(&mut self, seqnum: u64) {
            if self.get_seqnum() != seqnum {
                self.set_seqnum(seqnum);
            }
        }
        fn walked_made(&self) -> bool {
            made(self)
        }
        fn walked_identifiers(&self) -> Option<&Identifiers> {
            Some(self.get_identifiers())
        }
        fn walked_origin_of(&self, kind: &IdType) -> Option<IdType> {
            origin_of(self, kind)
        }
        fn walked_follow_identity(&mut self, live: &Self) -> bool {
            self.follow_identity(live)
        }
        fn walked_note_conflict(&mut self, cited: &str) {
            self.note_conflict(cited);
        }
        fn walked_restating(self, live: &Self) -> Self {
            self.restating(live)
        }
        fn walked_restamp(&mut self) {
            let code = self.get_hashcode();
            self.finalized(code);
        }
        fn walked_fill_execution(&mut self) {
            let _ = super::super::market::fill_execution(self);
        }
        fn walked_own_fills(&self) -> Option<OwnFills> {
            Some(OwnFills::read(self))
        }
        fn walked_account_fills(
            &mut self,
            own: &OwnFills,
            kept: Option<Decimal>,
            fills: &mut Fills,
        ) -> Accounted {
            account_fills(self, own, kept, fills)
        }
        fn walked_hiddenqty(&self) -> Option<Decimal> {
            self.get_hiddenqty()
        }
        fn walked_clear_fill(&mut self) {
            clear_fill(self);
        }
        fn is_walked(&self) -> bool {
            true
        }
        fn walked_side(&self) -> Side {
            self.marketdatakind().stored_side(self.get_side())
        }
        fn walked_slot(&self) -> Side {
            self.get_side().tagged()
        }
        fn walked_instcode(&self) -> Option<&str> {
            self.get_instcode()
        }
        fn walked_kind(&self) -> MarketDataKind {
            self.marketdatakind()
        }
    }

    #[allow(private_interfaces)]
    impl Walked for MarketData {
        fn walked_transunix(&self) -> Option<i64> {
            self.as_event().map(|event| event.get_transunix())
        }
        /// By instant, an undated value first: a total order, where
        /// [`Element::is_after`] states none between an undated value and
        /// any other.
        fn walked_order(&self, other: &Self) -> std::cmp::Ordering {
            self.walked_transunix().cmp(&other.walked_transunix())
        }
        fn walked_set_transunix(&mut self, unix: i64) {
            if let Some(operation) = self.as_event_operation_mut() {
                operation.set_transunix(unix);
            }
        }
        fn walked_state(&self) -> Option<&State> {
            match self.as_event_operation() {
                Some(operation) => Some(operation.get_state()),
                None => None,
            }
        }
        fn walked_set_state(&mut self, state: State) {
            if let Some(operation) = self.as_event_operation_mut() {
                operation.set_state(state);
            }
        }
        fn walked_creaunix(&self) -> Option<i64> {
            self.as_event_operation().and_then(Event::get_creaunix)
        }
        fn walked_set_creaunix(&mut self, unix: Option<i64>) {
            if let Some(operation) = self.as_event_operation_mut() {
                operation.set_creaunix(unix);
            }
        }
        fn walked_exprunix(&self) -> Option<i64> {
            self.as_event_operation().and_then(Event::get_exprunix)
        }
        fn walked_set_execunix(&mut self, unix: Option<i64>) {
            if let Some(operation) = self.as_event_operation_mut() {
                operation.set_execunix(unix, true);
            }
        }
        fn walked_set_sendunix(&mut self, unix: Option<i64>) {
            if let Some(operation) = self.as_event_operation_mut() {
                operation.set_sendunix(unix);
            }
        }
        fn walked_snapunix(&self) -> Option<i64> {
            self.as_event().and_then(|event| event.get_snapunix())
        }
        fn walked_set_snapunix(&mut self, unix: Option<i64>) {
            if let Some(operation) = self.as_event_operation_mut() {
                operation.set_snapunix(unix);
            }
        }
        fn walked_set_seqnum(&mut self, seqnum: u64) {
            if let Some(operation) = self.as_event_operation_mut()
                && operation.get_seqnum() != seqnum
            {
                operation.set_seqnum(seqnum);
            }
        }
        fn walked_made(&self) -> bool {
            self.as_event_operation().is_some_and(made)
        }
        fn walked_identifiers(&self) -> Option<&Identifiers> {
            match self.as_event_operation() {
                Some(operation) => Some(operation.get_identifiers()),
                None => None,
            }
        }
        fn walked_origin_of(&self, kind: &IdType) -> Option<IdType> {
            origin_of(self.as_event_operation()?, kind)
        }
        /// Through the market doors both statements hold, whichever leaf
        /// each is: a typed follower of a FIX message takes the side and the
        /// stored cross code as a follower of its own leaf does.
        fn walked_follow_identity(&mut self, live: &Self) -> bool {
            super::super::market::follow_identity(self, live)
        }
        fn walked_note_conflict(&mut self, cited: &str) {
            if let Some(operation) = self.as_event_operation_mut() {
                operation.note_conflict(cited);
            }
        }
        /// Restates through the leaf's own [`Event::restating`] - a FIX
        /// message through its own, as it follows through its own
        /// `with_previous` - where the live element is the same walked
        /// variant; a statement over another leaf of its category stands as
        /// it is, as it does where it follows one.
        fn walked_restating(self, live: &Self) -> Self {
            match (self, live) {
                (Self::OrderEvent(this), Self::OrderEvent(live)) => {
                    Self::OrderEvent(this.restating(live))
                }
                (Self::QuoteEvent(this), Self::QuoteEvent(live)) => {
                    Self::QuoteEvent(this.restating(live))
                }
                (Self::ExecutionEvent(this), Self::ExecutionEvent(live)) => {
                    Self::ExecutionEvent(this.restating(live))
                }
                (Self::TradeEvent(this), Self::TradeEvent(live)) => {
                    Self::TradeEvent(this.restating(live))
                }
                (Self::Fix(this), Self::Fix(live)) => Self::Fix(this.restating(live.as_ref())),
                (this, _) => this,
            }
        }
        /// Through the leaf's own [`Event::finalized`]; a value no walk
        /// keeps alive is settled whole.
        fn walked_restamp(&mut self) {
            match self.as_event_operation_mut() {
                Some(operation) => {
                    let code = operation.get_hashcode();
                    operation.finalized(code);
                }
                None => self.finalize(),
            }
        }
        fn walked_fill_execution(&mut self) {
            if let Some(operation) = self.as_event_operation_mut() {
                let _ = super::super::market::fill_execution(operation);
            }
        }
        fn walked_own_fills(&self) -> Option<OwnFills> {
            self.as_event_operation().map(OwnFills::read)
        }
        fn walked_account_fills(
            &mut self,
            own: &OwnFills,
            kept: Option<Decimal>,
            fills: &mut Fills,
        ) -> Accounted {
            match self.as_event_operation_mut() {
                Some(operation) => account_fills(operation, own, kept, fills),
                None => Accounted::default(),
            }
        }
        fn walked_hiddenqty(&self) -> Option<Decimal> {
            self.as_event_operation()?.get_hiddenqty()
        }
        fn walked_clear_fill(&mut self) {
            if let Some(operation) = self.as_event_operation_mut() {
                clear_fill(operation);
            }
        }
        fn is_walked(&self) -> bool {
            self.as_event_operation().is_some()
        }
        fn walked_side(&self) -> Side {
            self.as_event_operation()
                .map_or(Side::Unknown, |operation| {
                    operation.marketdatakind().stored_side(operation.get_side())
                })
        }
        fn walked_slot(&self) -> Side {
            self.as_event_operation()
                .map_or(Side::Unknown, |operation| operation.get_side().tagged())
        }
        fn walked_instcode(&self) -> Option<&str> {
            self.as_event_operation()?.get_instcode()
        }
        fn walked_kind(&self) -> MarketDataKind {
            self.marketdatakind()
        }
    }
}

use sealed::Walked;

/// The elements a walk reads, in the order it reads them.
#[derive(Debug)]
enum Source<E, I> {
    /// The caller's iterator read as it comes, because the caller said the
    /// elements arrive in their own order.
    Streamed(I),
    /// The caller's elements collected and sorted by their own order.
    Sorted(vec::IntoIter<E>),
}

/// A walk over timed elements that chains each one to the live element it
/// follows and yields it enriched.
///
/// The walk keeps the elements still alive - in a live
/// [`State`], and not
/// past their expiration - under the identity every incarnation of one
/// thing shares: the cross element, which is the element's own identity
/// where it states no cross code, within the element's
/// [`MarketDataKind`] - a chain holds one category,
/// so an order and an execution under one cross code are two chains and a
/// fill never follows the order it filled - and, for a sided kind
/// ([`MarketDataKind::is_sided`]), within its side, so a buy and a sell are
/// two chains whatever they share. A current element matches the previous
/// alive element of its kind, of the same instrument
/// ([`Market::get_instcode`]) where both state one, of the same side
/// whenever both state one - the side an order's or an execution's chain
/// is keyed by, the leg a quote or a book entry tags, so a bid and an offer
/// going by one `MDEntryID(278)` are two entries - sharing one identifier
/// of the same type and value ([`IdType::is_chain_name`]): a chain goes by
/// names beside its cross element - every value of a chain identity type
/// ([`IdType::is_chain_identity`]) that its statements stated, the old value
/// beside the new after a replace; the chain's first value a lineage field
/// names (`OrigClOrdID(41)`, `OrigTradeID(1126)`, `TradeReportRefID(572)`)
/// under the base it names; and, for every other type its live statement
/// holds - an `ExecID`, a bridge's own word - that value alone, released by
/// the next statement that holds another or none, so a chain of many
/// reports is bounded by its identities and never by its reports. An
/// identifier many elements share names none: a match's `TrdMatchID(880)`,
/// which both orders it filled state, a request many chains answer
/// (`QuoteReqID(131)`, `MDReqID(262)`) and a parent order's slot
/// (`parentorderid`, `parentclordid`). Each name is held by the first live
/// chain of its category, side and instrument that stated it until that
/// chain ends, and matched by type and value whatever source spelled it. An
/// element arriving under an identity a live element of its category holds,
/// or citing a chain by such a name - so a report that spells only the
/// `ClOrdID` a live order was placed under still finds the order - is
/// stated as the one after it by its own
/// [`Element::with_previous`], so it records its predecessor, stands after
/// it where they share an instant and carries the lifecycle forward; what
/// that answers is what the walk yields. An element citing two live chains
/// at once is a conflict, never a pick: it stands under its own identity -
/// its own live chain where that is one of the two, a chain of its own
/// otherwise - and the conflict is told on it
/// ([`Operation::note_conflict`](super::Operation::note_conflict): a FIX
/// message records it as an anomaly) and warned once per kind. The
/// yielded element then stands as the live one under that identity where
/// it is still alive, and retires it where it is not: a filled order, an
/// expired quote, ends its chain, and a later element under the same
/// identity starts one afresh.
///
/// Every element the walk states as a chain's - a follower, another
/// statement of the live one, a late one, an expiration - is re-keyed onto
/// the chain: it takes the live element's side where it is sided and states
/// none, then the live element's stored cross code where that states one,
/// so an identifier change moves no element onto another cross code and
/// splits no book; its cross hash and cross element derive from that code,
/// and its identity is settled once under them
/// ([`Operation::follow_identity`](super::Operation::follow_identity)).
///
/// Two elements never chain against their order. One that is
/// [`Element::is_before`] the live element - out of order on a walk the
/// caller called sorted - follows nothing and changes nothing, but is
/// yielded under the chain's side and cross code; one the element's own
/// reading refuses, or that following changes nothing on, is yielded under
/// the chain's identity and still stands as the live one. An element under
/// no live identity is yielded as it came, and stands. What the walk yields
/// is always the caller's own copy: the live element is a clone the walk
/// keeps, never a reference into it.
///
/// An order's chain counts the fills it reports once by execution
/// identifier ([`Operation::fill_of`](super::Operation::fill_of)), over the
/// total its first statement stated - what traded before the walk saw the
/// chain: a fill whose identifier the chain already counted adds nothing,
/// moves no quantity of the chain and promotes nothing; a new one adds its
/// `lastqty`, a bust takes the fill it names back, a correction replaces
/// it. The count is the order's `cumqty`, its `leavesqty` the accepted order
/// quantity less it - floored at nothing, an overfill warned - and a
/// partial fill whose count reaches that quantity reads `FILLED`, never the
/// reverse: a stated `FILLED` whose count falls short stays `FILLED`,
/// warned, while an ended state that is no word of its holder's own - a
/// FIX trade report's `FILLED` that its `LeavesQty(151)` of nothing reads
/// ([`Operation::states_end`]) - reads, over a chain the walk counted, as
/// the partial fill the count says it is. A stated total that
/// disagrees with the count is warned once per kind and never adopted. The
/// fills of a chain that ended stay findable for
/// [`Self::with_window_ns`] of event time, so a copy of one of them logged
/// at another hop and dated apart - a resend, a frame hop - starts no live
/// chain from a count: an order's report of it is yielded as it came,
/// outside every chain, and a second statement of an execution is another
/// statement of the first, which a deduplicating caller yields once.
///
/// Every walked element leaves stating when its lifecycle was created: one
/// stating no creation takes its chain's - the earliest the fold keeps - or,
/// starting a chain, its own instant; a stated creation is never replaced.
/// A chain whose first element states no cross code stands under that
/// element's identity, and every element after it carries that identity as
/// its cross element, so one chain is one cross element.
///
/// An element arriving under the identity the live element *arrived*
/// under - the same instant, the same content: one message a capture
/// logged at every hop it passed - is another statement of the live
/// element and not the one after it. It is yielded [restating](yggdryl::graph::Event::restating)
/// the live one, so it takes the live one's predecessor and place and
/// finalizes to the same identity, and the chain grows by nothing. One
/// arriving under the identity a statement the chain moved past at the live
/// element's own instant arrived under is yielded restating that statement,
/// and the chain stays where it moved. A chain a step ends keeps that step
/// and the statements it moved past at the step's instant the same way, so
/// one arriving under the identity one of them arrived under is yielded
/// restating it and the chain stays ended, until the walk reads a later
/// instant.
///
/// Opened over elements the caller says are sorted, the walk reads them as
/// they come and yields each as it is read; over elements the caller does
/// not, it collects them first and sorts them by their own order, stably,
/// so two neither after nor before one another keep the order they arrived
/// in. Either way each source element is cloned twice at most: once to hold
/// the live one, or the step that ended its chain, once to keep an element
/// its predecessor refuses. Expiration and grid views are owned snapshots
/// and necessarily clone their live event.
///
/// A live element with a finite expiration produces one final owned event at
/// that exact instant in the `EXPIRED` state, then retires. Replacing or
/// ending its generation removes the old scheduled deadline. A deadline at
/// the same instant as source input is read first; after the source ends,
/// the finite deadlines still live are drained in their own order. The
/// expirations of one deadline take its places in the order the walk hands
/// them over - the order of the identities they retire - each the next;
/// a source element keeps the place it came with.
///
/// Given a grid - [`Self::with_snapshot_ns`], a step in nanoseconds aligned
/// on the epoch - the walk also yields an owned view of every living identity
/// at each crossed grid instant. A view is the live event as of that
/// instant: dated at it - [`Event::get_transunix`](yggdryl::graph::Event::get_transunix) and
/// [`Event::get_snapunix`](yggdryl::graph::Event::get_snapunix) both - so it has the
/// identity that instant derives, a row of its own wherever rows are keyed by
/// identity within a time, while its content, its place and its
/// cross element are the live event's; it does not advance the chain. All source events at an exact
/// boundary are read before its views, while expirations at that boundary are
/// read before either. Source events keep the snapshot fact they stated.
/// At EOF the grid stops at the greatest remaining finite expiration, or at
/// the last source instant where nothing live expires, so a nonexpiring
/// identity never makes a finite source infinite. Without a grid the walk
/// leaves snapshot instants as they came.
///
/// The walk reads any `E: Event + Operation + Clone` - a typed event, a
/// FIX lifecycle message - and [`MarketData`](super::MarketData): of a
/// `MarketData`, the operation events (`OrderEvent`, `QuoteEvent`,
/// `ExecutionEvent`, `TradeEvent`) and a FIX message (`Fix`) chain, each
/// following and restating through its own reading, and every other
/// variant is yielded unchanged where it stands - a dated one at its
/// instant, an undated one where it is read - and never stands live.
///
/// ```
/// # yggdryl_market::install().unwrap();
/// use yggdryl::graph::{Element, Event};
/// use yggdryl_market::graph::{EventIterator, OrderEvent};
/// use yggdryl::State;
///
/// // One order's life as three events sharing its cross code, plus one
/// // event of another order: a market event orders by instant and follows
/// // by the timed reading.
/// let event = |order: &str, unix: i64, state: &str| {
///     let mut event = OrderEvent::at(unix);
///     event.set_crosscode(order.to_owned());
///     event.set_state(State::from_spelling(state).expect("a shipped state"));
///     event.finalize();
///     event
/// };
/// let arrived = vec![
///     event("O-100", 30, "Filled"),
///     event("O-100", 10, "New"),
///     event("O-900", 15, "New"),
///     event("O-100", 20, "PartiallyFilled"),
///     event("O-100", 40, "New"),
/// ];
///
/// // Unsorted, the walk sorts by the elements' own order first.
/// let mut walk = EventIterator::new(arrived, false);
/// let first = walk.next().expect("the earliest");
/// assert_eq!((first.get_transunix(), first.get_seqnum(), first.get_prevuuid()), (10, 0, None));
/// let other = walk.next().expect("the other order's");
/// assert_eq!((other.get_crosscode(), other.get_seqnum()), ("10:0:O-900", 0));
/// let second = walk.next().expect("the partial fill");
/// assert_eq!((second.get_seqnum(), second.get_prevuuid()), (0, Some(first.get_uuid())));
/// let filled = walk.next().expect("the fill");
/// assert_eq!((filled.get_seqnum(), filled.get_prevuuid()), (0, Some(second.get_uuid())));
/// // A filled order ended its chain: the next event under its identity
/// // starts one afresh, and is alive beside the other order.
/// let again = walk.next().expect("the late one");
/// assert_eq!((again.get_seqnum(), again.get_prevuuid()), (0, None));
/// let mut alive = walk.alive().map(|held| (held.get_crosscode().to_owned(), held.get_transunix())).collect::<Vec<_>>();
/// alive.sort();
/// assert_eq!(alive, [("10:0:O-100".to_owned(), 40), ("10:0:O-900".to_owned(), 15)]);
/// assert!(walk.next().is_none());
///
/// // The same event read twice - a message logged at two hops - is one
/// // event: the second statement takes the first one's place and identity.
/// let mut walk = EventIterator::new(
///     vec![event("O-100", 10, "New"), event("O-100", 20, "PartiallyFilled"), event("O-100", 20, "PartiallyFilled")],
///     true,
/// );
/// let first = walk.next().expect("the order");
/// let second = walk.next().expect("the partial fill");
/// let twin = walk.next().expect("the partial fill, logged again");
/// assert_eq!((second.get_seqnum(), second.get_prevuuid()), (0, Some(first.get_uuid())));
/// assert_eq!((twin.get_seqnum(), twin.get_prevuuid(), twin.get_uuid()), (0, second.get_prevuuid(), second.get_uuid()));
/// ```
#[derive(Debug)]
pub struct EventIterator<E, I> {
    source: Source<E, I>,
    lookahead: Option<E>,
    source_started: bool,
    alive: HashMap<Chain, Live<E>>,
    /// Every name a live chain goes by ([`chain_names`]: its chain
    /// identities under their types, its first value a lineage field names
    /// under the base, every other type's value its live statement holds
    /// under that type), by scheme then name, under the identity that holds
    /// it and the side the name is alive on ([`Walked::walked_slot`]) - a
    /// sided chain's side, a quote's or an entry's tag - one holder per
    /// side, category and instrument ([`Walked::walked_instcode`], read off
    /// the holder's live element), the first chain that stated it, until
    /// that chain ends: a name an element states is a chain it cites, on
    /// its side where both state one, of its instrument where both state
    /// one, and an element stating no side or no instrument cites every
    /// one a name is alive on. Two levels, so a name is looked up by the
    /// borrowed scheme and name an element states and never by a copy of
    /// them, and a slot holds at most one chain per category, side and
    /// instrument going by the name, never one per chain
    /// (`rust/market/tests/graph/iterator.rs` pins the slot's length).
    /// Bounded by the identifier changes of the live chains - one entry per
    /// distinct chain identity a chain stated, one per other type its live
    /// statement holds - never by their reports.
    named: HashMap<IdType, HashMap<String, Vec<(Side, Chain)>>>,
    /// The live identities of each base cross code - the code without the
    /// prefix [`Market::stored_crosscode`](super::Market::stored_crosscode)
    /// gives it - of the sided elements stating a side, the only ones whose
    /// chain is keyed by a side, so an element stating no side joins the
    /// one side its code is alive on.
    bases: HashMap<String, Vec<Chain>>,
    /// The names each live identity is known by, so retiring it forgets
    /// exactly those.
    names_of: HashMap<Chain, Vec<(IdType, String)>>,
    /// The one current finite deadline of each live identity, ordered so a
    /// deadline is retired before anything arriving at the same instant.
    expirations: BTreeSet<(i64, Chain)>,
    /// The grid step in nanoseconds, or nothing positive for no grid.
    snapshot_ns: i64,
    /// The next epoch-aligned grid instant still to read.
    next_snapshot: Option<i64>,
    /// Identities copied at the active grid instant, one clone emitted at a
    /// time rather than a queue proportional to the number of ticks.
    snapshot_at: Option<i64>,
    snapshot_identities: Vec<Chain>,
    snapshot_index: usize,
    /// A source instant whose whole equal-time group was just read. Its
    /// exact grid snapshot is owed after the group.
    after_group: Option<i64>,
    /// The latest source or emitted deadline reached. It bounds a grid at
    /// EOF after the last finite deadline is removed from the schedule.
    watermark: Option<i64>,
    /// The statements of the chains a step ended at `retired_at`, each under
    /// the identity it arrived under and beside the chain it stood in: the
    /// step that ended it and the statements the chain moved past at that
    /// instant. A sorted walk reads another statement of one of them nowhere
    /// but at that instant, so they are dropped once it reads a later one.
    retired: Vec<(Uuid, Chain, E)>,
    /// The instant the retired statements were stated at.
    retired_at: Option<i64>,
    /// The places the walk gives at each instant: its expirations always,
    /// and where it places what it reads, every source element too.
    sequence: InstantSequence,
    /// Whether the walk places the source elements it reads, rather than
    /// keeping the places they came with.
    placing: bool,
    /// The fills of the chains ended within the window ([`Tombstones`]).
    tombstones: Tombstones,
}

/// A chain the walk keeps alive: its cross element, within one market data
/// category - an order and an execution under one cross code are two
/// chains, and a lifecycle matches an element with a chain of its own
/// category only.
type Chain = (Uuid, MarketDataKind);

/// One identity's live element and the identity it arrived under - what it
/// was before the walk stated it, which is what another statement of the
/// same element still carries - beside the statements the chain moved past
/// at the live element's own instant, each under the identity it arrived
/// under: a sorted walk reads another statement of one of them nowhere but
/// at that instant, so they are dropped once the chain moves to a later one.
#[derive(Debug)]
struct Live<E> {
    element: E,
    arrived: Uuid,
    passed: Vec<(Uuid, E)>,
    /// The chain's fill ledger ([`Fills`]): every fill it counted, by
    /// execution identifier, over its first stated total.
    fills: Fills,
}

/// The fills of the chains that ended within the walk's window, under the
/// chain they ended - its base cross code, its category and its side - so a
/// late copy of one of them - the same fill logged at another hop, dated
/// apart - restates the ended chain rather than starting a live one from a
/// count, while a fill of another chain under the same base - the other
/// side of one match under one execution identifier - stays its own. Held
/// for [`EventIterator::with_window_ns`] of event time past the instant the
/// chain ended, swept whenever the table has doubled since the last sweep,
/// so remembering costs amortized constant time and the table holds at most
/// twice what the window does; the chain ended last is held inline, so a
/// walk over one report - an order filled by its first report, an
/// execution - remembers it without allocating.
#[derive(Debug)]
struct Tombstones {
    /// The window in nanoseconds of event time; nonpositive remembers none.
    span: i64,
    /// The chain ended last.
    last: Option<Tomb>,
    /// Every other chain ended within the window, by base cross code: the
    /// chains of one base, each of its own category and side, linked.
    held: HashMap<Str, Tomb>,
    /// The size past which the table is swept again.
    sweep_at: usize,
}

/// One ended chain's fills, under its base cross code, its category and its
/// side, dated at the instant it ended; `next` the next chain ended under
/// the same base, of another category or side.
#[derive(Debug)]
struct Tomb {
    base: Str,
    kind: MarketDataKind,
    side: Side,
    at: i64,
    fills: Fills,
    next: Option<Box<Tomb>>,
}

impl Tomb {
    fn new(base: &str, kind: MarketDataKind, side: Side, at: i64, fills: Fills) -> Self {
        Self {
            base: Str::from(base),
            kind,
            side,
            at,
            fills,
            next: None,
        }
    }

    /// Whether this is the chain ended under `base`, `kind` and `side`.
    fn is(&self, base: &str, kind: MarketDataKind, side: Side) -> bool {
        self.kind == kind && self.side == side && self.base == base
    }

    /// The chain of `kind` and `side` linked from this one, this one included.
    fn find_mut(&mut self, kind: MarketDataKind, side: Side) -> Option<&mut Self> {
        if self.kind == kind && self.side == side {
            return Some(self);
        }
        self.next.as_deref_mut()?.find_mut(kind, side)
    }

    /// This chain and every one linked from it.
    fn chain(&self) -> impl Iterator<Item = &Self> {
        std::iter::successors(Some(self), |tomb| tomb.next.as_deref())
    }

    /// Unlinks every chain after this one that ended before `horizon`.
    fn prune_next(&mut self, horizon: i64) {
        if let Some(mut next) = self.next.take() {
            next.prune_next(horizon);
            self.next = if next.at >= horizon {
                Some(next)
            } else {
                next.next.take()
            };
        }
    }

    /// Takes the fills of a chain ended again under the same key.
    fn absorb(&mut self, at: i64, fills: Fills) {
        self.at = self.at.max(at);
        self.fills.absorb(fills);
    }
}

impl Tombstones {
    /// The fewest chains the table holds before it is swept at all.
    const SWEEP_FLOOR: usize = 1_024;

    fn new(span: i64) -> Self {
        Self {
            span,
            last: None,
            held: HashMap::new(),
            sweep_at: Self::SWEEP_FLOOR,
        }
    }

    /// Remembers the `fills` of a chain of `kind` and `side` under `base`
    /// that ended at `at`, where they counted anything and the window
    /// remembers: a chain ended under a key already held - a later
    /// incarnation of it - adds its fills to the ones held.
    fn bury(
        &mut self,
        base: &str,
        kind: MarketDataKind,
        side: Side,
        at: i64,
        fills: Fills,
        watermark: i64,
    ) {
        if self.span <= 0 || fills.is_empty() {
            return;
        }
        let horizon = watermark.saturating_sub(self.span);
        let Some(last) = &mut self.last else {
            self.last = Some(Tomb::new(base, kind, side, at, fills));
            return;
        };
        if last.is(base, kind, side) {
            last.absorb(at, fills);
            return;
        }
        if let Some(held) = self
            .held
            .get_mut(base)
            .and_then(|head| head.find_mut(kind, side))
        {
            held.absorb(at, fills);
            return;
        }
        // The chain ended before this one moves to the table, where it
        // stays until the window passes it.
        let mut moved = std::mem::replace(last, Tomb::new(base, kind, side, at, fills));
        if moved.at >= horizon {
            match self.held.get_mut(moved.base.as_str()) {
                Some(head) => {
                    moved.next = head.next.take();
                    head.next = Some(Box::new(moved));
                }
                None => {
                    self.held.insert(moved.base.clone(), moved);
                }
            }
        }
        if self.held.len() >= self.sweep_at {
            self.held.retain(|_, head| {
                head.prune_next(horizon);
                if head.at >= horizon {
                    return true;
                }
                match head.next.take() {
                    Some(next) => {
                        *head = *next;
                        true
                    }
                    None => false,
                }
            });
            self.sweep_at = (self.held.len() * 2).max(Self::SWEEP_FLOOR);
        }
    }

    /// The settled statement that counted one of `own`'s fills in a chain
    /// of `kind` ended under `base` within the window, where one did: the
    /// chain of `side`, or of any side where either states none.
    fn counted_by(
        &self,
        base: &str,
        kind: MarketDataKind,
        side: Side,
        own: &OwnFills,
        watermark: i64,
    ) -> Option<Uuid> {
        if self.span <= 0 {
            return None;
        }
        let horizon = watermark.saturating_sub(self.span);
        let last = self.last.as_ref().filter(|last| last.base == base);
        last.into_iter()
            .chain(self.held.get(base).into_iter().flat_map(Tomb::chain))
            .filter(|tomb| {
                tomb.kind == kind
                    && (tomb.side == side || side == Side::Unknown || tomb.side == Side::Unknown)
                    && tomb.at >= horizon
            })
            .find_map(|tomb| own.ids().find_map(|id| tomb.fills.counted_by(id)))
    }
}

impl<E, I> EventIterator<E, I>
where
    E: Walked,
    I: Iterator<Item = E>,
{
    /// Opens a walk over `elements`.
    ///
    /// `sorted` states that the elements arrive in their own order already -
    /// none [`Element::is_before`] one before it - and the walk reads them
    /// as they come. Where they do not, the walk collects them first and
    /// sorts them by that order, stably, so elements neither after nor
    /// before one another keep the order they arrived in.
    pub fn new(elements: impl IntoIterator<IntoIter = I>, sorted: bool) -> Self {
        let source = if sorted {
            Source::Streamed(elements.into_iter())
        } else {
            let mut collected: Vec<E> = elements.into_iter().collect();
            collected.sort_by(E::walked_order);
            Source::Sorted(collected.into_iter())
        };
        Self {
            source,
            lookahead: None,
            source_started: false,
            alive: HashMap::new(),
            named: HashMap::new(),
            bases: HashMap::new(),
            names_of: HashMap::new(),
            expirations: BTreeSet::new(),
            retired: Vec::new(),
            retired_at: None,
            snapshot_ns: 0,
            next_snapshot: None,
            snapshot_at: None,
            snapshot_identities: Vec::new(),
            snapshot_index: 0,
            after_group: None,
            watermark: None,
            sequence: InstantSequence::default(),
            placing: false,
            tombstones: Tombstones::new(Self::DEFAULT_WINDOW_NS),
        }
    }

    /// The window of event time an ended chain's fills are remembered for
    /// by default: one minute, the FIX codec's deduplication window.
    pub const DEFAULT_WINDOW_NS: i64 = 60_000_000_000;

    /// The walk remembering the fills of a chain for `window_ns` nanoseconds
    /// of event time past the instant the chain ended, so a copy of one of
    /// them logged at another hop and dated apart restates the ended chain
    /// rather than starting one afresh; a window of zero or less remembers
    /// none.
    #[must_use]
    pub fn with_window_ns(mut self, window_ns: i64) -> Self {
        self.tombstones = Tombstones::new(window_ns);
        self
    }

    /// The walk placing every source element it reads by content among
    /// what it handed over at that instant, before it follows anything -
    /// after the expirations of that instant, which the walk hands over
    /// first - rather than keeping the place the element came with. What a
    /// walk made - [`Walked::walked_made`] - keeps the place the walk gave
    /// it, so a walked stream read again answers itself.
    #[must_use]
    pub(crate) fn with_placing(mut self, placing: bool) -> Self {
        self.placing = placing;
        self
    }

    /// The walk reading one snapshot per grid step of `snapshot_ns`
    /// nanoseconds per identity, the grid aligned on the epoch; a step of
    /// zero or less takes the grid away.
    #[must_use]
    pub const fn with_snapshot_ns(mut self, snapshot_ns: i64) -> Self {
        self.snapshot_ns = snapshot_ns;
        self
    }

    /// Places `element` by content among what the walk handed over at its
    /// instant.
    fn place(&mut self, element: &mut E) {
        if let Some(unix) = element.walked_transunix() {
            let seqnum = self.sequence.place(unix, Some(element.get_hashcode()));
            element.walked_set_seqnum(seqnum);
        }
    }

    /// The grid step in nanoseconds, where the walk reads snapshots.
    #[must_use]
    pub fn snapshot_ns(&self) -> Option<i64> {
        (self.snapshot_ns > 0).then_some(self.snapshot_ns)
    }

    /// The elements still alive after what the walk has read so far, one
    /// per identity, in no order.
    pub fn alive(&self) -> impl Iterator<Item = &E> {
        self.alive.values().map(|live| &live.element)
    }

    /// Records `element` as the live one under `identity` where it is still
    /// alive, its names and its chain's fill ledger with it, and retires the
    /// identity where it is not, the ledger into the tombstones.
    fn settle(&mut self, identity: Chain, element: &E, arrived: Uuid, mut fills: Fills) {
        fills.stamp(element.get_uuid());
        if let Some(deadline) = self
            .alive
            .get(&identity)
            .and_then(|live| live.element.walked_exprunix())
        {
            self.expirations.remove(&(deadline, identity));
        }
        if is_alive(element) {
            let slot = element.walked_slot();
            let instrument = element.walked_instcode();
            let alive = &self.alive;
            for (scheme, name) in chain_names(element) {
                // Looked up borrowed first, as the bases are: a chain settled
                // again under the names it already goes by allocates nothing.
                let names = match self.named.get_mut(&scheme) {
                    Some(names) => names,
                    None => self.named.entry(scheme.clone()).or_default(),
                };
                let slots = match names.get_mut(name) {
                    Some(slots) => slots,
                    None => names.entry(name.to_owned()).or_default(),
                };
                // One side per identity: the one its latest statement tags.
                let held = slots.len();
                slots.retain(|(held, chain)| *chain != identity || *held == slot);
                let moved_off = slots.len() != held;
                // One live holder per side, category and instrument: the
                // first chain that stated the name holds it until it ends,
                // so a second chain stating it files nothing for it. The
                // holder's instrument is its live element's, so a chain
                // that learnt one keeps the name without filing it again.
                if slots.iter().any(|(held, chain)| {
                    *held == slot
                        && (*chain == identity
                            || (chain.1 == identity.1
                                && alive
                                    .get(chain)
                                    .and_then(|live| live.element.walked_instcode())
                                    == instrument))
                }) {
                    if moved_off
                        && !slots.iter().any(|(_, chain)| *chain == identity)
                        && let Some(known) = self.names_of.get_mut(&identity)
                    {
                        known.retain(|(held_scheme, held_name)| {
                            *held_scheme != scheme || held_name.as_str() != name
                        });
                    }
                    continue;
                }
                slots.push((slot, identity));
                let known = self.names_of.entry(identity).or_default();
                if !known
                    .iter()
                    .any(|(held_scheme, held_name)| *held_scheme == scheme && held_name == name)
                {
                    known.push((scheme, name.to_owned()));
                }
            }
            self.release(identity, element);
            let base = base_crosscode(element.get_crosscode());
            // Only a sided element stating a side is a side its base is
            // alive on: no other chain is keyed by a side.
            if element.walked_side() != Side::Unknown {
                // Looked up borrowed first: a chain settled again under its
                // base allocates nothing.
                match self.bases.get_mut(base) {
                    Some(live) if live.contains(&identity) => {}
                    Some(live) => live.push(identity),
                    None => {
                        self.bases.insert(base.to_owned(), vec![identity]);
                    }
                }
            }
            // The statement this one replaces stays the chain's at their
            // shared instant, unless this one is another statement of it.
            let instant = element.walked_transunix();
            let passed = match self.alive.remove(&identity) {
                Some(mut live) if live.element.walked_transunix() == instant => {
                    if live.arrived != arrived {
                        live.passed.push((live.arrived, live.element));
                    }
                    live.passed
                }
                _ => Vec::new(),
            };
            self.alive.insert(
                identity,
                Live {
                    element: element.clone(),
                    arrived,
                    passed,
                    fills,
                },
            );
            if let Some(deadline) = element.walked_exprunix() {
                self.expirations.insert((deadline, identity));
            }
        } else {
            let instant = element.walked_transunix();
            if let Some(live) = self.retire(identity) {
                // The step that ends the chain, and the statements it moved
                // past at this instant, stay the chain's until the walk
                // passes it.
                if self.retired_at != instant {
                    self.retired.clear();
                    self.retired_at = instant;
                }
                if live.element.walked_transunix() == instant {
                    self.retired.extend(
                        live.passed
                            .into_iter()
                            .map(|(held, statement)| (held, identity, statement)),
                    );
                    if live.arrived != arrived {
                        self.retired.push((live.arrived, identity, live.element));
                    }
                }
                self.retired.push((arrived, identity, element.clone()));
            }
            // The ended chain's fills stay findable for the window, whether
            // the chain lived or its first statement ended it.
            if let Some(at) = instant {
                let watermark = self.watermark.unwrap_or(at).max(at);
                self.tombstones.bury(
                    base_crosscode(element.get_crosscode()),
                    identity.1,
                    element.walked_side(),
                    at,
                    fills,
                    watermark,
                );
            }
        }
    }

    /// Releases the names `identity` filed that are not its live
    /// statement's: a chain identity's every value stays the chain's until
    /// it ends, so a late report citing the old one still finds it; any
    /// other type's value is the live statement's alone, so a chain of many
    /// reports holds one record per type, never one per report. One walk of
    /// the chain's own record, a `retain` on each slot released, nothing per
    /// live chain.
    fn release(&mut self, identity: Chain, element: &E) {
        let Some(known) = self.names_of.get_mut(&identity) else {
            return;
        };
        let mut at = 0;
        while let Some((scheme, name)) = known.get(at) {
            let kept = scheme.is_chain_identity()
                || chain_names(element).any(|(held, value)| held == *scheme && value == name);
            if kept {
                at += 1;
                continue;
            }
            let (scheme, name) = known.remove(at);
            let empty = self.named.get_mut(&scheme).is_some_and(|names| {
                if let Some(slots) = names.get_mut(&name) {
                    slots.retain(|(_, held)| *held != identity);
                    if slots.is_empty() {
                        names.remove(&name);
                    }
                }
                names.is_empty()
            });
            if empty {
                self.named.remove(&scheme);
            }
        }
        if known.is_empty() {
            self.names_of.remove(&identity);
        }
    }

    /// Retires the live identity and every index and deadline it owns.
    fn retire(&mut self, identity: Chain) -> Option<Live<E>> {
        let live = self.alive.remove(&identity)?;
        if let Some(deadline) = live.element.walked_exprunix() {
            self.expirations.remove(&(deadline, identity));
        }
        for (scheme, name) in self.names_of.remove(&identity).unwrap_or_default() {
            let empty = self.named.get_mut(&scheme).is_some_and(|names| {
                if let Some(slots) = names.get_mut(&name) {
                    slots.retain(|(_, held)| *held != identity);
                    if slots.is_empty() {
                        names.remove(&name);
                    }
                }
                names.is_empty()
            });
            if empty {
                self.named.remove(&scheme);
            }
        }
        let base = base_crosscode(live.element.get_crosscode());
        if live.element.walked_side() != Side::Unknown
            && let Some(held) = self.bases.get_mut(base)
        {
            held.retain(|held| *held != identity);
            if held.is_empty() {
                self.bases.remove(base);
            }
        }
        Some(live)
    }

    /// Pulls the next source element into lookahead once, and opens a grid
    /// no earlier than that first observed fact.
    fn fill_lookahead(&mut self) {
        if self.source_started {
            return;
        }
        self.source_started = true;
        self.lookahead = match &mut self.source {
            Source::Streamed(source) => source.next(),
            Source::Sorted(source) => source.next(),
        };
        if let (Some(element), Some(step)) = (&self.lookahead, self.snapshot_ns()) {
            self.next_snapshot = element
                .walked_transunix()
                .and_then(|unix| grid_at_or_after(unix, step));
        }
    }

    /// Advances source lookahead after one element was consumed.
    fn advance_source(&mut self) {
        self.lookahead = match &mut self.source {
            Source::Streamed(source) => source.next(),
            Source::Sorted(source) => source.next(),
        };
    }

    /// Opens one grid instant over the identities alive after all source
    /// events at that instant, in deterministic identity order.
    fn open_snapshot(&mut self, unix: i64) {
        self.snapshot_at = Some(unix);
        self.snapshot_identities.clear();
        self.snapshot_identities.extend(self.alive.keys().copied());
        self.snapshot_identities.sort_unstable();
        self.snapshot_index = 0;
        self.next_snapshot = self.snapshot_ns().and_then(|step| unix.checked_add(step));
    }

    /// The next owned view at the active grid instant.
    fn next_snapshot_copy(&mut self) -> Option<E> {
        let unix = self.snapshot_at?;
        while let Some(identity) = self.snapshot_identities.get(self.snapshot_index).copied() {
            self.snapshot_index += 1;
            if let Some(live) = self.alive.get(&identity) {
                // The live event as of the grid instant: dated at it, so it
                // derives that instant's identity, and standing under its
                // chain's cross element whatever that identity derives. Its
                // snapshot instant is the one its content was stated at -
                // the live event's own, or the one a view it is kept - never
                // the tick, which is its instant, nor a predecessor's. Only
                // instants moved, and they feed no code, so the live code
                // stands and the identity is stamped again over it.
                let mut snapshot = live.element.clone();
                let cross = snapshot.get_crossuuid();
                let original = snapshot
                    .walked_snapunix()
                    .or_else(|| snapshot.walked_transunix());
                snapshot.walked_set_transunix(unix);
                snapshot.walked_set_snapunix(original);
                snapshot.walked_restamp();
                snapshot.set_crossuuid(cross);
                return Some(snapshot);
            }
        }
        self.snapshot_at = None;
        self.snapshot_identities.clear();
        None
    }

    /// Emits and retires the current generation whose deadline is next.
    fn expire(&mut self, deadline: i64, identity: Chain) -> Option<E> {
        self.expirations.remove(&(deadline, identity));
        if self
            .alive
            .get(&identity)
            .and_then(|live| live.element.walked_exprunix())
            != Some(deadline)
        {
            return None;
        }
        let live = self.retire(identity)?;
        self.watermark = Some(self.watermark.map_or(deadline, |held| held.max(deadline)));
        let previous = live.element;
        self.tombstones.bury(
            base_crosscode(previous.get_crosscode()),
            identity.1,
            previous.walked_side(),
            deadline,
            live.fills,
            self.watermark.unwrap_or(deadline),
        );
        let mut expired = previous.clone();
        expired.walked_set_transunix(deadline);
        expired.walked_set_state(State::Expired);
        expired.walked_clear_fill();
        expired.walked_set_execunix(None);
        expired.walked_set_sendunix(None);
        expired.walked_set_snapunix(None);
        // An expiration is an event of its own deadline, handed over before
        // anything the walk reads at it: it takes the next place the walk
        // gives there, and stands after the live one where that is later.
        expired.finalize();
        self.place(&mut expired);
        let fallback = expired.clone();
        let mut expired = expired.with_previous(&previous).unwrap_or(fallback);
        rekeyed(&mut expired, &previous, identity);
        Some(expired)
    }

    /// States one source element against the live generation it reaches.
    fn walk_source(&mut self, mut element: E) -> E {
        if self
            .retired_at
            .is_some_and(|at| element.walked_transunix().is_some_and(|unix| unix > at))
        {
            self.retired.clear();
            self.retired_at = None;
        }
        if !element.is_walked() {
            return element;
        }
        let arrived = element.get_uuid();
        let (identity, conflict) = self.identity_of(&element);
        if let Some(cited) = conflict {
            // Told on the element before anything restates or finalizes it,
            // so another statement of it, told the same, digests alike - and
            // once per kind in the log, counted after.
            element.walked_note_conflict(&cited);
            warned!(
                "lifecycle element cites two live chains: it stands under its own identity",
                element.walked_kind().as_str(),
                "{cited}"
            );
        }
        // A chain that ended at this instant goes by no name and no live
        // identity, so its statements are found by the identity they
        // arrived under alone.
        let kind = element.walked_kind();
        let passed = self
            .retired
            .iter()
            .find(|(held, chain, _)| *held == arrived && chain.1 == kind)
            .map(|(_, chain, statement)| (*chain, statement))
            .or_else(|| {
                self.alive.get(&identity).and_then(|live| {
                    live.passed
                        .iter()
                        .find(|(held, _)| *held == arrived)
                        .map(|(_, statement)| (identity, statement))
                })
            });
        if let Some((chain, statement)) = passed {
            // A statement the chain moved past at this instant - or the
            // step that ended it - logged again: another statement of that
            // one, and the chain stays where it moved.
            let mut element = element.walked_restating(statement);
            rekeyed(&mut element, statement, chain);
            return element;
        }
        // The statement's own words of its fill, read before following
        // carries the chain's total and identifiers onto it; the chain's
        // ledger taken out of the live record for the step and handed back
        // at the settle, never cloned.
        let own = element.walked_own_fills();
        let mut fills = self
            .alive
            .get_mut(&identity)
            .map(|live| std::mem::take(&mut live.fills))
            .unwrap_or_default();
        let mut element = match self.alive.get(&identity) {
            Some(live) if live.arrived == arrived => element.walked_restating(&live.element),
            Some(live) if element.is_before(&live.element) => {
                // Out of order: it follows nothing and moves the live element
                // not at all, but it is one of the chain's statements, so it
                // stands under the chain's side and cross code.
                element.walked_fill_execution();
                rekeyed(&mut element, &live.element, identity);
                created(&mut element, None);
                if let Some(live) = self.alive.get_mut(&identity) {
                    live.fills = fills;
                }
                return element;
            }
            Some(live) => {
                // What the statement said, read before the fold: following
                // keeps the higher rank of the two, so a `NEW` over a live
                // `ACTIVE`, `RUNNING` or `REPLACED` no longer reads `NEW`.
                let stated = element.walked_state().copied();
                let stated_new = stated == Some(State::New);
                let kept = live.element.walked_hiddenqty();
                let mut element = element
                    .clone()
                    .with_previous(&live.element)
                    .unwrap_or(element);
                // The chain's fill accounting over what following answered,
                // then today's refinement: a `NEW` stated over a live
                // element that is itself new - or carrying on, or restated -
                // is that element updated.
                let accounted = match &own {
                    Some(own) => element.walked_account_fills(own, kept, &mut fills),
                    None => Accounted::default(),
                };
                let mut moved = accounted.moved;
                if stated_new
                    && live
                        .element
                        .walked_state()
                        .is_some_and(|held| held.is_new_like())
                {
                    element.walked_set_state(State::Updated);
                    moved = true;
                }
                if moved {
                    element.finalize();
                }
                // An execution is the one fill it reports: a second
                // statement of it saying what the first said - the same
                // fill in the same state - is another statement of the
                // first; one saying something else of it, a correction,
                // stays a statement of its own.
                if let Some(counted_by) = accounted.repeated
                    && reports_fills(kind)
                    && stated == live.element.walked_state().copied()
                    && own.as_ref().is_some_and(|own| {
                        live.element
                            .walked_own_fills()
                            .is_some_and(|held| own.repeats(&held))
                    })
                {
                    element.set_uuid(counted_by);
                }
                element
            }
            None => {
                element.walked_fill_execution();
                // A fill of a chain that ended within the window: a copy
                // logged at another hop and dated apart restates that chain
                // - an execution the statement that counted it - and starts
                // no live chain from a count.
                if let Some(own) = &own
                    && let Some(counted_by) = self.tombstones.counted_by(
                        base_crosscode(element.get_crosscode()),
                        kind,
                        element.walked_side(),
                        own,
                        self.watermark.unwrap_or(i64::MIN),
                    )
                {
                    created(&mut element, None);
                    if reports_fills(kind) {
                        element.set_uuid(counted_by);
                    }
                    return element;
                }
                // A chain's first statement anchors its ledger.
                if let Some(own) = &own
                    && element.walked_account_fills(own, None, &mut fills).moved
                {
                    element.finalize();
                }
                element
            }
        };
        // Whatever following answered - a fold, nothing, a statement over
        // another leaf - the element stands under its chain's identity.
        if let Some(live) = self.alive.get(&identity) {
            rekeyed(&mut element, &live.element, identity);
        }
        created(
            &mut element,
            self.alive
                .get(&identity)
                .and_then(|live| live.element.walked_creaunix()),
        );
        let identity = if self.alive.contains_key(&identity) {
            identity
        } else {
            (element.get_crossuuid(), element.walked_kind())
        };
        self.settle(identity, &element, arrived, fills);
        element
    }

    /// The live identity `element` belongs to, beside what it cited where it
    /// cited two live chains: every chain it names is gathered - its own
    /// cross element where that is alive, the one live side of its base code
    /// where it states no side, and the chain each name it goes by is held
    /// by ([`chain_names`]) - so a report that spells only the `ClOrdID` a
    /// live order was placed under belongs to that order. Naming no live
    /// chain, it starts one under its own cross element; naming one, it is
    /// that chain's; naming two or more, it is a conflict - never a pick -
    /// and it stands under its own identity: its own live chain where that
    /// is one of those cited, a chain of its own otherwise, with the detail
    /// naming its stored cross code and each cited chain's by what cited it.
    ///
    /// A sided kind's chains are keyed by side: an order's or an
    /// execution's cross code carries the side, so a buy and a sell under
    /// one `ClOrdID` are two chains; a quote's are not, so every statement
    /// of one quote under its code is one chain whatever side it tags. A
    /// name is alive on the side its holder's statement tags, whatever the
    /// kind, and an element names by it the holder of its side where both
    /// state one - a bid and an offer going by one `MDEntryID` are two
    /// entries, a tagged statement names the untagged quote holding both
    /// legs - and of its instrument where both state one; stating no side,
    /// or no instrument, it names every holder the name is alive under. An
    /// element stating no side names the one side alive under its base
    /// cross code too - where both are, the base names neither and its
    /// names decide. No element names by a name a chain one of its siblings
    /// stands in - an element split off the same message, such as two
    /// entries of one batch going by the batch's own identifier, are two
    /// entries. Every chain an element names is one of its own category: an
    /// execution names executions, never the order it filled.
    fn identity_of(&self, element: &E) -> (Chain, Option<String>) {
        let kind = element.walked_kind();
        let own = (element.get_crossuuid(), kind);
        let mut cited = Cited::default();
        if self.alive.contains_key(&own) {
            cited.add(own, Cite::Own);
        }
        let alive = |identity: &Chain| identity.1 == kind && self.alive.contains_key(identity);
        if element.walked_side() == Side::Unknown {
            let base = base_crosscode(element.get_crosscode());
            if let Some(held) = (!base.is_empty()).then(|| self.bases.get(base)).flatten() {
                let mut live = held.iter().filter(|identity| alive(identity));
                if let (Some(identity), None) = (live.next(), live.next()) {
                    cited.add(*identity, Cite::Base);
                }
            }
        }
        // A chain an element split off the same message stands in - its
        // sibling's, never one it continues - and one of another instrument
        // are no chain it names.
        let sources = element.get_srcuuids();
        let instrument = element.walked_instcode();
        let joined = |identity: &Chain| {
            identity.1 == kind
                && self.alive.get(identity).is_some_and(|live| {
                    let live = &live.element;
                    (sources.is_empty()
                        || !live
                            .get_srcuuids()
                            .iter()
                            .any(|source| sources.contains(source)))
                        && instrument.is_none_or(|code| {
                            live.walked_instcode().is_none_or(|held| held == code)
                        })
                })
        };
        // The side a name is matched on wherever both state one: stating
        // none, the element cites every side the name is alive on.
        let slot = element.walked_slot();
        let on_side = |held: Side| slot == Side::Unknown || held == Side::Unknown || held == slot;
        for (scheme, name) in chain_names(element) {
            if cited.is_conflict() {
                break;
            }
            let Some(slots) = self.named.get(&scheme).and_then(|names| names.get(name)) else {
                continue;
            };
            // A slot holds at most one chain per category, side and
            // instrument going by the name.
            for (_, identity) in slots
                .iter()
                .filter(|(held, identity)| on_side(*held) && joined(identity))
            {
                cited.add(*identity, Cite::Name(scheme.clone(), name));
            }
        }
        match cited {
            Cited {
                first: Some((first, first_cite)),
                second: Some((second, second_cite)),
            } => {
                let chain = |identity: &Chain| {
                    let code = self
                        .alive
                        .get(identity)
                        .map_or("", |live| live.element.get_crosscode());
                    Spelled(code, identity.0)
                };
                let detail = format!(
                    "{} cites {} by {first_cite} and {} by {second_cite}",
                    Spelled(element.get_crosscode(), element.get_crossuuid()),
                    chain(&first),
                    chain(&second),
                );
                (own, Some(detail))
            }
            Cited {
                first: Some((first, _)),
                ..
            } => (first, None),
            Cited { first: None, .. } => (own, None),
        }
    }
}

impl<E, I> Iterator for EventIterator<E, I>
where
    E: Walked,
    I: Iterator<Item = E>,
{
    type Item = E;

    fn next(&mut self) -> Option<E> {
        self.fill_lookahead();
        loop {
            if let Some(snapshot) = self.next_snapshot_copy() {
                return Some(snapshot);
            }

            // The exact boundary is read after every source event at it.
            if let Some(unix) = self.after_group.take()
                && self.next_snapshot == Some(unix)
            {
                self.open_snapshot(unix);
                continue;
            }

            // An element stating no instant is read where it stands.
            if self
                .lookahead
                .as_ref()
                .is_some_and(|element| element.walked_transunix().is_none())
            {
                let element = self.lookahead.take();
                self.advance_source();
                return element;
            }
            let source_at = self.lookahead.as_ref().and_then(Walked::walked_transunix);
            if self.alive.is_empty() {
                self.next_snapshot = source_at.and_then(|unix| {
                    self.snapshot_ns()
                        .and_then(|step| grid_at_or_after(unix, step))
                });
            }
            let end = source_at.or_else(|| {
                self.expirations
                    .last()
                    .map(|(deadline, _)| *deadline)
                    .or(self.watermark)
            });
            let expiry =
                self.expirations
                    .first()
                    .copied()
                    .filter(|(deadline, _)| match source_at {
                        Some(source) => *deadline <= source,
                        None => end.is_some_and(|end| *deadline <= end),
                    });
            let snapshot = self.next_snapshot.filter(|snapshot| match source_at {
                Some(source) => *snapshot < source,
                None => end.is_some_and(|end| *snapshot <= end),
            });

            // Deadlines win ties with both a source event and a grid tick.
            if let Some((deadline, identity)) =
                expiry.filter(|(deadline, _)| snapshot.is_none_or(|snapshot| *deadline <= snapshot))
            {
                if let Some(expired) = self.expire(deadline, identity) {
                    return Some(expired);
                }
                continue;
            }
            if let Some(snapshot) = snapshot {
                self.open_snapshot(snapshot);
                continue;
            }

            let element = self.lookahead.take()?;
            self.advance_source();
            let Some(unix) = element.walked_transunix() else {
                return Some(element);
            };
            self.watermark = Some(self.watermark.map_or(unix, |held| held.max(unix)));
            if self.lookahead.as_ref().and_then(Walked::walked_transunix) != Some(unix) {
                self.after_group = Some(unix);
            }
            let mut element = element;
            if self.placing && element.is_walked() && !element.walked_made() {
                self.place(&mut element);
            }
            return Some(self.walk_source(element));
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let source = match &self.source {
            Source::Streamed(source) => source.size_hint().0,
            Source::Sorted(source) => source.size_hint().0,
        };
        (source + usize::from(self.lookahead.is_some()), None)
    }
}

impl<E, I> FusedIterator for EventIterator<E, I>
where
    E: Walked,
    I: FusedIterator<Item = E>,
{
}

/// The names a chain goes by in the walk's index, read off one element as
/// [`EventIterator::settle`] files them and [`EventIterator::identity_of`]
/// looks them up: each identifier of a type that names a chain
/// ([`IdType::is_chain_name`]: every type but one many elements share - a
/// match's `TrdMatchID`, a request many chains answer, a parent order's
/// slot) under its own type, matched by type and value whatever source
/// spelled it, and each a lineage field states - the chain's first value -
/// under the base it names too ([`Walked::walked_origin_of`]). Borrowed: a
/// type is a member or a word held inline, cloned for nothing.
fn chain_names<E: Walked>(element: &E) -> impl Iterator<Item = (IdType, &str)> {
    element
        .walked_identifiers()
        .into_iter()
        .flat_map(Identifiers::iter)
        .flat_map(move |id| {
            let kind = id.kind();
            kind.is_chain_name()
                .then(|| kind.clone())
                .into_iter()
                .chain(element.walked_origin_of(kind))
                .map(move |scheme| (scheme, id.value()))
        })
}

/// What named a chain an element may stand in.
enum Cite<'element> {
    /// The element's own cross element, alive.
    Own,
    /// The one live side of the element's base cross code.
    Base,
    /// A name the element goes by, under the type the index files it by.
    Name(IdType, &'element str),
}

impl std::fmt::Display for Cite<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Own => formatter.write_str("its cross code"),
            Self::Base => formatter.write_str("its base code"),
            Self::Name(scheme, name) => write!(formatter, "{scheme}={name}"),
        }
    }
}

/// The distinct live chains an element names, the first two kept with what
/// named each: a third is still two or more.
#[derive(Default)]
struct Cited<'element> {
    first: Option<(Chain, Cite<'element>)>,
    second: Option<(Chain, Cite<'element>)>,
}

impl<'element> Cited<'element> {
    /// Notes `chain` named by `cite`, where it is not one already named.
    fn add(&mut self, chain: Chain, cite: Cite<'element>) {
        match &self.first {
            None => self.first = Some((chain, cite)),
            Some((first, _)) if *first != chain && self.second.is_none() => {
                self.second = Some((chain, cite));
            }
            Some(_) => {}
        }
    }

    /// Whether two distinct chains were named.
    const fn is_conflict(&self) -> bool {
        self.second.is_some()
    }
}

/// A chain or an element as a conflict's detail names it: its stored cross
/// code, else - stating none - its cross element.
struct Spelled<'code>(&'code str, Uuid);

impl std::fmt::Display for Spelled<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_empty() {
            write!(formatter, "cross element {}", self.1)
        } else {
            formatter.write_str(self.0)
        }
    }
}

/// Stands `element` under the chain it stands in, `live` its live
/// statement: the live element's side where this one states none and the
/// chain is sided, the live element's stored cross code where it states one
/// and this one's differs, its cross hash and cross element derived from the
/// code and its identity settled once under them
/// ([`Walked::walked_follow_identity`], then one finalize where that moved
/// anything) - then the chain's cross element, which a chain whose first
/// element states no code is that element's identity, which no finalize of
/// another element derives, so every element after it - a follower stating
/// a code of its own included - is stated under it here. Where following
/// already forced the side and the code, it compares them and moves
/// nothing. Whether anything moved.
fn rekeyed<E: Walked>(element: &mut E, live: &E, identity: Chain) -> bool {
    let moved = element.walked_follow_identity(live);
    if moved {
        element.finalize();
    }
    if element.get_crossuuid() == identity.0 {
        return moved;
    }
    element.set_crossuuid(identity.0);
    true
}

/// States when `element`'s lifecycle was created where it states none: the
/// creation of the chain it stands in - `chain`, the earliest the fold kept -
/// else its own instant, since an element starting a chain is its creation.
/// A stated creation is never replaced, and the identity does not move: an
/// instant is no part of what an event digests.
fn created<E: Walked>(element: &mut E, chain: Option<i64>) {
    if element.walked_creaunix().is_some() {
        return;
    }
    if let Some(unix) = chain.or_else(|| element.walked_transunix()) {
        element.walked_set_creaunix(Some(unix));
    }
}

/// Whether an element can still be followed: its state can still change,
/// and it is not past its expiration.
fn is_alive<E: Walked>(element: &E) -> bool {
    element.walked_state().is_some_and(|state| state.is_live())
        && element.walked_exprunix().is_none_or(|expiration| {
            element
                .walked_transunix()
                .is_none_or(|unix| expiration > unix)
        })
}

/// The first epoch-aligned grid instant at or after `unix`, without an
/// intermediate multiplication that can overflow at either i64 extreme.
fn grid_at_or_after(unix: i64, step: i64) -> Option<i64> {
    let remainder = unix.rem_euclid(step);
    if remainder == 0 {
        Some(unix)
    } else {
        unix.checked_add(step - remainder)
    }
}

/// The elements' own order as a sort reads it: after is greater, before is
/// less, and neither is equal.
pub(crate) fn order<E: Element>(left: &E, right: &E) -> Ordering {
    if left.is_after(right) {
        Ordering::Greater
    } else if left.is_before(right) {
        Ordering::Less
    } else {
        Ordering::Equal
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/market/tests/graph/iterator.rs` pins and a caller cannot reach.
    //!
    //! The name index is how an event arriving under no live identity finds
    //! the chain it belongs to, and it is bookkeeping a caller only ever sees
    //! the result of: the walk hands out events, never the two maps it kept
    //! them by. Settling and retiring an identity directly is what pins that
    //! retiring a name forgets exactly the records it opened.
    use super::{EventIterator, Walked};
    use yggdryl::Uuid;

    /// Settle `element` as the element alive under `identity`, within its
    /// own category, arriving as `arrived`.
    pub fn settle<E, I>(walk: &mut EventIterator<E, I>, identity: Uuid, element: &E, arrived: Uuid)
    where
        E: Walked,
        I: Iterator<Item = E>,
    {
        walk.settle(
            (identity, element.walked_kind()),
            element,
            arrived,
            super::Fills::default(),
        );
    }

    /// Retire the element alive under `identity`, of whichever category,
    /// answering whether one was.
    pub fn retire<E, I>(walk: &mut EventIterator<E, I>, identity: Uuid) -> bool
    where
        E: Walked,
        I: Iterator<Item = E>,
    {
        let chain = walk
            .alive
            .keys()
            .copied()
            .find(|(held, _)| *held == identity);
        chain.is_some_and(|chain| walk.retire(chain).is_some())
    }

    /// How many schemes the walk currently indexes names under.
    pub fn named_schemes<E, I>(walk: &EventIterator<E, I>) -> usize {
        walk.named.len()
    }

    /// Every chain `scheme`/`name` is alive under, on any side and of any
    /// category, in the order they came to hold it: empty for a name the
    /// index files under none.
    pub fn named_chains<E, I>(
        walk: &EventIterator<E, I>,
        scheme: &crate::IdType,
        name: &str,
    ) -> Vec<Uuid> {
        walk.named
            .get(scheme)
            .and_then(|names| names.get(name))
            .map_or_else(Vec::new, |slots| {
                slots.iter().map(|(_, (identity, _))| *identity).collect()
            })
    }

    /// Stand `element` under the chain whose cross element is `identity`,
    /// of `live`'s category, `live` its live statement, as the walk does
    /// every element it states as a chain's; whether anything moved.
    pub fn rekeyed<E: Walked>(element: &mut E, live: &E, identity: Uuid) -> bool {
        super::rekeyed(element, live, (identity, live.walked_kind()))
    }

    /// How many identities hold a reverse record of the names they go by.
    pub fn named_identities<E, I>(walk: &EventIterator<E, I>) -> usize {
        walk.names_of.len()
    }

    /// How many reverse ownership records the walk holds in all.
    pub fn name_records<E, I>(walk: &EventIterator<E, I>) -> usize {
        walk.names_of.values().map(Vec::len).sum()
    }
}
