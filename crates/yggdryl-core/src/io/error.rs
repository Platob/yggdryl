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
    /// A **checked** capacity reservation ([`try_reserve`](crate::io::memory::IOBase::try_reserve) /
    /// [`try_reserve_exact`](crate::io::memory::IOBase::try_reserve_exact)) could not be
    /// satisfied — the new size would overflow, or the allocator refused the request. Reserve
    /// less, shrink the source, or free memory first. (The unchecked `reserve` aborts the
    /// process in this situation; the `try_*` twins return this error instead.)
    CapacityOverflow {
        /// How many additional bytes were requested.
        additional: u64,
        /// The capacity at the time of the request.
        capacity: u64,
    },
    /// A file-backed operation (open / map / grow / flush on an
    /// [`Mmap`](crate::io::local::Mmap)) failed at the OS level. The message names the
    /// operation, the path, and the OS detail — check that the path exists, is accessible,
    /// and the disk has room.
    FileIo {
        /// The operation that failed (`"open"`, `"map"`, `"grow"`, `"flush"`, `"write"`).
        op: &'static str,
        /// The file path involved.
        path: String,
        /// The OS-level error detail.
        detail: String,
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
    /// A [`Compression`](crate::compression::Compression) codec operation failed, or no codec is
    /// available (the media type is not a compression, or the `compression` cargo feature is
    /// off). The message names the codec, the operation, and the fix.
    Compression {
        /// The codec's mime essence (`"application/gzip"`), or `"?"` when none resolved.
        codec: String,
        /// The operation (`"compress"`, `"decompress"`, `"resolve"`).
        op: &'static str,
        /// The underlying detail (the codec error, or why no codec was available).
        detail: String,
    },
    /// A typed [`cast_field`](crate::typed::Serie) on a compile-time-typed column
    /// ([`FixedSerie`](crate::typed::FixedSerie)) or scalar ([`FixedScalar`](crate::typed::FixedScalar))
    /// was asked to do something the typed layer cannot express: **change the element type** (which
    /// belongs to the erased `Serie.cast_field` / [`resize_dtype`](crate::io::memory::IOBase::resize_dtype)),
    /// or **drop nulls** a non-nullable target forbids. The `detail` is a self-contained guided
    /// message — it names the offending value and the concrete next step, so it reads well as a
    /// Python `ValueError` / a thrown Node `Error` with no code around it.
    TypedCast {
        /// The complete, guided message (the offending value + the fix).
        detail: String,
    },
    /// A string handed to a flexible/typed value parser
    /// ([`FlexibleFromStr::parse_flexible`](crate::typed::FlexibleFromStr) /
    /// [`parse_exact`](crate::typed::FlexibleFromStr), or the
    /// [`Encoder::encode_str`](crate::typed::Encoder) family) could not be read as the target
    /// numeric / boolean type. The message names the offending string, the target type, and the
    /// accepted forms + the fix.
    ParseError {
        /// The target type name (`"i64"`, `"f64"`, `"bool"`, …).
        kind: &'static str,
        /// The offending input string.
        input: String,
        /// The accepted forms and the fix hint.
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
            Self::CapacityOverflow {
                additional,
                capacity,
            } => write!(
                f,
                "cannot reserve {additional} more bytes (current capacity {capacity}): the                  size overflows or the allocator refused; reserve less, shrink the source, or                  free memory first"
            ),
            Self::FileIo { op, path, detail } => write!(
                f,
                "cannot {op} {path:?}: {detail}; check that the path exists, is accessible,                  and the disk has room"
            ),
            Self::UnknownName {
                kind,
                input,
                expected,
            } => write!(f, "unknown {kind} {input:?}: expected one of {expected}"),
            Self::Compression { codec, op, detail } => write!(
                f,
                "cannot {op} with codec {codec:?}: {detail}"
            ),
            // The detail is already a complete, guided sentence — render it verbatim.
            Self::TypedCast { detail } => f.write_str(detail),
            Self::ParseError {
                kind,
                input,
                expected,
            } => write!(f, "cannot parse {input:?} as {kind}: {expected}"),
        }
    }
}

impl std::error::Error for IoError {}
