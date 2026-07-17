//! [`IoError`] — the failure modes of the byte-I/O traits ([`IOBase`](crate::io::memory::IOBase) /
//! [`IOCursor`](crate::io::memory::IOCursor) / [`IOSlice`](crate::io::memory::IOSlice)) and the [`Heap`](crate::io::memory::Heap)
//! source.

use core::fmt;

use super::Whence;

/// An error raised by a positioned / cursor read-write or a slice.
///
/// The **infallible** primitives ([`pread_byte_array`](crate::io::memory::IOBase::pread_byte_array) /
/// [`pwrite_byte_array`](crate::io::memory::IOBase::pwrite_byte_array)) never produce one — they short-read
/// at the end and grow on write. Errors come only from the operations that carry a hard
/// requirement: a **full** or **typed** read that hit the end, a **seek** before the start, or a
/// **slice** past the end. Every variant names the offending numbers and the fix; in the bindings
/// it surfaces as a Python `ValueError` / a thrown `Error`.
///
/// ```
/// use yggdryl_core::io::memory::{Heap, IOBase, IoError};
///
/// // A typed read that runs off the end names the shortfall.
/// let data = Heap::from_slice(b"abc");
/// let err = data.pread_i32(0).unwrap_err(); // only 3 bytes, i32 needs 4
/// assert!(matches!(err, IoError::UnexpectedEof { .. }));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IoError {
    /// A read with a hard length requirement — [`pread_exact`](crate::io::memory::IOBase::pread_exact),
    /// [`read_exact`](crate::io::memory::IOCursor::read_exact), or a typed reader
    /// ([`pread_i32`](crate::io::memory::IOBase::pread_i32) / [`pread_byte`](crate::io::memory::IOBase::pread_byte) /
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
    /// A [`seek`](crate::io::memory::IOCursor::seek) resolved to a position **before the start** (or past
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
    /// A [`window`](crate::io::memory::IOBase::window) window `[offset, offset+len)` runs past the end.
    /// Request a window that fits within the available length.
    SliceOutOfBounds {
        /// The window's start.
        offset: u64,
        /// The window's requested length.
        len: u64,
        /// The total length the window had to fit inside.
        available: u64,
    },
    /// Bytes read as UTF-8 text (a `pread_utf8` / `read_utf8`) are not valid UTF-8. Read the
    /// raw bytes with `pread_byte_array` instead, or fix the offset/length to span whole
    /// characters.
    InvalidUtf8 {
        /// The byte index (within the read range) at which decoding failed.
        position: usize,
    },
    /// A name/value handed to a parser ([`IOMode::parse_str`](crate::io::IOMode::parse_str) /
    /// [`IOKind::from_u8`](crate::io::IOKind::from_u8), …) matched none of the accepted
    /// tokens. Pass one of the expected values.
    UnknownName {
        /// Which type was being parsed (`"IOMode"`, `"IOKind"`, …).
        kind: &'static str,
        /// The offending input.
        input: String,
        /// The accepted tokens, listed.
        expected: &'static str,
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
            Self::InvalidUtf8 { position } => write!(
                f,
                "invalid UTF-8 at byte {position}: read the raw bytes with pread_byte_array \
                 instead, or adjust the offset/length to span whole characters"
            ),
            Self::UnknownName {
                kind,
                input,
                expected,
            } => write!(f, "unknown {kind} {input:?}: expected one of {expected}"),
        }
    }
}

impl std::error::Error for IoError {}
