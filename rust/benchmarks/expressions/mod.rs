//! Benchmarks for the expression engine.
//!
//! Every performance claim this change makes carries a baseline the reader can
//! check: the hand-written kernel a legitimate implementation would use, and -
//! where this replaces something - the code it replaced, on the same data.

mod common;

pub mod bind;
pub mod eval;
pub mod parse;
pub mod prune;

pub(crate) use common::{fixture_batch, fixture_rows, schema};
