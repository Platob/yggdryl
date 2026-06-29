//! Node.js extension for **yggdryl**.
//!
//! Thin napi-rs wrappers over the `yggdryl_core` types; each type gets its own
//! module mirroring the Rust crate, with all logic living in the shared core so
//! the Node and Python bindings behave identically.
//!
//! The implementation was removed in a project reset; this scaffold surfaces only
//! `version()` so the addon builds and the cross-language pattern is in place.

use napi_derive::napi;

/// The `yggdryl-core` version string.
#[napi]
pub fn version() -> String {
    yggdryl_core::version().to_string()
}
