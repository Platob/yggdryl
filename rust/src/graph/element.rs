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
use std::hash::Hasher;

use crate::hashing::txhash::TxHash;
use crate::hashing::xxhash::Xxh3;
use crate::types::{
    Bloomberg, Cfi, CodeValue, Currency, Cusip, Decimal, Isin, Mic, Sedol, Side, State, Uuid,
};
use crate::{Digest, DigestAlgorithm, Error, Result, TimeUnit};

/// One element of a graph: a node that knows its own identity, the identity
/// it has elsewhere, the names it goes by, and the identities of the
/// elements it descends from.
///
/// The parents are an ordered list, and the order is the implementor's to
/// state: a message's chain, a node's lineage. An element with no parent is
/// a root. The cross element, `crossuuid`, is what this element is in another
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
///     crossuuid: Option<Uuid>,
///     identifiers: BTreeMap<String, String>,
///     parents: Vec<Uuid>,
/// }
///
/// impl Node {
///     fn new(uuid: u128) -> Self {
///         Self { uuid: Uuid::from_v8(uuid), crossuuid: None, identifiers: BTreeMap::new(), parents: Vec::new() }
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
///     fn get_crossuuid(&self) -> Option<Uuid> {
///         self.crossuuid
///     }
///     fn set_crossuuid(&mut self, crossuuid: Option<Uuid>) {
///         self.crossuuid = crossuuid;
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
///     // A node's identity is assigned, so there is nothing to finalize.
///     fn finalize(&mut self) {}
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
/// child.set_crossuuid(Some(Uuid::from_v8(20)));
/// child.set_identifiers(BTreeMap::from([("ClOrdID".to_owned(), "C-1".to_owned())]));
/// assert!(child.get_parentuuids().is_empty(), "a node naming no parent is a root");
/// assert!(!child.is_after(&root) && !child.is_before(&root), "unrelated, so neither");
/// let child = child.with_previous(&root).expect("a node follows another");
/// assert_eq!(child.get_parentuuids(), [root.get_current_uuid()]);
/// assert!(child.is_after(&root) && root.is_before(&child));
/// assert_eq!(child.get_crossuuid(), Some(Uuid::from_v8(20)));
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
/// assert_eq!(merged.get_crossuuid(), Some(Uuid::from_v8(20)));
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
    fn get_crossuuid(&self) -> Option<Uuid>;

    /// Records the identity this element has in another graph; `None`
    /// states it has none.
    fn set_crossuuid(&mut self, crossuuid: Option<Uuid>);

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

    /// Finalizes the hashing: recomputes what this element's identity
    /// derives from its content, and resets the current identity where it
    /// derives from that.
    ///
    /// The implementor's, because only it knows its content: a timed
    /// element digests what it says and hands the code to
    /// [`TimeElement::finalized`], which sets the code and the identity the
    /// instant and the code derive; an element whose identity is assigned
    /// does nothing. Every provided reading that changes an element calls
    /// this once it has, so a followed or merged element never carries the
    /// identity of what it was.
    fn finalize(&mut self);

    /// Starts the digest of this element's content: an XXH3-64 state
    /// already fed the facts every element states that are not when it
    /// happened - the cross element, the names it goes by and its parents,
    /// each under its own name - for an implementor to feed what it says
    /// and finish.
    ///
    /// Provided, and what an implementor's [`Self::finalize`] starts from:
    /// a timed element continues with [`TimeElement::digest_timed`], a
    /// market element with [`MarketElement::digest_market`], and each
    /// feeds its own content behind them and reads `as_u64` for the code.
    /// The facts are fed through their typed accessors, so two elements
    /// stating the same things digest alike whatever holds them.
    fn digest(&self) -> Xxh3 {
        let mut state = Xxh3::new();
        if let Some(crossuuid) = self.get_crossuuid() {
            feed(&mut state, "crossuuid", &crossuuid.into_bytes());
        }
        for (scheme, identifier) in self.get_identifiers() {
            feed(&mut state, scheme, identifier.as_bytes());
        }
        for parent in self.get_parentuuids() {
            feed(&mut state, "parentuuid", &parent.into_bytes());
        }
        state
    }

    /// This element stated as the one after `previous`, or nothing where it
    /// cannot follow it - its own predecessor, an element it precedes - or
    /// where following it changes nothing.
    ///
    /// Takes the element by value and answers it by value, so a chain is
    /// built by threading one element through the next; nothing for no
    /// change lets a caller skip what it already holds.
    fn with_previous(self, previous: &Self) -> Option<Self>
    where
        Self: Sized;

    /// This element with another statement of itself folded in, or nothing
    /// where `other` is another element or where the fold changes nothing.
    ///
    /// Provided: the cross element is taken from `other` where this one
    /// states none, the identifiers this one lacks are taken from it, and
    /// the parents become the union - this element's in its order, then the
    /// ones only `other` names, in its - and the element is finalized where
    /// any of that moved. A timed element delegates to
    /// [`TimeElement::merging`], which folds the rest.
    fn merge_with(mut self, other: &Self) -> Option<Self>
    where
        Self: Sized,
    {
        if other.get_current_uuid() != self.get_current_uuid() || !merge_element(&mut self, other) {
            return None;
        }
        self.finalize();
        Some(self)
    }
}

/// The facts an element takes from another statement of itself: the cross
/// element it left out, the identifiers it lacks, and the parents it did
/// not name; whether any of them moved.
fn merge_element<E: Element + ?Sized>(this: &mut E, other: &E) -> bool {
    let mut changed = false;
    if this.get_crossuuid().is_none() && other.get_crossuuid().is_some() {
        this.set_crossuuid(other.get_crossuuid());
        changed = true;
    }
    changed |= take_identifiers(this, other);
    let mut parents = this.get_parentuuids().to_vec();
    let known = parents.len();
    for parent in other.get_parentuuids() {
        if !parents.contains(parent) {
            parents.push(*parent);
        }
    }
    if parents.len() != known {
        this.set_parentuuids(parents);
        changed = true;
    }
    changed
}

/// The identifiers `other` knows and `this` does not, taken; the ones this
/// element states are its own word and stay. Whether any was taken.
fn take_identifiers<E: Element + ?Sized>(this: &mut E, other: &E) -> bool {
    if other
        .get_identifiers()
        .keys()
        .all(|scheme| this.get_identifiers().contains_key(scheme))
    {
        return false;
    }
    let mut identifiers = this.get_identifiers().clone();
    for (scheme, identifier) in other.get_identifiers() {
        identifiers
            .entry(scheme.clone())
            .or_insert_with(|| identifier.clone());
    }
    this.set_identifiers(identifiers);
    true
}

/// Feeds one named fact to a digest: the name, the bytes, each closed by a
/// byte no name or value holds, so two facts never read as one.
fn feed(state: &mut Xxh3, name: &str, bytes: &[u8]) {
    state.write(name.as_bytes());
    state.write(&[0]);
    state.write(bytes);
    state.write(&[0]);
}

/// Records `next` where it differs from `current`, answering whether it did.
fn moved<T: PartialEq>(current: T, next: T, set: impl FnOnce(T)) -> bool {
    if current == next {
        return false;
    }
    set(next);
    true
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

/// The timed facts an element takes from following `previous`: the
/// predecessor's identity and instant, the next place in the chain, the
/// names it went by, the lifecycle carried forward; whether any moved.
fn follow_timed<E: TimeElement>(this: &mut E, previous: &E) -> bool {
    let mut changed = moved(
        this.get_previous_uuid(),
        Some(previous.get_current_uuid()),
        |uuid| this.set_previous_uuid(uuid),
    );
    changed |= moved(
        this.get_previous_unix(),
        Some(previous.get_unix()),
        |unix| this.set_previous_unix(unix),
    );
    changed |= moved(
        this.get_sequence_num(),
        previous.get_sequence_num().saturating_add(1),
        |place| this.set_sequence_num(place),
    );
    changed |= take_identifiers(this, previous);
    changed |= this.fold_lifecycle(previous);
    changed
}

/// The timed facts an element takes from another statement of itself: the
/// later statement's instant and codes, the further place in the chain,
/// the lifecycle folded, the predecessor and the snapshot instant where
/// this one states none; whether any moved.
fn merge_timed<E: TimeElement>(this: &mut E, other: &E) -> bool {
    let mut changed = false;
    if other.get_unix() > this.get_unix() {
        this.set_unix(other.get_unix());
        this.set_hashcode(other.get_hashcode());
        this.set_xhashcode(other.get_xhashcode());
        changed = true;
    }
    if other.get_sequence_num() > this.get_sequence_num() {
        this.set_sequence_num(other.get_sequence_num());
        changed = true;
    }
    changed |= this.fold_lifecycle(other);
    if this.get_previous_uuid().is_none() && other.get_previous_uuid().is_some() {
        this.set_previous_uuid(other.get_previous_uuid());
        this.set_previous_unix(other.get_previous_unix());
        changed = true;
    }
    if this.get_snapshot_unix().is_none() && other.get_snapshot_unix().is_some() {
        this.set_snapshot_unix(other.get_snapshot_unix());
        changed = true;
    }
    changed
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
/// [`Self::time_crossuuid`].
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
///     crossuuid: Option<Uuid>,
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
///             crossuuid: None,
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
///     fn get_crossuuid(&self) -> Option<Uuid> {
///         self.crossuuid
///     }
///     fn set_crossuuid(&mut self, crossuuid: Option<Uuid>) {
///         self.crossuuid = crossuuid;
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
///     // The event's identity is assigned here, so finalizing keeps it; an
///     // event whose identity is its instant and content would hand the
///     // content's digest to `finalized`.
///     fn finalize(&mut self) {}
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
/// assert!(first.time_crossuuid().expect("an instant a TxHash holds").is_some());
/// assert!(Event::at(4, 40).time_crossuuid().expect("nothing to couple").is_none());
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
    /// one that happened after it - or where following it changes nothing,
    /// because it already follows it.
    ///
    /// The predecessor's identity and instant are recorded on it, its place
    /// in the chain is the one after the predecessor's, and the lifecycle
    /// carries forward: the creation is the earliest the two know, the
    /// expiration the latest, the state the furthest along, and each where
    /// only one states it is that one's; the names the predecessor went by
    /// and this element does not state are taken, because a name issued at
    /// creation holds for the whole lifecycle. What the element itself
    /// says - its instant, its codes, its parents, its cross element, the
    /// snapshot it is - is its own and moves nowhere. An element that moved
    /// is finalized, so it never carries the identity of what it was.
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
            || !follow_timed(&mut self, previous)
        {
            return None;
        }
        self.finalize();
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
    /// Nothing where `other` is another element, and nothing where the fold
    /// changes nothing, so a caller skips a restatement it already holds;
    /// an element that moved is finalized.
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
        let changed = merge_element(&mut self, other);
        if !(merge_timed(&mut self, other) || changed) {
            return None;
        }
        self.finalize();
        Some(self)
    }

    /// Folds another element's lifecycle into this one: the earliest
    /// creation, the latest expiration, the furthest state - the better of
    /// the two as [`CodeValue::merge_with`] reads a state; whether any of
    /// them moved.
    ///
    /// Provided, and what [`Self::following`] and [`Self::merging`] share.
    fn fold_lifecycle(&mut self, other: &Self) -> bool
    where
        Self: Sized,
    {
        let mut changed = moved(
            self.get_creation_unix(),
            earliest(self.get_creation_unix(), other.get_creation_unix()),
            |unix| self.set_creation_unix(unix),
        );
        changed |= moved(
            self.get_expiration_unix(),
            latest(self.get_expiration_unix(), other.get_expiration_unix()),
            |unix| self.set_expiration_unix(unix),
        );
        let state = self.get_state().clone().merge_with(other.get_state());
        changed |= moved(self.get_state().clone(), state, |state| {
            self.set_state(state)
        });
        changed
    }

    /// Records the code this element's content digests to and resets the
    /// identity the instant and the code derive.
    ///
    /// Provided, and what an implementor's [`Element::finalize`] hands the
    /// digest of its content to. The identity is [`Self::time_uuid`], and
    /// an instant a UUIDv7 cannot hold leaves the identity as it was.
    fn finalized(&mut self, hashcode: u64) {
        self.set_hashcode(hashcode);
        if let Ok(uuid) = self.time_uuid() {
            self.set_current_uuid(uuid);
        }
    }

    /// Continues [`Element::digest`] with the facts a timed element states
    /// that are not instants: the cross code, the state, the place in the
    /// chain and the predecessor's identity.
    ///
    /// Provided, for an implementor's [`Element::finalize`] to feed its own
    /// content behind. The instants - when it happened, was created,
    /// expires, the predecessor's and the snapshot's - are left out, so the
    /// code says what an element states and not when.
    fn digest_timed(&self) -> Xxh3 {
        let mut state = self.digest();
        feed(&mut state, "xhashcode", &self.get_xhashcode().to_le_bytes());
        feed(&mut state, "state", self.get_state().as_str().as_bytes());
        feed(
            &mut state,
            "sequencenum",
            &self.get_sequence_num().to_le_bytes(),
        );
        if let Some(previous) = self.get_previous_uuid() {
            feed(&mut state, "previousuuid", &previous.into_bytes());
        }
        state
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
    /// this from [`Element::get_crossuuid`].
    ///
    /// # Errors
    ///
    /// Returns what [`Self::txhash`] returns, for the creation instant.
    fn time_crossuuid(&self) -> Result<Option<Uuid>> {
        self.get_creation_unix()
            .map(|unix| coupled(unix, self.get_xhashcode())?.into_uuid())
            .transpose()
    }
}

/// An element that happened in a market: a price, a quantity, and which
/// side of the market it stood on.
///
/// Five facts beside what a timed element already states, each read and
/// written: `px` is the price, a [`Decimal`] - exact, as a market's numbers
/// are - and `currency` the [`Currency`] it is quoted in; `qty` is the
/// quantity, a [`Decimal`] too, and `unit` the text it is counted in - a
/// lot, a barrel, a megawatt-hour, whatever the market says; `side` is the
/// crate's [`Side`] code, FIX's `Side(54)`. Two lanes state the quote the
/// element makes, each optional and each the same four facts - `bidpx`,
/// `bidcurrency`, `bidqty`, `bidunit` for what the element would pay, and
/// the `ask` four for what it would be paid - and [`Self::fill_lanes`] fills
/// the lane the side implies from the price, the currency, the quantity and
/// the unit where the lane states nothing. Six more name the instrument and the
/// market, each optional because a market names an instrument the way it
/// does: the [`Isin`], the [`Cusip`], the [`Sedol`], the [`Bloomberg`]
/// identifier, the [`Cfi`] classification and the [`Mic`] of the market it
/// traded on, the crate's own validated codes. The traits state signatures
/// and one more provided reading: [`Self::merging_market`] is what merging
/// means for a market element - the timed merge, then the later statement's
/// price, quantity and unit, and each code the better of the two as
/// [`CodeValue::merge_with`] reads it, the later statement leading - which
/// an implementor's [`Element::merge_with`] delegates to. What a price of
/// nothing or a quantity of zero means is the market's to say, and following
/// is the timed reading unchanged.
///
/// ```
/// use std::collections::BTreeMap;
///
/// use yggdryl::graph::{Element, MarketElement, TimeElement};
/// use yggdryl::types::{Bloomberg, Cfi, Currency, Cusip, Decimal, Isin, Mic, Sedol, Side, State, Uuid};
///
/// # #[derive(Clone)]
/// struct Trade {
///     uuid: Uuid,
///     unix: i128,
///     hashcode: u64,
///     px: Decimal,
///     currency: Currency,
///     qty: Decimal,
///     unit: String,
///     side: Side,
///     isincode: Option<Isin>,
/// #   cusipcode: Option<Cusip>,
/// #   sedolcode: Option<Sedol>,
/// #   bloombergcode: Option<Bloomberg>,
/// #   cficode: Option<Cfi>,
/// #   miccode: Option<Mic>,
/// #   bid: (Option<Decimal>, Option<Currency>, Option<Decimal>, Option<String>),
/// #   ask: (Option<Decimal>, Option<Currency>, Option<Decimal>, Option<String>),
/// #   crossuuid: Option<Uuid>,
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
/// #     fn get_crossuuid(&self) -> Option<Uuid> { self.crossuuid }
/// #     fn set_crossuuid(&mut self, crossuuid: Option<Uuid>) { self.crossuuid = crossuuid; }
/// #     fn get_identifiers(&self) -> &BTreeMap<String, String> { &self.identifiers }
/// #     fn set_identifiers(&mut self, identifiers: BTreeMap<String, String>) { self.identifiers = identifiers; }
/// #     fn get_parentuuids(&self) -> &[Uuid] { &self.parents }
/// #     fn set_parentuuids(&mut self, parents: Vec<Uuid>) { self.parents = parents; }
/// #     fn is_after(&self, other: &Self) -> bool { self.unix > other.unix }
/// #     fn finalize(&mut self) {}
/// #     fn with_previous(self, previous: &Self) -> Option<Self> { self.following(previous) }
/// #     fn merge_with(self, other: &Self) -> Option<Self> { self.merging_market(other) }
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
///     fn get_px(&self) -> Decimal {
///         self.px
///     }
///     fn set_px(&mut self, px: Decimal) {
///         self.px = px;
///     }
///     fn get_currency(&self) -> &Currency {
///         &self.currency
///     }
///     fn set_currency(&mut self, currency: Currency) {
///         self.currency = currency;
///     }
///     fn get_qty(&self) -> Decimal {
///         self.qty
///     }
///     fn set_qty(&mut self, qty: Decimal) {
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
/// #   fn get_bidpx(&self) -> Option<Decimal> { self.bid.0 }
/// #   fn set_bidpx(&mut self, px: Option<Decimal>) { self.bid.0 = px; }
/// #   fn get_bidcurrency(&self) -> Option<&Currency> { self.bid.1.as_ref() }
/// #   fn set_bidcurrency(&mut self, currency: Option<Currency>) { self.bid.1 = currency; }
/// #   fn get_bidqty(&self) -> Option<Decimal> { self.bid.2 }
/// #   fn set_bidqty(&mut self, qty: Option<Decimal>) { self.bid.2 = qty; }
/// #   fn get_bidunit(&self) -> Option<&str> { self.bid.3.as_deref() }
/// #   fn set_bidunit(&mut self, unit: Option<String>) { self.bid.3 = unit; }
/// #   fn get_askpx(&self) -> Option<Decimal> { self.ask.0 }
/// #   fn set_askpx(&mut self, px: Option<Decimal>) { self.ask.0 = px; }
/// #   fn get_askcurrency(&self) -> Option<&Currency> { self.ask.1.as_ref() }
/// #   fn set_askcurrency(&mut self, currency: Option<Currency>) { self.ask.1 = currency; }
/// #   fn get_askqty(&self) -> Option<Decimal> { self.ask.2 }
/// #   fn set_askqty(&mut self, qty: Option<Decimal>) { self.ask.2 = qty; }
/// #   fn get_askunit(&self) -> Option<&str> { self.ask.3.as_deref() }
/// #   fn set_askunit(&mut self, unit: Option<String>) { self.ask.3 = unit; }
/// }
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut trade = Trade {
///     uuid: Uuid::from_v8(1),
///     unix: 10,
///     hashcode: 0,
///     px: "82.5".parse()?,
///     currency: Currency::new("USD")?,
///     qty: Decimal::from_int(1_000),
///     unit: "bbl".to_owned(),
///     side: Side::read("1")?,
///     isincode: Some(Isin::new("US0378331005")?),
/// #   cusipcode: None, sedolcode: None, bloombergcode: None, cficode: None, miccode: None,
/// #   bid: (None, None, None, None), ask: (None, None, None, None),
/// #   crossuuid: None, identifiers: BTreeMap::new(), parents: Vec::new(), xhashcode: 0,
/// #   state: State::from_spelling("New").expect("a shipped state"), sequence_num: 0,
/// #   creation_unix: None, expiration_unix: None, previous_unix: None, previous_uuid: None,
/// #   snapshot_unix: None,
/// };
/// assert_eq!(trade.get_px().to_string(), "82.5");
/// assert_eq!(trade.get_currency().as_str(), "USD");
/// assert_eq!(trade.get_unit(), "bbl");
/// trade.set_qty(Decimal::from_int(2_000));
/// assert_eq!(trade.get_qty(), Decimal::from_int(2_000));
/// // A buy is a bid: the lane the side implies fills from the trade's own
/// // facts, and the other lane stays empty.
/// trade.fill_lanes();
/// assert_eq!(trade.get_bidpx(), Some("82.5".parse()?));
/// assert_eq!(trade.get_bidcurrency().map(Currency::as_str), Some("USD"));
/// assert_eq!((trade.get_bidqty(), trade.get_bidunit()), (Some(Decimal::from_int(2_000)), Some("bbl")));
/// assert_eq!(trade.get_askpx(), None);
/// // A market element is a timed element is an element: one walk reads all.
/// let held: &dyn MarketElement = &trade;
/// assert_eq!(held.get_side().as_str(), "BUY");
/// assert_eq!(held.get_isincode().map(Isin::as_str), Some("US0378331005"));
/// assert!(held.get_cusipcode().is_none(), "an instrument is named the way the market names it");
/// assert_eq!(held.get_unix(), 10);
/// assert_eq!(held.get_current_uuid(), Uuid::from_v8(1));
/// // A later statement of the trade merges in: its price has the last word,
/// // and the CFI it states fills what this one left unknown.
/// let later = Trade { px: "83".parse()?, unix: 20, cficode: Some(Cfi::new("ESVUFR")?), ..trade.clone() };
/// trade.set_cficode(Some(Cfi::new("ESXXXR")?));
/// let merged = trade.merge_with(&later).expect("the same trade");
/// assert_eq!(merged.get_px(), Decimal::from_int(83));
/// assert_eq!(merged.get_cficode().map(Cfi::as_str), Some("ESVUFR"));
/// # Ok(())
/// # }
/// ```
pub trait MarketElement: TimeElement {
    /// The price.
    fn get_px(&self) -> Decimal;

    /// Records the price.
    fn set_px(&mut self, px: Decimal);

    /// The currency the price is quoted in.
    fn get_currency(&self) -> &Currency;

    /// Records the currency the price is quoted in.
    fn set_currency(&mut self, currency: Currency);

    /// The quantity.
    fn get_qty(&self) -> Decimal;

    /// Records the quantity.
    fn set_qty(&mut self, qty: Decimal);

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

    /// The price the element would pay, where it states a bid.
    fn get_bidpx(&self) -> Option<Decimal>;

    /// Records the price the element would pay; `None` states no bid.
    fn set_bidpx(&mut self, px: Option<Decimal>);

    /// The currency the bid is quoted in, where stated.
    fn get_bidcurrency(&self) -> Option<&Currency>;

    /// Records the currency the bid is quoted in.
    fn set_bidcurrency(&mut self, currency: Option<Currency>);

    /// The quantity the element would pay for, where stated.
    fn get_bidqty(&self) -> Option<Decimal>;

    /// Records the quantity the element would pay for.
    fn set_bidqty(&mut self, qty: Option<Decimal>);

    /// The unit the bid quantity is counted in, where stated.
    fn get_bidunit(&self) -> Option<&str>;

    /// Records the unit the bid quantity is counted in.
    fn set_bidunit(&mut self, unit: Option<String>);

    /// The price the element would be paid, where it states an ask.
    fn get_askpx(&self) -> Option<Decimal>;

    /// Records the price the element would be paid; `None` states no ask.
    fn set_askpx(&mut self, px: Option<Decimal>);

    /// The currency the ask is quoted in, where stated.
    fn get_askcurrency(&self) -> Option<&Currency>;

    /// Records the currency the ask is quoted in.
    fn set_askcurrency(&mut self, currency: Option<Currency>);

    /// The quantity the element would be paid for, where stated.
    fn get_askqty(&self) -> Option<Decimal>;

    /// Records the quantity the element would be paid for.
    fn set_askqty(&mut self, qty: Option<Decimal>);

    /// The unit the ask quantity is counted in, where stated.
    fn get_askunit(&self) -> Option<&str>;

    /// Records the unit the ask quantity is counted in.
    fn set_askunit(&mut self, unit: Option<String>);

    /// Fills the lane the side implies from the element's own facts, where
    /// the lane states nothing of its own.
    ///
    /// A side that takes the bid lane - [`Side::is_bid`] - is a party
    /// willing to pay the price for the quantity, so the bid price, currency,
    /// quantity and unit each take the element's where the lane left them
    /// out; a side that takes the ask lane fills the ask the same way; a
    /// side that takes neither - a cross, `OPPOSITE`, one stated as none -
    /// fills nothing. What a lane already states stands: a quote carrying
    /// its own lanes is never overwritten.
    ///
    /// Provided, and what a reader that lifts a quote out of an order calls.
    fn fill_lanes(&mut self)
    where
        Self: Sized,
    {
        let side = self.get_side();
        let (bid, ask) = (side.is_bid(), side.is_ask());
        if !bid && !ask {
            return;
        }
        let (px, qty) = (self.get_px(), self.get_qty());
        let currency = self.get_currency().clone();
        let unit = self.get_unit().to_owned();
        if bid {
            if self.get_bidpx().is_none() {
                self.set_bidpx(Some(px));
            }
            if self.get_bidcurrency().is_none() {
                self.set_bidcurrency(Some(currency));
            }
            if self.get_bidqty().is_none() {
                self.set_bidqty(Some(qty));
            }
            if self.get_bidunit().is_none() {
                self.set_bidunit(Some(unit));
            }
        } else {
            if self.get_askpx().is_none() {
                self.set_askpx(Some(px));
            }
            if self.get_askcurrency().is_none() {
                self.set_askcurrency(Some(currency));
            }
            if self.get_askqty().is_none() {
                self.set_askqty(Some(qty));
            }
            if self.get_askunit().is_none() {
                self.set_askunit(Some(unit));
            }
        }
    }

    /// Continues [`TimeElement::digest_timed`] with the market's facts: the
    /// price, the currency, the quantity, the unit, the side, each
    /// instrument code the market names, the market itself, and each lane
    /// fact stated.
    ///
    /// Provided, for an implementor's [`Element::finalize`] to feed its own
    /// content behind.
    fn digest_market(&self) -> Xxh3 {
        let mut state = self.digest_timed();
        feed(&mut state, "px", &self.get_px().units().to_le_bytes());
        feed(
            &mut state,
            "currency",
            self.get_currency().as_str().as_bytes(),
        );
        feed(&mut state, "qty", &self.get_qty().units().to_le_bytes());
        feed(&mut state, "unit", self.get_unit().as_bytes());
        feed(&mut state, "side", self.get_side().as_str().as_bytes());
        let codes: [(&str, Option<&str>); 6] = [
            ("isincode", self.get_isincode().map(Isin::as_str)),
            ("cusipcode", self.get_cusipcode().map(Cusip::as_str)),
            ("sedolcode", self.get_sedolcode().map(Sedol::as_str)),
            (
                "bloombergcode",
                self.get_bloombergcode().map(Bloomberg::as_str),
            ),
            ("cficode", self.get_cficode().map(Cfi::as_str)),
            ("miccode", self.get_miccode().map(Mic::as_str)),
        ];
        for (name, code) in codes {
            if let Some(code) = code {
                feed(&mut state, name, code.as_bytes());
            }
        }
        feed_lane(
            &mut state,
            "bid",
            self.get_bidpx(),
            self.get_bidcurrency(),
            self.get_bidqty(),
            self.get_bidunit(),
        );
        feed_lane(
            &mut state,
            "ask",
            self.get_askpx(),
            self.get_askcurrency(),
            self.get_askqty(),
            self.get_askunit(),
        );
        state
    }

    /// This element with another statement of itself folded in, by the
    /// market reading, or nothing where `other` is another element.
    ///
    /// The timed merge first - [`TimeElement::merging`] - and then the
    /// market's facts: the later statement's price, quantity and unit have
    /// the last word, as its instant and codes do; the currency, the side
    /// and each instrument code are the better of the two statements as
    /// [`CodeValue::merge_with`] reads them, the later statement leading and
    /// the earlier filling what it leaves unknown - a `XXX` currency, an
    /// `UNKNOWN` side, an `X` in a CFI - and a code only one statement
    /// names is that one's; each lane fact is the later statement's where
    /// it states one, else this one's. An equal instant keeps this element
    /// leading. Nothing where the fold changes nothing, and an element that
    /// moved is finalized.
    ///
    /// Provided, so an implementor's [`Element::merge_with`] has a default
    /// to delegate to.
    fn merging_market(mut self, other: &Self) -> Option<Self>
    where
        Self: Sized,
    {
        if other.get_current_uuid() != self.get_current_uuid() {
            return None;
        }
        // Which statement is the later one is read before the timed merge
        // moves this element's instant to it.
        let later = other.get_unix() > self.get_unix();
        let changed = merge_element(&mut self, other);
        let changed = merge_timed(&mut self, other) || changed;
        if !(merge_market(&mut self, other, later) || changed) {
            return None;
        }
        self.finalize();
        Some(self)
    }
}

/// The market's facts an element takes from another statement of itself:
/// the later statement's price, quantity, unit and lane facts, and each
/// code the better of the two; whether any moved. `later` says whether
/// `other` is the later statement.
fn merge_market<E: MarketElement>(this: &mut E, other: &E, later: bool) -> bool {
    let mut changed = false;
    if later {
        changed |= moved(this.get_px(), other.get_px(), |px| this.set_px(px));
        changed |= moved(this.get_qty(), other.get_qty(), |qty| this.set_qty(qty));
        changed |= moved(
            this.get_unit().to_owned(),
            other.get_unit().to_owned(),
            |unit| this.set_unit(unit),
        );
    }
    changed |= moved(
        this.get_bidpx(),
        stated(this.get_bidpx(), other.get_bidpx(), later),
        |px| this.set_bidpx(px),
    );
    changed |= moved(
        this.get_bidcurrency().cloned(),
        better_stated(
            this.get_bidcurrency().cloned(),
            other.get_bidcurrency(),
            later,
        ),
        |currency| this.set_bidcurrency(currency),
    );
    changed |= moved(
        this.get_bidqty(),
        stated(this.get_bidqty(), other.get_bidqty(), later),
        |qty| this.set_bidqty(qty),
    );
    changed |= moved(
        this.get_bidunit().map(str::to_owned),
        stated(
            this.get_bidunit().map(str::to_owned),
            other.get_bidunit().map(str::to_owned),
            later,
        ),
        |unit| this.set_bidunit(unit),
    );
    changed |= moved(
        this.get_askpx(),
        stated(this.get_askpx(), other.get_askpx(), later),
        |px| this.set_askpx(px),
    );
    changed |= moved(
        this.get_askcurrency().cloned(),
        better_stated(
            this.get_askcurrency().cloned(),
            other.get_askcurrency(),
            later,
        ),
        |currency| this.set_askcurrency(currency),
    );
    changed |= moved(
        this.get_askqty(),
        stated(this.get_askqty(), other.get_askqty(), later),
        |qty| this.set_askqty(qty),
    );
    changed |= moved(
        this.get_askunit().map(str::to_owned),
        stated(
            this.get_askunit().map(str::to_owned),
            other.get_askunit().map(str::to_owned),
            later,
        ),
        |unit| this.set_askunit(unit),
    );
    changed |= moved(
        this.get_currency().clone(),
        better(this.get_currency().clone(), other.get_currency(), later),
        |currency| this.set_currency(currency),
    );
    changed |= moved(
        this.get_side().clone(),
        better(this.get_side().clone(), other.get_side(), later),
        |side| this.set_side(side),
    );
    changed |= moved(
        this.get_isincode().cloned(),
        better_stated(this.get_isincode().cloned(), other.get_isincode(), later),
        |code| this.set_isincode(code),
    );
    changed |= moved(
        this.get_cusipcode().cloned(),
        better_stated(this.get_cusipcode().cloned(), other.get_cusipcode(), later),
        |code| this.set_cusipcode(code),
    );
    changed |= moved(
        this.get_sedolcode().cloned(),
        better_stated(this.get_sedolcode().cloned(), other.get_sedolcode(), later),
        |code| this.set_sedolcode(code),
    );
    changed |= moved(
        this.get_bloombergcode().cloned(),
        better_stated(
            this.get_bloombergcode().cloned(),
            other.get_bloombergcode(),
            later,
        ),
        |code| this.set_bloombergcode(code),
    );
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
    changed
}

/// Feeds one quote lane to a digest: each fact it states, under the lane's
/// name.
fn feed_lane(
    state: &mut Xxh3,
    lane: &str,
    px: Option<Decimal>,
    currency: Option<&Currency>,
    qty: Option<Decimal>,
    unit: Option<&str>,
) {
    if let Some(px) = px {
        feed(state, lane, &px.units().to_le_bytes());
    }
    if let Some(currency) = currency {
        feed(state, lane, currency.as_str().as_bytes());
    }
    if let Some(qty) = qty {
        feed(state, lane, &qty.units().to_le_bytes());
    }
    if let Some(unit) = unit {
        feed(state, lane, unit.as_bytes());
    }
}

/// The better of two statements of one code: the later statement leading,
/// the earlier filling what it leaves unknown.
fn better<C: CodeValue>(this: C, other: &C, later: bool) -> C {
    if later {
        other.clone().merge_with(&this)
    } else {
        this.merge_with(other)
    }
}

/// The fact the later statement states, else this one's, else nothing.
fn stated<T>(this: Option<T>, other: Option<T>, later: bool) -> Option<T> {
    if later {
        other.or(this)
    } else {
        this.or(other)
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
