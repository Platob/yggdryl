//! [`i256`] — a 256-bit signed integer, re-exported from Arrow.
#![allow(non_camel_case_types)]

/// A 256-bit signed two's-complement integer.
///
/// This is Arrow's own [`arrow_buffer::i256`] — since the core is Arrow-backed, the
/// 256-bit integer *is* Arrow's, with its full, tested arithmetic (`+`, `-`, `*`,
/// `/`, `%`, unary `-`, plus `checked_*` / `wrapping_*`), base-10 `Display`, ordering,
/// and 32-byte little-endian round-trip (`to_le_bytes` / `from_le_bytes`). It is an
/// [`IoPrimitive`](crate::IoPrimitive), so `TypedCursor<i256>` reads and writes it.
///
/// ```
/// use yggdryl_buffer::i256;
///
/// let max = i256::from_i128(i128::MAX);
/// let big = max * i256::from_i128(2); // exceeds i128
/// assert_eq!(big, max + max);
/// assert_eq!(big.to_i128(), None);
/// assert_eq!(i256::from_le_bytes(big.to_le_bytes()), big);
/// ```
pub use arrow_buffer::i256;
