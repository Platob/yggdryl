//! A walk over timed elements that states each one as the element after the
//! live one it follows.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::iter::FusedIterator;
use std::vec;

use super::{Element, TimeElement};
use crate::types::Uuid;

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
/// [`State`](crate::types::State), and not
/// past their expiration - under the identity every incarnation of one
/// thing shares: the cross element, or the element's own where it has none.
/// An element arriving under an identity a live element holds is stated as
/// the one after it by its own [`Element::with_previous`], so it records
/// its predecessor, takes the next place in the chain and carries the
/// lifecycle forward; what that answers is what the walk yields. The
/// yielded element then stands as the live one under that identity where
/// it is still alive, and retires it where it is not: a filled order, an
/// expired quote, ends its chain, and a later element under the same
/// identity starts one afresh.
///
/// Two elements never chain against their order. One that is
/// [`Element::is_before`] the live element - out of order on a walk the
/// caller called sorted - is yielded as it came and changes nothing; one the
/// element's own reading refuses is yielded as it came and still stands as
/// the live one. An element under no live identity is yielded as it came,
/// and stands.
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
/// [`TimeElement::get_snapshot_unix`], and every later element in a step
/// already consumed is stamped with none. The steps consumed go with the
/// identity, so a chain that ended and started afresh reads its snapshots
/// afresh. Without a grid the walk leaves the snapshot instant as it came.
///
/// ```
/// use std::collections::BTreeMap;
///
/// use yggdryl::graph::{Element, ElementIterator, TimeElement};
/// use yggdryl::types::{State, Uuid};
///
/// # #[derive(Clone)]
/// # struct Event {
/// #     uuid: Uuid, xuuid: Option<Uuid>, identifiers: BTreeMap<String, String>, parents: Vec<Uuid>,
/// #     unix: i128, hashcode: u64, xhashcode: u64, state: State, sequence_num: u64,
/// #     creation_unix: Option<i128>, expiration_unix: Option<i128>,
/// #     previous_unix: Option<i128>, previous_uuid: Option<Uuid>, snapshot_unix: Option<i128>,
/// # }
/// # impl Element for Event {
/// #     fn get_current_uuid(&self) -> Uuid { self.uuid }
/// #     fn set_current_uuid(&mut self, uuid: Uuid) { self.uuid = uuid; }
/// #     fn get_xuuid(&self) -> Option<Uuid> { self.xuuid }
/// #     fn set_xuuid(&mut self, xuuid: Option<Uuid>) { self.xuuid = xuuid; }
/// #     fn get_identifiers(&self) -> &BTreeMap<String, String> { &self.identifiers }
/// #     fn set_identifiers(&mut self, identifiers: BTreeMap<String, String>) { self.identifiers = identifiers; }
/// #     fn get_parentuuids(&self) -> &[Uuid] { &self.parents }
/// #     fn set_parentuuids(&mut self, parents: Vec<Uuid>) { self.parents = parents; }
/// #     fn is_after(&self, other: &Self) -> bool { self.unix > other.unix }
/// #     fn with_previous(self, previous: &Self) -> Option<Self> { self.following(previous) }
/// #     fn merge_with(self, other: &Self) -> Option<Self> { self.merging(other) }
/// # }
/// # impl TimeElement for Event {
/// #     fn get_unix(&self) -> i128 { self.unix }
/// #     fn set_unix(&mut self, unix: i128) { self.unix = unix; }
/// #     fn get_hashcode(&self) -> u64 { self.hashcode }
/// #     fn set_hashcode(&mut self, hashcode: u64) { self.hashcode = hashcode; }
/// #     fn get_xhashcode(&self) -> u64 { self.xhashcode }
/// #     fn set_xhashcode(&mut self, xhashcode: u64) { self.xhashcode = xhashcode; }
/// #     fn get_state(&self) -> &State { &self.state }
/// #     fn set_state(&mut self, state: State) { self.state = state; }
/// #     fn get_sequence_num(&self) -> u64 { self.sequence_num }
/// #     fn set_sequence_num(&mut self, sequence_num: u64) { self.sequence_num = sequence_num; }
/// #     fn get_creation_unix(&self) -> Option<i128> { self.creation_unix }
/// #     fn set_creation_unix(&mut self, unix: Option<i128>) { self.creation_unix = unix; }
/// #     fn get_expiration_unix(&self) -> Option<i128> { self.expiration_unix }
/// #     fn set_expiration_unix(&mut self, unix: Option<i128>) { self.expiration_unix = unix; }
/// #     fn get_previous_unix(&self) -> Option<i128> { self.previous_unix }
/// #     fn set_previous_unix(&mut self, unix: Option<i128>) { self.previous_unix = unix; }
/// #     fn get_previous_uuid(&self) -> Option<Uuid> { self.previous_uuid }
/// #     fn set_previous_uuid(&mut self, uuid: Option<Uuid>) { self.previous_uuid = uuid; }
/// #     fn get_snapshot_unix(&self) -> Option<i128> { self.snapshot_unix }
/// #     fn set_snapshot_unix(&mut self, unix: Option<i128>) { self.snapshot_unix = unix; }
/// # }
/// // One order's life as three events sharing its cross identity, plus one
/// // event of another order: `Event` implements the two traits, with
/// // `is_after` by instant and `with_previous` delegating to `following`.
/// let event = |uuid: u128, order: u128, unix: i128, state: &str| Event {
///     uuid: Uuid::from_v8(uuid),
///     xuuid: Some(Uuid::from_v8(order)),
///     state: State::from_spelling(state).expect("a shipped state"),
///     unix,
/// #   identifiers: BTreeMap::new(), parents: Vec::new(), hashcode: 0, xhashcode: 0, sequence_num: 0,
/// #   creation_unix: None, expiration_unix: None, previous_unix: None, previous_uuid: None,
/// #   snapshot_unix: None,
/// };
/// let arrived = vec![
///     event(3, 100, 30, "Filled"),
///     event(1, 100, 10, "New"),
///     event(9, 900, 15, "New"),
///     event(2, 100, 20, "PartiallyFilled"),
///     event(4, 100, 40, "New"),
/// ];
///
/// // Unsorted, the walk sorts by the elements' own order first.
/// let mut walk = ElementIterator::new(arrived, false);
/// let first = walk.next().expect("the earliest");
/// assert_eq!((first.get_current_uuid(), first.get_sequence_num(), first.get_previous_uuid()), (Uuid::from_v8(1), 0, None));
/// let other = walk.next().expect("the other order's");
/// assert_eq!((other.get_current_uuid(), other.get_sequence_num()), (Uuid::from_v8(9), 0));
/// let second = walk.next().expect("the partial fill");
/// assert_eq!((second.get_sequence_num(), second.get_previous_uuid()), (1, Some(Uuid::from_v8(1))));
/// let filled = walk.next().expect("the fill");
/// assert_eq!((filled.get_sequence_num(), filled.get_previous_uuid()), (2, Some(Uuid::from_v8(2))));
/// // A filled order ended its chain: the next event under its identity
/// // starts one afresh, and is alive beside the other order.
/// let again = walk.next().expect("the late one");
/// assert_eq!((again.get_sequence_num(), again.get_previous_uuid()), (0, None));
/// let mut alive = walk.alive().map(Element::get_current_uuid).collect::<Vec<_>>();
/// alive.sort();
/// assert_eq!(alive, [Uuid::from_v8(4), Uuid::from_v8(9)]);
/// assert!(walk.next().is_none());
/// ```
#[derive(Debug)]
pub struct ElementIterator<E, I> {
    source: Source<E, I>,
    alive: HashMap<Uuid, Live<E>>,
    /// The grid step in nanoseconds, or nothing positive for no grid.
    snapshot_ns: i128,
}

/// One identity's live element and the highest grid step it consumed.
#[derive(Debug)]
struct Live<E> {
    element: E,
    step: Option<i128>,
}

impl<E, I> ElementIterator<E, I>
where
    E: TimeElement + Clone,
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
            snapshot_ns: 0,
        }
    }

    /// The walk reading one snapshot per grid step of `snapshot_ns`
    /// nanoseconds per identity, the grid aligned on the epoch; a step of
    /// zero or less takes the grid away.
    #[must_use]
    pub const fn with_snapshot_ns(mut self, snapshot_ns: i128) -> Self {
        self.snapshot_ns = snapshot_ns;
        self
    }

    /// The grid step in nanoseconds, where the walk reads snapshots.
    #[must_use]
    pub fn snapshot_ns(&self) -> Option<i128> {
        (self.snapshot_ns > 0).then_some(self.snapshot_ns)
    }

    /// The elements still alive after what the walk has read so far, one
    /// per identity, in no order.
    pub fn alive(&self) -> impl Iterator<Item = &E> {
        self.alive.values().map(|live| &live.element)
    }

    /// The opening instant of the grid step `element` falls in, where the
    /// walk has a grid.
    fn step_of(&self, element: &E) -> Option<i128> {
        let step = self.snapshot_ns()?;
        Some(element.get_unix().div_euclid(step) * step)
    }

    /// Stamps `element` as the snapshot of its grid step where its identity
    /// has not consumed that step yet, and with no snapshot where it has;
    /// answers the highest step the identity has consumed after it.
    fn snapshot(&self, identity: Uuid, element: &mut E) -> Option<i128> {
        let consumed = self.alive.get(&identity).and_then(|live| live.step);
        let Some(step) = self.step_of(element) else {
            return consumed;
        };
        if consumed.is_none_or(|consumed| step > consumed) {
            element.set_snapshot_unix(Some(step));
            Some(step)
        } else {
            element.set_snapshot_unix(None);
            consumed
        }
    }

    /// Records `element` as the live one under `identity` where it is still
    /// alive, and retires the identity where it is not.
    fn settle(&mut self, identity: Uuid, element: &E, step: Option<i128>) {
        if is_alive(element) {
            self.alive.insert(
                identity,
                Live {
                    element: element.clone(),
                    step,
                },
            );
        } else {
            self.alive.remove(&identity);
        }
    }
}

impl<E, I> Iterator for ElementIterator<E, I>
where
    E: TimeElement + Clone,
    I: Iterator<Item = E>,
{
    type Item = E;

    fn next(&mut self) -> Option<E> {
        let element = match &mut self.source {
            Source::Streamed(source) => source.next()?,
            Source::Sorted(source) => source.next()?,
        };
        let identity = chain_identity(&element);
        let mut element = match self.alive.get(&identity) {
            Some(live) if element.is_before(&live.element) => return Some(element),
            Some(live) => element
                .clone()
                .with_previous(&live.element)
                .unwrap_or(element),
            None => element,
        };
        let step = self.snapshot(identity, &mut element);
        self.settle(identity, &element, step);
        Some(element)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.source {
            Source::Streamed(source) => source.size_hint(),
            Source::Sorted(source) => source.size_hint(),
        }
    }
}

impl<E, I> FusedIterator for ElementIterator<E, I>
where
    E: TimeElement + Clone,
    I: FusedIterator<Item = E>,
{
}

/// The identity every incarnation of one thing shares: the cross element,
/// or the element's own where it has none.
fn chain_identity<E: Element>(element: &E) -> Uuid {
    element
        .get_xuuid()
        .unwrap_or_else(|| element.get_current_uuid())
}

/// Whether an element can still be followed: its state can still change,
/// and it is not past its expiration.
fn is_alive<E: TimeElement>(element: &E) -> bool {
    element.get_state().is_live()
        && element
            .get_expiration_unix()
            .is_none_or(|expiration| expiration > element.get_unix())
}

/// The elements' own order as a sort reads it: after is greater, before is
/// less, and neither is equal.
fn order<E: Element>(left: &E, right: &E) -> Ordering {
    if left.is_after(right) {
        Ordering::Greater
    } else if left.is_before(right) {
        Ordering::Less
    } else {
        Ordering::Equal
    }
}
