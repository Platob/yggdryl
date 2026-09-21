//! Graph vocabulary integration tests.

#[path = "graph/column.rs"]
mod column;
#[path = "graph/element.rs"]
mod element;
#[cfg(feature = "internals")]
#[path = "graph/instrument.rs"]
mod instrument;
#[path = "graph/iterator.rs"]
mod iterator;
