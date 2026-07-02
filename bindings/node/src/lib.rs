//! Node.js extension for **yggdryl**.
//!
//! Each Rust crate is exposed under its own JS namespace — `yggdryl.core` (the
//! foundations) and `yggdryl.data` (the Arrow data-model layer) — mirroring the
//! crate tree. The wrappers are thin: all logic lives in the Rust crates, so the
//! Node and Python bindings behave identically.

mod core;
mod data;

// Re-export so plain `cargo` / `clippy` does not flag the napi items as unused;
// napi exports them under their namespaces regardless.
pub use core::{hello, version, BitBuffer, ByteBuffer, Whence};
pub use data::*;
