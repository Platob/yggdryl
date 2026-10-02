//! One test file per file under `rust/src/json/`, mirrored file for file.

#[path = "json/column.rs"]
mod column;
#[path = "json/field.rs"]
mod field;
#[path = "json/mod_.rs"]
mod mod_;
#[path = "json/parser.rs"]
mod parser;
#[path = "json/wire.rs"]
mod wire;
