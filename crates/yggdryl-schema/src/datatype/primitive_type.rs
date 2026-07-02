//! The subtrait for fixed-width primitive data types.

use core::fmt::Debug;

use crate::DataType;

/// A fixed-width data type whose values are single Rust primitives.
///
/// Compute kernels are written once against `Native`; the bit width is the
/// storage width of one value ([`Boolean`](crate::Boolean) is bit-packed, so
/// its width is 1).
///
/// ```
/// use yggdryl_schema::{Int32, PrimitiveType};
///
/// assert_eq!(Int32::BIT_WIDTH, 32);
/// let native: <Int32 as PrimitiveType>::Native = 7i32;
/// # let _ = native;
/// ```
pub trait PrimitiveType: DataType {
    /// The Rust value type holding one element of this data type.
    type Native: Copy + Debug + PartialEq + Send + Sync + 'static;

    /// The storage width of one value, in bits.
    const BIT_WIDTH: usize;
}
