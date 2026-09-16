//! An element of a graph, one that happened at an instant, and one that
//! happened in a market.
//!
//! Three traits: what an element answers about itself, what it takes, and
//! the two readings every element has - following another, and merging with
//! another statement of itself. The identity is the crate's own [`Uuid`], so
//! an element is addressed the way every identified value in the crate is,
//! and a parent, a predecessor or a cross element is named by the same
//! identity rather than by a reference, so an element can name one it does
//! not hold.

use std::collections::BTreeMap;

use crate::hashing::txhash::TxHash;
use crate::types::{Bloomberg, Cfi, Currency, Cusip, Isin, Mic, Sedol, Side, State, Uuid};
use crate::{Digest, DigestAlgorithm, Error, Result, TimeUnit};

/// One element of a graph: a node that knows its own identity, the identity
/// it has elsewhere, the names it goes by, and the identities of the
/// elements it descends from.
///
/// The parents are an ordered list, and the order is the implementor's to
/// state: a message's chain, a node's lineage. An element with no parent is
/// a root. The cross element, `xuuid`, is what this element is in another
/// graph - the same order in a venue's book and in a ledger - where it has
/// one. The identifiers are the names it goes by elsewhere, each under the
/// scheme that issued it - an order's `ClOrdID` and `OrderID`, a trade's
/// `ExecID` - held as text under text, so an element can be found by any
/// name a system gave it. Every fact is read and written through the trait,
/// so a store or a walk that only knows an element as `dyn Element` can
/// still place it; every accessor is `get_` and every mutator `set_`, so
/// the traits claim no bare name and an implementor keeps its own `uuid()`
/// or `state()` for whatever it means by them.
///
/// Three readings come with the facts. [`Self::is_after`] is the order the
/// implementor states between two elements - a timed element's instant, a
/// node's lineage - and [`Self::is_before`] its mirror, provided.
/// [`Self::with_previous`] states this element as the one after another,
/// and is the other signature the trait leaves to the implementor: what
/// following means is the element's own - a chain entry records its
/// predecessor, a snapshot its base - and [`TimeElement::following`] is the
/// reading a timed element delegates to. [`Self::merge_with`] folds another
/// statement of the same element into this one, and is provided: the cross
/// element is taken where this one states none, the identifiers this one
/// lacks are taken, and the parents are the union in this element's order,
/// then the other's. A timed element delegates to
/// [`TimeElement::merging`], which folds the instants and the state too.
///
/// ```
/// use std::collections::BTreeMap;
///
/// use yggdryl::graph::Element;
/// use yggdryl::types::Uuid;
///
/// struct Node {
///     uuid: Uuid,
///     xuuid: Option<Uuid>,
///     identifiers: BTreeMap<String, String>,
///     parents: Vec<Uuid>,
/// }
///
/// impl Node {
///     fn new(uuid: u128) -> Self {
///         Self { uuid: Uuid::from_v8(uuid), xuuid: None, identifiers: BTreeMap::new(), parents: Vec::new() }
///     }
/// }
///
/// impl Element for Node {
///     fn get_current_uuid(&self) -> Uuid {
///         self.uuid
///     }
///     fn set_current_uuid(&mut self, uuid: Uuid) {
///         self.uuid = uuid;
///     }
///     fn get_xuuid(&self) -> Option<Uuid> {
///         self.xuuid
///     }
///     fn set_xuuid(&mut self, xuuid: Option<Uuid>) {
///         self.xuuid = xuuid;
///     }
///     fn get_identifiers(&self) -> &BTreeMap<String, String> {
///         &self.identifiers
///     }
///     fn set_identifiers(&mut self, identifiers: BTreeMap<String, String>) {
///         self.identifiers = identifiers;
///     }
///     fn get_parentuuids(&self) -> &[Uuid] {
///         &self.parents
///     }
///     fn set_parentuuids(&mut self, parents: Vec<Uuid>) {
///         self.parents = parents;
///     }
///     // A node's order is its lineage: it is after the nodes it descends from.
///     fn is_after(&self, other: &Self) -> bool {
///         self.parents.contains(&other.get_current_uuid())
///     }
///     // A node follows another by descending from it, and never itself.
///     fn with_previous(mut self, previous: &Self) -> Option<Self> {
///         if previous.get_current_uuid() == self.get_current_uuid() {
///             return None;
///         }
///         self.parents = vec![previous.get_current_uuid()];
///         Some(self)
///     }
/// }
///
/// let root = Node::new(1);
/// let mut child = Node::new(2);
/// child.set_xuuid(Some(Uuid::from_v8(20)));
/// child.set_identifiers(BTreeMap::from([("ClOrdID".to_owned(), "C-1".to_owned())]));
/// assert!(child.get_parentuuids().is_empty(), "a node naming no parent is a root");
/// assert!(!child.is_after(&root) && !child.is_before(&root), "unrelated, so neither");
/// let child = child.with_previous(&root).expect("a node follows another");
/// assert_eq!(child.get_parentuuids(), [root.get_current_uuid()]);
/// assert!(child.is_after(&root) && root.is_before(&child));
/// assert_eq!(child.get_xuuid(), Some(Uuid::from_v8(20)));
/// // The implementor's rule: a node never follows itself.
/// assert!(Node::new(1).with_previous(&root).is_none());
///
/// // A second statement of the same node merges into the first: the cross
/// // element and the identifiers it states fill the ones the first left
/// // out, and the parents are the union in order. Another node does not
/// // merge at all.
/// let mut again = Node::new(2);
/// again.set_identifiers(BTreeMap::from([
///     ("ClOrdID".to_owned(), "other".to_owned()),
///     ("OrderID".to_owned(), "O-1".to_owned()),
/// ]));
/// again.set_parentuuids(vec![Uuid::from_v8(9), root.get_current_uuid()]);
/// let merged = child.merge_with(&again).expect("the same node");
/// assert_eq!(merged.get_xuuid(), Some(Uuid::from_v8(20)));
/// assert_eq!(merged.get_identifiers()["ClOrdID"], "C-1", "this element's word wins");
/// assert_eq!(merged.get_identifiers()["OrderID"], "O-1", "and what it lacked is taken");
/// assert_eq!(merged.get_parentuuids(), [root.get_current_uuid(), Uuid::from_v8(9)]);
/// assert!(merged.merge_with(&root).is_none());
/// ```
pub trait Element {
    /// This element's identity.
    fn get_current_uuid(&self) -> Uuid;

    /// Records this element's identity.
    fn set_current_uuid(&mut self, uuid: Uuid);

    /// The identity this element has in another graph - the cross element
    /// it is the same thing as, elsewhere - or nothing where it has none.
    fn get_xuuid(&self) -> Option<Uuid>;

    /// Records the identity this element has in another graph; `None`
    /// states it has none.
    fn set_xuuid(&mut self, xuuid: Option<Uuid>);

    /// The names this element goes by elsewhere, each under the scheme that
    /// issued it; empty where no system named it.
    fn get_identifiers(&self) -> &BTreeMap<String, String>;

    /// Records the names this element goes by elsewhere; the map is
    /// replaced whole.
    fn set_identifiers(&mut self, identifiers: BTreeMap<String, String>);

    /// The identities of the elements this one descends from, in the order
    /// the element states them; empty for a root.
    fn get_parentuuids(&self) -> &[Uuid];

    /// Records the identities of the elements this one descends from, in
    /// the given order; an empty list makes it a root.
    fn set_parentuuids(&mut self, parents: Vec<Uuid>);

    /// Whether this element comes after `other` in the order the implementor
    /// states: a timed element's instant, a node's lineage.
    ///
    /// With [`Self::is_before`], one strict weak order: an element is never
    /// after itself, and two that are neither after nor before each other
    /// are equal in it, which is what a sort of elements goes by.
    fn is_after(&self, other: &Self) -> bool
    where
        Self: Sized;

    /// Whether this element comes before `other`: the mirror of
    /// [`Self::is_after`], provided as `other.is_after(self)`.
    fn is_before(&self, other: &Self) -> bool
    where
        Self: Sized,
    {
        other.is_after(self)
    }

    /// This element stated as the one after `previous`, or nothing where it
    /// cannot follow it - its own predecessor, an element it precedes.
    ///
    /// Takes the element by value and answers it by value, so a chain is
    /// built by threading one element through the next.
    fn with_previous(self, previous: &Self) -> Option<Self>
    where
        Self: Sized;

    /// This element with another statement of itself folded in, or nothing
    /// where `other` is another element.
    ///
    /// Provided: the cross element is taken from `other` where this one
    /// states none, the identifiers this one lacks are taken from it, and
    /// the parents become the union - this element's in its order, then the
    /// ones only `other` names, in its. A timed element delegates to
    /// [`TimeElement::merging`], which folds the rest.
    fn merge_with(mut self, other: &Self) -> Option<Self>
    where
        Self: Sized,
    {
        if other.get_current_uuid() != self.get_current_uuid() {
            return None;
        }
        merge_element(&mut self, other);
        Some(self)
    }
}

/// The facts an element takes from another statement of itself: the cross
/// element it left out, the identifiers it lacks, and the parents it did
/// not name.
fn merge_element<E: Element + ?Sized>(this: &mut E, other: &E) {
    if this.get_xuuid().is_none() {
        this.set_xuuid(other.get_xuuid());
    }
    take_identifiers(this, other);
    let mut parents = this.get_parentuuids().to_vec();
    for parent in other.get_parentuuids() {
        if !parents.contains(parent) {
            parents.push(*parent);
        }
    }
    this.set_parentuuids(parents);
}

/// The identifiers `other` knows and `this` does not, taken; the ones this
/// element states are its own word and stay.
fn take_identifiers<E: Element + ?Sized>(this: &mut E, other: &E) {
    if other
        .get_identifiers()
        .keys()
        .all(|scheme| this.get_identifiers().contains_key(scheme))
    {
        return;
    }
    let mut identifiers = this.get_identifiers().clone();
    for (scheme, identifier) in other.get_identifiers() {
        identifiers
            .entry(scheme.clone())
            .or_insert_with(|| identifier.clone());
    }
    this.set_identifiers(identifiers);
}

/// The instant and the code of one element coupled: a [`TxHash`] of the
/// instant at nanosecond resolution and the code as the XXH3-64 digest it
/// is, which is the crate's own time-ordered identity.
fn coupled(unix: i128, hashcode: u64) -> Result<TxHash> {
    let unix = i64::try_from(unix).map_err(|_| Error::ArithmeticOverflow {
        operation: "graph element instant",
        kind: "int64",
    })?;
    TxHash::new_in(
        unix,
        TimeUnit::Nanosecond,
        Digest::new(DigestAlgorithm::Xxh3, u128::from(hashcode)),
    )
}

/// The earlier of two optional instants, or whichever is stated.
fn earliest(left: Option<i128>, right: Option<i128>) -> Option<i128> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (left, right) => left.or(right),
    }
}

/// The later of two optional instants, or whichever is stated.
fn latest(left: Option<i128>, right: Option<i128>) -> Option<i128> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left, right) => left.or(right),
    }
}

/// An element that happened at one instant and digests to one code.
///
/// The instant is `unix`: a count of nanoseconds since the Unix epoch, UTC,
/// held as an `i128` so no clock this crate reads is out of its range. The
/// code is `hashcode`: the XXH3-64 digest the element's content answers to,
/// held beside the element rather than recomputed, exactly as a message's
/// `msghash` is a column of the row. Coupled, the two are the element's
/// identity: [`Self::txhash`] is the crate's own [`TxHash`] of them, and
/// [`Self::time_uuid`] the UUID it answers - RFC 9562 UUIDv7 with the
/// instant in front, so identities sort by instant first, to the
/// microsecond, and by content second - which is what an implementor's
/// [`Element::get_current_uuid`] answers where the element's identity is
/// when it happened and what it says. The cross
/// element is derived the same way from what it is: `xhashcode`, its code,
/// coupled with `creation_unix`, when it was created, by
/// [`Self::time_xuuid`].
///
/// Where the element stands is its [`State`], the crate's ranked lifecycle
/// code, and every timed element has one: an element that reached no state
/// says so with the code that means exactly that, `00UNKNOWN`, never with an
/// absence. Where it stands in its chain is `sequence_num`: the count of
/// elements before it, which following increments and merging keeps the
/// highest of. Four more instants and one more identity are optional, because
/// an element states them only where it knows them: when it was created and
/// when it expires, each an instant in the same count; the element it
/// follows - `previous_uuid` and `previous_unix`, the predecessor's identity
/// and instant; and `snapshot_unix`, the grid instant this element was read
/// as the snapshot of, where a walk over a grid took one of it.
///
/// Two readings are provided. [`Self::following`] is what following means
/// for a timed element, which an implementor's [`Element::with_previous`]
/// delegates to; [`Self::merging`] is what merging means, which its
/// [`Element::merge_with`] delegates to. Both fold the lifecycle the same
/// way: the earliest creation, the latest expiration, the furthest state,
/// and the identifiers the other knew. The order a timed element states
/// through [`Element::is_after`] is its instant: later is after.
///
/// ```
/// use std::collections::BTreeMap;
///
/// use yggdryl::graph::{Element, TimeElement};
/// use yggdryl::types::{State, Uuid};
///
/// struct Event {
///     uuid: Uuid,
///     xuuid: Option<Uuid>,
///     identifiers: BTreeMap<String, String>,
///     parents: Vec<Uuid>,
///     unix: i128,
///     hashcode: u64,
///     xhashcode: u64,
///     state: State,
///     sequence_num: u64,
///     creation_unix: Option<i128>,
///     expiration_unix: Option<i128>,
///     previous_unix: Option<i128>,
///     previous_uuid: Option<Uuid>,
///     snapshot_unix: Option<i128>,
/// }
///
/// impl Event {
///     fn at(uuid: u128, unix: i128) -> Self {
///         Self {
///             uuid: Uuid::from_v8(uuid),
///             xuuid: None,
///             identifiers: BTreeMap::new(),
///             parents: Vec::new(),
///             unix,
///             hashcode: 0,
///             xhashcode: 0,
///             state: State::from_spelling("New").expect("a shipped state"),
///             sequence_num: 0,
///             creation_unix: None,
///             expiration_unix: None,
///             previous_unix: None,
///             previous_uuid: None,
///             snapshot_unix: None,
///         }
///     }
/// }
///
/// impl Element for Event {
///     fn get_current_uuid(&self) -> Uuid {
///         self.uuid
///     }
///     fn set_current_uuid(&mut self, uuid: Uuid) {
///         self.uuid = uuid;
///     }
///     fn get_xuuid(&self) -> Option<Uuid> {
///         self.xuuid
///     }
///     fn set_xuuid(&mut self, xuuid: Option<Uuid>) {
///         self.xuuid = xuuid;
///     }
///     fn get_identifiers(&self) -> &BTreeMap<String, String> {
///         &self.identifiers
///     }
///     fn set_identifiers(&mut self, identifiers: BTreeMap<String, String>) {
///         self.identifiers = identifiers;
///     }
///     fn get_parentuuids(&self) -> &[Uuid] {
///         &self.parents
///     }
///     fn set_parentuuids(&mut self, parents: Vec<Uuid>) {
///         self.parents = parents;
///     }
///     // A timed element's order is its instant.
///     fn is_after(&self, other: &Self) -> bool {
///         self.unix > other.unix
///     }
///     // The timed readings, which the subtrait provides.
///     fn with_previous(self, previous: &Self) -> Option<Self> {
///         self.following(previous)
///     }
///     fn merge_with(self, other: &Self) -> Option<Self> {
///         self.merging(other)
///     }
/// }
///
/// impl TimeElement for Event {
///     fn get_unix(&self) -> i128 {
///         self.unix
///     }
///     fn set_unix(&mut self, unix: i128) {
///         self.unix = unix;
///     }
///     fn get_hashcode(&self) -> u64 {
///         self.hashcode
///     }
///     fn set_hashcode(&mut self, hashcode: u64) {
///         self.hashcode = hashcode;
///     }
///     fn get_xhashcode(&self) -> u64 {
///         self.xhashcode
///     }
///     fn set_xhashcode(&mut self, xhashcode: u64) {
///         self.xhashcode = xhashcode;
///     }
///     fn get_state(&self) -> &State {
///         &self.state
///     }
///     fn set_state(&mut self, state: State) {
///         self.state = state;
///     }
///     fn get_sequence_num(&self) -> u64 {
///         self.sequence_num
///     }
///     fn set_sequence_num(&mut self, sequence_num: u64) {
///         self.sequence_num = sequence_num;
///     }
///     fn get_creation_unix(&self) -> Option<i128> {
///         self.creation_unix
///     }
///     fn set_creation_unix(&mut self, unix: Option<i128>) {
///         self.creation_unix = unix;
///     }
///     fn get_expiration_unix(&self) -> Option<i128> {
///         self.expiration_unix
///     }
///     fn set_expiration_unix(&mut self, unix: Option<i128>) {
///         self.expiration_unix = unix;
///     }
///     fn get_previous_unix(&self) -> Option<i128> {
///         self.previous_unix
///     }
///     fn set_previous_unix(&mut self, unix: Option<i128>) {
///         self.previous_unix = unix;
///     }
///     fn get_previous_uuid(&self) -> Option<Uuid> {
///         self.previous_uuid
///     }
///     fn set_previous_uuid(&mut self, uuid: Option<Uuid>) {
///         self.previous_uuid = uuid;
///     }
///     fn get_snapshot_unix(&self) -> Option<i128> {
///         self.snapshot_unix
///     }
///     fn set_snapshot_unix(&mut self, unix: Option<i128>) {
///         self.snapshot_unix = unix;
///     }
/// }
///
/// let mut first = Event::at(1, 10_000);
/// first.set_creation_unix(Some(5_000));
/// first.set_identifiers(BTreeMap::from([("ClOrdID".to_owned(), "C-1".to_owned())]));
/// let second = Event::at(2, 20_000);
/// assert!(second.is_after(&first) && first.is_before(&second));
/// let second = second.with_previous(&first).expect("the later one follows");
/// assert_eq!(second.get_previous_uuid(), Some(first.get_current_uuid()));
/// assert_eq!(second.get_previous_unix(), Some(10_000));
/// // Following carries the lifecycle forward: the earliest creation known,
/// // the names the predecessor went by, and the next place in the chain.
/// assert_eq!(second.get_creation_unix(), Some(5_000));
/// assert_eq!(second.get_identifiers()["ClOrdID"], "C-1");
/// assert_eq!(second.get_sequence_num(), 1);
/// // An element follows neither itself nor one that happened after it.
/// assert!(Event::at(1, 10_000).with_previous(&first).is_none());
/// assert!(Event::at(3, 5_000).with_previous(&second).is_none());
/// // A time element is an element: one walk reads both.
/// let held: &dyn TimeElement = &second;
/// assert_eq!(held.get_current_uuid(), Uuid::from_v8(2));
/// assert_eq!(held.get_unix(), 20_000);
/// assert_eq!(held.get_creation_unix(), Some(5_000));
/// assert!(held.get_state().is_live());
/// // The identity its instant and code derive: later sorts later.
/// let earlier = first.time_uuid().expect("an instant a TxHash holds");
/// let later = second.time_uuid().expect("an instant a TxHash holds");
/// assert!(earlier < later);
/// assert_eq!(first.txhash().expect("a TxHash").unix(), 10_000);
/// // And the cross identity its creation and cross code derive, where it
/// // knows when it was created.
/// assert!(first.time_xuuid().expect("an instant a TxHash holds").is_some());
/// assert!(Event::at(4, 40).time_xuuid().expect("nothing to couple").is_none());
/// ```
pub trait TimeElement: Element {
    /// When this element happened: nanoseconds since the Unix epoch, UTC.
    fn get_unix(&self) -> i128;

    /// Records when this element happened, as nanoseconds since the Unix
    /// epoch, UTC.
    fn set_unix(&mut self, unix: i128);

    /// The code this element's content digests to.
    fn get_hashcode(&self) -> u64;

    /// Records the code this element's content digests to.
    fn set_hashcode(&mut self, hashcode: u64);

    /// The code the cross element's content digests to: what this element
    /// is in another graph, as a digest.
    fn get_xhashcode(&self) -> u64;

    /// Records the code the cross element's content digests to.
    fn set_xhashcode(&mut self, xhashcode: u64);

    /// Where this element stands in its lifecycle: the crate's ranked
    /// [`State`] code, never absent - an element that reached no state says
    /// `00UNKNOWN`.
    fn get_state(&self) -> &State;

    /// Records where this element stands in its lifecycle.
    fn set_state(&mut self, state: State);

    /// Where this element stands in its chain: how many came before it.
    fn get_sequence_num(&self) -> u64;

    /// Records where this element stands in its chain.
    fn set_sequence_num(&mut self, sequence_num: u64);

    /// When this element was created, in the same count as [`Self::get_unix`],
    /// where it knows.
    fn get_creation_unix(&self) -> Option<i128>;

    /// Records when this element was created; `None` states it does not
    /// know.
    fn set_creation_unix(&mut self, unix: Option<i128>);

    /// When this element expires, in the same count as [`Self::get_unix`], where
    /// it has an expiry.
    fn get_expiration_unix(&self) -> Option<i128>;

    /// Records when this element expires; `None` states it does not expire,
    /// or does not know.
    fn set_expiration_unix(&mut self, unix: Option<i128>);

    /// When the element this one follows happened, where it follows one.
    fn get_previous_unix(&self) -> Option<i128>;

    /// Records when the element this one follows happened; `None` states it
    /// follows none.
    fn set_previous_unix(&mut self, unix: Option<i128>);

    /// The identity of the element this one follows, where it follows one.
    fn get_previous_uuid(&self) -> Option<Uuid>;

    /// Records the identity of the element this one follows; `None` states
    /// it follows none.
    fn set_previous_uuid(&mut self, uuid: Option<Uuid>);

    /// The grid instant this element was read as the snapshot of, in the
    /// same count as [`Self::get_unix`], where a walk over a grid took one
    /// of it: the opening instant of the grid step its instant fell in.
    fn get_snapshot_unix(&self) -> Option<i128>;

    /// Records the grid instant this element is the snapshot of; `None`
    /// states no snapshot was taken of it.
    fn set_snapshot_unix(&mut self, unix: Option<i128>);

    /// This element stated as the one after `previous`, by the timed
    /// reading, or nothing where it cannot follow - its own predecessor, or
    /// one that happened after it.
    ///
    /// The predecessor's identity and instant are recorded on it, its place
    /// in the chain is the one after the predecessor's, and the lifecycle
    /// carries forward: the creation is the earliest the two know, the
    /// expiration the latest, the state the furthest along, and each where
    /// only one states it is that one's; the names the predecessor went by
    /// and this element does not state are taken, because a name issued at
    /// creation holds for the whole lifecycle. What the element itself
    /// says - its instant, its codes, its parents, its cross element, the
    /// snapshot it is - is its own and moves nowhere.
    ///
    /// Provided, so an implementor's [`Element::with_previous`] has a
    /// default to delegate to; an element that means something else by
    /// following states its own there instead.
    fn following(mut self, previous: &Self) -> Option<Self>
    where
        Self: Sized,
    {
        if previous.get_current_uuid() == self.get_current_uuid()
            || previous.get_unix() > self.get_unix()
        {
            return None;
        }
        self.set_previous_uuid(Some(previous.get_current_uuid()));
        self.set_previous_unix(Some(previous.get_unix()));
        self.set_sequence_num(previous.get_sequence_num().saturating_add(1));
        take_identifiers(&mut self, previous);
        self.fold_lifecycle(previous);
        Some(self)
    }

    /// This element with another statement of itself folded in, by the
    /// timed reading, or nothing where `other` is another element.
    ///
    /// The element-level merge first - the cross element where this one
    /// states none, the parents' union - and then the timed facts: the
    /// instant is the later of the two and the codes are the later
    /// statement's, because the last word on what an element says is the
    /// latest one; the place in the chain is the further of the two; the
    /// lifecycle folds as [`Self::following`] folds it - earliest creation,
    /// latest expiration, furthest state; and the predecessor and the
    /// snapshot instant are this element's where it states them, else the
    /// other's.
    ///
    /// Provided, so an implementor's [`Element::merge_with`] has a default
    /// to delegate to.
    fn merging(mut self, other: &Self) -> Option<Self>
    where
        Self: Sized,
    {
        if other.get_current_uuid() != self.get_current_uuid() {
            return None;
        }
        merge_element(&mut self, other);
        if other.get_unix() > self.get_unix() {
            self.set_unix(other.get_unix());
            self.set_hashcode(other.get_hashcode());
            self.set_xhashcode(other.get_xhashcode());
        }
        if other.get_sequence_num() > self.get_sequence_num() {
            self.set_sequence_num(other.get_sequence_num());
        }
        self.fold_lifecycle(other);
        if self.get_previous_uuid().is_none() {
            self.set_previous_uuid(other.get_previous_uuid());
            self.set_previous_unix(other.get_previous_unix());
        }
        if self.get_snapshot_unix().is_none() {
            self.set_snapshot_unix(other.get_snapshot_unix());
        }
        Some(self)
    }

    /// Folds another element's lifecycle into this one: the earliest
    /// creation, the latest expiration, the furthest state.
    ///
    /// Provided, and what [`Self::following`] and [`Self::merging`] share.
    fn fold_lifecycle(&mut self, other: &Self)
    where
        Self: Sized,
    {
        self.set_creation_unix(earliest(
            self.get_creation_unix(),
            other.get_creation_unix(),
        ));
        self.set_expiration_unix(latest(
            self.get_expiration_unix(),
            other.get_expiration_unix(),
        ));
        if other.get_state() > self.get_state() {
            self.set_state(other.get_state().clone());
        }
    }

    /// The instant and the code coupled: a [`TxHash`] of [`Self::get_unix`] at
    /// nanosecond resolution and [`Self::get_hashcode`] as the XXH3-64 digest it
    /// is, which is the crate's own time-ordered identity.
    ///
    /// Provided, so every timed element derives it the same way.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ArithmeticOverflow`] where the instant does not fit
    /// the signed 64-bit nanoseconds a `TxHash` holds.
    fn txhash(&self) -> Result<TxHash> {
        coupled(self.get_unix(), self.get_hashcode())
    }

    /// The identity the instant and the code derive: the UUID
    /// [`TxHash::into_uuid`] answers for [`Self::txhash`], RFC 9562 UUIDv7
    /// with the instant in front so identities sort by instant first, to
    /// the microsecond, and by content second.
    ///
    /// Provided: an implementor whose identity is when it happened and what
    /// it says answers this from [`Element::get_current_uuid`], and one whose identity
    /// is assigned keeps its own.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::txhash`] returns.
    fn time_uuid(&self) -> Result<Uuid> {
        self.txhash()?.into_uuid()
    }

    /// The cross identity the creation and the cross code derive: the same
    /// coupling as [`Self::time_uuid`], of [`Self::get_creation_unix`] with
    /// [`Self::get_xhashcode`], or nothing where the element does not know when
    /// it was created.
    ///
    /// Provided: an implementor whose cross element is its creation answers
    /// this from [`Element::get_xuuid`].
    ///
    /// # Errors
    ///
    /// Returns what [`Self::txhash`] returns, for the creation instant.
    fn time_xuuid(&self) -> Result<Option<Uuid>> {
        self.get_creation_unix()
            .map(|unix| coupled(unix, self.get_xhashcode())?.into_uuid())
            .transpose()
    }
}

/// An element that happened in a market: a price, a quantity, and which
/// side of the market it stood on.
///
/// Five facts beside what a timed element already states, each read and
/// written: `px` is the price and `currency` the [`Currency`] it is quoted
/// in; `qty` is the quantity and `unit` the text it is counted in - a lot, a
/// barrel, a megawatt-hour, whatever the market says; `side` is the crate's
/// [`Side`] code, FIX's `Side(54)`. Six more name the instrument and the
/// market, each optional because a market names an instrument the way it
/// does: the [`Isin`], the [`Cusip`], the [`Sedol`], the [`Bloomberg`]
/// identifier, the [`Cfi`] classification and the [`Mic`] of the market it
/// traded on, the crate's own validated codes. The traits state signatures
/// and nothing else: what a price of nothing or a quantity of zero means is
/// the market's to say, and following and merging are the timed readings
/// unchanged.
///
/// ```
/// use std::collections::BTreeMap;
///
/// use yggdryl::graph::{Element, MarketElement, TimeElement};
/// use yggdryl::types::{Bloomberg, Cfi, Currency, Cusip, Isin, Mic, Sedol, Side, State, Uuid};
///
/// struct Trade {
///     uuid: Uuid,
///     unix: i128,
///     hashcode: u64,
///     px: f64,
///     currency: Currency,
///     qty: f64,
///     unit: String,
///     side: Side,
///     isincode: Option<Isin>,
/// #   cusipcode: Option<Cusip>,
/// #   sedolcode: Option<Sedol>,
/// #   bloombergcode: Option<Bloomberg>,
/// #   cficode: Option<Cfi>,
/// #   miccode: Option<Mic>,
/// #   xuuid: Option<Uuid>,
/// #   identifiers: BTreeMap<String, String>,
/// #   parents: Vec<Uuid>,
/// #   xhashcode: u64,
/// #   state: State,
/// #   sequence_num: u64,
/// #   creation_unix: Option<i128>,
/// #   expiration_unix: Option<i128>,
/// #   previous_unix: Option<i128>,
/// #   previous_uuid: Option<Uuid>,
/// #   snapshot_unix: Option<i128>,
/// }
/// # impl Element for Trade {
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
/// # impl TimeElement for Trade {
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
///
/// impl MarketElement for Trade {
///     fn get_px(&self) -> f64 {
///         self.px
///     }
///     fn set_px(&mut self, px: f64) {
///         self.px = px;
///     }
///     fn get_currency(&self) -> &Currency {
///         &self.currency
///     }
///     fn set_currency(&mut self, currency: Currency) {
///         self.currency = currency;
///     }
///     fn get_qty(&self) -> f64 {
///         self.qty
///     }
///     fn set_qty(&mut self, qty: f64) {
///         self.qty = qty;
///     }
///     fn get_unit(&self) -> &str {
///         &self.unit
///     }
///     fn set_unit(&mut self, unit: String) {
///         self.unit = unit;
///     }
///     fn get_side(&self) -> &Side {
///         &self.side
///     }
///     fn set_side(&mut self, side: Side) {
///         self.side = side;
///     }
///     fn get_isincode(&self) -> Option<&Isin> {
///         self.isincode.as_ref()
///     }
///     fn set_isincode(&mut self, isincode: Option<Isin>) {
///         self.isincode = isincode;
///     }
/// #   fn get_cusipcode(&self) -> Option<&Cusip> { self.cusipcode.as_ref() }
/// #   fn set_cusipcode(&mut self, cusipcode: Option<Cusip>) { self.cusipcode = cusipcode; }
/// #   fn get_sedolcode(&self) -> Option<&Sedol> { self.sedolcode.as_ref() }
/// #   fn set_sedolcode(&mut self, sedolcode: Option<Sedol>) { self.sedolcode = sedolcode; }
/// #   fn get_bloombergcode(&self) -> Option<&Bloomberg> { self.bloombergcode.as_ref() }
/// #   fn set_bloombergcode(&mut self, bloombergcode: Option<Bloomberg>) { self.bloombergcode = bloombergcode; }
/// #   fn get_cficode(&self) -> Option<&Cfi> { self.cficode.as_ref() }
/// #   fn set_cficode(&mut self, cficode: Option<Cfi>) { self.cficode = cficode; }
/// #   fn get_miccode(&self) -> Option<&Mic> { self.miccode.as_ref() }
/// #   fn set_miccode(&mut self, miccode: Option<Mic>) { self.miccode = miccode; }
/// }
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut trade = Trade {
///     uuid: Uuid::from_v8(1),
///     unix: 10,
///     hashcode: 0,
///     px: 82.5,
///     currency: Currency::new("USD")?,
///     qty: 1_000.0,
///     unit: "bbl".to_owned(),
///     side: Side::read("1")?,
///     isincode: Some(Isin::new("US0378331005")?),
/// #   cusipcode: None, sedolcode: None, bloombergcode: None, cficode: None, miccode: None,
/// #   xuuid: None, identifiers: BTreeMap::new(), parents: Vec::new(), xhashcode: 0,
/// #   state: State::from_spelling("New").expect("a shipped state"), sequence_num: 0,
/// #   creation_unix: None, expiration_unix: None, previous_unix: None, previous_uuid: None,
/// #   snapshot_unix: None,
/// };
/// assert_eq!(trade.get_px(), 82.5);
/// assert_eq!(trade.get_currency().as_str(), "USD");
/// assert_eq!(trade.get_unit(), "bbl");
/// trade.set_qty(2_000.0);
/// assert_eq!(trade.get_qty(), 2_000.0);
/// // A market element is a timed element is an element: one walk reads all.
/// let held: &dyn MarketElement = &trade;
/// assert_eq!(held.get_side().as_str(), "BUY");
/// assert_eq!(held.get_isincode().map(Isin::as_str), Some("US0378331005"));
/// assert!(held.get_cusipcode().is_none(), "an instrument is named the way the market names it");
/// assert_eq!(held.get_unix(), 10);
/// assert_eq!(held.get_current_uuid(), Uuid::from_v8(1));
/// # Ok(())
/// # }
/// ```
pub trait MarketElement: TimeElement {
    /// The price.
    fn get_px(&self) -> f64;

    /// Records the price.
    fn set_px(&mut self, px: f64);

    /// The currency the price is quoted in.
    fn get_currency(&self) -> &Currency;

    /// Records the currency the price is quoted in.
    fn set_currency(&mut self, currency: Currency);

    /// The quantity.
    fn get_qty(&self) -> f64;

    /// Records the quantity.
    fn set_qty(&mut self, qty: f64);

    /// The unit the quantity is counted in, as the market spells it.
    fn get_unit(&self) -> &str;

    /// Records the unit the quantity is counted in.
    fn set_unit(&mut self, unit: String);

    /// Which side of the market the element stood on: FIX's `Side(54)`.
    fn get_side(&self) -> &Side;

    /// Records which side of the market the element stood on.
    fn set_side(&mut self, side: Side);

    /// The instrument's ISIN, where the market names it so.
    fn get_isincode(&self) -> Option<&Isin>;

    /// Records the instrument's ISIN; `None` states the market names none.
    fn set_isincode(&mut self, isincode: Option<Isin>);

    /// The instrument's CUSIP, where the market names it so.
    fn get_cusipcode(&self) -> Option<&Cusip>;

    /// Records the instrument's CUSIP; `None` states the market names none.
    fn set_cusipcode(&mut self, cusipcode: Option<Cusip>);

    /// The instrument's SEDOL, where the market names it so.
    fn get_sedolcode(&self) -> Option<&Sedol>;

    /// Records the instrument's SEDOL; `None` states the market names none.
    fn set_sedolcode(&mut self, sedolcode: Option<Sedol>);

    /// The instrument's Bloomberg identifier, where the market names it so.
    fn get_bloombergcode(&self) -> Option<&Bloomberg>;

    /// Records the instrument's Bloomberg identifier; `None` states the
    /// market names none.
    fn set_bloombergcode(&mut self, bloombergcode: Option<Bloomberg>);

    /// The instrument's CFI classification, where the market states it.
    fn get_cficode(&self) -> Option<&Cfi>;

    /// Records the instrument's CFI classification; `None` states the
    /// market states none.
    fn set_cficode(&mut self, cficode: Option<Cfi>);

    /// The market the element traded on, as its ISO 10383 MIC, where known.
    fn get_miccode(&self) -> Option<&Mic>;

    /// Records the market the element traded on; `None` states it is not
    /// known.
    fn set_miccode(&mut self, miccode: Option<Mic>);
}
