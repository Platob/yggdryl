//! Node.js extension for **yggdryl**.
//!
//! Blank slate. The napi-rs wrappers were removed in a project reset; the
//! bindings will be reimplemented as thin wrappers over the Arrow-centralized
//! Rust core. All logic lives in the shared core so the Node and Python bindings
//! stay in lockstep — see `CLAUDE.md` for the contributor rules.

use napi_derive::napi;

/// Placeholder export so `napi build` still emits the JS loader. Replaced when
/// the Arrow-centralized bindings are reimplemented.
#[napi]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
