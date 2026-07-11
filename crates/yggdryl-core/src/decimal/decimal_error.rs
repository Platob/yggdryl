//! [`DecimalError`] — the error type for decimal construction and byte decoding.

use core::fmt;

/// An error building or decoding a fixed-width decimal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecimalError {
    /// The serialized bytes were not the expected length (mantissa width + 1 scale byte).
    InvalidByteLength {
        /// The number of bytes given.
        len: usize,
        /// The number of bytes expected (mantissa width + 1).
        expected: usize,
    },
    /// Rescaling or converting overflowed the target mantissa width.
    Overflow {
        /// The width (bits) that overflowed.
        bits: u32,
    },
}

impl fmt::Display for DecimalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidByteLength { len, expected } => write!(
                f,
                "invalid decimal byte length: expected {expected} (mantissa + scale byte), got {len}"
            ),
            Self::Overflow { bits } => write!(
                f,
                "decimal value overflows the {bits}-bit mantissa; use a wider decimal (e.g. Decimal256)"
            ),
        }
    }
}

impl std::error::Error for DecimalError {}
