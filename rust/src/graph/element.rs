//! An element of a graph, and one that happened at an instant.
//!
//! Two traits: what an element answers about itself, what it takes, and
//! the two readings every element has - following another, and merging with
//! another statement of itself. What an element that stands in a market
//! answers is [`Market`](super::Market) and [`MarketOperation`](super::MarketOperation). The identity is the crate's own [`Uuid`], so
//! an element is addressed the way every identified value in the crate is,
//! and a predecessor or a cross element is named by the same
//! identity rather than by a reference, so an element can name one it does
//! not hold.

use std::hash::Hasher;

use crate::txhash::TxHash;
use crate::xxhash::Xxh3;
use crate::{CodeValue, State, Uuid};
use crate::{Digest, DigestAlgorithm, Result, TimeUnit};

/// One element of a graph: a node that knows its own identity, the identity
/// it has elsewhere, the codes it digests to and the identities of the
/// elements it was read from.
///
/// The sources are its provenance: the elements it was read from - a message
/// parsed from a text line has that line's identity as its one source - so
/// an element follows another without being read from it and is read from a
/// line without following it. Where an element stands in its chain is the
/// one element before it, which an event records as its `prevuuid`
/// ([`Event::get_prevuuid`]); nothing records the chain further back.
/// Sources travel along no chain: following and restating leave them as they
/// are, and only a merge of two statements of one element unions them,
/// because the merged element was built from both. The sources never feed
/// the code the element digests to: a source is where an element was read,
/// not what it states. The cross
/// element, `crossuuid`, is what this element is in another
/// graph - the same order in a venue's book and in a ledger - and
/// `crosscode` is the text that names it there: the identifier every
/// incarnation of one thing shares, an order's `OrderID`, a quote's
/// `QuoteID`, empty where the element states none. Two codes are derived:
/// `currhashcode` is the XXH3-64 digest of the element's content, what
/// [`Self::finalize`] recomputes, and `crosshashcode` the XXH3-64 of the
/// cross code, zero where none is stated, what [`Self::sync_cross`] keeps in
/// step with it. The cross element is never absent: it is the identity the
/// cross hash code derives where a cross code is stated, and the element's
/// own identity where none is, [`Self::cross_uuid`], so every element stands
/// in exactly one chain. The names an operation goes by elsewhere - an
/// order's `ClOrdID` and `OrderID`, a trade's `ExecID` - are the
/// operation's own facts, [`MarketOperation::get_altids`](super::MarketOperation::get_altids),
/// not the node's.
/// Every fact is read and written through the trait, so a store or a walk
/// that only knows an element as `dyn Element` can still place it; every
/// accessor is `get_` and every mutator `set_`, so the traits claim no bare
/// name and an implementor keeps its own `uuid()` or `state()` for whatever
/// it means by them.
///
/// Three readings come with the facts. [`Self::is_after`] is the order the
/// implementor states between two elements - an event's instant, a node's
/// predecessor - and [`Self::is_before`] its mirror, provided.
/// [`Self::with_previous`] states this element as the one after another,
/// and is the other signature the trait leaves to the implementor: what
/// following means is the element's own - a chain entry records its
/// predecessor, a snapshot its base - and [`Event::following`] is the
/// reading an event delegates to. [`Self::merge_with`] folds another
/// statement of the same element into this one, and is provided: the cross
/// element and the cross code are taken where this one states none, and the
/// sources become a sorted unique union. An event
/// delegates to
/// [`Event::merging`], which folds the instants and the state too.
///
/// ```
/// use yggdryl::graph::Element;
/// use yggdryl::Uuid;
///
/// struct Node {
///     curruuid: Uuid,
///     crossuuid: Uuid,
///     crosscode: String,
///     hashcode: u64,
///     crosshashcode: u64,
///     previous: Option<Uuid>,
///     sources: Vec<Uuid>,
/// }
///
/// impl Node {
///     fn new(uuid: u128) -> Self {
///         Self {
///             curruuid: Uuid::from_v8(uuid),
///             crossuuid: Uuid::from_v8(uuid),
///             crosscode: String::new(),
///             hashcode: 0,
///             crosshashcode: 0,
///             previous: None,
///             sources: Vec::new(),
///         }
///     }
/// }
///
/// impl Element for Node {
///     fn get_curruuid(&self) -> Uuid {
///         self.curruuid
///     }
///     fn set_curruuid(&mut self, curruuid: Uuid) {
///         self.curruuid = curruuid;
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
///     fn get_currhashcode(&self) -> u64 {
///         self.hashcode
///     }
///     fn set_currhashcode(&mut self, hashcode: u64) {
///         self.hashcode = hashcode;
///     }
///     fn get_crosshashcode(&self) -> u64 {
///         self.crosshashcode
///     }
///     fn set_crosshashcode(&mut self, crosshashcode: u64) {
///         self.crosshashcode = crosshashcode;
///     }
///     fn get_srcuuids(&self) -> &[Uuid] {
///         &self.sources
///     }
///     fn set_srcuuids(&mut self, mut sources: Vec<Uuid>) {
///         sources.sort_unstable();
///         sources.dedup();
///         self.sources = sources;
///     }
///     // A node's order is its predecessor: it is after the node it follows.
///     fn is_after(&self, other: &Self) -> bool {
///         self.previous == Some(other.get_curruuid())
///     }
///     // A node's identity is assigned, so finalizing keeps it and only
///     // recomputes the code its content digests to.
///     fn finalize(&mut self) {
///         self.sync_cross();
///         self.hashcode = self.digest().as_u64();
///     }
///     // A node follows another by recording it - and never itself.
///     fn with_previous(mut self, previous: &Self) -> Option<Self> {
///         if previous.get_curruuid() == self.get_curruuid() {
///             return None;
///         }
///         self.previous = Some(previous.get_curruuid());
///         self.finalize();
///         Some(self)
///     }
/// }
///
/// let root = Node::new(1);
/// assert_eq!(root.get_crossuuid(), root.get_curruuid(), "no cross code: its own chain");
/// let mut child = Node::new(2);
/// child.set_crosscode("O-100".to_owned());
/// assert!(!child.is_after(&root) && !child.is_before(&root), "unrelated, so neither");
/// let child = child.with_previous(&root).expect("a node follows another");
/// assert!(child.is_after(&root) && root.is_before(&child));
/// // The cross code stated, its digest and the cross identity follow it.
/// assert_ne!(child.get_crosshashcode(), 0);
/// assert_eq!(child.get_crossuuid(), child.cross_uuid());
/// assert_ne!(child.get_currhashcode(), 0);
/// // The implementor's rule: a node never follows itself.
/// assert!(Node::new(1).with_previous(&root).is_none());
///
/// // A second statement of the same node merges into the first; another
/// // node does not merge at all.
/// let mut again = Node::new(2);
/// again.set_srcuuids(vec![Uuid::from_v8(70)]);
/// let merged = child.merge_with(&again).expect("the same node");
/// assert_eq!(merged.get_crosscode(), "O-100", "this element's word wins");
/// // A source is provenance: unioned by the merge, and never fed to the code.
/// assert_eq!(merged.get_srcuuids(), [Uuid::from_v8(70)]);
/// assert!(merged.merge_with(&root).is_none());
/// ```
pub trait Element {
    /// This element's identity.
    fn get_curruuid(&self) -> Uuid;

    /// Records this element's identity.
    fn set_curruuid(&mut self, curruuid: Uuid);

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
    fn get_currhashcode(&self) -> u64;

    /// Records the code this element's content digests to.
    fn set_currhashcode(&mut self, hashcode: u64);

    /// The code the cross code digests to: what this element is in another
    /// graph, as a digest; zero where it states no cross code.
    fn get_crosshashcode(&self) -> u64;

    /// Records the code the cross code digests to.
    fn set_crosshashcode(&mut self, crosshashcode: u64);

    /// The identities of the elements this one was read from: its
    /// provenance, never its chain. Empty for an element read from a
    /// handle rather than from another element.
    ///
    /// The identities are sorted and unique. Reference selection is carried
    /// by the event clocks rather than encoded in list position.
    fn get_srcuuids(&self) -> &[Uuid];

    /// Records the identities of the elements this one was read from as a
    /// sorted unique list; an empty one states none.
    fn set_srcuuids(&mut self, sources: Vec<Uuid>);

    /// Whether this element comes after `other` in the order the implementor
    /// states: an event's instant, a node's predecessor.
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
    /// hands the code to [`Self::set_currhashcode`] - or, for an event, to
    /// [`Event::finalized`], which sets the identity the instant, sequence,
    /// cross hash and code derive; an element whose identity is assigned
    /// keeps it. Every
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
    /// the cross code and the names it goes by, each under its own name -
    /// for an implementor to feed what it says and finish. What
    /// is derived is never fed: not the current identity, not the cross hash
    /// code and not the cross element, which the code and the cross code
    /// derive; and neither are the sources, because where an element was
    /// read from is its provenance and not what it states.
    ///
    /// Provided, and what an implementor's [`Self::finalize`] starts from:
    /// an event continues with [`Event::digest_event`], a market element
    /// with [`Market::digest_market`](super::Market::digest_market), a market
    /// event with [`MarketEvent::digest_market_event`](super::MarketEvent::digest_market_event),
    /// and each feeds its own content
    /// behind them and reads `as_u64` for the code. The facts are fed
    /// through their typed accessors, so two elements stating the same
    /// things digest alike whatever holds them.
    fn digest(&self) -> Xxh3 {
        let mut state = Xxh3::new();
        if !self.get_crosscode().is_empty() {
            feed(&mut state, "crosscode", self.get_crosscode().as_bytes());
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
    /// none, the sources become a sorted unique union, the cross codes are
    /// brought in step, and
    /// the element is finalized where any of that moved. An event delegates to
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
/// code it left out and the sources it did not name; whether any of them
/// moved.
pub(super) fn merge_element<E: Element + ?Sized>(this: &mut E, other: &E) -> bool {
    let mut changed = false;
    if this.get_crosscode().is_empty() && !other.get_crosscode().is_empty() {
        this.set_crosscode(other.get_crosscode().to_owned());
        changed = true;
    }
    changed |= this.sync_cross();
    changed |= union_sources(this, other);
    changed
}

/// The element facts of two event statements, with the recording-selected
/// reference leading conflicts and list order. An unstated fact on the
/// reference is still filled by the other statement.
pub(super) fn merge_event_element<E: Element + ?Sized>(
    this: &mut E,
    other: &E,
    other_is_reference: bool,
) -> bool {
    if !other_is_reference {
        return merge_element(this, other);
    }

    let mut changed = false;
    if !other.get_crosscode().is_empty() && this.get_crosscode() != other.get_crosscode() {
        this.set_crosscode(other.get_crosscode().to_owned());
        changed = true;
    }
    changed |= this.sync_cross();

    let sources = union_uuids(other.get_srcuuids(), this.get_srcuuids())
        .unwrap_or_else(|| other.get_srcuuids().to_vec());
    if sources != this.get_srcuuids() {
        this.set_srcuuids(sources);
        changed = true;
    }
    changed
}

/// The sorted identities `other` names and `held` does not, merged once;
/// nothing where `other` adds none, so a union that changes nothing costs no
/// list. Disjoint suffixes append in one allocation.
///
/// The one union rule for the sources.
fn union_uuids(held: &[Uuid], other: &[Uuid]) -> Option<Vec<Uuid>> {
    let mut held_at = 0;
    let mut other_at = 0;
    while held_at < held.len() && other_at < other.len() {
        match held[held_at].cmp(&other[other_at]) {
            std::cmp::Ordering::Less => held_at += 1,
            std::cmp::Ordering::Equal => {
                held_at += 1;
                other_at += 1;
            }
            std::cmp::Ordering::Greater => break,
        }
    }
    if other_at == other.len() {
        return None;
    }
    if held.is_empty() {
        return Some(other.to_vec());
    }
    if other.first().is_some_and(|first| held.last() < Some(first)) {
        let mut union = Vec::with_capacity(held.len() + other.len());
        union.extend_from_slice(held);
        union.extend_from_slice(other);
        return Some(union);
    }
    let mut union = Vec::with_capacity(held.len() + other.len());
    let (mut left, mut right) = (0, 0);
    while left < held.len() && right < other.len() {
        match held[left].cmp(&other[right]) {
            std::cmp::Ordering::Less => {
                union.push(held[left]);
                left += 1;
            }
            std::cmp::Ordering::Equal => {
                union.push(held[left]);
                left += 1;
                right += 1;
            }
            std::cmp::Ordering::Greater => {
                union.push(other[right]);
                right += 1;
            }
        }
    }
    union.extend_from_slice(&held[left..]);
    union.extend_from_slice(&other[right..]);
    Some(union)
}

/// Normalizes an owned UUID list without reallocating its buffer. Already
/// strictly sorted input is untouched; sorted duplicates only compact.
pub(crate) fn canonicalize_uuids(values: &mut Vec<Uuid>) {
    if values.len() < 2 {
        return;
    }
    let sorted = values.windows(2).all(|pair| pair[0] <= pair[1]);
    if !sorted {
        values.sort_unstable();
    }
    values.dedup();
}

/// The sources `other` names and `this` does not, taken by [`union_uuids`];
/// whether any was.
fn union_sources<E: Element + ?Sized>(this: &mut E, other: &E) -> bool {
    match union_uuids(this.get_srcuuids(), other.get_srcuuids()) {
        Some(sources) => {
            this.set_srcuuids(sources);
            true
        }
        None => false,
    }
}

/// The facts an event takes from restating `live`, another statement of
/// the same event: the predecessor's chain facts as [`follow_element`]
/// takes them, and the place `live` holds in its chain - its predecessor, its position, its snapshot - because the two
/// statements are one event and the chain grows by nothing. Its sources
/// stay its own: where a statement was read from travels along no chain,
/// so a twin keeps the line it came from and never the live one's.
pub(super) fn restate_event<E: Event + ?Sized>(this: &mut E, live: &E) {
    follow_element(this, live);
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
    changed
}

/// The XXH3-64 digest of one cross code's bytes: the one derivation of a
/// cross hash code, which every element that resolves its own reads.
pub(crate) fn crosshash(crosscode: &str) -> u64 {
    let mut state = Xxh3::new();
    state.write(crosscode.as_bytes());
    state.as_u64()
}

/// Feeds one named fact to a digest: the name, the bytes, each closed by a
/// byte no name or value holds, so two facts never read as one.
pub(crate) fn feed(state: &mut Xxh3, name: &str, bytes: &[u8]) {
    state.write(name.as_bytes());
    state.write(&[0]);
    state.write(bytes);
    state.write(&[0]);
}

/// Records `next` where it differs from `current`, answering whether it did.
pub(super) fn moved<T: PartialEq>(current: T, next: T, set: impl FnOnce(T)) -> bool {
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

/// Whether `right` is the reference statement of two observations, by their
/// `recdunix`. The most recently recorded statement leads; a stated recording
/// clock leads an unstated one, and equal or absent recording clocks fall
/// back to the later event instant. Exact ties keep `left`.
///
/// A folded statement keeps the earliest recording its statements know, so
/// it ranks by that clock against a third, and the reference of three
/// statements folded pair by pair depends on the order they are folded in.
/// A caller holding every observation at once chooses the reference over
/// all of them first and folds the rest into it.
pub(crate) fn right_is_reference(
    left_recdunix: Option<i64>,
    left_currunix: i64,
    right_recdunix: Option<i64>,
    right_currunix: i64,
) -> bool {
    match (left_recdunix, right_recdunix) {
        (Some(left), Some(right)) if left != right => right > left,
        (None, Some(_)) => true,
        (Some(_), None) => false,
        _ => right_currunix > left_currunix,
    }
}

/// The execution instant an event states or, while it is still an unstamped
/// lifecycle input whose state itself reports an execution, its own instant.
/// A predecessor marks a lifecycle output: its state may have been inherited,
/// so replaying that output must not reinterpret the folded state as this
/// event's own execution report.
fn execution_unix<E: Event + ?Sized>(event: &E) -> Option<i64> {
    event.get_execunix().or_else(|| {
        (event.get_prevuuid().is_none() && event.is_execution()).then_some(event.get_currunix())
    })
}

/// Fills an execution event's unstated execution instant from its own instant;
/// whether it moved.
pub(super) fn fill_execution<E: Event + ?Sized>(event: &mut E) -> bool {
    let Some(unix) = execution_unix(event) else {
        return false;
    };
    if event.get_execunix() == Some(unix) {
        return false;
    }
    event.set_execunix(Some(unix));
    true
}

/// Folds the per-event instants of two statements of the same event: the
/// earliest execution and recording either statement knows; whether any
/// moved. An unstamped execution observation first dates itself from its own
/// event instant. These never fold between successive events in one lifecycle.
pub(super) fn fold_event_instants<E: Event + ?Sized>(this: &mut E, other: &E) -> bool {
    let execunix = earliest(execution_unix(this), execution_unix(other));
    let mut changed = moved(this.get_execunix(), execunix, |unix| {
        this.set_execunix(unix)
    });
    let recdunix = earliest(this.get_recdunix(), other.get_recdunix());
    changed |= moved(this.get_recdunix(), recdunix, |unix| {
        this.set_recdunix(unix)
    });
    changed
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
/// what any element takes from following - the cross code, the names it went by - with the
/// lifecycle carried forward. An unstamped execution input dates itself, then
/// the later of that precise clock and the predecessor's remains the latest
/// execution the lifecycle has reached; whether any fact moved.
pub(super) fn follow_timed<E: Event>(this: &mut E, previous: &E) -> bool {
    let execunix = latest(execution_unix(this), previous.get_execunix());
    let mut changed = moved(this.get_execunix(), execunix, |unix| {
        this.set_execunix(unix)
    });
    changed |= moved(this.get_prevuuid(), Some(previous.get_curruuid()), |uuid| {
        this.set_prevuuid(uuid)
    });
    changed |= moved(this.get_prevunix(), Some(previous.get_currunix()), |unix| {
        this.set_prevunix(unix)
    });
    changed |= moved(
        this.get_seqnum(),
        previous.get_seqnum().saturating_add(1),
        |place| this.set_seqnum(place),
    );
    changed |= follow_element(this, previous);
    let stated_expiry = this.get_exprtime();
    changed |= this.fold_lifecycle(previous);
    // A replacement may shorten its lifetime. Folding simultaneous statements
    // still keeps the latest expiry, but a newer explicit deadline is decisive.
    if let Some(expiry) = stated_expiry {
        changed |= moved(this.get_exprtime(), Some(expiry), |unix| {
            this.set_exprtime(unix)
        });
    }
    changed
}

/// The timed facts an event takes from another statement of itself: the
/// earliest execution and recording instants, the reference statement's
/// instant and code, the further place in the chain, the lifecycle folded,
/// and the reference's predecessor and snapshot where stated, otherwise the
/// other statement's; whether any moved.
pub(super) fn merge_timed<E: Event>(this: &mut E, other: &E, other_is_reference: bool) -> bool {
    let mut changed = fold_event_instants(this, other);
    if other_is_reference {
        changed |= moved(this.get_currunix(), other.get_currunix(), |unix| {
            this.set_currunix(unix)
        });
        changed |= moved(
            this.get_currhashcode(),
            other.get_currhashcode(),
            |hashcode| this.set_currhashcode(hashcode),
        );
    }
    if other.get_seqnum() > this.get_seqnum() {
        this.set_seqnum(other.get_seqnum());
        changed = true;
    }
    changed |= this.fold_lifecycle(other);
    let (prevuuid, prevunix) = if other_is_reference && other.get_prevuuid().is_some() {
        (other.get_prevuuid(), other.get_prevunix())
    } else if this.get_prevuuid().is_some() {
        (this.get_prevuuid(), this.get_prevunix())
    } else {
        (other.get_prevuuid(), other.get_prevunix())
    };
    if this.get_prevuuid() != prevuuid || this.get_prevunix() != prevunix {
        this.set_prevuuid(prevuuid);
        this.set_prevunix(prevunix);
        changed = true;
    }
    let snapunix = stated(
        this.get_snapunix(),
        other.get_snapunix(),
        other_is_reference,
    );
    changed |= moved(this.get_snapunix(), snapunix, |unix| {
        this.set_snapunix(unix)
    });
    changed
}

/// Facts staged on the stack and written to a digest a chunk at a time.
///
/// A fact is four writes, and an element states dozens - its names, its
/// market, a side's every live entry: staged, a digest costs a write per
/// chunk instead, and the state reads the same bytes in the same order, so
/// the code is the one [`feed`] answers.
/// What is still staged is written when the stage is dropped.
pub(super) struct Staged<'state> {
    state: &'state mut Xxh3,
    held: [u8; 512],
    len: usize,
}

impl<'state> Staged<'state> {
    pub(super) fn new(state: &'state mut Xxh3) -> Self {
        Self {
            state,
            held: [0; 512],
            len: 0,
        }
    }

    /// [`feed`], staged.
    pub(super) fn feed(&mut self, name: &str, bytes: &[u8]) {
        self.write(name.as_bytes());
        self.write(&[0]);
        self.write(bytes);
        self.write(&[0]);
    }

    pub(super) fn write(&mut self, bytes: &[u8]) {
        if self.len + bytes.len() > self.held.len() {
            self.flush();
            if bytes.len() > self.held.len() {
                self.state.write(bytes);
                return;
            }
        }
        self.held[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
    }

    fn flush(&mut self) {
        if self.len > 0 {
            self.state.write(&self.held[..self.len]);
            self.len = 0;
        }
    }
}

impl Drop for Staged<'_> {
    fn drop(&mut self) {
        self.flush();
    }
}

/// Feeds what [`Event::digest_event`] feeds, less the cross code: the selected
/// names the event goes by, its state, its place in the chain and
/// its predecessor's identity. A holder can leave out a name that records
/// capture provenance rather than event content without duplicating the
/// framing this digest owns.
pub(crate) fn feed_event_facts<E: Event + ?Sized>(state: &mut Xxh3, this: &E) {
    feed_timed(state, this);
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
/// The instant is `currunix`: a count of nanoseconds since the Unix epoch, UTC,
/// held as an `i64`, the count every clock this crate reads states. Coupled
/// with the code the element's content digests to, its place in the chain and
/// its cross code, it is the event's identity: [`Self::txhash`] is the crate's
/// own [`TxHash`] of the instant and [`Element::get_currhashcode`], and
/// [`Self::time_uuid`] is RFC 9562 UUIDv7 ordered by millisecond and sequence,
/// with an XXH3 payload seeded by the cross hash code. That UUID is what an
/// implementor's [`Element::get_curruuid`] answers where the event's identity
/// is when it happened and what it says.
///
/// Where the event stands is its [`State`], the crate's ranked lifecycle
/// code, and every event has one: an event that reached no state says so
/// with the code that means exactly that, `00UNKNOWN`, never with an
/// absence. Where it stands in its chain is `seqnum`: the count of events
/// before it, which following increments and merging keeps the highest of.
/// Six more instants and one more identity are optional, because an event
/// states them only where it knows them: when it was created, the latest
/// execution its lifecycle has reached, when it was recorded and when it
/// expires, each an instant in the same count; the
/// event it follows - `prevuuid` and `prevunix`, the predecessor's identity
/// and instant; and `snapunix`, the grid instant this event was read as the
/// snapshot of, where a walk over a grid took one of it.
///
/// An event whose current identity is [`Self::time_uuid`] keeps that identity
/// in step with its content, because the identity is the instant beside the
/// code and nothing else. A cross code therefore moves the identity exactly
/// where it moves the code - [`Element::digest`] feeds it, so an event
/// digesting through that reading moves; one that states its own code decides
/// for itself, as [`crate::FixMsg`] does in leaving the chain out. The crate's
/// concrete event holders reset eagerly or invalidate their lazy UUID; an
/// event with an assigned identity keeps the assignment.
///
/// Two readings are provided. [`Self::following`] is what following means
/// for an event, which an implementor's [`Element::with_previous`]
/// delegates to; [`Self::merging`] is what merging means, which its
/// [`Element::merge_with`] delegates to. Both fold the lifecycle the same
/// way: the earliest creation, the latest expiration and the furthest state.
/// Following then keeps a newer explicit
/// expiration, including one that shortens the lifetime. Recording belongs
/// to one observation and never follows; execution is the lifecycle's latest
/// execution clock, so an unstated non-execution carries its predecessor's,
/// while two observations of the same event keep their earliest clocks.
/// The order an event states through [`Element::is_after`] is its instant:
/// later is after.
///
/// ```
/// use yggdryl::graph::{Element, Event};
/// use yggdryl::{State, Uuid};
///
/// struct Report {
///     curruuid: Uuid,
///     crossuuid: Uuid,
///     crosscode: String,
///     hashcode: u64,
///     crosshashcode: u64,
///     sources: Vec<Uuid>,
///     unix: i64,
///     state: State,
///     seqnum: u64,
///     creaunix: Option<i64>,
///     execunix: Option<i64>,
///     recdunix: Option<i64>,
///     exprtime: Option<i64>,
///     prevunix: Option<i64>,
///     prevuuid: Option<Uuid>,
///     snapunix: Option<i64>,
/// }
///
/// impl Report {
///     fn at(uuid: u128, unix: i64) -> Self {
///         Self {
///             curruuid: Uuid::from_v8(uuid),
///             crossuuid: Uuid::from_v8(uuid),
///             crosscode: String::new(),
///             hashcode: 0,
///             crosshashcode: 0,
///             sources: Vec::new(),
///             unix,
///             state: State::from_spelling("New").expect("a shipped state"),
///             seqnum: 0,
///             creaunix: None,
///             execunix: None,
///             recdunix: None,
///             exprtime: None,
///             prevunix: None,
///             prevuuid: None,
///             snapunix: None,
///         }
///     }
/// }
///
/// impl Element for Report {
///     fn get_curruuid(&self) -> Uuid {
///         self.curruuid
///     }
///     fn set_curruuid(&mut self, curruuid: Uuid) {
///         self.curruuid = curruuid;
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
///     fn get_currhashcode(&self) -> u64 {
///         self.hashcode
///     }
///     fn set_currhashcode(&mut self, hashcode: u64) {
///         self.hashcode = hashcode;
///     }
///     fn get_crosshashcode(&self) -> u64 {
///         self.crosshashcode
///     }
///     fn set_crosshashcode(&mut self, crosshashcode: u64) {
///         self.crosshashcode = crosshashcode;
///     }
///     fn get_srcuuids(&self) -> &[Uuid] {
///         &self.sources
///     }
///     fn set_srcuuids(&mut self, sources: Vec<Uuid>) {
///         self.sources = sources;
///     }
///     // An event's order is its instant.
///     fn is_after(&self, other: &Self) -> bool {
///         self.unix > other.unix
///     }
///     // The report's identity is assigned here, so finalizing keeps it; an
///     // event with a derived identity would hand the content's digest to
///     // `finalized`, which couples it to the instant, sequence and cross seed.
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
///     fn get_currunix(&self) -> i64 {
///         self.unix
///     }
///     fn set_currunix(&mut self, unix: i64) {
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
///     fn get_creaunix(&self) -> Option<i64> {
///         self.creaunix
///     }
///     fn set_creaunix(&mut self, unix: Option<i64>) {
///         self.creaunix = unix;
///     }
///     fn get_execunix(&self) -> Option<i64> {
///         self.execunix
///     }
///     fn set_execunix(&mut self, unix: Option<i64>) {
///         self.execunix = unix;
///     }
///     fn get_recdunix(&self) -> Option<i64> {
///         self.recdunix
///     }
///     fn set_recdunix(&mut self, unix: Option<i64>) {
///         self.recdunix = unix;
///     }
///     fn get_exprtime(&self) -> Option<i64> {
///         self.exprtime
///     }
///     fn set_exprtime(&mut self, unix: Option<i64>) {
///         self.exprtime = unix;
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
/// first.set_creaunix(Some(5_000));
/// first.set_crosscode("O-100".to_owned());
/// let second = Report::at(2, 20_000);
/// assert!(second.is_after(&first) && first.is_before(&second));
/// let second = second.with_previous(&first).expect("the later one follows");
/// assert_eq!(second.get_prevuuid(), Some(first.get_curruuid()));
/// assert_eq!(second.get_prevunix(), Some(10_000));
/// // Following carries the lifecycle forward: the earliest creation known,
/// // the cross code the chain shares - its digest and the cross identity in
/// // step - and the next place in it.
/// assert_eq!(second.get_creaunix(), Some(5_000));
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
/// assert_eq!(held.get_currunix(), 20_000);
/// assert_eq!(held.get_creaunix(), Some(5_000));
/// assert!(held.get_state().is_live());
/// // The identity its millisecond, sequence and seeded content derive.
/// let earlier = first.time_uuid().expect("an instant a TxHash holds");
/// let later = second.time_uuid().expect("an instant a TxHash holds");
/// assert!(earlier < later);
/// assert_eq!(first.txhash().expect("a TxHash").unix(), 10_000);
/// ```
pub trait Event: Element {
    /// When this event happened: nanoseconds since the Unix epoch, UTC.
    fn get_currunix(&self) -> i64;

    /// Records when this event happened, as nanoseconds since the Unix
    /// epoch, UTC.
    fn set_currunix(&mut self, unix: i64);

    /// Where this event stands in its lifecycle: the crate's ranked
    /// [`State`] code, never absent - an event that reached no state says
    /// `00UNKNOWN`.
    fn get_state(&self) -> &State;

    /// Records where this event stands in its lifecycle.
    fn set_state(&mut self, state: State);

    /// Whether this event itself reports an execution.
    ///
    /// Provided from the generic lifecycle [`State`]. A protocol whose report
    /// kind and lifecycle state are separate facts overrides this event-level
    /// classification hook, leaving the shared state vocabulary unchanged.
    fn is_execution(&self) -> bool {
        self.get_state().is_execution()
    }

    /// Where this event stands in its chain: how many came before it.
    fn get_seqnum(&self) -> u64;

    /// Records where this event stands in its chain.
    fn set_seqnum(&mut self, seqnum: u64);

    /// When this event was created, in the same count as [`Self::get_currunix`],
    /// where it knows.
    fn get_creaunix(&self) -> Option<i64>;

    /// Records when this event was created; `None` states it does not
    /// know.
    fn set_creaunix(&mut self, unix: Option<i64>);

    /// The latest execution instant this lifecycle has reached as of this
    /// event, in the same count as [`Self::get_currunix`], where it knows. An
    /// execution state with no stated instant is filled from this event's own
    /// instant; a later non-execution event carries the predecessor's clock.
    fn get_execunix(&self) -> Option<i64>;

    /// Records when this event executed; `None` states it does not know.
    fn set_execunix(&mut self, unix: Option<i64>);

    /// When this event was recorded, in the same count as
    /// [`Self::get_currunix`], where it knows.
    fn get_recdunix(&self) -> Option<i64>;

    /// Records when this event was recorded; `None` states it does not know.
    fn set_recdunix(&mut self, unix: Option<i64>);

    /// When this event expires, in the same count as [`Self::get_currunix`], where
    /// it has an expiry.
    fn get_exprtime(&self) -> Option<i64>;

    /// Records when this event expires; `None` states it does not expire,
    /// or does not know.
    fn set_exprtime(&mut self, unix: Option<i64>);

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
    /// same count as [`Self::get_currunix`], where a walk over a grid took one
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
    /// of one chain share it. What the event itself says - its instant, recording clock,
    /// sources and snapshot - is its own and moves nowhere. An execution state
    /// with no precise clock takes its own instant; following then keeps the
    /// later of this event's clock and the predecessor's latest execution, so
    /// a delayed report cannot regress the lifecycle. An event that moved is
    /// finalized, so it never carries the identity of what it was.
    ///
    /// Provided, so an implementor's [`Element::with_previous`] has a
    /// default to delegate to; an event that means something else by
    /// following states its own there instead.
    fn following(mut self, previous: &Self) -> Option<Self>
    where
        Self: Sized,
    {
        if previous.get_curruuid() == self.get_curruuid()
            || previous.get_currunix() > self.get_currunix()
        {
            return None;
        }
        if !follow_timed(&mut self, previous) {
            return None;
        }
        self.finalize();
        Some(self)
    }

    /// This event stated as another statement of `live`: the same event -
    /// the same instant, the same content - read a second time, as a
    /// capture logs one message at every hop it passes. It takes the place
    /// `live` holds in its chain - the predecessor, the position, the
    /// snapshot - the chain's cross code, the names `live` knows, and the
    /// lifecycle folded, so the two statements finalize to
    /// one identity and the chain grows by nothing. Their execution and
    /// recording instants fold to the earliest either statement knows. Its
    /// sources stay its own: provenance travels along no chain. What the
    /// event states of its own - its instant, its content - is its own.
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
        fold_event_instants(&mut self, live);
        self.fold_lifecycle(live);
        self.finalize();
        self
    }

    /// This event with another statement of itself folded in, by the
    /// timed reading, or nothing where `other` is another event.
    ///
    /// The element-level merge first - the reference statement's cross code
    /// and list order, with the other statement filling what it leaves
    /// unstated - and then the timed facts: the instant and code are
    /// the reference's. The reference is the statement with the latest
    /// `recdunix`; a stated clock leads an unstated one, a tie falls back
    /// to the later event instant, and an exact tie keeps this one. The
    /// place in the chain is the further of the two; the
    /// lifecycle folds as [`Self::following`] folds it - earliest creation,
    /// latest expiration, furthest state; the execution and recording clocks
    /// are the earliest either statement of this event knows; and the
    /// predecessor and snapshot instant are the reference's where it states
    /// them, else the other's. A merged statement therefore ranks by the
    /// earliest recording it knows against a third, so which of three leads
    /// depends on the order they are merged in: a caller holding every
    /// statement at once picks the reference over all of them first.
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
        let other_is_reference = right_is_reference(
            self.get_recdunix(),
            self.get_currunix(),
            other.get_recdunix(),
            other.get_currunix(),
        );
        let changed = merge_event_element(&mut self, other, other_is_reference);
        if !(merge_timed(&mut self, other, other_is_reference) || changed) {
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
            self.get_creaunix(),
            earliest(self.get_creaunix(), other.get_creaunix()),
            |unix| self.set_creaunix(unix),
        );
        changed |= moved(
            self.get_exprtime(),
            latest(self.get_exprtime(), other.get_exprtime()),
            |unix| self.set_exprtime(unix),
        );
        let state = self.get_state().clone().merge_with(other.get_state());
        changed |= moved(self.get_state().clone(), state, |state| {
            self.set_state(state)
        });
        changed
    }

    /// Records the code this event's content digests to and resets the
    /// identity the instant, sequence, cross hash and that code derive, and
    /// the cross element behind it.
    ///
    /// Provided, and what an implementor's [`Element::finalize`] hands the
    /// digest of its content to. The identity is [`Self::time_uuid`], and
    /// an instant a UUIDv7 cannot hold leaves the identity as it was; the
    /// cross element is [`Element::cross_uuid`] over the identity that
    /// results, which is the identity itself for an event in no chain. An
    /// implementor whose public code setter already projects both identities
    /// overrides this method to batch the final write and projection; the
    /// provided behavior remains correct for ordinary independent setters.
    fn finalized(&mut self, hashcode: u64) {
        self.set_currhashcode(hashcode);
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
    /// executed, recorded, expires, the predecessor's and the snapshot's -
    /// are left out, so the code says what an event states and not when.
    fn digest_event(&self) -> Xxh3 {
        let mut state = self.digest();
        feed_timed(&mut state, self);
        state
    }

    /// The instant and the code coupled: a [`TxHash`] of [`Self::get_currunix`]
    /// at nanosecond resolution and [`Element::get_currhashcode`] as the XXH3-64
    /// digest it is, which is the crate's own time-ordered identity.
    ///
    /// Provided, so every event derives it the same way.
    ///
    /// # Errors
    ///
    /// Returns the [`TxHash`]'s own refusal, which a nanosecond count never
    /// raises.
    fn txhash(&self) -> Result<TxHash> {
        coupled(self.get_currunix(), self.get_currhashcode())
    }

    /// The generic event identity: RFC 9562 UUIDv7 with
    /// [`Self::get_currunix`] floored to milliseconds in its timestamp,
    /// [`Self::get_seqnum`] in `rand_a`, and a 62-bit XXH3 payload over the
    /// content code and the whole sequence, seeded by
    /// [`Element::get_crosshashcode`]. The explicit sequence lane saturates at
    /// `4095`; feeding the whole `u64` sequence into the payload keeps larger
    /// sequence values probabilistically distinct in that terminal lane.
    /// Identities therefore sort by millisecond and then by every sequence
    /// the UUID lane can represent, while the seeded payload separates
    /// content, cross chains and sequence overflow.
    ///
    /// Provided: an implementor whose identity is when it happened and what
    /// it says answers this from [`Element::get_curruuid`], and one whose
    /// identity is assigned keeps its own.
    ///
    /// # Errors
    ///
    /// Returns a clock-restatement failure or the UUIDv7 timestamp refusal.
    fn time_uuid(&self) -> Result<Uuid> {
        self.txhash()?
            .into_sequenced_uuid(self.get_seqnum(), self.get_crosshashcode())
    }
}

/// The fact the selected statement states, else the other's, else nothing.
pub(super) fn stated<T>(this: Option<T>, other: Option<T>, later: bool) -> Option<T> {
    if later {
        other.or(this)
    } else {
        this.or(other)
    }
}
