//! A walk over timed elements that states each one as the element after the
//! live one it follows.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::iter::FusedIterator;
use std::vec;

use super::{Element, Event, MarketOperationEvent};
use crate::{State, Uuid};

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
/// where it states no cross code. An element arriving under an identity a
/// live element holds - or, where its own is alive under nothing, under a
/// name a live element goes by, so a report that spells only the `ClOrdID`
/// a live order was placed under still finds the order - is stated as the
/// one after it by its own [`Element::with_previous`], so it records its
/// predecessor, takes the next place in the chain and carries the
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
/// An element arriving under the identity the live element *arrived*
/// under - the same instant, the same content: one message a capture
/// logged at every hop it passed - is another statement of the live
/// element and not the one after it. It is yielded [restating](Event::restating)
/// the live one, so it takes the place the live one holds in its chain and
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
/// that exact instant in the `95EXPIRED` state, then retires. Replacing or
/// ending its generation removes the old scheduled deadline. A deadline at
/// the same instant as source input is read first; after the source ends,
/// the finite deadlines still live are drained in their own order.
///
/// Given a grid - [`Self::with_snapshot_ns`], a step in nanoseconds aligned
/// on the epoch - the walk also yields an owned view of every living identity
/// at each crossed grid instant. A view carries that instant in
/// [`Event::get_snapunix`] and otherwise keeps the live event's identity and
/// content; it does not advance the chain. All source events at an exact
/// boundary are read before its views, while expirations at that boundary are
/// read before either. Source events keep the snapshot fact they stated.
/// At EOF the grid stops at the greatest remaining finite expiration, or at
/// the last source instant where nothing live expires, so a nonexpiring
/// identity never makes a finite source infinite. Without a grid the walk
/// leaves snapshot instants as they came.
///
/// ```
/// use yggdryl::graph::{Element, EventIterator, Event, MarketOperationEventData};
/// use yggdryl::State;
///
/// // One order's life as three events sharing its cross code, plus one
/// // event of another order: a market event orders by instant and follows
/// // by the timed reading.
/// let event = |order: &str, unix: i64, state: &str| {
///     let mut event = MarketOperationEventData::at(unix);
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
/// assert_eq!((other.get_crosscode(), other.get_seqnum()), ("O-900", 0));
/// let second = walk.next().expect("the partial fill");
/// assert_eq!((second.get_seqnum(), second.get_prevuuid()), (1, Some(first.get_curruuid())));
/// let filled = walk.next().expect("the fill");
/// assert_eq!((filled.get_seqnum(), filled.get_prevuuid()), (2, Some(second.get_curruuid())));
/// // A filled order ended its chain: the next event under its identity
/// // starts one afresh, and is alive beside the other order.
/// let again = walk.next().expect("the late one");
/// assert_eq!((again.get_seqnum(), again.get_prevuuid()), (0, None));
/// let mut alive = walk.alive().map(|held| (held.get_crosscode().to_owned(), held.get_currunix())).collect::<Vec<_>>();
/// alive.sort();
/// assert_eq!(alive, [("O-100".to_owned(), 40), ("O-900".to_owned(), 15)]);
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
/// assert_eq!((second.get_seqnum(), second.get_prevuuid()), (1, Some(first.get_curruuid())));
/// assert_eq!((twin.get_seqnum(), twin.get_prevuuid(), twin.get_curruuid()), (1, second.get_prevuuid(), second.get_curruuid()));
/// ```
#[derive(Debug)]
pub struct EventIterator<E, I> {
    source: Source<E, I>,
    lookahead: Option<E>,
    source_started: bool,
    alive: HashMap<Uuid, Live<E>>,
    /// Every name a live element goes by, by scheme then name, under the
    /// identity it is alive under: where an element arrives under no live
    /// identity, a name it shares with a live element is the chain it
    /// belongs to. Two levels, so a name is looked up by the borrowed
    /// scheme and name an element states and never by a copy of them.
    named: HashMap<String, HashMap<String, Uuid>>,
    /// The names each live identity is known by, so retiring it forgets
    /// exactly those.
    names_of: HashMap<Uuid, Vec<(String, String)>>,
    /// The one current finite deadline of each live identity, ordered so a
    /// deadline is retired before anything arriving at the same instant.
    expirations: BTreeSet<(i64, Uuid)>,
    /// The grid step in nanoseconds, or nothing positive for no grid.
    snapshot_ns: i64,
    /// The next epoch-aligned grid instant still to read.
    next_snapshot: Option<i64>,
    /// Identities copied at the active grid instant, one clone emitted at a
    /// time rather than a queue proportional to the number of ticks.
    snapshot_at: Option<i64>,
    snapshot_identities: Vec<Uuid>,
    snapshot_index: usize,
    /// A source instant whose whole equal-time group was just read. Its
    /// exact grid snapshot is owed after the group.
    after_group: Option<i64>,
    /// The latest source or emitted deadline reached. It bounds a grid at
    /// EOF after the last finite deadline is removed from the schedule.
    watermark: Option<i64>,
}

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
    E: MarketOperationEvent + Clone,
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
            collected.sort_by(order);
            Source::Sorted(collected.into_iter())
        };
        Self {
            source,
            lookahead: None,
            source_started: false,
            alive: HashMap::new(),
            named: HashMap::new(),
            names_of: HashMap::new(),
            expirations: BTreeSet::new(),
            snapshot_ns: 0,
            next_snapshot: None,
            snapshot_at: None,
            snapshot_identities: Vec::new(),
            snapshot_index: 0,
            after_group: None,
            watermark: None,
        }
    }

    /// The walk reading one snapshot per grid step of `snapshot_ns`
    /// nanoseconds per identity, the grid aligned on the epoch; a step of
    /// zero or less takes the grid away.
    #[must_use]
    pub const fn with_snapshot_ns(mut self, snapshot_ns: i64) -> Self {
        self.snapshot_ns = snapshot_ns;
        self
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
    fn settle(&mut self, identity: Uuid, element: &E, arrived: Uuid) {
        if let Some(deadline) = self
            .alive
            .get(&identity)
            .and_then(|live| live.element.get_exprtime())
        {
            self.expirations.remove(&(deadline, identity));
        }
        if is_alive(element) {
            for (scheme, name) in element.get_altids().iter() {
                let held = self
                    .named
                    .entry(scheme.to_owned())
                    .or_default()
                    .insert(name.to_owned(), identity);
                if held == Some(identity) {
                    continue;
                }
                if let Some(known) = held.and_then(|held| self.names_of.get_mut(&held)) {
                    known.retain(|(held_scheme, held_name)| {
                        held_scheme.as_str() != scheme || held_name.as_str() != name
                    });
                }
                self.names_of
                    .entry(identity)
                    .or_default()
                    .push((scheme.to_owned(), name.to_owned()));
            }
            self.alive.insert(
                identity,
                Live {
                    element: element.clone(),
                    arrived,
                },
            );
            if let Some(deadline) = element.get_exprtime() {
                self.expirations.insert((deadline, identity));
            }
        } else {
            self.retire(identity);
        }
    }

    /// Retires the live identity and every index and deadline it owns.
    fn retire(&mut self, identity: Uuid) -> Option<Live<E>> {
        let live = self.alive.remove(&identity)?;
        if let Some(deadline) = live.element.get_exprtime() {
            self.expirations.remove(&(deadline, identity));
        }
        for (scheme, name) in self.names_of.remove(&identity).unwrap_or_default() {
            let empty = self.named.get_mut(&scheme).is_some_and(|names| {
                if names.get(&name) == Some(&identity) {
                    names.remove(&name);
                }
                names.is_empty()
            });
            if empty {
                self.named.remove(&scheme);
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
            self.next_snapshot = grid_at_or_after(element.get_currunix(), step);
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
                let mut snapshot = live.element.clone();
                snapshot.set_snapunix(Some(unix));
                return Some(snapshot);
            }
        }
        self.snapshot_at = None;
        self.snapshot_identities.clear();
        None
    }

    /// Emits and retires the current generation whose deadline is next.
    fn expire(&mut self, deadline: i64, identity: Uuid) -> Option<E> {
        self.expirations.remove(&(deadline, identity));
        if self
            .alive
            .get(&identity)
            .and_then(|live| live.element.get_exprtime())
            != Some(deadline)
        {
            return None;
        }
        let previous = self.retire(identity)?.element;
        self.watermark = Some(self.watermark.map_or(deadline, |held| held.max(deadline)));
        let mut expired = previous.clone();
        expired.set_currunix(deadline);
        expired.set_state(State::read("expired").expect("the shipped expired state"));
        expired.set_execunix(None);
        expired.set_recdunix(None);
        expired.set_snapunix(None);
        expired.finalize();
        let fallback = expired.clone();
        Some(expired.with_previous(&previous).unwrap_or(fallback))
    }

    /// States one source element against the live generation it reaches.
    fn walk_source(&mut self, mut element: E) -> E {
        let identity = self.identity_of(&element);
        let arrived = element.get_curruuid();
        let element = match self.alive.get(&identity) {
            Some(live) if live.arrived == arrived => element.restating(&live.element),
            Some(live) if element.is_before(&live.element) => {
                super::element::fill_execution(&mut element);
                return element;
            }
            Some(live) => element
                .clone()
                .with_previous(&live.element)
                .unwrap_or(element),
            None => {
                super::element::fill_execution(&mut element);
                element
            }
        };
        let identity = if self.alive.contains_key(&identity) {
            identity
        } else {
            element.get_crossuuid()
        };
        self.settle(identity, &element, arrived);
        element
    }

    /// The live identity `element` belongs to: its own cross element where
    /// that is alive, else the identity of a live element it shares a name
    /// with - an element that spells no chain identifier of its own but
    /// carries the `ClOrdID` a live order was placed under belongs to that
    /// order - else its own cross element, under which it starts a chain.
    fn identity_of(&self, element: &E) -> Uuid {
        let own = element.get_crossuuid();
        if self.alive.contains_key(&own) {
            return own;
        }
        element
            .get_altids()
            .iter()
            .find_map(|(scheme, name)| {
                self.named
                    .get(scheme)
                    .and_then(|names| names.get(name))
                    .copied()
            })
            .filter(|identity| self.alive.contains_key(identity))
            .unwrap_or(own)
    }
}

impl<E, I> Iterator for EventIterator<E, I>
where
    E: MarketOperationEvent + Clone,
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
            if let Some(unix) = self.after_group.take() {
                if self.next_snapshot == Some(unix) {
                    self.open_snapshot(unix);
                    continue;
                }
            }

            let source_at = self.lookahead.as_ref().map(Event::get_currunix);
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
            let unix = element.get_currunix();
            self.watermark = Some(self.watermark.map_or(unix, |held| held.max(unix)));
            self.advance_source();
            if self.lookahead.as_ref().map(Event::get_currunix) != Some(unix) {
                self.after_group = Some(unix);
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
    E: MarketOperationEvent + Clone,
    I: FusedIterator<Item = E>,
{
}

/// Whether an element can still be followed: its state can still change,
/// and it is not past its expiration.
fn is_alive<E: Event>(element: &E) -> bool {
    element.get_state().is_live()
        && element
            .get_exprtime()
            .is_none_or(|expiration| expiration > element.get_currunix())
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
    use super::EventIterator;
    use crate::Uuid;
    use crate::graph::MarketOperationEvent;

    /// Settle `element` as the element alive under `identity`, arriving as
    /// `arrived`.
    pub fn settle<E, I>(walk: &mut EventIterator<E, I>, identity: Uuid, element: &E, arrived: Uuid)
    where
        E: MarketOperationEvent + Clone,
        I: Iterator<Item = E>,
    {
        walk.settle(identity, element, arrived);
    }

    /// Retire the element alive under `identity`, answering whether one was.
    pub fn retire<E, I>(walk: &mut EventIterator<E, I>, identity: Uuid) -> bool
    where
        E: MarketOperationEvent + Clone,
        I: Iterator<Item = E>,
    {
        walk.retire(identity).is_some()
    }

    /// How many schemes the walk currently indexes names under.
    pub fn named_schemes<E, I>(walk: &EventIterator<E, I>) -> usize {
        walk.named.len()
    }

    /// The identity `scheme`/`name` currently looks up to.
    pub fn named_identity<E, I>(
        walk: &EventIterator<E, I>,
        scheme: &str,
        name: &str,
    ) -> Option<Uuid> {
        walk.named.get(scheme)?.get(name).copied()
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
