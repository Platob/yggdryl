//! The market's products: what the market did, read out of what a venue
//! said.
//!
//! A message is what a venue said; a product is what the market did. Five
//! products, one file each: an [`Order`] is one order's life, a [`Quote`]
//! a price stated at an instant, an [`Execution`] one fill, a [`Trade`]
//! the settled transaction an execution reports, and a [`Book`] the
//! ladder for one instrument at one instant. Each is a trait - the
//! product's own accessors and mutators, and the readings its stated
//! facts imply, read on every call and never stored - beside its holder,
//! [`OrderData`], [`QuoteData`], [`ExecutionData`], [`TradeData`] and
//! [`BookData`], a value of this crate with its own layout, its own Arrow
//! row and its own identity - not a view over a message and not a second
//! model of any protocol - and each holder is a graph event exactly as a
//! message is: it implements [`Element`](crate::graph::Element),
//! [`Event`](crate::graph::Event) and
//! [`MarketElement`](crate::graph::MarketElement), so its identity is what
//! its content digests to, its `srcuuids` are the identities of the
//! messages it was read from, its chain is the walk's to fill and its row
//! opens with the sixteen [`EventColumn`](crate::graph::EventColumn)s
//! every event's does. That is the whole interop story: a product's table
//! joins the message table on `srcuuids` to `curruuid` with no mapping, a
//! message's joins the line table the same way, and a walked product's
//! `prevuuid` and `parentuuids` join its own table.
//!
//! [`Symbol`] is the key an instrument's books are read under, the global
//! one for no instrument; [`Statement`] is one value over the four
//! products that state something to a ladder, so a mixed stream is a
//! stream of market events; and [`BookIterator`] reads one book per
//! symbol per instant out of such a stream, the live orders and quotes
//! kept per symbol and the prints counted. [`product`] holds what the
//! five share: [`MarketColumn`], the columns the market's facts are
//! stated in; [`Product`], what a product answers about its row and the
//! row it gets for it; [`Folded`], the fold that makes two statements of
//! one product one; [`Walked`], the one walk over a fallible stream of
//! statements; and the row doors every product's Arrow twin composes.
//! Nothing here reads a protocol: the FIX reading of a product - one door
//! per product over a stream of messages, one stream of every statement,
//! and one over an Arrow reader of message rows - lives beside the
//! [codec](crate::FixCodec), so this stays a vocabulary a second protocol
//! could feed.

pub mod book;
pub mod execution;
pub mod iterator;
pub mod order;
pub mod product;
pub mod quote;
pub mod statement;
pub mod symbol;
pub mod trade;

pub use book::{Book, BookData, Level};
pub use execution::{Execution, ExecutionData};
pub use iterator::BookIterator;
pub use order::{Order, OrderData, Pricing};
pub use product::{
    Cells, Folded, MarketColumn, Product, RowPlan, Walked, arrow_reader, read_arrow_reader,
};
pub use quote::{Quote, QuoteData};
pub use statement::Statement;
pub use symbol::Symbol;
pub use trade::{Party, Trade, TradeData};
