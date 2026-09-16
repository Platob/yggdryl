//! The graph vocabulary: what an element of a graph answers about itself.
//!
//! A graph is elements that know their own identity and the identities of
//! the elements they descend from, and [`element`] holds the three traits
//! that state it. [`Element`] is the node: its [`Uuid`](crate::types::Uuid),
//! the one it has elsewhere and its parents' UUIDs, read and written, and
//! how it follows and merges. [`TimeElement`] is an element that also
//! happened at one instant, stands in one state and digests to one code,
//! which is what an event is. [`MarketElement`] is one that happened in a
//! market: a price, a quantity and a side. The traits state signatures and
//! the two provided readings - no storage, no walk - so a message, a chain
//! entry and a lifecycle incarnation can each be an element without the
//! graph owning any of them.

pub mod element;

pub use element::{Element, MarketElement, TimeElement};
