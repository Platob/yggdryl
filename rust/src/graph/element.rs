//! An element of a graph, and one that happened at an instant.
//!
//! Two traits, and only signatures: what an element answers about itself and
//! what it takes. The identity is the crate's own [`Uuid`], so an element is
//! addressed the way every identified value in the crate is, and a parent is
//! named by the same identity rather than by a reference, so an element can
//! name a parent it does not hold.

use crate::types::Uuid;

/// One element of a graph: a node that knows its own identity and the
/// identities of the elements it descends from.
///
/// The parents are an ordered list, and the order is the implementor's to
/// state: a message's chain, a node's lineage. An element with no parent is
/// a root. Both facts are read and written through the trait, so a store or
/// a walk that only knows an element as `dyn Element` can still place it.
///
/// ```
/// use yggdryl::graph::Element;
/// use yggdryl::types::Uuid;
///
/// struct Node {
///     uuid: Uuid,
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
///     fn parentuuids(&self) -> &[Uuid] {
///         &self.parents
///     }
///     fn set_parentuuids(&mut self, parents: Vec<Uuid>) {
///         self.parents = parents;
///     }
/// }
///
/// let root = Uuid::from_v8(1);
/// let mut child = Node { uuid: Uuid::from_v8(2), parents: Vec::new() };
/// assert!(child.parentuuids().is_empty(), "a node naming no parent is a root");
/// child.set_parentuuids(vec![root]);
/// assert_eq!(child.parentuuids(), [root]);
/// assert_eq!(child.uuid(), Uuid::from_v8(2));
/// ```
pub trait Element {
    /// This element's identity.
    fn uuid(&self) -> Uuid;

    /// Records this element's identity.
    fn set_uuid(&mut self, uuid: Uuid);

    /// The identities of the elements this one descends from, in the order
    /// the element states them; empty for a root.
    fn parentuuids(&self) -> &[Uuid];

    /// Records the identities of the elements this one descends from, in
    /// the given order; an empty list makes it a root.
    fn set_parentuuids(&mut self, parents: Vec<Uuid>);
}

/// An element that happened at one instant and digests to one code.
///
/// The instant is `unix`: a count of nanoseconds since the Unix epoch, UTC,
/// held as an `i128` so no clock this crate reads is out of its range. The
/// code is `hashcode`: the digest the element's content answers to, held
/// beside the element rather than recomputed, exactly as a message's
/// `msghash` is a column of the row.
///
/// ```
/// use yggdryl::graph::{Element, TimeElement};
/// use yggdryl::types::Uuid;
///
/// struct Event {
///     uuid: Uuid,
///     parents: Vec<Uuid>,
///     unix: i128,
///     hashcode: u64,
/// }
///
/// impl Element for Event {
///     fn uuid(&self) -> Uuid {
///         self.uuid
///     }
///     fn set_uuid(&mut self, uuid: Uuid) {
///         self.uuid = uuid;
///     }
///     fn parentuuids(&self) -> &[Uuid] {
///         &self.parents
///     }
///     fn set_parentuuids(&mut self, parents: Vec<Uuid>) {
///         self.parents = parents;
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
/// }
///
/// let mut event = Event { uuid: Uuid::from_v8(7), parents: Vec::new(), unix: 0, hashcode: 0 };
/// event.set_unix(1_700_000_000_000_000_000);
/// event.set_hashcode(0xDEAD_BEEF);
/// // A time element is an element: one walk reads both.
/// let held: &dyn TimeElement = &event;
/// assert_eq!(held.uuid(), Uuid::from_v8(7));
/// assert_eq!(held.unix(), 1_700_000_000_000_000_000);
/// assert_eq!(held.hashcode(), 0xDEAD_BEEF);
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
}
