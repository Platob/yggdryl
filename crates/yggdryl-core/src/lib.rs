//! # yggdryl-core
//!
//! The dependency-light foundations shared by the yggdryl type crates:
//!
//! - [`Buffer`] — an `Arc`-backed, zero-copy byte buffer (O(1) clone, zero-copy
//!   slicing).
//! - [`Charset`] — text encodings (UTF-8, ASCII, Latin-1) for the JSON byte form.
//! - [`Io`] / [`Whence`] — positional and cursor byte access; reads hand back
//!   zero-copy [`Buffer`] views.
//! - [`Jsonable`] / [`JsonParams`] — the JSON/BSON serialization trait and its
//!   process-global format + charset parameters.
//! - The error types ([`TypeError`], [`ScalarError`], [`FieldError`], [`IoError`],
//!   [`CharsetError`]) and the [`mapping`] component-map codec.
//!
//! The Arrow-centric types built on these foundations live in the sibling crates
//! `yggdryl-dtype` (data types), `yggdryl-scalar` (values) and `yggdryl-field`
//! (fields).
//!
//! ```
//! use yggdryl_core::{Buffer, Charset};
//!
//! let buf = Buffer::from_slice(b"hello world");
//! assert_eq!(buf.slice(0..5).as_slice(), b"hello"); // zero-copy view
//! assert_eq!(Charset::Latin1.encode("é"), vec![0xe9]);
//! ```

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise (so the crate stays dependency-free by default and pays no runtime
/// cost). Shared by every submodule via `crate::log_event!`.
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        log::$level!($($arg)+);
    }};
}
pub(crate) use log_event;

mod buffer;
mod charset;
#[cfg(any(
    feature = "gzip",
    feature = "zstd",
    feature = "snappy",
    feature = "brotli"
))]
pub mod compress;
mod error;
mod io;
#[cfg(feature = "json")]
mod json;
pub mod mapping;

pub use buffer::Buffer;
pub use charset::Charset;
#[cfg(feature = "json")]
pub use error::JsonError;
pub use error::{CharsetError, FieldError, IoError, ScalarError, TypeError};
pub use io::{Io, Whence};
#[cfg(feature = "json")]
pub use json::{json_params, reset_json_params, set_json_params, JsonParams, Jsonable};
