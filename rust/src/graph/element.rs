//! An element of a graph, one that happened at an instant, one that
//! happened in a market, and one that did both.
//!
//! Four traits: what an element answers about itself, what it takes, and
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
use crate::{Digest, DigestAlgorithm, Result, TimeUnit};

/// One element of a graph: a node that knows its own identity, the identity
/// it has elsewhere, the codes it digests to, the names it goes by, and the
/// identities of the elements it descends from.
///
/// The parents are an ordered list, and the order is the implementor's to
/// state: a message's chain, a node's lineage. An element with no parent is
/// a root. The cross element, `crossuuid`, is what this element is in another
/// graph - the same order in a venue's book and in a ledger - and
/// `crosscode` is the text that names it there: the identifier every
/// incarnation of one thing shares, an order's `OrderID`, a quote's
/// `QuoteID`, empty where the element states none. Two codes are derived:
/// `hashcode` is the XXH3-64 digest of the element's content, what
/// [`Self::finalize`] recomputes, and `crosshashcode` the XXH3-64 of the
/// cross code, zero where none is stated, what [`Self::sync_cross`] keeps in
/// step with it. The cross element is never absent: it is the identity the
/// cross hash code derives where a cross code is stated, and the element's
/// own identity where none is, [`Self::cross_uuid`], so every element stands
/// in exactly one chain. The identifiers are the names the element
/// goes by elsewhere, each under the scheme that issued it - an order's
/// `ClOrdID` and `OrderID`, a trade's `ExecID` - held as text under text in
/// sorted order, so an element can be found by any name a system gave it.
/// Every fact is read and written through the trait, so a store or a walk
/// that only knows an element as `dyn Element` can still place it; every
/// accessor is `get_` and every mutator `set_`, so the traits claim no bare
/// name and an implementor keeps its own `uuid()` or `state()` for whatever
/// it means by them.
///
/// Three readings come with the facts. [`Self::is_after`] is the order the
/// implementor states between two elements - an event's instant, a node's
/// lineage - and [`Self::is_before`] its mirror, provided.
/// [`Self::with_previous`] states this element as the one after another,
/// and is the other signature the trait leaves to the implementor: what
/// following means is the element's own - a chain entry records its
/// predecessor, a snapshot its base - and [`Event::following`] is the
/// reading an event delegates to. [`Self::merge_with`] folds another
/// statement of the same element into this one, and is provided: the cross
/// element and the cross code are taken where this one states none, the
/// identifiers this one lacks are taken, and the parents are the union in
/// this element's order, then the other's. An event delegates to
/// [`Event::merging`], which folds the instants and the state too.
///
/// ```
/// use std::collections::BTreeMap;
///
/// use yggdryl::graph::Element;
/// use yggdryl::types::Uuid;
///
/// struct Node {
///     uuid: Uuid,
///     crossuuid: Uuid,
///     crosscode: String,
///     hashcode: u64,
///     crosshashcode: u64,
///     identifiers: BTreeMap<String, String>,
///     parents: Vec<Uuid>,
/// }
///
/// impl Node {
///     fn new(uuid: u128) -> Self {
///         Self {
///             uuid: Uuid::from_v8(uuid),
///             crossuuid: Uuid::from_v8(uuid),
///             crosscode: String::new(),
///             hashcode: 0,
///             crosshashcode: 0,
///             identifiers: BTreeMap::new(),
///             parents: Vec::new(),
///         }
///     }
/// }
///
/// impl Element for Node {
///     fn get_curruuid(&self) -> Uuid {
///         self.uuid
///     }
///     fn set_curruuid(&mut self, uuid: Uuid) {
///         self.uuid = uuid;
///     }
///     fn get_crossuuid(&self) -> Uuid {
///         self.crossuuid
///     }
///     fn set_crossuuid(&mut self, crossuuid: Uuid) {
///         self.crossuuid = crossuuid;
///     }
///     fn get_crosscode(&self) -> &str {
///         &self.crosscode
///     }
///     fn set_crosscode(&mut self, crosscode: String) {
///         self.crosscode = crosscode;
///     }
///     fn get_hashcode(&self) -> u64 {
///         self.hashcode
///     }
///     fn set_hashcode(&mut self, hashcode: u64) {
///         self.hashcode = hashcode;
///     }
///     fn get_crosshashcode(&self) -> u64 {
///         self.crosshashcode
///     }
///     fn set_crosshashcode(&mut self, crosshashcode: u64) {
///         self.crosshashcode = crosshashcode;
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
///         self.parents.contains(&other.get_curruuid())
///     }
///     // A node's identity is assigned, so finalizing keeps it and only
///     // recomputes the code its content digests to.
///     fn finalize(&mut self) {
///         self.sync_cross();
///         self.hashcode = self.digest().as_u64();
///     }
///     // A node follows another by descending from it, and never itself.
///     fn with_previous(mut self, previous: &Self) -> Option<Self> {
///         if previous.get_curruuid() == self.get_curruuid() {
///             return None;
///         }
///         self.parents = vec![previous.get_curruuid()];
///         self.finalize();
///         Some(self)
///     }
/// }
///
/// let root = Node::new(1);
/// assert_eq!(root.get_crossuuid(), root.get_curruuid(), "no cross code: its own chain");
/// let mut child = Node::new(2);
/// child.set_crosscode("O-100".to_owned());
/// child.set_identifiers(BTreeMap::from([("ClOrdID".to_owned(), "C-1".to_owned())]));
/// assert!(child.get_parentuuids().is_empty(), "a node naming no parent is a root");
/// assert!(!child.is_after(&root) && !child.is_before(&root), "unrelated, so neither");
/// let child = child.with_previous(&root).expect("a node follows another");
/// assert_eq!(child.get_parentuuids(), [root.get_curruuid()]);
/// assert!(child.is_after(&root) && root.is_before(&child));
/// // The cross code stated, its digest and the cross identity follow it.
/// assert_ne!(child.get_crosshashcode(), 0);
/// assert_eq!(child.get_crossuuid(), child.cross_uuid());
/// assert_ne!(child.get_hashcode(), 0);
/// // The implementor's rule: a node never follows itself.
/// assert!(Node::new(1).with_previous(&root).is_none());
///
/// // A second statement of the same node merges into the first: the
/// // identifiers it states fill the ones the first left out, and the
/// // parents are the union in order. Another node does not merge at all.
/// let mut again = Node::new(2);
/// again.set_identifiers(BTreeMap::from([
///     ("ClOrdID".to_owned(), "other".to_owned()),
///     ("OrderID".to_owned(), "O-1".to_owned()),
/// ]));
/// again.set_parentuuids(vec![Uuid::from_v8(9), root.get_curruuid()]);
/// let merged = child.merge_with(&again).expect("the same node");
/// assert_eq!(merged.get_crosscode(), "O-100", "this element's word wins");
/// assert_eq!(merged.get_identifiers()["ClOrdID"], "C-1");
/// assert_eq!(merged.get_identifiers()["OrderID"], "O-1", "and what it lacked is taken");
/// assert_eq!(merged.get_parentuuids(), [root.get_curruuid(), Uuid::from_v8(9)]);
/// assert!(merged.merge_with(&root).is_none());
/// ```
pub trait Element {
    /// This element's identity.
    fn get_curruuid(&self) -> Uuid;

    /// Records this element's identity.
    fn set_curruuid(&mut self, uuid: Uuid);

    /// The identity this element has in another graph - the cross element
    /// it is the same thing as, elsewhere - which is its own identity where
    /// it states no cross code.
    fn get_crossuuid(&self) -> Uuid;

    /// Records the identity this element has in another graph.
    fn set_crossuuid(&mut self, crossuuid: Uuid);

    /// The text naming this element in another graph: the identifier every
    /// incarnation of one thing shares, empty where the element states none.
    fn get_crosscode(&self) -> &str;

    /// Records the text naming this element in another graph; empty states
    /// none. [`Self::sync_cross`] brings the cross hash code and the cross
    /// element in step with it.
    fn set_crosscode(&mut self, crosscode: String);

    /// The code this element's content digests to.
    fn get_hashcode(&self) -> u64;

    /// Records the code this element's content digests to.
    fn set_hashcode(&mut self, hashcode: u64);

    /// The code the cross code digests to: what this element is in another
    /// graph, as a digest; zero where it states no cross code.
    fn get_crosshashcode(&self) -> u64;

    /// Records the code the cross code digests to.
    fn set_crosshashcode(&mut self, crosshashcode: u64);

    /// The names this element goes by elsewhere, each under the scheme that
    /// issued it, in sorted order; empty where no system named it.
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
    /// states: an event's instant, a node's lineage.
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
    /// The implementor's, because only it knows its content: it brings the
    /// cross codes in step with [`Self::sync_cross`], digests what it says
    /// from [`Self::digest`] or the continuation its traits provide, and
    /// hands the code to [`Self::set_hashcode`] - or, for an event, to
    /// [`Event::finalized`], which sets the identity the instant and the
    /// code derive; an element whose identity is assigned keeps it. Every
    /// provided reading that changes an element calls this once it has, so
    /// a followed or merged element never carries the code of what it was.
    fn finalize(&mut self);

    /// Brings the cross hash code and the cross element in step with the
    /// cross code, answering whether either moved.
    ///
    /// The cross hash code is the cross code's XXH3-64 digest, zero where
    /// none is stated, and the cross element is [`Self::cross_uuid`],
    /// whatever either held before. Provided, and what every reading that
    /// takes a cross code from another element calls; an implementor's
    /// [`Self::finalize`] calls it before digesting, and again through
    /// [`Event::finalized`] once the identity is derived, because the cross
    /// element of an element in no chain is that identity.
    fn sync_cross(&mut self) -> bool {
        let crosshashcode = if self.get_crosscode().is_empty() {
            0
        } else {
            crosshash(self.get_crosscode())
        };
        let mut changed = moved(self.get_crosshashcode(), crosshashcode, |code| {
            self.set_crosshashcode(code);
        });
        let crossuuid = self.cross_uuid();
        changed |= moved(self.get_crossuuid(), crossuuid, |uuid| {
            self.set_crossuuid(uuid);
        });
        changed
    }

    /// The identity the cross hash code derives: RFC 9562 UUIDv8 over the
    /// code where it is not zero, and the element's own identity where it
    /// is - an element in no chain is a chain of one.
    ///
    /// Provided: every element that shares a cross code with another shares
    /// this identity with it, whichever holds them.
    fn cross_uuid(&self) -> Uuid {
        match self.get_crosshashcode() {
            0 => self.get_curruuid(),
            crosshashcode => Uuid::from_v8(u128::from(crosshashcode)),
        }
    }

    /// Starts the digest of this element's content: an XXH3-64 state
    /// already fed the facts every element states that are not derived -
    /// the cross code, the names it goes by and its parents, each under its
    /// own name - for an implementor to feed what it says and finish. The
    /// identities are derived from the digest and never fed to it.
    ///
    /// Provided, and what an implementor's [`Self::finalize`] starts from:
    /// an event continues with [`Event::digest_event`], a market element
    /// with [`MarketElement::digest_market`], a market event with
    /// [`MarketEvent::digest_market_event`], and each feeds its own content
    /// behind them and reads `as_u64` for the code. The facts are fed
    /// through their typed accessors, so two elements stating the same
    /// things digest alike whatever holds them.
    fn digest(&self) -> Xxh3 {
        let mut state = Xxh3::new();
        if !self.get_crosscode().is_empty() {
            feed(&mut state, "crosscode", self.get_crosscode().as_bytes());
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
    /// Provided: the cross code is taken from `other` where this one states
    /// none, the identifiers this one lacks are taken from it, the parents
    /// become the union - this element's in its order, then the ones only
    /// `other` names, in its - the cross codes are brought in step, and the
    /// element is finalized where any of that moved. An event delegates to
    /// [`Event::merging`], which folds the rest.
    fn merge_with(mut self, other: &Self) -> Option<Self>
    where
        Self: Sized,
    {
        if other.get_curruuid() != self.get_curruuid() || !merge_element(&mut self, other) {
            return None;
        }
        self.finalize();
        Some(self)
    }
}

/// The facts an element takes from another statement of itself: the cross
/// code it left out, the identifiers it lacks, and the parents it did not
/// name; whether any of them moved.
pub(super) fn merge_element<E: Element + ?Sized>(this: &mut E, other: &E) -> bool {
    let mut changed = false;
    if this.get_crosscode().is_empty() && !other.get_crosscode().is_empty() {
        this.set_crosscode(other.get_crosscode().to_owned());
        changed = true;
    }
    changed |= this.sync_cross();
    changed |= take_identifiers(this, other);
    changed |= union_parents(this, other);
    changed
}

/// The parents `other` names and `this` does not, appended in `other`'s
/// order behind this element's own; whether any was.
fn union_parents<E: Element + ?Sized>(this: &mut E, other: &E) -> bool {
    let mut parents = this.get_parentuuids().to_vec();
    let known = parents.len();
    for parent in other.get_parentuuids() {
        if !parents.contains(parent) {
            parents.push(*parent);
        }
    }
    if parents.len() == known {
        return false;
    }
    this.set_parentuuids(parents);
    true
}

/// The facts an event takes from restating `live`, another statement of
/// the same event: the predecessor's chain facts as [`follow_element`]
/// takes them, the parents `live` names, and the place `live` holds in its
/// chain - its predecessor, its position, its snapshot - because the two
/// statements are one event and the chain grows by nothing.
pub(super) fn restate_event<E: Event + ?Sized>(this: &mut E, live: &E) {
    follow_element(this, live);
    union_parents(this, live);
    this.set_prevuuid(live.get_prevuuid());
    this.set_prevunix(live.get_prevunix());
    this.set_seqnum(live.get_seqnum());
    this.set_snapunix(live.get_snapunix());
}

/// The facts an element takes from following `previous`: the predecessor's
/// cross code where this one's differs, forced, because two elements of one
/// chain share it, the cross codes brought in step, and the names the
/// predecessor went by; whether any moved.
pub(super) fn follow_element<E: Element + ?Sized>(this: &mut E, previous: &E) -> bool {
    let mut changed = false;
    if !previous.get_crosscode().is_empty() && this.get_crosscode() != previous.get_crosscode() {
        this.set_crosscode(previous.get_crosscode().to_owned());
        changed = true;
    }
    changed |= this.sync_cross();
    changed |= take_identifiers(this, previous);
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

/// The XXH3-64 digest of one cross code's bytes.
fn crosshash(crosscode: &str) -> u64 {
    let mut state = Xxh3::new();
    state.write(crosscode.as_bytes());
    state.as_u64()
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
fn coupled(unix: i64, hashcode: u64) -> Result<TxHash> {
    TxHash::new_in(
        unix,
        TimeUnit::Nanosecond,
        Digest::new(DigestAlgorithm::Xxh3, u128::from(hashcode)),
    )
}

/// The earlier of two optional instants, or whichever is stated.
fn earliest(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (left, right) => left.or(right),
    }
}

/// The later of two optional instants, or whichever is stated.
fn latest(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left, right) => left.or(right),
    }
}

/// The timed facts an event takes from following `previous`: the
/// predecessor's identity and instant, the next place in the chain, and
/// what any element takes from following - the cross code, the names it
/// went by - with the lifecycle carried forward; whether any moved.
fn follow_timed<E: Event>(this: &mut E, previous: &E) -> bool {
    let mut changed = moved(this.get_prevuuid(), Some(previous.get_curruuid()), |uuid| {
        this.set_prevuuid(uuid)
    });
    changed |= moved(this.get_prevunix(), Some(previous.get_unix()), |unix| {
        this.set_prevunix(unix)
    });
    changed |= moved(
        this.get_seqnum(),
        previous.get_seqnum().saturating_add(1),
        |place| this.set_seqnum(place),
    );
    changed |= follow_element(this, previous);
    changed |= this.fold_lifecycle(previous);
    changed
}

/// The timed facts an event takes from another statement of itself: the
/// later statement's instant and codes, the further place in the chain,
/// the lifecycle folded, the predecessor and the snapshot instant where
/// this one states none; whether any moved.
fn merge_timed<E: Event>(this: &mut E, other: &E) -> bool {
    let mut changed = false;
    if other.get_unix() > this.get_unix() {
        this.set_unix(other.get_unix());
        this.set_hashcode(other.get_hashcode());
        changed = true;
    }
    if other.get_seqnum() > this.get_seqnum() {
        this.set_seqnum(other.get_seqnum());
        changed = true;
    }
    changed |= this.fold_lifecycle(other);
    if this.get_prevuuid().is_none() && other.get_prevuuid().is_some() {
        this.set_prevuuid(other.get_prevuuid());
        this.set_prevunix(other.get_prevunix());
        changed = true;
    }
    if this.get_snapunix().is_none() && other.get_snapunix().is_some() {
        this.set_snapunix(other.get_snapunix());
        changed = true;
    }
    changed
}

/// Continues a digest with the facts an event states that are not
/// instants: the state, the place in the chain and the predecessor's
/// identity.
fn feed_timed<E: Event + ?Sized>(state: &mut Xxh3, this: &E) {
    feed(state, "state", this.get_state().as_str().as_bytes());
    feed(state, "seqnum", &this.get_seqnum().to_le_bytes());
    if let Some(previous) = this.get_prevuuid() {
        feed(state, "prevuuid", &previous.into_bytes());
    }
}

/// An element that happened at one instant: an event.
///
/// The instant is `unix`: a count of nanoseconds since the Unix epoch, UTC,
/// held as an `i64`, the count every clock this crate reads states. Coupled
/// with the code the element's content digests to,
/// [`Element::get_hashcode`], it is the event's identity: [`Self::txhash`]
/// is the crate's own [`TxHash`] of the two, and [`Self::time_uuid`] the
/// UUID it answers - RFC 9562 UUIDv7 with the instant in front, so
/// identities sort by instant first, to the microsecond, and by content
/// second - which is what an implementor's [`Element::get_curruuid`]
/// answers where the event's identity is when it happened and what it says.
///
/// Where the event stands is its [`State`], the crate's ranked lifecycle
/// code, and every event has one: an event that reached no state says so
/// with the code that means exactly that, `00UNKNOWN`, never with an
/// absence. Where it stands in its chain is `seqnum`: the count of events
/// before it, which following increments and merging keeps the highest of.
/// Four more instants and one more identity are optional, because an event
/// states them only where it knows them: when it was created and when it
/// expires, each an instant in the same count; the event it follows -
/// `prevuuid` and `prevunix`, the predecessor's identity and instant; and
/// `snapunix`, the grid instant this event was read as the snapshot of,
/// where a walk over a grid took one of it.
///
/// Two readings are provided. [`Self::following`] is what following means
/// for an event, which an implementor's [`Element::with_previous`]
/// delegates to; [`Self::merging`] is what merging means, which its
/// [`Element::merge_with`] delegates to. Both fold the lifecycle the same
/// way: the earliest creation, the latest expiration, the furthest state,
/// and the identifiers the other knew. The order an event states through
/// [`Element::is_after`] is its instant: later is after.
///
/// ```
/// use std::collections::BTreeMap;
///
/// use yggdryl::graph::{Element, Event};
/// use yggdryl::types::{State, Uuid};
///
/// struct Report {
///     uuid: Uuid,
///     crossuuid: Uuid,
///     crosscode: String,
///     hashcode: u64,
///     crosshashcode: u64,
///     identifiers: BTreeMap<String, String>,
///     parents: Vec<Uuid>,
///     unix: i64,
///     state: State,
///     seqnum: u64,
///     creatunix: Option<i64>,
///     expirunix: Option<i64>,
///     prevunix: Option<i64>,
///     prevuuid: Option<Uuid>,
///     snapunix: Option<i64>,
/// }
///
/// impl Report {
///     fn at(uuid: u128, unix: i64) -> Self {
///         Self {
///             uuid: Uuid::from_v8(uuid),
///             crossuuid: Uuid::from_v8(uuid),
///             crosscode: String::new(),
///             hashcode: 0,
///             crosshashcode: 0,
///             identifiers: BTreeMap::new(),
///             parents: Vec::new(),
///             unix,
///             state: State::from_spelling("New").expect("a shipped state"),
///             seqnum: 0,
///             creatunix: None,
///             expirunix: None,
///             prevunix: None,
///             prevuuid: None,
///             snapunix: None,
///         }
///     }
/// }
///
/// impl Element for Report {
///     fn get_curruuid(&self) -> Uuid {
///         self.uuid
///     }
///     fn set_curruuid(&mut self, uuid: Uuid) {
///         self.uuid = uuid;
///     }
///     fn get_crossuuid(&self) -> Uuid {
///         self.crossuuid
///     }
///     fn set_crossuuid(&mut self, crossuuid: Uuid) {
///         self.crossuuid = crossuuid;
///     }
///     fn get_crosscode(&self) -> &str {
///         &self.crosscode
///     }
///     fn set_crosscode(&mut self, crosscode: String) {
///         self.crosscode = crosscode;
///     }
///     fn get_hashcode(&self) -> u64 {
///         self.hashcode
///     }
///     fn set_hashcode(&mut self, hashcode: u64) {
///         self.hashcode = hashcode;
///     }
///     fn get_crosshashcode(&self) -> u64 {
///         self.crosshashcode
///     }
///     fn set_crosshashcode(&mut self, crosshashcode: u64) {
///         self.crosshashcode = crosshashcode;
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
///     // An event's order is its instant.
///     fn is_after(&self, other: &Self) -> bool {
///         self.unix > other.unix
///     }
///     // The report's identity is assigned here, so finalizing keeps it; an
///     // event whose identity is its instant and content would hand the
///     // content's digest to `finalized`.
///     fn finalize(&mut self) {
///         self.sync_cross();
///         self.hashcode = self.digest_event().as_u64();
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
/// impl Event for Report {
///     fn get_unix(&self) -> i64 {
///         self.unix
///     }
///     fn set_unix(&mut self, unix: i64) {
///         self.unix = unix;
///     }
///     fn get_state(&self) -> &State {
///         &self.state
///     }
///     fn set_state(&mut self, state: State) {
///         self.state = state;
///     }
///     fn get_seqnum(&self) -> u64 {
///         self.seqnum
///     }
///     fn set_seqnum(&mut self, seqnum: u64) {
///         self.seqnum = seqnum;
///     }
///     fn get_creatunix(&self) -> Option<i64> {
///         self.creatunix
///     }
///     fn set_creatunix(&mut self, unix: Option<i64>) {
///         self.creatunix = unix;
///     }
///     fn get_expirunix(&self) -> Option<i64> {
///         self.expirunix
///     }
///     fn set_expirunix(&mut self, unix: Option<i64>) {
///         self.expirunix = unix;
///     }
///     fn get_prevunix(&self) -> Option<i64> {
///         self.prevunix
///     }
///     fn set_prevunix(&mut self, unix: Option<i64>) {
///         self.prevunix = unix;
///     }
///     fn get_prevuuid(&self) -> Option<Uuid> {
///         self.prevuuid
///     }
///     fn set_prevuuid(&mut self, uuid: Option<Uuid>) {
///         self.prevuuid = uuid;
///     }
///     fn get_snapunix(&self) -> Option<i64> {
///         self.snapunix
///     }
///     fn set_snapunix(&mut self, unix: Option<i64>) {
///         self.snapunix = unix;
///     }
/// }
///
/// let mut first = Report::at(1, 10_000);
/// first.set_creatunix(Some(5_000));
/// first.set_crosscode("O-100".to_owned());
/// first.set_identifiers(BTreeMap::from([("ClOrdID".to_owned(), "C-1".to_owned())]));
/// let second = Report::at(2, 20_000);
/// assert!(second.is_after(&first) && first.is_before(&second));
/// let second = second.with_previous(&first).expect("the later one follows");
/// assert_eq!(second.get_prevuuid(), Some(first.get_curruuid()));
/// assert_eq!(second.get_prevunix(), Some(10_000));
/// // Following carries the lifecycle forward: the earliest creation known,
/// // the names the predecessor went by, the cross code the chain shares -
/// // its digest and the cross identity in step - and the next place in it.
/// assert_eq!(second.get_creatunix(), Some(5_000));
/// assert_eq!(second.get_identifiers()["ClOrdID"], "C-1");
/// assert_eq!(second.get_crosscode(), "O-100");
/// assert_ne!(second.get_crosshashcode(), 0);
/// assert_eq!(second.get_crossuuid(), second.cross_uuid());
/// assert_eq!(second.get_seqnum(), 1);
/// // An event follows neither itself nor one that happened after it.
/// assert!(Report::at(1, 10_000).with_previous(&first).is_none());
/// assert!(Report::at(3, 5_000).with_previous(&second).is_none());
/// // An event is an element: one walk reads both.
/// let held: &dyn Event = &second;
/// assert_eq!(held.get_curruuid(), Uuid::from_v8(2));
/// assert_eq!(held.get_unix(), 20_000);
/// assert_eq!(held.get_creatunix(), Some(5_000));
/// assert!(held.get_state().is_live());
/// // The identity its instant and code derive: later sorts later.
/// let earlier = first.time_uuid().expect("an instant a TxHash holds");
/// let later = second.time_uuid().expect("an instant a TxHash holds");
/// assert!(earlier < later);
/// assert_eq!(first.txhash().expect("a TxHash").unix(), 10_000);
/// ```
pub trait Event: Element {
    /// When this event happened: nanoseconds since the Unix epoch, UTC.
    fn get_unix(&self) -> i64;

    /// Records when this event happened, as nanoseconds since the Unix
    /// epoch, UTC.
    fn set_unix(&mut self, unix: i64);

    /// Where this event stands in its lifecycle: the crate's ranked
    /// [`State`] code, never absent - an event that reached no state says
    /// `00UNKNOWN`.
    fn get_state(&self) -> &State;

    /// Records where this event stands in its lifecycle.
    fn set_state(&mut self, state: State);

    /// Where this event stands in its chain: how many came before it.
    fn get_seqnum(&self) -> u64;

    /// Records where this event stands in its chain.
    fn set_seqnum(&mut self, seqnum: u64);

    /// When this event was created, in the same count as [`Self::get_unix`],
    /// where it knows.
    fn get_creatunix(&self) -> Option<i64>;

    /// Records when this event was created; `None` states it does not
    /// know.
    fn set_creatunix(&mut self, unix: Option<i64>);

    /// When this event expires, in the same count as [`Self::get_unix`], where
    /// it has an expiry.
    fn get_expirunix(&self) -> Option<i64>;

    /// Records when this event expires; `None` states it does not expire,
    /// or does not know.
    fn set_expirunix(&mut self, unix: Option<i64>);

    /// When the event this one follows happened, where it follows one.
    fn get_prevunix(&self) -> Option<i64>;

    /// Records when the event this one follows happened; `None` states it
    /// follows none.
    fn set_prevunix(&mut self, unix: Option<i64>);

    /// The identity of the event this one follows, where it follows one.
    fn get_prevuuid(&self) -> Option<Uuid>;

    /// Records the identity of the event this one follows; `None` states
    /// it follows none.
    fn set_prevuuid(&mut self, uuid: Option<Uuid>);

    /// The grid instant this event was read as the snapshot of, in the
    /// same count as [`Self::get_unix`], where a walk over a grid took one
    /// of it: the opening instant of the grid step its instant fell in.
    fn get_snapunix(&self) -> Option<i64>;

    /// Records the grid instant this event is the snapshot of; `None`
    /// states no snapshot was taken of it.
    fn set_snapunix(&mut self, unix: Option<i64>);

    /// This event stated as the one after `previous`, by the timed
    /// reading, or nothing where it cannot follow - its own predecessor, or
    /// one that happened after it - or where following it changes nothing,
    /// because it already follows it.
    ///
    /// The predecessor's identity and instant are recorded on it, its place
    /// in the chain is the one after the predecessor's, and the lifecycle
    /// carries forward: the creation is the earliest the two know, the
    /// expiration the latest, the state the furthest along, and each where
    /// only one states it is that one's; the names the predecessor went by
    /// and this event does not state are taken, because a name issued at
    /// creation holds for the whole lifecycle; and the predecessor's cross
    /// code is forced onto this event where its own differs, with the cross
    /// hash code and the cross element brought in step, because two events
    /// of one chain share it. What the event itself says - its instant, its
    /// parents, the snapshot it is - is its own and moves nowhere. An event
    /// that moved is finalized, so it never carries the identity of what it
    /// was.
    ///
    /// Provided, so an implementor's [`Element::with_previous`] has a
    /// default to delegate to; an event that means something else by
    /// following states its own there instead.
    fn following(mut self, previous: &Self) -> Option<Self>
    where
        Self: Sized,
    {
        if previous.get_curruuid() == self.get_curruuid()
            || previous.get_unix() > self.get_unix()
            || !follow_timed(&mut self, previous)
        {
            return None;
        }
        self.finalize();
        Some(self)
    }

    /// This event stated as another statement of `live`: the same event -
    /// the same instant, the same content - read a second time, as a
    /// capture logs one message at every hop it passes. It takes the place
    /// `live` holds in its chain - the predecessor, the position, the
    /// snapshot - the chain's cross code, the names and parents `live`
    /// knows, and the lifecycle folded, so the two statements finalize to
    /// one identity and the chain grows by nothing. What the event states
    /// of its own - its instant, its content - is its own.
    ///
    /// The caller establishes that the event is `live`'s twin, by the
    /// identity `live` arrived under: once `live` has followed something
    /// its identity has moved, and the event alone cannot tell a twin from
    /// a successor. Provided, and what [`EventIterator`](super::EventIterator)
    /// yields for an arrival under the identity a live element arrived
    /// under.
    fn restating(mut self, live: &Self) -> Self
    where
        Self: Sized,
    {
        restate_event(&mut self, live);
        self.fold_lifecycle(live);
        self.finalize();
        self
    }

    /// This event with another statement of itself folded in, by the
    /// timed reading, or nothing where `other` is another event.
    ///
    /// The element-level merge first - the cross code where this one states
    /// none, the parents' union - and then the timed facts: the instant is
    /// the later of the two and the code the later
    /// statement's, because the last word on what an event says is the
    /// latest one; the place in the chain is the further of the two; the
    /// lifecycle folds as [`Self::following`] folds it - earliest creation,
    /// latest expiration, furthest state; and the predecessor and the
    /// snapshot instant are this event's where it states them, else the
    /// other's.
    ///
    /// Nothing where `other` is another event, and nothing where the fold
    /// changes nothing, so a caller skips a restatement it already holds;
    /// an event that moved is finalized.
    ///
    /// Provided, so an implementor's [`Element::merge_with`] has a default
    /// to delegate to.
    fn merging(mut self, other: &Self) -> Option<Self>
    where
        Self: Sized,
    {
        if other.get_curruuid() != self.get_curruuid() {
            return None;
        }
        let changed = merge_element(&mut self, other);
        if !(merge_timed(&mut self, other) || changed) {
            return None;
        }
        self.finalize();
        Some(self)
    }

    /// Folds another event's lifecycle into this one: the earliest
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
            self.get_creatunix(),
            earliest(self.get_creatunix(), other.get_creatunix()),
            |unix| self.set_creatunix(unix),
        );
        changed |= moved(
            self.get_expirunix(),
            latest(self.get_expirunix(), other.get_expirunix()),
            |unix| self.set_expirunix(unix),
        );
        let state = self.get_state().clone().merge_with(other.get_state());
        changed |= moved(self.get_state().clone(), state, |state| {
            self.set_state(state)
        });
        changed
    }

    /// Records the code this event's content digests to and resets the
    /// identity the instant and the code derive, and the cross element
    /// behind it.
    ///
    /// Provided, and what an implementor's [`Element::finalize`] hands the
    /// digest of its content to. The identity is [`Self::time_uuid`], and
    /// an instant a UUIDv7 cannot hold leaves the identity as it was; the
    /// cross element is [`Element::cross_uuid`] over the identity that
    /// results, which is the identity itself for an event in no chain.
    fn finalized(&mut self, hashcode: u64) {
        self.set_hashcode(hashcode);
        if let Ok(uuid) = self.time_uuid() {
            self.set_curruuid(uuid);
        }
        let crossuuid = self.cross_uuid();
        self.set_crossuuid(crossuuid);
    }

    /// Continues [`Element::digest`] with the facts an event states that
    /// are not instants: the state, the place in the chain and the
    /// predecessor's identity.
    ///
    /// Provided, for an implementor's [`Element::finalize`] to feed its own
    /// content behind. The instants - when it happened, was created,
    /// expires, the predecessor's and the snapshot's - are left out, so the
    /// code says what an event states and not when.
    fn digest_event(&self) -> Xxh3 {
        let mut state = self.digest();
        feed_timed(&mut state, self);
        state
    }

    /// The instant and the code coupled: a [`TxHash`] of [`Self::get_unix`]
    /// at nanosecond resolution and [`Element::get_hashcode`] as the XXH3-64
    /// digest it is, which is the crate's own time-ordered identity.
    ///
    /// Provided, so every event derives it the same way.
    ///
    /// # Errors
    ///
    /// Returns the [`TxHash`]'s own refusal, which a nanosecond count never
    /// raises.
    fn txhash(&self) -> Result<TxHash> {
        coupled(self.get_unix(), self.get_hashcode())
    }

    /// The identity the instant and the code derive: the UUID
    /// [`TxHash::into_uuid`] answers for [`Self::txhash`], RFC 9562 UUIDv7
    /// with the instant in front so identities sort by instant first, to
    /// the microsecond, and by content second.
    ///
    /// Provided: an implementor whose identity is when it happened and what
    /// it says answers this from [`Element::get_curruuid`], and one whose
    /// identity is assigned keeps its own.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::txhash`] returns.
    fn time_uuid(&self) -> Result<Uuid> {
        self.txhash()?.into_uuid()
    }
}

/// An element that stands in a market: a price, a quantity, and which side
/// of the market it stood on, with no instant of its own.
///
/// Five facts beside what an element already states, each read and
/// written: `px` is the price, a [`Decimal`] - exact, as a market's numbers
/// are - and `currency` the [`Currency`] it is quoted in; `qty` is the
/// quantity, a [`Decimal`] too, and `unit` the text it is counted in - a
/// lot, a barrel, a megawatt-hour, whatever the market says; `side` is the
/// crate's [`Side`] code, FIX's `Side(54)`. Two lanes state the quote the
/// element makes, each optional and each the same four facts - `bidpx`,
/// `bidcurrency`, `bidqty`, `bidunit` for what the element would pay, and
/// the `ask` four for what it would be paid - and [`Self::fill_lanes`] fills
/// the lane the side implies from the price, the currency, the quantity and
/// the unit where the lane states nothing. Six more name the instrument and
/// the market, each optional because a market names an instrument the way
/// it does: the [`Isin`], the [`Cusip`], the [`Sedol`], the [`Bloomberg`]
/// identifier, the [`Cfi`] classification and the [`Mic`] of the market it
/// traded on, the crate's own validated codes.
///
/// Two more readings are provided. [`Self::digest_market`] continues
/// [`Element::digest`] with the market's facts, for an implementor's
/// [`Element::finalize`]. [`Self::merging_market`] is what merging means
/// for a market element with no instant to say which statement is later:
/// the element-level merge, then this element's price, quantity and unit
/// standing, and each code the better of the two as
/// [`CodeValue::merge_with`] reads it, this element leading - which an
/// implementor's [`Element::merge_with`] delegates to. A market element
/// that also happened at an instant is a [`MarketEvent`], whose readings
/// let the later statement lead. What a price of nothing or a quantity of
/// zero means is the market's to say.
///
/// ```
/// use yggdryl::graph::{Element, MarketElement, MarketElementData};
/// use yggdryl::types::{Cfi, Currency, Decimal, Isin, Side};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut trade = MarketElementData::default();
/// trade.set_px("82.5".parse()?);
/// trade.set_currency(Currency::new("USD")?);
/// trade.set_qty(Decimal::from_int(1_000));
/// trade.set_unit("bbl".to_owned());
/// trade.set_side(Side::read("1")?);
/// trade.set_isincode(Some(Isin::new("US0378331005")?));
/// assert_eq!(trade.get_px().to_string(), "82.5");
/// assert_eq!(trade.get_currency().as_str(), "USD");
/// assert_eq!(trade.get_unit(), "bbl");
/// // A buy is a bid: the lane the side implies fills from the trade's own
/// // facts, and the other lane stays empty.
/// trade.fill_lanes();
/// assert_eq!(trade.get_bidpx(), Some("82.5".parse()?));
/// assert_eq!(trade.get_bidcurrency().map(Currency::as_str), Some("USD"));
/// assert_eq!((trade.get_bidqty(), trade.get_bidunit()), (Some(Decimal::from_int(1_000)), Some("bbl")));
/// assert_eq!(trade.get_askpx(), None);
/// // A market element is an element: one walk reads both.
/// let held: &dyn MarketElement = &trade;
/// assert_eq!(held.get_side().as_str(), "BUY");
/// assert_eq!(held.get_isincode().map(Isin::as_str), Some("US0378331005"));
/// assert!(held.get_cusipcode().is_none(), "an instrument is named the way the market names it");
/// // Finalized, its identity is what it states.
/// trade.finalize();
/// let mut same = trade.clone();
/// same.set_curruuid(Default::default());
/// same.finalize();
/// assert_eq!(same.get_curruuid(), trade.get_curruuid());
/// // Another statement of the trade merges in: this one's price stands,
/// // and the CFI the other states fills what this one left unknown.
/// let mut other = trade.clone();
/// other.set_px("83".parse()?);
/// other.set_cficode(Some(Cfi::new("ESVUFR")?));
/// trade.set_cficode(Some(Cfi::new("ESXXXR")?));
/// let merged = trade.merge_with(&other).expect("the same trade");
/// assert_eq!(merged.get_px().to_string(), "82.5");
/// assert_eq!(merged.get_cficode().map(Cfi::as_str), Some("ESVUFR"));
/// # Ok(())
/// # }
/// ```
pub trait MarketElement: Element {
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

    /// The unit the quantity is counted in; empty where the market says none.
    fn get_unit(&self) -> &str;

    /// Records the unit the quantity is counted in.
    fn set_unit(&mut self, unit: String);

    /// Which side of the market the element stood on.
    fn get_side(&self) -> &Side;

    /// Records which side of the market the element stood on.
    fn set_side(&mut self, side: Side);

    /// The instrument's ISIN, where the market named it by one.
    fn get_isincode(&self) -> Option<&Isin>;

    /// Records the instrument's ISIN; `None` states the market named none.
    fn set_isincode(&mut self, isincode: Option<Isin>);

    /// The instrument's CUSIP, where the market named it by one.
    fn get_cusipcode(&self) -> Option<&Cusip>;

    /// Records the instrument's CUSIP; `None` states the market named none.
    fn set_cusipcode(&mut self, cusipcode: Option<Cusip>);

    /// The instrument's SEDOL, where the market named it by one.
    fn get_sedolcode(&self) -> Option<&Sedol>;

    /// Records the instrument's SEDOL; `None` states the market named none.
    fn set_sedolcode(&mut self, sedolcode: Option<Sedol>);

    /// The instrument's Bloomberg identifier, where the market named it by
    /// one.
    fn get_bloombergcode(&self) -> Option<&Bloomberg>;

    /// Records the instrument's Bloomberg identifier; `None` states the
    /// market named none.
    fn set_bloombergcode(&mut self, bloombergcode: Option<Bloomberg>);

    /// The instrument's CFI classification, where the market stated it.
    fn get_cficode(&self) -> Option<&Cfi>;

    /// Records the instrument's CFI classification; `None` states the
    /// market stated none.
    fn set_cficode(&mut self, cficode: Option<Cfi>);

    /// The market the element traded on, as its ISO 10383 MIC, where the
    /// element names it.
    fn get_miccode(&self) -> Option<&Mic>;

    /// Records the market the element traded on; `None` states it names
    /// none.
    fn set_miccode(&mut self, miccode: Option<Mic>);

    /// The price the element last traded at, where it states one.
    ///
    /// What [`Self::get_px`] settles on is the price the element is *about*:
    /// what it orders, else what it last traded, else what it averaged. This
    /// is the last trade alone, so a fill and the order it fills are told
    /// apart without reading which field each settled from.
    fn get_lastpx(&self) -> Option<Decimal>;

    /// Records the price the element last traded at; `None` states none.
    fn set_lastpx(&mut self, px: Option<Decimal>);

    /// The quantity the element last traded, where it states one.
    fn get_lastqty(&self) -> Option<Decimal>;

    /// Records the quantity the element last traded; `None` states none.
    fn set_lastqty(&mut self, qty: Option<Decimal>);

    /// How long the element stands, where it says: FIX's `TimeInForce`, as
    /// the element states it. A market fact rather than a protocol one - it
    /// is what a resting order and a fill-or-kill differ by - and free text
    /// here, because what the code `1` names is the dictionary's to say and
    /// not this trait's: a caller that wants `GoodTillCancel` reads the
    /// code set the registry holds for the tag.
    fn get_tif(&self) -> Option<&str>;

    /// Records how long the element stands; `None` states nothing.
    fn set_tif(&mut self, tif: Option<String>);

    /// Whether the element can be traded right now, where the market says.
    ///
    /// A fact about the instrument and its session rather than about the
    /// element: a halt, a closed session, a delisting. `None` states the
    /// market said nothing either way, which is not the same as a `false`
    /// - a status a venue spells `Unknown or Invalid` closes nothing.
    fn get_tradable(&self) -> Option<bool>;

    /// Records whether the element can be traded right now; `None` states
    /// the market said nothing either way.
    fn set_tradable(&mut self, tradable: Option<bool>);

    /// The ticker the element's instrument is known by, where it is known
    /// by one: the human-readable name a screen shows it under, free text
    /// rather than a code, because a venue's ticker answers to no standard
    /// the way an ISIN or a MIC does.
    ///
    /// Beside the instrument codes rather than among them: the codes name
    /// the instrument to a system, and this names it to a person.
    fn get_symbolticker(&self) -> Option<&str>;

    /// Records the ticker the element's instrument is known by; `None`
    /// states it is known by none.
    fn set_symbolticker(&mut self, ticker: Option<String>);

    /// The volume-weighted price the element averaged, where it states one.
    fn get_avgpx(&self) -> Option<Decimal>;

    /// Records the price the element averaged; `None` states none.
    fn set_avgpx(&mut self, px: Option<Decimal>);

    /// How much of the element's quantity is done, where it states it.
    fn get_cumqty(&self) -> Option<Decimal>;

    /// Records how much of it is done; `None` states none.
    fn set_cumqty(&mut self, qty: Option<Decimal>);

    /// How much of it is still open, where it states it.
    fn get_leavesqty(&self) -> Option<Decimal>;

    /// Records how much of it is still open; `None` states none.
    fn set_leavesqty(&mut self, qty: Option<Decimal>);

    /// The price stated before this element, where one was.
    ///
    /// What the element itself says about the price before its own - a
    /// closing price it carries - else what the statement before it in its
    /// chain stated, which a walk fills as it fills the instants. It is what
    /// a move is measured against: a price beside the price it moved from.
    fn get_prevpx(&self) -> Option<Decimal>;

    /// Records the price stated before this element; `None` states none.
    fn set_prevpx(&mut self, px: Option<Decimal>);

    /// The quantity stated before this element, where one was.
    fn get_prevqty(&self) -> Option<Decimal>;

    /// Records the quantity stated before this element; `None` states none.
    fn set_prevqty(&mut self, qty: Option<Decimal>);

    /// The bid lane's price, where the element states one.
    fn get_bidpx(&self) -> Option<Decimal>;

    /// Records the bid lane's price; `None` states the lane has none.
    fn set_bidpx(&mut self, px: Option<Decimal>);

    /// The currency the bid lane is quoted in, where the element states one.
    fn get_bidcurrency(&self) -> Option<&Currency>;

    /// Records the currency the bid lane is quoted in.
    fn set_bidcurrency(&mut self, currency: Option<Currency>);

    /// The bid lane's quantity, where the element states one.
    fn get_bidqty(&self) -> Option<Decimal>;

    /// Records the bid lane's quantity.
    fn set_bidqty(&mut self, qty: Option<Decimal>);

    /// The unit the bid lane's quantity is counted in, where stated.
    fn get_bidunit(&self) -> Option<&str>;

    /// Records the unit the bid lane's quantity is counted in.
    fn set_bidunit(&mut self, unit: Option<String>);

    /// The ask lane's price, where the element states one.
    fn get_askpx(&self) -> Option<Decimal>;

    /// Records the ask lane's price; `None` states the lane has none.
    fn set_askpx(&mut self, px: Option<Decimal>);

    /// The currency the ask lane is quoted in, where the element states one.
    fn get_askcurrency(&self) -> Option<&Currency>;

    /// Records the currency the ask lane is quoted in.
    fn set_askcurrency(&mut self, currency: Option<Currency>);

    /// The ask lane's quantity, where the element states one.
    fn get_askqty(&self) -> Option<Decimal>;

    /// Records the ask lane's quantity.
    fn set_askqty(&mut self, qty: Option<Decimal>);

    /// The unit the ask lane's quantity is counted in, where stated.
    fn get_askunit(&self) -> Option<&str>;

    /// Records the unit the ask lane's quantity is counted in.
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
    /// Fills every market fact this element implies from the ones it
    /// states, and stops where it would be inventing.
    ///
    /// What a market element says comes in three shapes that repeat one
    /// another: the price and quantity it is *about*, the last trade it
    /// reports, and the two lanes a quote is made of. A message states some
    /// of them and leaves the rest to be read off what it stated, so this
    /// reads them, in one order, each rule filling only what is still
    /// unstated:
    ///
    /// 1. The price is what the element is about, else what it last traded,
    ///    else what it averaged, else what its own side's lane quotes - a
    ///    report stating only `LastPx` is about that price, and a quote
    ///    stating only its bid is about that bid. The quantity reads the
    ///    same way: what it orders, else what it last traded, else the
    ///    lane's size. How much is done and how much is left are not on
    ///    that ladder, because together they *are* the quantity ordered and
    ///    the dictionary already says so - one rule, in one place.
    /// 2. The currency and the unit are the element's own, else the ones the
    ///    side's lane states: a lane priced in a currency prices the element
    ///    in it.
    /// 3. The side's lane is then filled from all of that, by
    ///    [`Self::fill_lanes`]: a buy at a price is a party willing to pay
    ///    it, and a sell at one is a party willing to be paid it.
    ///
    /// Nothing is invented: a price of nothing, a quantity of nothing, no
    /// currency and no unit fill nothing, an element that states no side
    /// fills no lane, and a fact the element stated is never overwritten.
    /// Running it twice changes nothing the first run did not.
    ///
    /// Provided, and what an implementor's [`Element::finalize`] runs before
    /// it digests, so the code an element answers to covers what it implies
    /// as well as what it wrote.
    fn fill_market(&mut self)
    where
        Self: Sized,
    {
        let side = self.get_side();
        let (bid, ask) = (side.is_bid(), side.is_ask());
        // The lane the element's own side quotes, which is the only lane its
        // own facts can be read off: a buy is about the bid it is willing to
        // pay, and the other lane is the other party's.
        let lane_px = if bid {
            self.get_bidpx()
        } else if ask {
            self.get_askpx()
        } else {
            None
        };
        let lane_qty = if bid {
            self.get_bidqty()
        } else if ask {
            self.get_askqty()
        } else {
            None
        };
        let lane_currency = if bid {
            self.get_bidcurrency().cloned()
        } else if ask {
            self.get_askcurrency().cloned()
        } else {
            None
        };
        let lane_unit = if bid {
            self.get_bidunit().map(str::to_owned)
        } else if ask {
            self.get_askunit().map(str::to_owned)
        } else {
            None
        };
        if self.get_px() == Decimal::ZERO {
            if let Some(px) = self.get_lastpx().or_else(|| self.get_avgpx()).or(lane_px) {
                self.set_px(px);
            }
        }
        if self.get_qty() == Decimal::ZERO {
            if let Some(qty) = self.get_lastqty().or(lane_qty) {
                self.set_qty(qty);
            }
        }
        if self.get_currency() == &Currency::none() {
            if let Some(currency) = lane_currency {
                self.set_currency(currency);
            }
        }
        if self.get_unit().is_empty() {
            if let Some(unit) = lane_unit {
                self.set_unit(unit);
            }
        }
        self.fill_lanes();
    }

    fn fill_lanes(&mut self)
    where
        Self: Sized,
    {
        let side = self.get_side();
        let (bid, ask) = (side.is_bid(), side.is_ask());
        if !bid && !ask {
            return;
        }
        // Only a stated fact fills a lane: a price or a quantity of nothing,
        // no currency, no unit, is nothing to state on the lane either.
        let px = Some(self.get_px()).filter(|px| *px != Decimal::ZERO);
        let qty = Some(self.get_qty()).filter(|qty| *qty != Decimal::ZERO);
        let currency = Some(self.get_currency().clone()).filter(|held| *held != Currency::none());
        let unit = Some(self.get_unit().to_owned()).filter(|unit| !unit.is_empty());
        if bid {
            if self.get_bidpx().is_none() {
                self.set_bidpx(px);
            }
            if self.get_bidcurrency().is_none() {
                self.set_bidcurrency(currency);
            }
            if self.get_bidqty().is_none() {
                self.set_bidqty(qty);
            }
            if self.get_bidunit().is_none() {
                self.set_bidunit(unit);
            }
        } else {
            if self.get_askpx().is_none() {
                self.set_askpx(px);
            }
            if self.get_askcurrency().is_none() {
                self.set_askcurrency(currency);
            }
            if self.get_askqty().is_none() {
                self.set_askqty(qty);
            }
            if self.get_askunit().is_none() {
                self.set_askunit(unit);
            }
        }
    }

    /// Continues [`Element::digest`] with the market's facts: the price,
    /// the currency, the quantity, the unit, the side, each instrument code
    /// the market names, the market itself, and each lane fact stated.
    ///
    /// Provided, for an implementor's [`Element::finalize`] to feed its own
    /// content behind.
    fn digest_market(&self) -> Xxh3 {
        let mut state = self.digest();
        feed_market(&mut state, self);
        state
    }

    /// This element with another statement of itself folded in, by the
    /// market reading with no instant to say which is later, or nothing
    /// where `other` is another element or where the fold changes nothing.
    ///
    /// The element-level merge first - the cross code, the identifiers, the
    /// parents' union - and then the market's facts
    /// with this element leading: its price, quantity, unit and lane facts
    /// stand where it states them, and the currency, the side and each
    /// instrument code are the better of the two statements as
    /// [`CodeValue::merge_with`] reads them, the other filling what this
    /// one leaves unknown - a `XXX` currency, an `UNKNOWN` side, an `X` in
    /// a CFI. An element that moved is finalized.
    ///
    /// Provided, so an implementor's [`Element::merge_with`] has a default
    /// to delegate to.
    fn merging_market(mut self, other: &Self) -> Option<Self>
    where
        Self: Sized,
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
}

/// An element that happened in a market at one instant: an [`Event`] that
/// is a [`MarketElement`], which every type implementing both is.
///
/// Nothing to implement: the two supertraits state the facts, and this
/// trait provides the readings that need both. [`Self::digest_market_event`]
/// continues [`Event::digest_event`] with the market's facts, for an
/// implementor's [`Element::finalize`]; [`Self::merging_market_event`] is
/// what merging means for a market event - the timed merge, then the later
/// statement's price, quantity and unit, and each code the better of the
/// two as [`CodeValue::merge_with`] reads it, the later statement leading -
/// which an implementor's [`Element::merge_with`] delegates to. Following
/// is the timed reading, [`Event::following`], unchanged.
///
/// ```
/// use yggdryl::graph::{Element, Event, MarketElement, MarketEvent, MarketEventData};
/// use yggdryl::types::{Cfi, Currency, Decimal, Side};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut trade = MarketEventData::at(10);
/// trade.set_px("82.5".parse()?);
/// trade.set_currency(Currency::new("USD")?);
/// trade.set_qty(Decimal::from_int(1_000));
/// trade.set_side(Side::read("1")?);
/// trade.set_cficode(Some(Cfi::new("ESXXXR")?));
/// trade.finalize();
/// // A market event is an event is a market element is an element: one
/// // walk reads all.
/// let held: &dyn MarketEvent = &trade;
/// assert_eq!(held.get_unix(), 10);
/// assert_eq!(held.get_side().as_str(), "BUY");
/// assert_eq!(held.get_curruuid(), trade.time_uuid()?);
/// // A later statement of the trade merges in: its price has the last
/// // word, and the CFI it states fills what this one left unknown.
/// let mut later = trade.clone();
/// later.set_unix(20);
/// later.set_px("83".parse()?);
/// later.set_cficode(Some(Cfi::new("ESVUFR")?));
/// let merged = trade.merge_with(&later).expect("the same trade");
/// assert_eq!(merged.get_px(), Decimal::from_int(83));
/// assert_eq!(merged.get_unix(), 20);
/// assert_eq!(merged.get_cficode().map(Cfi::as_str), Some("ESVUFR"));
/// # Ok(())
/// # }
/// ```
pub trait MarketEvent: Event + MarketElement {
    /// Continues [`Event::digest_event`] with the market's facts, exactly
    /// as [`MarketElement::digest_market`] continues [`Element::digest`].
    ///
    /// Provided, for an implementor's [`Element::finalize`] to feed its own
    /// content behind.
    fn digest_market_event(&self) -> Xxh3 {
        let mut state = self.digest_event();
        feed_market(&mut state, self);
        state
    }

    /// This event with another statement of itself folded in, by the
    /// market reading, or nothing where `other` is another event.
    ///
    /// The timed merge first - [`Event::merging`] - and then the market's
    /// facts: the later statement's price, quantity and unit have the last
    /// word, as its instant and code do; the currency, the side and each
    /// instrument code are the better of the two statements as
    /// [`CodeValue::merge_with`] reads them, the later statement leading and
    /// the earlier filling what it leaves unknown - a `XXX` currency, an
    /// `UNKNOWN` side, an `X` in a CFI - and a code only one statement
    /// names is that one's; each lane fact is the later statement's where
    /// it states one, else this one's. An equal instant keeps this event
    /// leading. Nothing where the fold changes nothing, and an event that
    /// moved is finalized.
    ///
    /// Provided, so an implementor's [`Element::merge_with`] has a default
    /// to delegate to.
    /// This market event stated as the one after `previous`: the timed
    /// reading, then the price and the quantity that statement settled on
    /// as the step before this one, and what the chain is about - the
    /// instrument's names, its market, the currency, the unit, the side,
    /// the time in force and whether it can trade - where this event
    /// states none of it.
    ///
    /// Provided, and what an implementor's [`Element::with_previous`]
    /// delegates to where following means carrying the step before along.
    /// An event that moved is finalized.
    fn following_market(self, previous: &Self) -> Option<Self>
    where
        Self: Sized,
    {
        let mut this = self.following(previous)?;
        if follow_market(&mut this, previous) {
            this.finalize();
        }
        Some(this)
    }

    fn merging_market_event(mut self, other: &Self) -> Option<Self>
    where
        Self: Sized,
    {
        if other.get_curruuid() != self.get_curruuid() {
            return None;
        }
        // Which statement is the later one is read before the timed merge
        // moves this event's instant to it.
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

impl<E: Event + MarketElement + ?Sized> MarketEvent for E {}

/// Continues a digest with the market's facts: the price, the currency,
/// the quantity, the unit, the side, each instrument code the market
/// names, the market itself, and each lane fact stated.
fn feed_market<E: MarketElement + ?Sized>(state: &mut Xxh3, this: &E) {
    feed(state, "px", &this.get_px().units().to_le_bytes());
    feed(state, "currency", this.get_currency().as_str().as_bytes());
    feed(state, "qty", &this.get_qty().units().to_le_bytes());
    // What the element traded and how far it has got are its own statements
    // and part of what it says; what came before it is not, so the previous
    // price and quantity are left out exactly as the predecessor's instant
    // and identity are.
    if let Some(tif) = this.get_tif() {
        feed(state, "tif", tif.as_bytes());
    }
    if let Some(tradable) = this.get_tradable() {
        feed(state, "tradable", &[u8::from(tradable)]);
    }
    if let Some(ticker) = this.get_symbolticker() {
        feed(state, "symbolticker", ticker.as_bytes());
    }
    for (name, held) in [
        ("lastpx", this.get_lastpx()),
        ("lastqty", this.get_lastqty()),
        ("avgpx", this.get_avgpx()),
        ("cumqty", this.get_cumqty()),
        ("leavesqty", this.get_leavesqty()),
    ] {
        if let Some(held) = held {
            feed(state, name, &held.units().to_le_bytes());
        }
    }
    feed(state, "unit", this.get_unit().as_bytes());
    feed(state, "side", this.get_side().as_str().as_bytes());
    let codes: [(&str, Option<&str>); 6] = [
        ("isincode", this.get_isincode().map(Isin::as_str)),
        ("cusipcode", this.get_cusipcode().map(Cusip::as_str)),
        ("sedolcode", this.get_sedolcode().map(Sedol::as_str)),
        (
            "bloombergcode",
            this.get_bloombergcode().map(Bloomberg::as_str),
        ),
        ("cficode", this.get_cficode().map(Cfi::as_str)),
        ("miccode", this.get_miccode().map(Mic::as_str)),
    ];
    for (name, code) in codes {
        if let Some(code) = code {
            feed(state, name, code.as_bytes());
        }
    }
    feed_lane(
        state,
        "bid",
        this.get_bidpx(),
        this.get_bidcurrency(),
        this.get_bidqty(),
        this.get_bidunit(),
    );
    feed_lane(
        state,
        "ask",
        this.get_askpx(),
        this.get_askcurrency(),
        this.get_askqty(),
        this.get_askunit(),
    );
}

/// The market's facts an element takes from another statement of itself:
/// the later statement's price, quantity, unit and lane facts, and each
/// code the better of the two; whether any moved. `later` says whether
/// `other` is the later statement.
/// The market facts an event takes from the statement it follows: the price
/// and the quantity that statement settled on as the step before this one,
/// and what the chain itself is about where this statement says nothing of
/// it.
///
/// A chain is what a price moved along, and a message states where it is
/// rather than where it was, so the move is only readable with the step
/// before it beside it. What the event states stays: a message carrying its
/// own closing price has already said what it means by the price before.
pub(super) fn follow_market<E: MarketElement + ?Sized>(this: &mut E, previous: &E) -> bool {
    let mut changed = false;
    if this.get_prevpx().is_none() {
        let px = Some(previous.get_px()).filter(|px| *px != Decimal::ZERO);
        changed |= moved(this.get_prevpx(), px, |px| this.set_prevpx(px));
    }
    if this.get_prevqty().is_none() {
        let qty = Some(previous.get_qty()).filter(|qty| *qty != Decimal::ZERO);
        changed |= moved(this.get_prevqty(), qty, |qty| this.set_prevqty(qty));
    }
    changed | chain_market(this, previous)
}

/// The market facts a restatement takes from the live element it is another
/// reading of: the step before it, which is the place in the chain and not
/// something a second reading of one message sees for itself, and what the
/// chain is about. A twin that says nothing of either is about what the
/// live statement was about.
pub(super) fn restate_market<E: MarketElement + ?Sized>(this: &mut E, live: &E) -> bool {
    let mut changed = moved(
        this.get_prevpx(),
        stated(this.get_prevpx(), live.get_prevpx(), false),
        |px| this.set_prevpx(px),
    );
    changed |= moved(
        this.get_prevqty(),
        stated(this.get_prevqty(), live.get_prevqty(), false),
        |qty| this.set_prevqty(qty),
    );
    changed | chain_market(this, live)
}

/// What the chain an element stands in is about, taken from another
/// statement of that chain where this one says nothing of it; whether any
/// moved.
///
/// A chain follows one instrument in one session: the names it goes by, the
/// market it trades on, what it is quoted in and counted in, the side it
/// takes, how long it stands and whether it can trade at all are the
/// chain's, so a report that names none of them is about the ones the
/// statement beside it named. This statement always leads - a code it
/// spells better is never replaced by a weaker one - and nothing here is
/// about a step: the price and the quantity a predecessor settled on reach
/// an element as `prevpx` and `prevqty`, never as its own.
fn chain_market<E: MarketElement + ?Sized>(this: &mut E, previous: &E) -> bool {
    let mut changed = moved(
        this.get_currency().clone(),
        better(this.get_currency().clone(), previous.get_currency(), false),
        |currency| this.set_currency(currency),
    );
    changed |= moved(
        this.get_side().clone(),
        better(this.get_side().clone(), previous.get_side(), false),
        |side| this.set_side(side),
    );
    if this.get_unit().is_empty() {
        changed |= moved(
            this.get_unit().to_owned(),
            previous.get_unit().to_owned(),
            |unit| this.set_unit(unit),
        );
    }
    changed |= moved(
        this.get_tif().map(str::to_owned),
        stated(
            this.get_tif().map(str::to_owned),
            previous.get_tif().map(str::to_owned),
            false,
        ),
        |tif| this.set_tif(tif),
    );
    changed |= moved(
        this.get_tradable(),
        stated(this.get_tradable(), previous.get_tradable(), false),
        |tradable| this.set_tradable(tradable),
    );
    changed |= moved(
        this.get_symbolticker().map(str::to_owned),
        stated(
            this.get_symbolticker().map(str::to_owned),
            previous.get_symbolticker().map(str::to_owned),
            false,
        ),
        |ticker| this.set_symbolticker(ticker),
    );
    changed |= moved(
        this.get_isincode().cloned(),
        better_stated(this.get_isincode().cloned(), previous.get_isincode(), false),
        |code| this.set_isincode(code),
    );
    changed |= moved(
        this.get_cusipcode().cloned(),
        better_stated(
            this.get_cusipcode().cloned(),
            previous.get_cusipcode(),
            false,
        ),
        |code| this.set_cusipcode(code),
    );
    changed |= moved(
        this.get_sedolcode().cloned(),
        better_stated(
            this.get_sedolcode().cloned(),
            previous.get_sedolcode(),
            false,
        ),
        |code| this.set_sedolcode(code),
    );
    changed |= moved(
        this.get_bloombergcode().cloned(),
        better_stated(
            this.get_bloombergcode().cloned(),
            previous.get_bloombergcode(),
            false,
        ),
        |code| this.set_bloombergcode(code),
    );
    changed |= moved(
        this.get_cficode().cloned(),
        better_stated(this.get_cficode().cloned(), previous.get_cficode(), false),
        |code| this.set_cficode(code),
    );
    changed |= moved(
        this.get_miccode().cloned(),
        better_stated(this.get_miccode().cloned(), previous.get_miccode(), false),
        |code| this.set_miccode(code),
    );
    changed
}

fn merge_market<E: MarketElement + ?Sized>(this: &mut E, other: &E, later: bool) -> bool {
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
    // What the element traded, how far it has got, how long it stands,
    // whether it can trade at all, and the step before it all fold as a
    // lane folds: the statement that has one keeps it, and the later one
    // leads where both do.
    changed |= moved(
        this.get_lastpx(),
        stated(this.get_lastpx(), other.get_lastpx(), later),
        |px| this.set_lastpx(px),
    );
    changed |= moved(
        this.get_lastqty(),
        stated(this.get_lastqty(), other.get_lastqty(), later),
        |qty| this.set_lastqty(qty),
    );
    changed |= moved(
        this.get_avgpx(),
        stated(this.get_avgpx(), other.get_avgpx(), later),
        |px| this.set_avgpx(px),
    );
    changed |= moved(
        this.get_cumqty(),
        stated(this.get_cumqty(), other.get_cumqty(), later),
        |qty| this.set_cumqty(qty),
    );
    changed |= moved(
        this.get_leavesqty(),
        stated(this.get_leavesqty(), other.get_leavesqty(), later),
        |qty| this.set_leavesqty(qty),
    );
    changed |= moved(
        this.get_tif().map(str::to_owned),
        stated(
            this.get_tif().map(str::to_owned),
            other.get_tif().map(str::to_owned),
            later,
        ),
        |tif| this.set_tif(tif),
    );
    changed |= moved(
        this.get_tradable(),
        stated(this.get_tradable(), other.get_tradable(), later),
        |tradable| this.set_tradable(tradable),
    );
    changed |= moved(
        this.get_symbolticker().map(str::to_owned),
        stated(
            this.get_symbolticker().map(str::to_owned),
            other.get_symbolticker().map(str::to_owned),
            later,
        ),
        |ticker| this.set_symbolticker(ticker),
    );
    changed |= moved(
        this.get_prevpx(),
        stated(this.get_prevpx(), other.get_prevpx(), later),
        |px| this.set_prevpx(px),
    );
    changed |= moved(
        this.get_prevqty(),
        stated(this.get_prevqty(), other.get_prevqty(), later),
        |qty| this.set_prevqty(qty),
    );
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
