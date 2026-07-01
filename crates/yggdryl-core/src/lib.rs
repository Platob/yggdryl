//! # yggdryl-core
//!
//! The dependency-light foundation crate for yggdryl, on which every other crate
//! and binding builds.
//!
//! It holds the byte/array-IO foundation: the positional [`Io`] trait — a
//! random-access array of `T` values addressed from a [`Whence`] origin via
//! [`pread_one`](Io::pread_one) / [`pwrite_one`](Io::pwrite_one), with the in-memory
//! [`Vec`] as its leaf implementation, plus zero-copy whole-source transfers via
//! [`pread_io`](Io::pread_io) / [`pwrite_io`](Io::pwrite_io) — plus the [`IoCursor`]
//! (a stateful cursor) and [`IoSlice`] (a bounded window) that wrap an inner [`Io`],
//! and the [`hello`] / [`version`] scaffold. Reintroduce the rest of the foundational
//! types here as the design lands — one module per concern, each re-exported at the
//! crate root — following the rules in `CLAUDE.md`.

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise (so the crate stays dependency-free by default and pays no runtime
/// cost). Submodules reach it via `crate::log_event!`.
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        log::$level!($($arg)+);
    }};
}
pub(crate) use log_event;

mod io;
mod io_cursor;
mod io_slice;
mod whence;

pub use io::{Io, IoError};
pub use io_cursor::IoCursor;
pub use io_slice::IoSlice;
pub use whence::Whence;

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
