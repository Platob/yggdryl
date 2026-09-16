//! The graph vocabulary: what an element of a graph answers about itself,
//! and one walk over elements.
//!
//! A graph is elements that know their own identity and the identities of
//! the elements they descend from, and [`element`] holds the three traits
//! that state it. [`Element`] is the node: its [`Uuid`](crate::types::Uuid),
//! the one it has elsewhere, the names it goes by and its parents' UUIDs,
//! read and written, the order it stands in, and how it follows and merges.
//! [`TimeElement`] is an element that also happened at one instant, stands
//! in one state and digests to one code, which is what an event is.
//! [`MarketElement`] is one that happened in a market: a price, a quantity
//! and a side. The traits state signatures and the provided readings - no
//! storage - so a message, a chain entry and a lifecycle incarnation can
//! each be an element without the graph owning any of them. The one walk,
//! [`ElementIterator`], reads timed elements in their order and states each
//! as the one after the live element it follows.

pub mod element;
pub mod iterator;

pub use element::{Element, MarketElement, TimeElement};
pub use iterator::ElementIterator;
