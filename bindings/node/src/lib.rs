//! Node.js extension for **yggdryl**.
//!
//! Each Rust crate is exposed under its own JS namespace — `yggdryl.core` (the
//! foundations) and `yggdryl.schema` (the Arrow schema layer) — mirroring the
//! crate tree. The wrappers are thin: all logic lives in the Rust crates, so the
//! Node and Python bindings behave identically.

mod core;
mod schema;

// Re-export so plain `cargo` / `clippy` does not flag the napi items as unused;
// napi exports them under their namespaces regardless.
pub use core::version;
pub use schema::DataTypeId;
