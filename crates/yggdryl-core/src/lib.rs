//! # yggdryl-core
//!
//! The **generic foundation** every other yggdryl crate builds on. It holds only the
//! reusable, non-io-specific building blocks:
//!
//! - the **wide integers** ([`int`]) — [`i96`] and [`i256`] flanking native `i128`, each a
//!   serializable value type (the io element codec that reads/writes them lives one layer
//!   up in `yggdryl-buffer`);
//! - the **byte-codec base** ([`codec`]) — the [`Encoder`] / [`Decoder`] byte-array
//!   contracts, their element-generic [`TypedEncoder`] / [`TypedDecoder`] extensions, and
//!   the [`EncodeError`] / [`DecodeError`] types.
//!
//! Its only dependency is `arrow-buffer` (for `i256`). The concrete codecs build on this
//! base in the crates above: positioned IO and the typed buffers in `yggdryl-buffer`, the
//! compression codecs in `yggdryl-compression`, the representation converters in
//! `yggdryl-converter`. One module per concern, each re-exported at the crate root.

pub mod codec;
pub mod int;

pub use codec::{DecodeError, Decoder, EncodeError, Encoder, TypedDecoder, TypedEncoder};
pub use int::{i256, i96};

/// Re-export of the exact `arrow-buffer` the wide integers are backed by, so callers
/// construct values against a matching version.
pub use arrow_buffer;

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
