//! Expression grammar and field-path integration tests.

#[path = "expression/arrow.rs"]
mod arrow;
#[path = "expression/bind.rs"]
mod bind;
#[path = "expression/display.rs"]
mod display;
#[path = "expression/eval.rs"]
mod eval;
#[path = "expression/explain.rs"]
mod explain;
#[path = "expression/filter.rs"]
mod filter;
#[path = "expression/literal.rs"]
mod literal;
#[path = "expression/mod_.rs"]
mod mod_;
#[path = "expression/parser.rs"]
mod parser;
#[path = "expression/path.rs"]
mod path;
#[path = "expression/plan.rs"]
mod plan;
#[path = "expression/pushdown.rs"]
mod pushdown;
#[path = "expression/selector.rs"]
mod selector;
#[path = "expression/serde.rs"]
mod serde;
#[path = "expression/term.rs"]
mod term;
#[path = "expression/transform.rs"]
mod transform;
#[path = "expression/typing.rs"]
mod typing;
#[path = "expression/user.rs"]
mod user;
