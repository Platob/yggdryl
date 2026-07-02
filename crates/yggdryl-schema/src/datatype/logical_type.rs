//! The subtrait for data types layered over a physical anchor.

use crate::{DataType, PrimitiveType};

/// A data type carrying semantics over a physical anchor type: the values are
/// stored as the anchor's native representation and reinterpreted
/// ([`Date32`](crate::Date32) is days since the epoch stored as
/// [`Int32`](crate::Int32), [`Timestamp`](crate::Timestamp) is an offset
/// stored as [`Int64`](crate::Int64), …).
///
/// ```
/// use yggdryl_schema::{Date32, Int32, LogicalType};
///
/// assert_eq!(Date32.physical(), Int32);
/// ```
pub trait LogicalType: DataType {
    /// The primitive type this logical type anchors on.
    type Physical: PrimitiveType;

    /// The physical anchor of this value's type.
    fn physical(&self) -> Self::Physical;
}
