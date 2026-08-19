//! The expression engine's broad matrices, dispatched over category files.
//!
//! What lives here is what has to run against the *published* surface and
//! across every datatype the core has: the parity between the three
//! evaluators, the exhaustive type matrix, the optimizer's own properties, and
//! the Rust trait set. The module's own edge cases live beside the code in
//! `rust/src/expressions/tests.rs`.

#[path = "expressions/ergonomics.rs"]
mod ergonomics;
#[path = "expressions/matrix.rs"]
mod matrix;
#[path = "expressions/optimizer.rs"]
mod optimizer;
#[path = "expressions/parity.rs"]
mod parity;
#[path = "expressions/statements.rs"]
mod statements;
