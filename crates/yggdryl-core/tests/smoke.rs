//! Smoke test for the `yggdryl-core` `hello` / `version` entry points.

use yggdryl_core::{hello, version};

#[test]
fn version_is_the_package_version() {
    assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    assert!(!version().is_empty());
}

#[test]
fn hello_prints_without_panicking() {
    hello();
}
