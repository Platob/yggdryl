//! The subtrait for data types layered over a physical anchor.

use crate::{DataType, PrimitiveType};

/// A data type carrying semantics over a physical anchor type: the values are
/// stored as the anchor's native representation and reinterpreted
/// ([`Date32Type`](crate::Date32Type) is days since the epoch stored as
/// [`Int32Type`](crate::Int32Type), [`Timestamp`](crate::Timestamp) is an offset
/// stored as [`Int64Type`](crate::Int64Type), …).
///
/// ```
/// use yggdryl_schema::{Date32Type, Int32Type, LogicalType};
///
/// assert_eq!(Date32Type.physical(), Int32Type);
/// ```
pub trait LogicalType: DataType {
    /// The primitive type this logical type anchors on.
    type Physical: PrimitiveType;

    /// The physical anchor of this value's type.
    fn physical(&self) -> Self::Physical;
}
