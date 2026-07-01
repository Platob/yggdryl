//! Node.js extension for **yggdryl**.
//!
//! Each Rust crate is exposed under its own JS namespace — currently just
//! `yggdryl.core` (the foundations) — mirroring the crate tree. The wrappers are
//! thin: all logic lives in the Rust crates, so the Node and Python bindings behave
//! identically.

mod core;

// Re-export so plain `cargo` / `clippy` does not flag the napi items as unused;
// napi exports them under their namespaces regardless.
pub use core::{hello, version};
