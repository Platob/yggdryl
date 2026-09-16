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

use crate::hashing::txhash::TxHash;
use crate::types::{Bloomberg, Cfi, Currency, Cusip, Isin, Sedol, Side, State, Uuid};
use crate::{Digest, DigestAlgorithm, Error, Result, TimeUnit};

/// One element of a graph: a node that knows its own identity, the identity
/// it has elsewhere, and the identities of the elements it descends from.
///
/// The parents are an ordered list, and the order is the implementor's to
/// state: a message's chain, a node's lineage. An element with no parent is
/// a root. The cross element, `xuuid`, is what this element is in another
/// graph - the same order in a venue's book and in a ledger - where it has
/// one. Every fact is read and written through the trait, so a store or a
/// walk that only knows an element as `dyn Element` can still place it.
///
/// Two readings come with the facts. [`Self::with_previous`] states this
/// element as the one after another, and is the one signature the trait
/// leaves to the implementor: what following means is the element's own - a
/// chain entry records its predecessor, a snapshot its base - and
/// [`TimeElement::following`] is the reading a timed element delegates to.
/// [`Self::merge_with`] folds another statement of the same element into
/// this one, and is provided: the cross element is taken where this one
/// states none, and the parents are the union in this element's order, then
/// the other's. A timed element delegates to [`TimeElement::merging`], which
/// folds the instants and the state too.
///
/// ```
/// use yggdryl::graph::Element;
/// use yggdryl::types::Uuid;
///
/// struct Node {
///     uuid: Uuid,
///     xuuid: Option<Uuid>,
///     parents: Vec<Uuid>,
/// }
///
/// impl Element for Node {
///     fn uuid(&self) -> Uuid {
///         self.uuid
///     }
///     fn set_uuid(&mut self, uuid: Uuid) {
///         self.uuid = uuid;
///     }
///     fn xuuid(&self) -> Option<Uuid> {
///         self.xuuid
///     }
///     fn set_xuuid(&mut self, xuuid: Option<Uuid>) {
///         self.xuuid = xuuid;
///     }
///     fn parentuuids(&self) -> &[Uuid] {
///         &self.parents
///     }
///     fn set_parentuuids(&mut self, parents: Vec<Uuid>) {
///         self.parents = parents;
///     }
///     // A node follows another by descending from it, and never itself.
///     fn with_previous(mut self, previous: &Self) -> Option<Self> {
///         if previous.uuid() == self.uuid() {
///             return None;
///         }
///         self.parents = vec![previous.uuid()];
///         Some(self)
///     }
/// }
///
/// let root = Node { uuid: Uuid::from_v8(1), xuuid: None, parents: Vec::new() };
/// let child = Node { uuid: Uuid::from_v8(2), xuuid: Some(Uuid::from_v8(20)), parents: Vec::new() };
/// assert!(child.parentuuids().is_empty(), "a node naming no parent is a root");
/// let child = child.with_previous(&root).expect("a node follows another");
/// assert_eq!(child.parentuuids(), [root.uuid()]);
/// assert_eq!(child.xuuid(), Some(Uuid::from_v8(20)));
/// // The implementor's rule: a node never follows itself.
/// let twin = Node { uuid: Uuid::from_v8(1), xuuid: None, parents: Vec::new() };
/// assert!(twin.with_previous(&root).is_none());
///
/// // A second statement of the same node merges into the first: the cross
/// // element it states fills the one the first left out, and the parents
/// // are the union in order. Another node does not merge at all.
/// let again = Node { uuid: Uuid::from_v8(2), xuuid: None, parents: vec![Uuid::from_v8(9), root.uuid()] };
/// let merged = child.merge_with(&again).expect("the same node");
/// assert_eq!(merged.xuuid(), Some(Uuid::from_v8(20)));
/// assert_eq!(merged.parentuuids(), [root.uuid(), Uuid::from_v8(9)]);
/// assert!(merged.merge_with(&root).is_none());
/// ```
pub trait Element {
    /// This element's identity.
    fn uuid(&self) -> Uuid;

    /// Records this element's identity.
    fn set_uuid(&mut self, uuid: Uuid);

    /// The identity this element has in another graph - the cross element
    /// it is the same thing as, elsewhere - or nothing where it has none.
    fn xuuid(&self) -> Option<Uuid>;

    /// Records the identity this element has in another graph; `None`
    /// states it has none.
    fn set_xuuid(&mut self, xuuid: Option<Uuid>);

    /// The identities of the elements this one descends from, in the order
    /// the element states them; empty for a root.
    fn parentuuids(&self) -> &[Uuid];

    /// Records the identities of the elements this one descends from, in
    /// the given order; an empty list makes it a root.
    fn set_parentuuids(&mut self, parents: Vec<Uuid>);

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
    /// states none, and the parents become the union - this element's in
    /// its order, then the ones only `other` names, in its. A timed element
    /// delegates to [`TimeElement::merging`], which folds the rest.
    fn merge_with(mut self, other: &Self) -> Option<Self>
    where
        Self: Sized,
    {
        if other.uuid() != self.uuid() {
            return None;
        }
        merge_element(&mut self, other);
        Some(self)
    }
}

/// The facts an element takes from another statement of itself: the cross
/// element it left out, and the parents it did not name.
fn merge_element<E: Element + ?Sized>(this: &mut E, other: &E) {
    if this.xuuid().is_none() {
        this.set_xuuid(other.xuuid());
    }
    let mut parents = this.parentuuids().to_vec();
    for parent in other.parentuuids() {
        if !parents.contains(parent) {
            parents.push(*parent);
        }
    }
    this.set_parentuuids(parents);
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
/// [`Self::time_uuid`] the UUID it answers - RFC 9562 v8 with the instant in
/// front, so identities sort by instant first as UUIDv7's do and by content
/// second - which is what an implementor's [`Element::uuid`] answers where
/// the element's identity is when it happened and what it says. The cross
/// element is derived the same way from what it is: `xhashcode`, its code,
/// coupled with `creation_unix`, when it was created, by
/// [`Self::time_xuuid`].
///
/// Where the element stands is its [`State`], the crate's ranked lifecycle
/// code, and every timed element has one: an element that reached no state
/// says so with the code that means exactly that, `00UNKNOWN`, never with an
/// absence. Where it stands in its chain is `sequence_num`: the count of
/// elements before it, which following increments and merging keeps the
/// highest of. Three more instants and one more identity are optional, because
/// an element states them only where it knows them: when it was created and
/// when it expires, each an instant in the same count, and the element it
/// follows - `previous_uuid` and `previous_unix`, the predecessor's identity
/// and instant.
///
/// Two readings are provided. [`Self::following`] is what following means
/// for a timed element, which an implementor's [`Element::with_previous`]
/// delegates to; [`Self::merging`] is what merging means, which its
/// [`Element::merge_with`] delegates to. Both fold the lifecycle the same
/// way: the earliest creation, the latest expiration, the furthest state.
///
/// ```
/// use yggdryl::graph::{Element, TimeElement};
/// use yggdryl::types::{State, Uuid};
///
/// struct Event {
///     uuid: Uuid,
///     xuuid: Option<Uuid>,
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
/// }
///
/// impl Event {
///     fn at(uuid: u128, unix: i128) -> Self {
///         Self {
///             uuid: Uuid::from_v8(uuid),
///             xuuid: None,
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
///         }
///     }
/// }
///
/// impl Element for Event {
///     fn uuid(&self) -> Uuid {
///         self.uuid
///     }
///     fn set_uuid(&mut self, uuid: Uuid) {
///         self.uuid = uuid;
///     }
///     fn xuuid(&self) -> Option<Uuid> {
///         self.xuuid
///     }
///     fn set_xuuid(&mut self, xuuid: Option<Uuid>) {
///         self.xuuid = xuuid;
///     }
///     fn parentuuids(&self) -> &[Uuid] {
///         &self.parents
///     }
///     fn set_parentuuids(&mut self, parents: Vec<Uuid>) {
///         self.parents = parents;
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
///     fn unix(&self) -> i128 {
///         self.unix
///     }
///     fn set_unix(&mut self, unix: i128) {
///         self.unix = unix;
///     }
///     fn hashcode(&self) -> u64 {
///         self.hashcode
///     }
///     fn set_hashcode(&mut self, hashcode: u64) {
///         self.hashcode = hashcode;
///     }
///     fn xhashcode(&self) -> u64 {
///         self.xhashcode
///     }
///     fn set_xhashcode(&mut self, xhashcode: u64) {
///         self.xhashcode = xhashcode;
///     }
///     fn state(&self) -> &State {
///         &self.state
///     }
///     fn set_state(&mut self, state: State) {
///         self.state = state;
///     }
///     fn sequence_num(&self) -> u64 {
///         self.sequence_num
///     }
///     fn set_sequence_num(&mut self, sequence_num: u64) {
///         self.sequence_num = sequence_num;
///     }
///     fn creation_unix(&self) -> Option<i128> {
///         self.creation_unix
///     }
///     fn set_creation_unix(&mut self, unix: Option<i128>) {
///         self.creation_unix = unix;
///     }
///     fn expiration_unix(&self) -> Option<i128> {
///         self.expiration_unix
///     }
///     fn set_expiration_unix(&mut self, unix: Option<i128>) {
///         self.expiration_unix = unix;
///     }
///     fn previous_unix(&self) -> Option<i128> {
///         self.previous_unix
///     }
///     fn set_previous_unix(&mut self, unix: Option<i128>) {
///         self.previous_unix = unix;
///     }
///     fn previous_uuid(&self) -> Option<Uuid> {
///         self.previous_uuid
///     }
///     fn set_previous_uuid(&mut self, uuid: Option<Uuid>) {
///         self.previous_uuid = uuid;
///     }
/// }
///
/// let mut first = Event::at(1, 10);
/// first.set_creation_unix(Some(5));
/// let second = Event::at(2, 20).with_previous(&first).expect("the later one follows");
/// assert_eq!(second.previous_uuid(), Some(first.uuid()));
/// assert_eq!(second.previous_unix(), Some(10));
/// // Following carries the lifecycle forward: the earliest creation known,
/// // and the next place in the chain.
/// assert_eq!(second.creation_unix(), Some(5));
/// assert_eq!(second.sequence_num(), 1);
/// // An element follows neither itself nor one that happened after it.
/// assert!(Event::at(1, 10).with_previous(&first).is_none());
/// assert!(Event::at(3, 5).with_previous(&second).is_none());
/// // A time element is an element: one walk reads both.
/// let held: &dyn TimeElement = &second;
/// assert_eq!(held.uuid(), Uuid::from_v8(2));
/// assert_eq!(held.unix(), 20);
/// assert_eq!(held.creation_unix(), Some(5));
/// assert!(held.state().is_live());
/// // The identity its instant and code derive: later sorts later.
/// let earlier = first.time_uuid().expect("an instant a TxHash holds");
/// let later = second.time_uuid().expect("an instant a TxHash holds");
/// assert!(earlier < later);
/// assert_eq!(first.txhash().expect("a TxHash").unix(), 10);
/// // And the cross identity its creation and cross code derive, where it
/// // knows when it was created.
/// assert!(first.time_xuuid().expect("an instant a TxHash holds").is_some());
/// assert!(Event::at(4, 40).time_xuuid().expect("nothing to couple").is_none());
/// ```
pub trait TimeElement: Element {
    /// When this element happened: nanoseconds since the Unix epoch, UTC.
    fn unix(&self) -> i128;

    /// Records when this element happened, as nanoseconds since the Unix
    /// epoch, UTC.
    fn set_unix(&mut self, unix: i128);

    /// The code this element's content digests to.
    fn hashcode(&self) -> u64;

    /// Records the code this element's content digests to.
    fn set_hashcode(&mut self, hashcode: u64);

    /// The code the cross element's content digests to: what this element
    /// is in another graph, as a digest.
    fn xhashcode(&self) -> u64;

    /// Records the code the cross element's content digests to.
    fn set_xhashcode(&mut self, xhashcode: u64);

    /// Where this element stands in its lifecycle: the crate's ranked
    /// [`State`] code, never absent - an element that reached no state says
    /// `00UNKNOWN`.
    fn state(&self) -> &State;

    /// Records where this element stands in its lifecycle.
    fn set_state(&mut self, state: State);

    /// Where this element stands in its chain: how many came before it.
    fn sequence_num(&self) -> u64;

    /// Records where this element stands in its chain.
    fn set_sequence_num(&mut self, sequence_num: u64);

    /// When this element was created, in the same count as [`Self::unix`],
    /// where it knows.
    fn creation_unix(&self) -> Option<i128>;

    /// Records when this element was created; `None` states it does not
    /// know.
    fn set_creation_unix(&mut self, unix: Option<i128>);

    /// When this element expires, in the same count as [`Self::unix`], where
    /// it has an expiry.
    fn expiration_unix(&self) -> Option<i128>;

    /// Records when this element expires; `None` states it does not expire,
    /// or does not know.
    fn set_expiration_unix(&mut self, unix: Option<i128>);

    /// When the element this one follows happened, where it follows one.
    fn previous_unix(&self) -> Option<i128>;

    /// Records when the element this one follows happened; `None` states it
    /// follows none.
    fn set_previous_unix(&mut self, unix: Option<i128>);

    /// The identity of the element this one follows, where it follows one.
    fn previous_uuid(&self) -> Option<Uuid>;

    /// Records the identity of the element this one follows; `None` states
    /// it follows none.
    fn set_previous_uuid(&mut self, uuid: Option<Uuid>);

    /// This element stated as the one after `previous`, by the timed
    /// reading, or nothing where it cannot follow - its own predecessor, or
    /// one that happened after it.
    ///
    /// The predecessor's identity and instant are recorded on it, its place
    /// in the chain is the one after the predecessor's, and the lifecycle
    /// carries forward: the creation is the earliest the two know, the
    /// expiration the latest, the state the furthest along, and each where
    /// only one states it is that one's. What the element itself says - its
    /// instant, its codes, its parents, its cross element - is its own and
    /// moves nowhere.
    ///
    /// Provided, so an implementor's [`Element::with_previous`] has a
    /// default to delegate to; an element that means something else by
    /// following states its own there instead.
    fn following(mut self, previous: &Self) -> Option<Self>
    where
        Self: Sized,
    {
        if previous.uuid() == self.uuid() || previous.unix() > self.unix() {
            return None;
        }
        self.set_previous_uuid(Some(previous.uuid()));
        self.set_previous_unix(Some(previous.unix()));
        self.set_sequence_num(previous.sequence_num().saturating_add(1));
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
    /// latest expiration, furthest state; and the predecessor is this
    /// element's where it names one, else the other's.
    ///
    /// Provided, so an implementor's [`Element::merge_with`] has a default
    /// to delegate to.
    fn merging(mut self, other: &Self) -> Option<Self>
    where
        Self: Sized,
    {
        if other.uuid() != self.uuid() {
            return None;
        }
        merge_element(&mut self, other);
        if other.unix() > self.unix() {
            self.set_unix(other.unix());
            self.set_hashcode(other.hashcode());
            self.set_xhashcode(other.xhashcode());
        }
        if other.sequence_num() > self.sequence_num() {
            self.set_sequence_num(other.sequence_num());
        }
        self.fold_lifecycle(other);
        if self.previous_uuid().is_none() {
            self.set_previous_uuid(other.previous_uuid());
            self.set_previous_unix(other.previous_unix());
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
        self.set_creation_unix(earliest(self.creation_unix(), other.creation_unix()));
        self.set_expiration_unix(latest(self.expiration_unix(), other.expiration_unix()));
        if other.state() > self.state() {
            self.set_state(other.state().clone());
        }
    }

    /// The instant and the code coupled: a [`TxHash`] of [`Self::unix`] at
    /// nanosecond resolution and [`Self::hashcode`] as the XXH3-64 digest it
    /// is, which is the crate's own time-ordered identity.
    ///
    /// Provided, so every timed element derives it the same way.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ArithmeticOverflow`] where the instant does not fit
    /// the signed 64-bit nanoseconds a `TxHash` holds.
    fn txhash(&self) -> Result<TxHash> {
        coupled(self.unix(), self.hashcode())
    }

    /// The identity the instant and the code derive: the UUID
    /// [`TxHash::into_uuid`] answers for [`Self::txhash`], RFC 9562 v8 with
    /// the instant in front so identities sort by instant first, as
    /// UUIDv7's do, and by content second.
    ///
    /// Provided: an implementor whose identity is when it happened and what
    /// it says answers this from [`Element::uuid`], and one whose identity
    /// is assigned keeps its own.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::txhash`] returns.
    fn time_uuid(&self) -> Result<Uuid> {
        self.txhash()?.into_uuid()
    }

    /// The cross identity the creation and the cross code derive: the same
    /// coupling as [`Self::time_uuid`], of [`Self::creation_unix`] with
    /// [`Self::xhashcode`], or nothing where the element does not know when
    /// it was created.
    ///
    /// Provided: an implementor whose cross element is its creation answers
    /// this from [`Element::xuuid`].
    ///
    /// # Errors
    ///
    /// Returns what [`Self::txhash`] returns, for the creation instant.
    fn time_xuuid(&self) -> Result<Option<Uuid>> {
        self.creation_unix()
            .map(|unix| coupled(unix, self.xhashcode())?.into_uuid())
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
/// [`Side`] code, FIX's `Side(54)`. Five more name the instrument, each
/// optional because a market names an instrument the way it does: the
/// [`Isin`], the [`Cusip`], the [`Sedol`], the [`Bloomberg`] identifier and
/// the [`Cfi`] classification, the crate's own validated codes. The traits
/// state signatures and nothing else: what a price of nothing or a quantity
/// of zero means is the market's to say, and following and merging are the
/// timed readings unchanged.
///
/// ```
/// use yggdryl::graph::{Element, MarketElement, TimeElement};
/// use yggdryl::types::{Bloomberg, Cfi, Currency, Cusip, Isin, Sedol, Side, State, Uuid};
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
/// #   xuuid: Option<Uuid>,
/// #   parents: Vec<Uuid>,
/// #   xhashcode: u64,
/// #   state: State,
/// #   sequence_num: u64,
/// #   creation_unix: Option<i128>,
/// #   expiration_unix: Option<i128>,
/// #   previous_unix: Option<i128>,
/// #   previous_uuid: Option<Uuid>,
/// }
/// # impl Element for Trade {
/// #     fn uuid(&self) -> Uuid { self.uuid }
/// #     fn set_uuid(&mut self, uuid: Uuid) { self.uuid = uuid; }
/// #     fn xuuid(&self) -> Option<Uuid> { self.xuuid }
/// #     fn set_xuuid(&mut self, xuuid: Option<Uuid>) { self.xuuid = xuuid; }
/// #     fn parentuuids(&self) -> &[Uuid] { &self.parents }
/// #     fn set_parentuuids(&mut self, parents: Vec<Uuid>) { self.parents = parents; }
/// #     fn with_previous(self, previous: &Self) -> Option<Self> { self.following(previous) }
/// #     fn merge_with(self, other: &Self) -> Option<Self> { self.merging(other) }
/// # }
/// # impl TimeElement for Trade {
/// #     fn unix(&self) -> i128 { self.unix }
/// #     fn set_unix(&mut self, unix: i128) { self.unix = unix; }
/// #     fn hashcode(&self) -> u64 { self.hashcode }
/// #     fn set_hashcode(&mut self, hashcode: u64) { self.hashcode = hashcode; }
/// #     fn xhashcode(&self) -> u64 { self.xhashcode }
/// #     fn set_xhashcode(&mut self, xhashcode: u64) { self.xhashcode = xhashcode; }
/// #     fn state(&self) -> &State { &self.state }
/// #     fn set_state(&mut self, state: State) { self.state = state; }
/// #     fn sequence_num(&self) -> u64 { self.sequence_num }
/// #     fn set_sequence_num(&mut self, sequence_num: u64) { self.sequence_num = sequence_num; }
/// #     fn creation_unix(&self) -> Option<i128> { self.creation_unix }
/// #     fn set_creation_unix(&mut self, unix: Option<i128>) { self.creation_unix = unix; }
/// #     fn expiration_unix(&self) -> Option<i128> { self.expiration_unix }
/// #     fn set_expiration_unix(&mut self, unix: Option<i128>) { self.expiration_unix = unix; }
/// #     fn previous_unix(&self) -> Option<i128> { self.previous_unix }
/// #     fn set_previous_unix(&mut self, unix: Option<i128>) { self.previous_unix = unix; }
/// #     fn previous_uuid(&self) -> Option<Uuid> { self.previous_uuid }
/// #     fn set_previous_uuid(&mut self, uuid: Option<Uuid>) { self.previous_uuid = uuid; }
/// # }
///
/// impl MarketElement for Trade {
///     fn px(&self) -> f64 {
///         self.px
///     }
///     fn set_px(&mut self, px: f64) {
///         self.px = px;
///     }
///     fn currency(&self) -> &Currency {
///         &self.currency
///     }
///     fn set_currency(&mut self, currency: Currency) {
///         self.currency = currency;
///     }
///     fn qty(&self) -> f64 {
///         self.qty
///     }
///     fn set_qty(&mut self, qty: f64) {
///         self.qty = qty;
///     }
///     fn unit(&self) -> &str {
///         &self.unit
///     }
///     fn set_unit(&mut self, unit: String) {
///         self.unit = unit;
///     }
///     fn side(&self) -> &Side {
///         &self.side
///     }
///     fn set_side(&mut self, side: Side) {
///         self.side = side;
///     }
///     fn isincode(&self) -> Option<&Isin> {
///         self.isincode.as_ref()
///     }
///     fn set_isincode(&mut self, isincode: Option<Isin>) {
///         self.isincode = isincode;
///     }
/// #   fn cusipcode(&self) -> Option<&Cusip> { self.cusipcode.as_ref() }
/// #   fn set_cusipcode(&mut self, cusipcode: Option<Cusip>) { self.cusipcode = cusipcode; }
/// #   fn sedolcode(&self) -> Option<&Sedol> { self.sedolcode.as_ref() }
/// #   fn set_sedolcode(&mut self, sedolcode: Option<Sedol>) { self.sedolcode = sedolcode; }
/// #   fn bloombergcode(&self) -> Option<&Bloomberg> { self.bloombergcode.as_ref() }
/// #   fn set_bloombergcode(&mut self, bloombergcode: Option<Bloomberg>) { self.bloombergcode = bloombergcode; }
/// #   fn cficode(&self) -> Option<&Cfi> { self.cficode.as_ref() }
/// #   fn set_cficode(&mut self, cficode: Option<Cfi>) { self.cficode = cficode; }
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
///     side: Side::new("1")?,
///     isincode: Some(Isin::new("US0378331005")?),
/// #   cusipcode: None, sedolcode: None, bloombergcode: None, cficode: None,
/// #   xuuid: None, parents: Vec::new(), xhashcode: 0,
/// #   state: State::from_spelling("New").expect("a shipped state"), sequence_num: 0,
/// #   creation_unix: None, expiration_unix: None, previous_unix: None, previous_uuid: None,
/// };
/// assert_eq!(trade.px(), 82.5);
/// assert_eq!(trade.currency().as_str(), "USD");
/// assert_eq!(trade.unit(), "bbl");
/// trade.set_qty(2_000.0);
/// assert_eq!(trade.qty(), 2_000.0);
/// // A market element is a timed element is an element: one walk reads all.
/// let held: &dyn MarketElement = &trade;
/// assert_eq!(held.side().as_str(), "1");
/// assert_eq!(held.isincode().map(Isin::as_str), Some("US0378331005"));
/// assert!(held.cusipcode().is_none(), "an instrument is named the way the market names it");
/// assert_eq!(held.unix(), 10);
/// assert_eq!(held.uuid(), Uuid::from_v8(1));
/// # Ok(())
/// # }
/// ```
pub trait MarketElement: TimeElement {
    /// The price.
    fn px(&self) -> f64;

    /// Records the price.
    fn set_px(&mut self, px: f64);

    /// The currency the price is quoted in.
    fn currency(&self) -> &Currency;

    /// Records the currency the price is quoted in.
    fn set_currency(&mut self, currency: Currency);

    /// The quantity.
    fn qty(&self) -> f64;

    /// Records the quantity.
    fn set_qty(&mut self, qty: f64);

    /// The unit the quantity is counted in, as the market spells it.
    fn unit(&self) -> &str;

    /// Records the unit the quantity is counted in.
    fn set_unit(&mut self, unit: String);

    /// Which side of the market the element stood on: FIX's `Side(54)`.
    fn side(&self) -> &Side;

    /// Records which side of the market the element stood on.
    fn set_side(&mut self, side: Side);

    /// The instrument's ISIN, where the market names it so.
    fn isincode(&self) -> Option<&Isin>;

    /// Records the instrument's ISIN; `None` states the market names none.
    fn set_isincode(&mut self, isincode: Option<Isin>);

    /// The instrument's CUSIP, where the market names it so.
    fn cusipcode(&self) -> Option<&Cusip>;

    /// Records the instrument's CUSIP; `None` states the market names none.
    fn set_cusipcode(&mut self, cusipcode: Option<Cusip>);

    /// The instrument's SEDOL, where the market names it so.
    fn sedolcode(&self) -> Option<&Sedol>;

    /// Records the instrument's SEDOL; `None` states the market names none.
    fn set_sedolcode(&mut self, sedolcode: Option<Sedol>);

    /// The instrument's Bloomberg identifier, where the market names it so.
    fn bloombergcode(&self) -> Option<&Bloomberg>;

    /// Records the instrument's Bloomberg identifier; `None` states the
    /// market names none.
    fn set_bloombergcode(&mut self, bloombergcode: Option<Bloomberg>);

    /// The instrument's CFI classification, where the market states it.
    fn cficode(&self) -> Option<&Cfi>;

    /// Records the instrument's CFI classification; `None` states the
    /// market states none.
    fn set_cficode(&mut self, cficode: Option<Cfi>);
}
