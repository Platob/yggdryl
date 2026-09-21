//! The graph vocabulary: what an element of a graph answers about itself,
//! and one walk over elements.
//!
//! A graph is elements that know their own identity and the identities of
//! the elements they descend from, and [`element`] holds the four traits
//! that state it. [`Element`] is the node: its [`Uuid`](crate::Uuid),
//! the one it has elsewhere, the names it goes by and its parents' UUIDs,
//! read and written, the order it stands in, and how it follows and merges.
//! [`Event`] is an element that also happened at one instant and stands in
//! one state. [`MarketElement`] is one that stands in a market: a price, a
//! quantity and a side; [`MarketEvent`] is one that did both. The traits
//! state signatures and the provided readings - no storage - so a message,
//! a chain entry and a lifecycle incarnation can each be an element without
//! the graph owning any of them; [`MarketElementData`] and
//! [`MarketEventData`] hold the facts as plain fields for the holder that
//! wants nothing more. The one walk, [`EventIterator`], reads events in
//! their order and states each as the one after the live element it
//! follows. [`EventColumn`] is the nineteen columns every generated schema
//! of an event states - one per fact the traits answer, under one name and
//! one datatype each - so a text line's batch, a FIX row and a chained
//! message join on them without a mapping. Event-native schemas use
//! [`EventColumn::ALL`] order; a FIX row keeps its protocol-oriented bands.

pub mod column;
pub mod element;
pub mod event;
pub(crate) mod instrument;
pub mod iterator;

pub use column::EventColumn;
pub use element::{Element, Event, MarketElement, MarketEvent};
pub use event::{MarketElementData, MarketEventData};
pub use iterator::EventIterator;
