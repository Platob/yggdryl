//! The event vocabulary every element of a graph answers about itself,
//! whatever crate holds the element. [`Element`] is the node - its
//! [`Uuid`](crate::Uuid), the one it has elsewhere and the UUIDs of what it
//! was read from, read and written, the order it stands in, and how it
//! follows and merges - and [`Event`] is an element that also happened at
//! one instant and stands in one state. [`ElementColumn`] is the columns
//! every generated schema of an element opens with and [`EventColumn`] the
//! ones an event adds, one per fact the traits answer, so a text line's batch
//! and a market row open with the same columns and join on them without a
//! mapping. The market leaves, the walk over them and the columns a market
//! and an operation add are `yggdryl_market::graph`'s.

pub mod column;
pub mod element;
pub mod element_column;
pub use column::EventColumn;
pub use element::{Element, Event};
pub use element_column::ElementColumn;
