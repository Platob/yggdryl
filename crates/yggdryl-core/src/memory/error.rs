//! [`IoError`] — the failure modes of the byte-I/O traits ([`IOBase`](super::IOBase) /
//! [`IOCursor`](super::IOCursor) / [`IOSlice`](super::IOSlice)) and the [`Heap`](super::Heap)
//! source.

use core::fmt;

use super::Whence;

/// An error raised by a positioned / cursor read-write or a slice.
///
/// The **infallible** primitives ([`pread_byte_array`](super::IOBase::pread_byte_array) /
/// [`pwrite_byte_array`](super::IOBase::pwrite_byte_array)) never produce one — they short-read
/// at the end and grow on write. Errors come only from the operations that carry a hard
/// requirement: a **full** or **typed** read that hit the end, a **seek** before the start, or a
/// **slice** past the end. Every variant names the offending numbers and the fix; in the bindings
/// it surfaces as a Python `ValueError` / a thrown `Error`.
///
/// ```
/// use yggdryl_core::memory::{Heap, IOBase, IoError};
///
/// // A typed read that runs off the end names the shortfall.
/// let data = Heap::from_slice(b"abc");
/// let err = data.pread_i32(0).unwrap_err(); // only 3 bytes, i32 needs 4
/// assert!(matches!(err, IoError::UnexpectedEof { .. }));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IoError {
    /// A read with a hard length requirement — [`pread_exact`](super::IOBase::pread_exact),
    /// [`read_exact`](super::IOCursor::read_exact), or a typed reader
    /// ([`pread_i32`](super::IOBase::pread_i32) / [`pread_byte`](super::IOBase::pread_byte) /
    /// …) — could not be satisfied: fewer bytes remain than were requested. Read fewer bytes,
    /// or extend the data first.
    UnexpectedEof {
        /// The position at which the data ran out.
        offset: u64,
        /// How many bytes the caller asked to fill.
        requested: usize,
        /// How many were actually available from `offset`.
        available: usize,
    },
    /// A [`seek`](super::IOCursor::seek) resolved to a position **before the start** (or past
    /// `u64::MAX`). Seek to a non-negative position; a position past the end is allowed.
    InvalidSeek {
        /// The reference point the offset was measured from.
        whence: Whence,
        /// The signed offset that was applied.
        offset: i64,
        /// The cursor position at the time of the seek.
        position: u64,
        /// The total length at the time of the seek.
        len: u64,
    },
    /// A [`slice`](super::IOSlice::slice) window `[offset, offset+len)` runs past the end.
    /// Request a window that fits within the available length.
    SliceOutOfBounds {
        /// The window's start.
        offset: u64,
        /// The window's requested length.
        len: u64,
        /// The total length the window had to fit inside.
        available: u64,
    },
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof {
                offset,
                requested,
                available,
            } => write!(
                f,
                "unexpected end of data at offset {offset}: asked for {requested} bytes but \
                 only {available} remain; read fewer bytes or extend the data first"
            ),
            Self::InvalidSeek {
                whence,
                offset,
                position,
                len,
            } => write!(
                f,
                "invalid seek: {whence:?} offset {offset} (position {position}, length {len}) \
                 lands before the start; seek to a non-negative position (past the end is fine)"
            ),
            Self::SliceOutOfBounds {
                offset,
                len,
                available,
            } => write!(
                f,
                "slice [{offset}, {end}) runs past the end (length {available}); request a \
                 window that fits within {available}",
                end = *offset + *len
            ),
        }
    }
}

impl std::error::Error for IoError {}
