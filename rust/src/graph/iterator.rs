//! A walk over timed elements that states each one as the element after the
//! live one it follows.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::iter::FusedIterator;
use std::vec;

use super::{Element, Event};
use crate::Uuid;

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
/// [`State`](crate::State), and not
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
/// in. Either way the elements are cloned twice at most: once to hold the
/// live one, once to keep an element its predecessor refuses.
///
/// Given a grid - [`Self::with_snapshot_ns`], a step in nanoseconds aligned
/// on the epoch - the walk also reads one snapshot per step per identity:
/// the first element to reach a step its identity has not consumed is the
/// snapshot of that step, stamped with the step's opening instant as its
/// [`Event::get_snapunix`], and every later element in a step
/// already consumed is stamped with none. The steps consumed go with the
/// identity, so a chain that ended and started afresh reads its snapshots
/// afresh. Without a grid the walk leaves the snapshot instant as it came.
///
/// ```
/// use yggdryl::graph::{Element, EventIterator, Event, MarketEventData};
/// use yggdryl::State;
///
/// // One order's life as three events sharing its cross code, plus one
/// // event of another order: a market event orders by instant and follows
/// // by the timed reading.
/// let event = |order: &str, unix: i64, state: &str| {
///     let mut event = MarketEventData::at(unix);
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
    alive: HashMap<Uuid, Live<E>>,
    /// Every name a live element goes by, under the identity it is alive
    /// under: where an element arrives under no live identity, a name it
    /// shares with a live element is the chain it belongs to.
    named: HashMap<(String, String), Uuid>,
    /// The grid step in nanoseconds, or nothing positive for no grid.
    snapshot_ns: i64,
}

/// One identity's live element, the highest grid step it consumed, and
/// the identity it arrived under - what it was before the walk stated it,
/// which is what another statement of the same element still carries.
#[derive(Debug)]
struct Live<E> {
    element: E,
    step: Option<i64>,
    arrived: Uuid,
}

impl<E, I> EventIterator<E, I>
where
    E: Event + Clone,
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
            alive: HashMap::new(),
            named: HashMap::new(),
            snapshot_ns: 0,
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

    /// The opening instant of the grid step `element` falls in, where the
    /// walk has a grid.
    fn step_of(&self, element: &E) -> Option<i64> {
        let step = self.snapshot_ns()?;
        Some(element.get_currunix().div_euclid(step) * step)
    }

    /// Stamps `element` as the snapshot of its grid step where its identity
    /// has not consumed that step yet, and with no snapshot where it has;
    /// answers the highest step the identity has consumed after it.
    fn snapshot(&self, identity: Uuid, element: &mut E) -> Option<i64> {
        let consumed = self.alive.get(&identity).and_then(|live| live.step);
        let Some(step) = self.step_of(element) else {
            return consumed;
        };
        if consumed.is_none_or(|consumed| step > consumed) {
            element.set_snapunix(Some(step));
            Some(step)
        } else {
            element.set_snapunix(None);
            consumed
        }
    }

    /// Records `element` as the live one under `identity` where it is still
    /// alive, its names with it, and retires the identity where it is not.
    fn settle(&mut self, identity: Uuid, element: &E, step: Option<i64>, arrived: Uuid) {
        if is_alive(element) {
            for (scheme, name) in element.get_identifiers() {
                self.named.insert((scheme.clone(), name.clone()), identity);
            }
            self.alive.insert(
                identity,
                Live {
                    element: element.clone(),
                    step,
                    arrived,
                },
            );
        } else {
            self.named.retain(|_, held| *held != identity);
            self.alive.remove(&identity);
        }
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
            .get_identifiers()
            .iter()
            .find_map(|(scheme, name)| self.named.get(&(scheme.clone(), name.clone())).copied())
            .filter(|identity| self.alive.contains_key(identity))
            .unwrap_or(own)
    }
}

impl<E, I> Iterator for EventIterator<E, I>
where
    E: Event + Clone,
    I: Iterator<Item = E>,
{
    type Item = E;

    fn next(&mut self) -> Option<E> {
        let mut element = match &mut self.source {
            Source::Streamed(source) => source.next()?,
            Source::Sorted(source) => source.next()?,
        };
        let identity = self.identity_of(&element);
        let arrived = element.get_curruuid();
        let (element, step) = match self.alive.get(&identity) {
            // The live element read again: it restates the live one, and
            // the step the live one consumed is its own.
            Some(live) if live.arrived == arrived => {
                let step = live.step;
                (element.restating(&live.element), step)
            }
            Some(live) if element.is_before(&live.element) => return Some(element),
            Some(live) => {
                let mut element = element
                    .clone()
                    .with_previous(&live.element)
                    .unwrap_or(element);
                let step = self.snapshot(identity, &mut element);
                (element, step)
            }
            None => {
                let step = self.snapshot(identity, &mut element);
                (element, step)
            }
        };
        // Followed, the element carries the chain's cross code and stands
        // under the chain's identity; refused, it stands under its own.
        let identity = if self.alive.contains_key(&identity) {
            identity
        } else {
            element.get_crossuuid()
        };
        self.settle(identity, &element, step, arrived);
        Some(element)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.source {
            Source::Streamed(source) => source.size_hint(),
            Source::Sorted(source) => source.size_hint(),
        }
    }
}

impl<E, I> FusedIterator for EventIterator<E, I>
where
    E: Event + Clone,
    I: FusedIterator<Item = E>,
{
}

/// Whether an element can still be followed: its state can still change,
/// and it is not past its expiration.
fn is_alive<E: Event>(element: &E) -> bool {
    element.get_state().is_live()
        && element
            .get_expirunix()
            .is_none_or(|expiration| expiration > element.get_currunix())
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
