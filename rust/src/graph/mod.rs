//! The graph vocabulary: what an element of a graph answers about itself.
//!
//! A graph is elements that know their own identity and the identities of
//! the elements they descend from, and [`element`] holds the two traits that
//! state it. [`Element`] is the node: its [`Uuid`](crate::types::Uuid) and its
//! parents' UUIDs, read and written. [`TimeElement`] is an element that also
//! happened at one instant and digests to one code, which is what an event
//! is. The traits state signatures and nothing else - no storage, no
//! walk - so a message, a chain entry and a lifecycle incarnation can each be
//! an element without the graph owning any of them.

pub mod element;

pub use element::{Element, TimeElement};
