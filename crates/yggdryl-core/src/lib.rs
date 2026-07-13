//! yggdryl core — the Apache Arrow-backed foundation.
//!
//! Minimal example: a single [`version`] function, wired through to the Python and Node
//! extensions (`yggdryl.version()` in both). New features are added here first, in the
//! Rust core, then mirrored thinly in each binding.

/// The Arrow-backed physical I/O layer (raw byte/buffer primitives).
pub mod io;

/// The crate version string (from `Cargo.toml`), e.g. `"0.1.1"`.
///
/// This is the minimal end-to-end example: the same value is exposed by the Python and
/// Node extensions.
///
/// ```
/// assert_eq!(yggdryl_core::version(), env!("CARGO_PKG_VERSION"));
/// ```
pub const fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_the_crate_version() {
        assert_eq!(super::version(), env!("CARGO_PKG_VERSION"));
        assert!(!super::version().is_empty());
    }
}
