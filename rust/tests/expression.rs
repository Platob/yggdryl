//! Expression grammar and field-path integration tests.

#[cfg(feature = "internals")]
#[path = "expression/eval.rs"]
mod eval;
#[path = "expression/grammar.rs"]
mod grammar;
#[path = "expression/path.rs"]
mod path;
#[path = "expression/plan.rs"]
mod plan;
#[path = "expression/user.rs"]
mod user;
