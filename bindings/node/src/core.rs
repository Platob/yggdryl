//! The `yggdryl.core` namespace — thin wrappers over the `yggdryl-core` crate.
//!
//! It exposes the crate's `version` / `hello` entry points; the other `yggdryl-core`
//! modules surface as their own sibling namespaces (`compression`, `io`, `buffer`).

use napi_derive::napi;

/// The `yggdryl-core` version string.
#[napi(namespace = "core")]
pub fn version() -> String {
    yggdryl_core::version().to_string()
}

/// Prints `Hello, world!` to standard output — the minimal cross-language example.
#[napi(namespace = "core")]
pub fn hello() {
    yggdryl_core::hello()
}
