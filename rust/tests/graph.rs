//! Graph vocabulary integration tests.

#[path = "graph/arrow.rs"]
mod arrow;
#[path = "graph/book.rs"]
mod book;
#[path = "graph/column.rs"]
mod column;
#[path = "graph/element.rs"]
mod element;
#[path = "graph/execution.rs"]
mod execution;
#[cfg(feature = "internals")]
#[path = "graph/instrument.rs"]
mod instrument;
#[path = "graph/iterator.rs"]
mod iterator;
#[path = "graph/market_column.rs"]
mod market_column;
#[path = "graph/order.rs"]
mod order;
#[path = "graph/quote.rs"]
mod quote;
