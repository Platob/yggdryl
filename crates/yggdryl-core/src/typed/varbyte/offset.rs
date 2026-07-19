//! [`VarOffset`] — the **offset element width** of a variable-length column, and [`VarLenType`] —
//! the variable-length marker trait that pins one to its element type.
//!
//! A [`VarSerie`](crate::typed::VarSerie) stores `len + 1` little-endian offsets into its data
//! buffer. The normal [`Binary`](crate::typed::Binary) / [`Utf8`](crate::typed::Utf8) columns use
//! **`i32`** offsets (4 bytes each — Arrow's default); the [`LargeBinary`](crate::typed::LargeBinary)
//! / [`LargeUtf8`](crate::typed::LargeUtf8) columns use **`i64`** offsets (8 bytes — Arrow's
//! `Large*`), for a column whose packed data exceeds the `i32` offset range. `VarOffset` abstracts
//! that width so the carrier is written **once**, generic over it, and `VarLenType::Offset` selects
//! the width per element type — the only difference between a `Binary` and a `LargeBinary` column.

use crate::io::memory::{IOBase, IoError};
use crate::typed::VarType;

/// A variable-length column's **offset element** — an `i32` (4 bytes) or `i64` (8 bytes) little-endian
/// integer indexing into the data buffer. Reads are **total** (a missing/short read is `0`, per the
/// crate's "reads never fail on a missing source" rule); the value is always carried as an `i64`.
pub trait VarOffset: Copy {
    /// The offset element's storage width in bytes (`4` for `i32`, `8` for `i64`).
    const WIDTH: u64;

    /// Reads the offset at byte `pos` of `src`, widened to `i64`. A short read (past the end of a
    /// lazy/empty source) reads **zero**, never an error.
    fn read<R: IOBase>(src: &R, pos: u64) -> i64;

    /// Writes `value` as this offset width at byte `pos` of `dst`, growing the source as needed.
    fn write<W: IOBase>(dst: &mut W, pos: u64, value: i64) -> Result<(), IoError>;
}

impl VarOffset for i32 {
    const WIDTH: u64 = 4;

    fn read<R: IOBase>(src: &R, pos: u64) -> i64 {
        src.pread_i32(pos).unwrap_or(0) as i64
    }

    fn write<W: IOBase>(dst: &mut W, pos: u64, value: i64) -> Result<(), IoError> {
        dst.pwrite_i32(pos, value as i32)
    }
}

impl VarOffset for i64 {
    const WIDTH: u64 = 8;

    fn read<R: IOBase>(src: &R, pos: u64) -> i64 {
        src.pread_i64(pos).unwrap_or(0)
    }

    fn write<W: IOBase>(dst: &mut W, pos: u64, value: i64) -> Result<(), IoError> {
        dst.pwrite_i64(pos, value)
    }
}

/// A **variable-length** byte-blob marker (a [`VarType`] laid out as offsets + data) — it pins the
/// [`VarOffset`] width its [`VarSerie`](crate::typed::VarSerie) uses. The default
/// [`Binary`](crate::typed::Binary) / [`Utf8`](crate::typed::Utf8) markers select `i32`; the
/// [`LargeBinary`](crate::typed::LargeBinary) / [`LargeUtf8`](crate::typed::LargeUtf8) markers select
/// `i64`. The fixed-size [`FixedBinary`](crate::typed::FixedBinary) /
/// [`FixedUtf8`](crate::typed::FixedUtf8) markers are **not** `VarLenType` — they have no offsets.
pub trait VarLenType: VarType {
    /// The offset element width of a [`VarSerie`](crate::typed::VarSerie) over this type.
    type Offset: VarOffset;
}
