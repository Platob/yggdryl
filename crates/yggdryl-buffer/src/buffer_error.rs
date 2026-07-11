//! [`BufferError`] — the error type for the typed buffers.

use core::fmt;

/// The error raised when bytes cannot be decoded into a typed buffer.
///
/// Every variant names the fix (see `CLAUDE.md` rule 12): the expected width or
/// length alongside the offending value, so a caller knows exactly what to pass.
///
/// ```
/// use yggdryl_buffer::{BufferError, I32Buffer};
///
/// // 6 is not a multiple of 4 (the width of `i32`).
/// assert!(matches!(
///     I32Buffer::deserialize_bytes(&[0; 6]),
///     Err(BufferError::InvalidByteLength { len: 6, width: 4, .. })
/// ));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BufferError {
    /// The byte length is not a whole number of elements of the target type.
    InvalidByteLength {
        /// The byte length that was supplied.
        len: usize,
        /// The width, in bytes, of one element.
        width: usize,
        /// The name of the element type (`"i32"`, `"f64"`, …).
        ty: &'static str,
    },
    /// A boolean buffer's packed bytes don't match its declared bit length.
    InvalidBitLength {
        /// The number of packed bytes supplied.
        bytes: usize,
        /// The number of packed bytes the bit length requires (`ceil(len / 8)`).
        expected: usize,
        /// The declared number of bits.
        len: usize,
    },
    /// Fewer bytes than the serialized header requires.
    Truncated {
        /// The minimum number of bytes required.
        needed: usize,
        /// The number of bytes actually supplied.
        available: usize,
    },
}

impl fmt::Display for BufferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidByteLength { len, width, ty } => write!(
                f,
                "cannot decode a `{ty}` buffer from {len} bytes: the length must be a \
                 multiple of {width} (the width of `{ty}`); pass {width}·n bytes"
            ),
            Self::InvalidBitLength {
                bytes,
                expected,
                len,
            } => write!(
                f,
                "a boolean buffer of {len} bits needs {expected} packed byte(s), but got \
                 {bytes}; pass ceil(len / 8) bytes"
            ),
            Self::Truncated { needed, available } => write!(
                f,
                "truncated buffer: need at least {needed} bytes for the length header, got \
                 {available}"
            ),
        }
    }
}

impl std::error::Error for BufferError {}
