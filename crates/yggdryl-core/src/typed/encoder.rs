//! [`Encoder`] — writes a **native Rust value** as an element into any [`IOBase`] source.
//!
//! The encoder is stateless: it maps an **element index** (not a byte offset) to the physical
//! position and writes there, so the caller thinks in elements and the type owns its bit/byte
//! stride. The bulk [`encode_slice`](Encoder::encode_slice) forwards to the source's **vectorized**
//! typed array write (`pwrite_i32_array`, …), so a whole column encodes in one dense pass.

use super::DataType;
use crate::io::memory::{IOBase, IoError};

/// Encodes [`Native`](DataType::Native) values as elements of this type into an [`IOBase`].
pub trait Encoder: DataType {
    /// Writes `value` as the element at `index` (an **element** index; the impl maps it to the
    /// physical bit/byte position).
    fn encode<W: IOBase>(dst: &mut W, index: u64, value: Self::Native) -> Result<(), IoError>;

    /// Writes `values` as the contiguous elements starting at element `start` — the **bulk** path,
    /// forwarding to the source's vectorized typed array write.
    fn encode_slice<W: IOBase>(
        dst: &mut W,
        start: u64,
        values: &[Self::Native],
    ) -> Result<(), IoError>;
}
