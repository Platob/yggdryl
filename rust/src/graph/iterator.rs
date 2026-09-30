//! A walk over timed elements that states each one as the element after the
//! live one it follows.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::iter::FusedIterator;
use std::vec;

use super::Element;
use super::element::InstantSequence;
use super::market::base_crosscode;
use crate::{IdType, Identifiers};
use crate::{MarketDataKind, Side, State, Uuid};

mod sealed {
    use super::super::{Element, Event, Market, MarketData, Operation};
    use crate::{IdType, Identifiers};
    use crate::{MarketDataKind, Side, State};

    /// [`Walked::walked_made`] over an event's own facts.
    fn made<E: Event + ?Sized>(event: &E) -> bool {
        event.get_snapunix().is_some()
            || (*event.get_state() == State::Expired
                && event.get_prevuuid().is_some()
                && event.get_exprunix() == Some(event.get_currunix()))
    }

    /// What a walk needs of an element beyond [`Element`]: sealed, so only
    /// `E: Event + Operation + Clone` and [`MarketData`] can name it. Every
    /// `E: Event + Operation + Clone` answers through its own traits; a
    /// `MarketData` answers through its four operation-event variants and
    /// is not walked otherwise, so any other variant is yielded as it came
    /// and never enters the live map.
    pub trait Walked: Element + Clone {
        /// [`Event::get_currunix`]; `None` for an element that states no
        /// instant, which the walk yields where it reads it.
        fn walked_currunix(&self) -> Option<i64>;
        /// The order a walk opened over unsorted elements sorts them by.
        fn walked_order(&self, other: &Self) -> std::cmp::Ordering;
        /// [`Event::set_currunix`].
        fn walked_set_currunix(&mut self, unix: i64);
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
        /// [`Event::set_recdunix`].
        fn walked_set_recdunix(&mut self, unix: Option<i64>);
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
        /// [`Operation::parent_of`]: the base a parent type names.
        fn walked_parent_of(&self, kind: &IdType) -> Option<IdType>;
        /// [`Event::restating`].
        fn walked_restating(self, live: &Self) -> Self;
        /// [`super::super::market::fill_execution`].
        fn walked_fill_execution(&mut self);
        /// Whether the walk chains this element at all.
        fn is_walked(&self) -> bool;
        /// [`Market::get_side`](super::super::Market::get_side);
        /// `Side::Unknown` for an element the walk does not chain.
        fn walked_side(&self) -> Side;
        /// [`Market::is_sided`](super::super::Market::is_sided): whether
        /// the element's cross code carries its side, so its base code is
        /// what an element stating no side joins it by.
        fn walked_sided(&self) -> bool;
        /// Whether the element may join another chain by a name or a base
        /// code it shares with it: every chained element but an execution,
        /// [`Event::is_execution`], which is a chain of its own and follows
        /// only its own cross code.
        fn walked_joins(&self) -> bool;
        /// [`Market::marketdatakind`](super::super::Market::marketdatakind):
        /// the category a chain holds elements of only.
        fn walked_kind(&self) -> MarketDataKind;
    }

    impl<E: Event + Operation + Clone> Walked for E {
        fn walked_currunix(&self) -> Option<i64> {
            Some(self.get_currunix())
        }
        fn walked_order(&self, other: &Self) -> std::cmp::Ordering {
            super::order(self, other)
        }
        fn walked_set_currunix(&mut self, unix: i64) {
            self.set_currunix(unix);
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
        fn walked_set_recdunix(&mut self, unix: Option<i64>) {
            self.set_recdunix(unix);
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
        fn walked_parent_of(&self, kind: &IdType) -> Option<IdType> {
            self.parent_of(kind).map(|(base, _)| base)
        }
        fn walked_restating(self, live: &Self) -> Self {
            self.restating(live)
        }
        fn walked_fill_execution(&mut self) {
            let _ = super::super::market::fill_execution(self);
        }
        fn is_walked(&self) -> bool {
            true
        }
        fn walked_side(&self) -> Side {
            self.get_side()
        }
        fn walked_sided(&self) -> bool {
            self.is_sided()
        }
        fn walked_joins(&self) -> bool {
            !self.is_execution()
        }
        fn walked_kind(&self) -> MarketDataKind {
            self.marketdatakind()
        }
    }

    impl Walked for MarketData {
        fn walked_currunix(&self) -> Option<i64> {
            self.as_event().map(|event| event.get_currunix())
        }
        /// By instant, an undated value first: a total order, where
        /// [`Element::is_after`] states none between an undated value and
        /// any other.
        fn walked_order(&self, other: &Self) -> std::cmp::Ordering {
            self.walked_currunix().cmp(&other.walked_currunix())
        }
        fn walked_set_currunix(&mut self, unix: i64) {
            if let Some(operation) = self.as_event_operation_mut() {
                operation.set_currunix(unix);
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
        fn walked_set_recdunix(&mut self, unix: Option<i64>) {
            if let Some(operation) = self.as_event_operation_mut() {
                operation.set_recdunix(unix);
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
        fn walked_parent_of(&self, kind: &IdType) -> Option<IdType> {
            self.as_event_operation()?
                .parent_of(kind)
                .map(|(base, _)| base)
        }
        /// Restates through the leaf's own [`Event::restating`]: a chain
        /// holds one category, so the live element is the same walked
        /// variant; else this statement stands as it is.
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
                (this, _) => this,
            }
        }
        fn walked_fill_execution(&mut self) {
            if let Some(operation) = self.as_event_operation_mut() {
                let _ = super::super::market::fill_execution(operation);
            }
        }
        fn is_walked(&self) -> bool {
            self.as_event_operation().is_some()
        }
        fn walked_side(&self) -> Side {
            self.as_event_operation()
                .map_or(Side::Unknown, |operation| operation.get_side())
        }
        fn walked_sided(&self) -> bool {
            self.is_walked() && self.is_sided()
        }
        fn walked_joins(&self) -> bool {
            self.as_event_operation()
                .is_some_and(|operation| !operation.is_execution())
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
/// fill never follows the order it filled. An element arriving under an
/// identity a live element of its category holds - or, where its own is
/// alive under nothing, under a name a live element of its category goes
/// by, so a report that spells only the `ClOrdID` a live order was placed
/// under still finds the order - is stated as the
/// one after it by its own [`Element::with_previous`], so it records its
/// predecessor, stands after it where they share an instant and carries the
/// lifecycle forward; what that answers is what the walk yields. The
/// yielded element then stands as the live one under that identity where
/// it is still alive, and retires it where it is not: a filled order, an
/// expired quote, ends its chain, and a later element under the same
/// identity starts one afresh.
///
/// Two elements never chain against their order. One that is
/// [`Element::is_before`] the live element - out of order on a walk the
/// caller called sorted - is yielded as it came and changes nothing; one the
/// element's own reading refuses, or that following changes nothing on, is
/// yielded as it came and still stands as the live one. An element under
/// no live identity is yielded as it came, and stands. What the walk yields
/// is always the caller's own copy: the live element is a clone the walk
/// keeps, never a reference into it.
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
/// element and not the one after it. It is yielded [restating](super::Event::restating)
/// the live one, so it takes the live one's predecessor and place and
/// finalizes to the same identity, and the chain grows by nothing.
///
/// Opened over elements the caller says are sorted, the walk reads them as
/// they come and yields each as it is read; over elements the caller does
/// not, it collects them first and sorts them by their own order, stably,
/// so two neither after nor before one another keep the order they arrived
/// in. Either way each source element is cloned twice at most: once to hold
/// the live one, once to keep an element its predecessor refuses. Expiration
/// and grid views are owned snapshots and necessarily clone their live event.
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
/// instant: dated at it - [`Event::get_currunix`](super::Event::get_currunix) and
/// [`Event::get_snapunix`](super::Event::get_snapunix) both - so it has the
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
/// `ExecutionEvent`, `TradeEvent`) chain, and every other variant is yielded
/// unchanged where it stands - a dated one at its instant, an undated one
/// where it is read - and never stands live.
///
/// ```
/// use yggdryl::graph::{Element, EventIterator, Event, OrderEvent};
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
/// assert_eq!((first.get_currunix(), first.get_seqnum(), first.get_prevuuid()), (10, 0, None));
/// let other = walk.next().expect("the other order's");
/// assert_eq!((other.get_crosscode(), other.get_seqnum()), ("10:0:O-900", 0));
/// let second = walk.next().expect("the partial fill");
/// assert_eq!((second.get_seqnum(), second.get_prevuuid()), (0, Some(first.get_curruuid())));
/// let filled = walk.next().expect("the fill");
/// assert_eq!((filled.get_seqnum(), filled.get_prevuuid()), (0, Some(second.get_curruuid())));
/// // A filled order ended its chain: the next event under its identity
/// // starts one afresh, and is alive beside the other order.
/// let again = walk.next().expect("the late one");
/// assert_eq!((again.get_seqnum(), again.get_prevuuid()), (0, None));
/// let mut alive = walk.alive().map(|held| (held.get_crosscode().to_owned(), held.get_currunix())).collect::<Vec<_>>();
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
/// assert_eq!((second.get_seqnum(), second.get_prevuuid()), (0, Some(first.get_curruuid())));
/// assert_eq!((twin.get_seqnum(), twin.get_prevuuid(), twin.get_curruuid()), (0, second.get_prevuuid(), second.get_curruuid()));
/// ```
#[derive(Debug)]
pub struct EventIterator<E, I> {
    source: Source<E, I>,
    lookahead: Option<E>,
    source_started: bool,
    alive: HashMap<Chain, Live<E>>,
    /// Every name a live element goes by, by scheme then name, under the
    /// identity it is alive under and the side it takes, one per side and
    /// category: where an element
    /// arrives under no live identity, a name it shares with a live element
    /// of its own side is the chain it belongs to, and an element stating no
    /// side joins the one side a name is alive on. Two levels, so a name is
    /// looked up by the borrowed scheme and name an element states and never
    /// by a copy of them.
    named: HashMap<IdType, HashMap<String, Vec<(Side, Chain)>>>,
    /// The live identities of each base cross code - the code without the
    /// prefix [`Market::stored_crosscode`](super::Market::stored_crosscode)
    /// gives it - of the sided elements stating a side, so an element
    /// stating no side joins the one side its code is alive on.
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
    /// The places the walk gives at each instant: its expirations always,
    /// and where it places what it reads, every source element too.
    sequence: InstantSequence,
    /// Whether the walk places the source elements it reads, rather than
    /// keeping the places they came with.
    placing: bool,
}

/// A chain the walk keeps alive: its cross element, within one market data
/// category - an order and an execution under one cross code are two
/// chains, and a lifecycle matches an element with a chain of its own
/// category only.
type Chain = (Uuid, MarketDataKind);

/// One identity's live element and the identity it arrived under - what it
/// was before the walk stated it, which is what another statement of the
/// same element still carries.
#[derive(Debug)]
struct Live<E> {
    element: E,
    arrived: Uuid,
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
            snapshot_ns: 0,
            next_snapshot: None,
            snapshot_at: None,
            snapshot_identities: Vec::new(),
            snapshot_index: 0,
            after_group: None,
            watermark: None,
            sequence: InstantSequence::default(),
            placing: false,
        }
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
        if let Some(unix) = element.walked_currunix() {
            let seqnum = self.sequence.place(unix, Some(element.get_currhashcode()));
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
    /// alive, its names with it, and retires the identity where it is not.
    fn settle(&mut self, identity: Chain, element: &E, arrived: Uuid) {
        if let Some(deadline) = self
            .alive
            .get(&identity)
            .and_then(|live| live.element.walked_exprunix())
        {
            self.expirations.remove(&(deadline, identity));
        }
        if is_alive(element) {
            let side = element.walked_side();
            for (scheme, name) in element
                .walked_identifiers()
                .into_iter()
                .flat_map(Identifiers::iter)
                .map(|id| (id.kind(), id.value()))
            {
                // Looked up borrowed first, as the bases are: a chain settled
                // again under the names it already goes by allocates nothing.
                let names = match self.named.get_mut(scheme) {
                    Some(names) => names,
                    None => self.named.entry(scheme.clone()).or_default(),
                };
                let slots = match names.get_mut(name) {
                    Some(slots) => slots,
                    None => names.entry(name.to_owned()).or_default(),
                };
                // One identity per side and category a name is alive on.
                let held = match slots
                    .iter_mut()
                    .find(|(held, chain)| *held == side && chain.1 == identity.1)
                {
                    Some(slot) => Some(std::mem::replace(&mut slot.1, identity)),
                    None => {
                        slots.push((side, identity));
                        None
                    }
                };
                if held == Some(identity) {
                    continue;
                }
                if let Some(held) = held {
                    let still = slots.iter().any(|(_, other)| *other == held);
                    if !still && let Some(known) = self.names_of.get_mut(&held) {
                        known.retain(|(held_scheme, held_name)| {
                            held_scheme != scheme || held_name.as_str() != name
                        });
                    }
                }
                let known = self.names_of.entry(identity).or_default();
                if !known
                    .iter()
                    .any(|(held_scheme, held_name)| held_scheme == scheme && held_name == name)
                {
                    known.push((scheme.clone(), name.to_owned()));
                }
            }
            let base = base_crosscode(element.get_crosscode());
            // Only a sided element stating a side is a side its base is
            // alive on.
            if element.walked_sided() && element.walked_side() != Side::Unknown {
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
            self.alive.insert(
                identity,
                Live {
                    element: element.clone(),
                    arrived,
                },
            );
            if let Some(deadline) = element.walked_exprunix() {
                self.expirations.insert((deadline, identity));
            }
        } else {
            self.retire(identity);
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
        if live.element.walked_sided()
            && live.element.walked_side() != Side::Unknown
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
                .walked_currunix()
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
                // the tick, which is its instant, nor a predecessor's.
                let mut snapshot = live.element.clone();
                let cross = snapshot.get_crossuuid();
                let original = snapshot
                    .walked_snapunix()
                    .or_else(|| snapshot.walked_currunix());
                snapshot.walked_set_currunix(unix);
                snapshot.walked_set_snapunix(original);
                snapshot.finalize();
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
        let previous = self.retire(identity)?.element;
        self.watermark = Some(self.watermark.map_or(deadline, |held| held.max(deadline)));
        let mut expired = previous.clone();
        expired.walked_set_currunix(deadline);
        expired.walked_set_state(State::Expired);
        expired.walked_set_execunix(None);
        expired.walked_set_recdunix(None);
        expired.walked_set_snapunix(None);
        // An expiration is an event of its own deadline, handed over before
        // anything the walk reads at it: it takes the next place the walk
        // gives there, and stands after the live one where that is later.
        expired.finalize();
        self.place(&mut expired);
        let fallback = expired.clone();
        Some(expired.with_previous(&previous).unwrap_or(fallback))
    }

    /// States one source element against the live generation it reaches.
    fn walk_source(&mut self, mut element: E) -> E {
        if !element.is_walked() {
            return element;
        }
        let identity = self.identity_of(&element);
        let arrived = element.get_curruuid();
        let mut element = match self.alive.get(&identity) {
            Some(live) if live.arrived == arrived => element.walked_restating(&live.element),
            Some(live) if element.is_before(&live.element) => {
                element.walked_fill_execution();
                created(&mut element, None);
                return element;
            }
            Some(live) => {
                // What the statement said, read before the fold: following
                // keeps the higher rank of the two, so a `NEW` over a live
                // `ACTIVE`, `RUNNING` or `REPLACED` no longer reads `NEW`.
                let stated_new = element.walked_state() == Some(&State::New);
                let mut element = match element.clone().with_previous(&live.element) {
                    Some(mut followed) => {
                        // A chain with no cross code is the chain of its
                        // first element, whose identity is its cross
                        // element: every element after it stands under that
                        // one cross element, not under its own identity.
                        if followed.get_crosshashcode() == 0 {
                            followed.set_crossuuid(live.element.get_crossuuid());
                        }
                        followed
                    }
                    None => element,
                };
                // A `NEW` stated over a live element that is itself new -
                // or carrying on, or restated - is that element updated.
                if stated_new
                    && live
                        .element
                        .walked_state()
                        .is_some_and(|held| held.is_new_like())
                {
                    element.walked_set_state(State::Updated);
                    element.finalize();
                }
                element
            }
            None => {
                element.walked_fill_execution();
                element
            }
        };
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
        self.settle(identity, &element, arrived);
        element
    }

    /// The live identity `element` belongs to: its own cross element where
    /// that is alive, else the identity of a live element of its side it
    /// shares a name with - an element that spells no chain identifier of
    /// its own but carries the `ClOrdID` a live order was placed under
    /// belongs to that order - else its own cross element, under which it
    /// starts a chain.
    ///
    /// Chains are keyed by side: the cross code carries the side, so a buy
    /// and a sell under one `ClOrdID` are two chains, and a name is alive on
    /// each side apart. An element stating no side joins the one side alive
    /// under its base cross code, else under the first name it shares with a
    /// live element, where exactly one side is; where both are, it starts a
    /// chain of its own. An execution joins nothing by a name or a base: it
    /// is a chain of its own, followed only under its own cross code. Every
    /// chain an element joins is one of its own category.
    fn identity_of(&self, element: &E) -> Chain {
        let kind = element.walked_kind();
        let own = (element.get_crossuuid(), kind);
        if self.alive.contains_key(&own) || !element.walked_joins() {
            return own;
        }
        let side = element.walked_side();
        let alive = |identity: &Chain| identity.1 == kind && self.alive.contains_key(identity);
        if side == Side::Unknown {
            let base = base_crosscode(element.get_crosscode());
            if let Some(held) = (!base.is_empty()).then(|| self.bases.get(base)).flatten() {
                let mut live = held.iter().filter(|identity| alive(identity));
                if let (Some(identity), None) = (live.next(), live.next()) {
                    return *identity;
                }
            }
        }
        // An element names a live chain by any of its identifiers' values,
        // and a parent identifier - a value its base held before - by its
        // value under that base too.
        // Read as they are walked: an element names any number of them.
        let names = element
            .walked_identifiers()
            .into_iter()
            .flat_map(Identifiers::iter)
            .flat_map(|id| {
                std::iter::once((id.kind().clone(), id.value())).chain(
                    element
                        .walked_parent_of(id.kind())
                        .map(|base| (base, id.value())),
                )
            });
        for (scheme, name) in names {
            let Some(slots) = self.named.get(&scheme).and_then(|names| names.get(name)) else {
                continue;
            };
            if side == Side::Unknown {
                let mut live = slots
                    .iter()
                    .map(|(_, identity)| identity)
                    .filter(|identity| alive(identity));
                match (live.next(), live.next()) {
                    (Some(identity), None) => return *identity,
                    (Some(_), Some(_)) => return own,
                    _ => {}
                }
            } else if let Some((_, identity)) = slots
                .iter()
                .find(|(held, identity)| *held == side && alive(identity))
            {
                return *identity;
            }
        }
        own
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
                .is_some_and(|element| element.walked_currunix().is_none())
            {
                let element = self.lookahead.take();
                self.advance_source();
                return element;
            }
            let source_at = self.lookahead.as_ref().and_then(Walked::walked_currunix);
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
            let Some(unix) = element.walked_currunix() else {
                return Some(element);
            };
            self.watermark = Some(self.watermark.map_or(unix, |held| held.max(unix)));
            if self.lookahead.as_ref().and_then(Walked::walked_currunix) != Some(unix) {
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

/// Whether an element can still be followed: its state can still change,
/// and it is not past its expiration.
/// States when `element`'s lifecycle was created where it states none: the
/// creation of the chain it stands in - `chain`, the earliest the fold kept -
/// else its own instant, since an element starting a chain is its creation.
/// A stated creation is never replaced, and the identity does not move: an
/// instant is no part of what an event digests.
fn created<E: Walked>(element: &mut E, chain: Option<i64>) {
    if element.walked_creaunix().is_some() {
        return;
    }
    if let Some(unix) = chain.or_else(|| element.walked_currunix()) {
        element.walked_set_creaunix(Some(unix));
    }
}

fn is_alive<E: Walked>(element: &E) -> bool {
    element.walked_state().is_some_and(|state| state.is_live())
        && element.walked_exprunix().is_none_or(|expiration| {
            element
                .walked_currunix()
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
    //! What `rust/tests/graph/iterator.rs` pins and a caller cannot reach.
    //!
    //! The name index is how an event arriving under no live identity finds
    //! the chain it belongs to, and it is bookkeeping a caller only ever sees
    //! the result of: the walk hands out events, never the two maps it kept
    //! them by. Settling and retiring an identity directly is what pins that
    //! retiring a name forgets exactly the records it opened.
    use super::{EventIterator, Walked};
    use crate::Uuid;

    /// Settle `element` as the element alive under `identity`, within its
    /// own category, arriving as `arrived`.
    pub fn settle<E, I>(walk: &mut EventIterator<E, I>, identity: Uuid, element: &E, arrived: Uuid)
    where
        E: Walked,
        I: Iterator<Item = E>,
    {
        walk.settle((identity, element.walked_kind()), element, arrived);
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

    /// The identity `scheme`/`name` currently looks up to, on the first
    /// side it is alive on.
    pub fn named_identity<E, I>(
        walk: &EventIterator<E, I>,
        scheme: &crate::IdType,
        name: &str,
    ) -> Option<Uuid> {
        walk.named
            .get(scheme)?
            .get(name)?
            .first()
            .map(|(_, (identity, _))| *identity)
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
