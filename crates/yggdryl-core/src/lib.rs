//! # yggdryl-core
//!
//! The dependency-light foundation crate for yggdryl, on which every other crate
//! and binding builds.
//!
//! It holds the zero-copy [`Buffer`] and the [`Io`] byte-source abstraction (with
//! its [`Whence`] seek origin and the in-memory [`BytesIo`] backend); reintroduce
//! the rest of the foundational types here — one module per concern, each
//! re-exported at the crate root — following the rules in `CLAUDE.md`.

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

mod buffer;
mod bytes_io;
mod io;
mod whence;

pub use buffer::Buffer;
pub use bytes_io::BytesIo;
pub use io::{Io, IoError};
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
