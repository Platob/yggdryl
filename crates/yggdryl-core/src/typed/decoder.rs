//! [`Decoder`] — reads an element back into a **native Rust value** from any [`IOBase`] source.
//!
//! The exact inverse of [`Encoder`](super::Encoder): it maps an **element index** to the physical
//! position and reads there. The bulk [`decode_slice`](Decoder::decode_slice) forwards to the
//! source's **vectorized** typed array read (`pread_i32_array`, …) into a caller-owned buffer, so a
//! whole column decodes with a single allocation the caller controls.

use super::DataType;
use crate::io::memory::{IOBase, IoError};

/// Decodes elements of this type from an [`IOBase`] back into [`Native`](DataType::Native) values.
pub trait Decoder: DataType {
    /// Reads the element at `index` (an **element** index).
    fn decode<R: IOBase>(src: &R, index: u64) -> Result<Self::Native, IoError>;

    /// Reads the contiguous elements starting at element `start` into `out` — the **bulk** path,
    /// forwarding to the source's vectorized typed array read (fills exactly `out.len()` elements).
    fn decode_slice<R: IOBase>(
        src: &R,
        start: u64,
        out: &mut [Self::Native],
    ) -> Result<(), IoError>;
}
