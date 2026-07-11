//! Wide signed integers beyond the native widths — [`i96`] and [`i256`] — that flank
//! native [`i128`], each a serializable value type; the io element codec `IoPrimitive` (in
//! `yggdryl-buffer`) reads and writes them little-endian through a typed cursor.
//!
//! [`i96`] is built on native `i128` (canonicalised to 96 bits); [`i256`] is Arrow's
//! own 256-bit integer (the core is Arrow-backed). Native `i128` needs no wrapper and
//! is used directly.

// The module names differ from the type names (`i96`/`i256`) they hold, since a
// module and a type cannot share a name in the same namespace.
#[path = "i256.rs"]
mod i256_impl;
#[path = "i96.rs"]
mod i96_impl;

pub use i256_impl::i256;
pub use i96_impl::i96;
