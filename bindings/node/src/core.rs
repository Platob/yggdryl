//! The `yggdryl.core` namespace — thin wrappers over the `yggdryl-core` crate.
//!
//! It currently exposes the hello-world entry points; more surface is added here as
//! the core crate grows.

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
