//! # yggdryl-core
//!
//! The dependency-light foundation crate for yggdryl, on which every other crate
//! and binding builds.
//!
//! The project is being rebuilt around an **Apache Arrow-centralized** data model.
//! This crate currently holds only the minimal [`hello`] / [`version`] entry points
//! — the clean-slate example that round-trips identically through the Python and
//! Node bindings. Add further foundational types here as the design lands, one
//! module per concern, each re-exported at the crate root, following the rules in
//! `CLAUDE.md`.

/// The crate version, as declared in `Cargo.toml`.
///
/// ```
/// assert_eq!(yggdryl_core::version(), env!("CARGO_PKG_VERSION"));
/// ```
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Prints `Hello, world!` to standard output — the minimal cross-language example,
/// surfaced identically from the Python and Node bindings.
///
/// ```
/// yggdryl_core::hello();
/// ```
pub fn hello() {
    println!("Hello, world!");
}
