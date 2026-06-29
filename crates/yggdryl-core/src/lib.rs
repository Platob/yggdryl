//! # yggdryl-core
//!
//! The dependency-light foundation crate for yggdryl, on which every other crate
//! and binding builds.
//!
//! The previous implementation was removed in a project reset; this is the
//! buildable scaffold. Reintroduce the foundational types here — one module per
//! concern, each re-exported at the crate root — following the rules in
//! `CLAUDE.md`.

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise (so the crate stays dependency-free by default and pays no runtime
/// cost). Add `pub(crate) use log_event;` here to share it with submodules via
/// `crate::log_event!` once the crate grows past a single file.
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        log::$level!($($arg)+);
    }};
}

/// The crate version, as declared in `Cargo.toml`.
///
/// A placeholder entry point so the scaffold builds and round-trips through the
/// Python and Node bindings; replace it with the real types as they land.
///
/// ```
/// assert_eq!(yggdryl_core::version(), env!("CARGO_PKG_VERSION"));
/// ```
pub fn version() -> &'static str {
    log_event!(trace, "yggdryl_core::version");
    env!("CARGO_PKG_VERSION")
}
