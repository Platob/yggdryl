//! yggdryl core — a dependency-free byte / memory-access foundation.
//!
//! Everything lives under the [`io`] layer: the abstract byte-access contract
//! ([`io::memory::IOBase`]) with its sources, the addressing [`io::uri`] family, and the
//! cross-cutting value types at the [`io`] root. New features are added here first, in the
//! Rust core, then mirrored thinly in the Python and Node extensions.

/// The io layer: byte / memory access, addressing, and the shared value types.
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
