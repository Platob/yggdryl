//! [`IoError`] — the failure modes of the byte-I/O traits ([`IOBase`](super::IOBase) /
//! [`IOCursor`](super::IOCursor) / [`IOSlice`](super::IOSlice)) and [`Bytes`](super::Bytes).

use core::fmt;

use super::Whence;

/// An error raised by a positioned / cursor read-write or a slice.
///
/// The **infallible** primitives ([`pread`](super::IOBase::pread) /
/// [`pwrite`](super::IOBase::pwrite)) never produce one — they short-read at the end and
/// grow on write. Errors come only from the operations that carry a hard requirement: a
/// **full** read that hit the end, a **seek** before the start, or a **slice** past the end.
/// Every variant names the offending numbers and the fix; in the bindings it surfaces as a
/// Python `ValueError` / a thrown `Error`.
///
/// ```
/// use yggdryl_core::io::{Bytes, IOBase, IoError};
///
/// // Asking for more bytes than remain names the shortfall.
/// let data = Bytes::from_slice(b"abc");
/// let err = data.pread_exact(1, &mut [0u8; 5]).unwrap_err();
/// assert!(matches!(err, IoError::UnexpectedEof { .. }));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IoError {
    /// A **full** read ([`pread_exact`](super::IOBase::pread_exact) /
    /// [`read_exact`](super::IOCursor::read_exact)) could not be satisfied — fewer bytes
    /// remain than were requested. Read fewer bytes, or extend the data first.
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
    /// A serialized header declared an element count so large that its byte length overflows
    /// `usize` — the data is corrupt or truncated. Nothing this size could be read, so the
    /// decode is refused rather than attempting a runaway allocation.
    CorruptLength {
        /// The declared element count.
        len: u64,
        /// The per-element byte width.
        width: usize,
    },
    /// A [`slice`](super::IOSlice::slice) of a **typed** buffer used a byte `offset` / `len`
    /// that is not a multiple of the element width, so the window would not start and span
    /// whole elements. Align both to a multiple of `width` (byte buffers, `width == 1`, never
    /// hit this).
    SliceMisaligned {
        /// The window's byte start.
        offset: u64,
        /// The window's byte length.
        len: u64,
        /// The element byte width the window must align to.
        width: usize,
    },
    /// Bytes assigned to a UTF-8 (`utf8`) value are not valid UTF-8. Store a `&str` (always
    /// valid), or use the binary type for arbitrary bytes.
    InvalidUtf8 {
        /// The byte index at which decoding failed.
        position: usize,
    },
    /// A deserialized variable-length column's **offsets** are corrupt — an offset is negative,
    /// out of order, or runs past the data buffer, so an element would index out of bounds. The
    /// serialized bytes are corrupt or truncated; re-read from a trusted, intact source.
    CorruptOffsets {
        /// The offending offset value.
        offset: i64,
        /// The length of the data buffer the offset had to fit inside.
        data_len: usize,
    },
    /// An in-place `set` / `set_range` addressed an element (or a range) outside the column. A
    /// `set` overwrites an **existing** element — grow the column with `push` first, or index
    /// within `[0, len)` (a bulk range within `[start, start + count) ⊆ [0, len)`).
    IndexOutOfBounds {
        /// The offending element index (a bulk op reports the first out-of-range index).
        index: usize,
        /// The column length the index had to fall inside.
        len: usize,
    },
    /// An operation was handed a value this crate does not model — e.g. importing an Arrow array
    /// whose type has no yggdryl mapping, or assembling a struct column from mismatched parts. The
    /// message names what was unsupported and, where relevant, the modeled alternative.
    Unsupported {
        /// A guided description of the unsupported value and the fix.
        what: String,
    },
    /// A deserialized schema or nested-column frame recurses past the maximum nesting depth — a
    /// hostile or corrupt input engineered to overflow the stack. The decode is refused before it
    /// can recurse that deep. Flatten the structure, or re-read from a trusted, intact source.
    NestingTooDeep {
        /// The maximum nesting depth the decoder allows.
        max: usize,
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
            Self::CorruptLength { len, width } => write!(
                f,
                "corrupt serialized length: {len} elements × {width} bytes overflows the \
                 address space; the data is corrupt or truncated"
            ),
            Self::SliceMisaligned { offset, len, width } => write!(
                f,
                "misaligned typed slice: byte offset {offset} / length {len} are not both \
                 multiples of the {width}-byte element width; align them to a multiple of {width}"
            ),
            Self::InvalidUtf8 { position } => write!(
                f,
                "invalid UTF-8 at byte {position}: store a `&str` (always valid UTF-8) or use \
                 the binary type for arbitrary bytes"
            ),
            Self::CorruptOffsets { offset, data_len } => write!(
                f,
                "corrupt variable-length offsets: offset {offset} is negative, out of order, or \
                 past the {data_len}-byte data buffer; the serialized column is corrupt or \
                 truncated — re-read from an intact source"
            ),
            Self::IndexOutOfBounds { index, len } => write!(
                f,
                "index {index} is out of bounds for a column of length {len}: `set` overwrites an \
                 existing element — `push` to grow the column, or index within [0, {len})"
            ),
            Self::Unsupported { what } => write!(f, "{what}"),
            Self::NestingTooDeep { max } => write!(
                f,
                "nesting too deep: the serialized schema/frame recurses past the maximum of {max} \
                 levels; the input is corrupt or hostile — flatten the structure or re-read from a \
                 trusted, intact source"
            ),
        }
    }
}

impl std::error::Error for IoError {}
