//! The `yggdryl.core` namespace — thin wrappers over the `yggdryl-core` crate.

use napi_derive::napi;

mod whence;

pub use whence::Whence;

/// The `yggdryl-core` version string.
#[napi(namespace = "core")]
pub fn version() -> String {
    yggdryl_core::version().to_string()
}

/// Prints a greeting to standard output — the minimal cross-language example.
#[napi(namespace = "core")]
pub fn hello() {
    yggdryl_core::hello()
}
