//! # yggdryl-core
//!
//! The dependency-light foundation crate for yggdryl, on which every other crate
//! and binding builds.
//!
//! It exposes the [`Charset`] trait (with the [`Utf8`] and [`Latin1`] encodings),
//! the positioned-I/O traits [`RawIOBase`] (raw bytes/bits) and [`IOBase`] (a typed
//! layer over it) with their [`Whence`] reference point, and — behind the
//! off-by-default `json` feature — the `Base` trait for content JSON plus an
//! implementor-defined byte form. The [`version`] and [`hello`] entry points remain
//! as the minimal cross-language round-trip example. Add further foundational types
//! here as the design lands — one module per concern, each re-exported at the crate
//! root — following the rules in `CLAUDE.md`.

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise (so the crate stays dependency-free by default and pays no runtime
/// cost). Submodules reach it via `crate::log_event!` thanks to the re-export
/// below.
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        ::log::$level!($($arg)+);
    }};
}
pub(crate) use log_event;

mod charset;
pub use charset::{Charset, CharsetError, Latin1, Utf8};

mod io;
pub use io::{IOBase, IOError, RawIOBase, Whence};

#[cfg(feature = "json")]
mod base;
#[cfg(feature = "json")]
pub use base::{Base, BaseError};

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

/// Prints a greeting to standard output — the minimal cross-language example,
/// surfaced identically from the Python and Node bindings.
///
/// ```
/// yggdryl_core::hello();
/// ```
pub fn hello() {
    log_event!(debug, "yggdryl_core::hello");
    println!("Hello from yggdryl {}!", version());
}
