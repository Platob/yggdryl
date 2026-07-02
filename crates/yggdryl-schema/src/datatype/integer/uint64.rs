//! The 64-bit unsigned integer data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The 64-bit unsigned integer type, mapping to Arrow `UInt64`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, PrimitiveType, UInt64Type};
    ///
    /// assert_eq!(UInt64Type::BIT_WIDTH, 64);
    /// assert_eq!(UInt64Type::from_arrow(&UInt64Type.to_arrow()), Ok(UInt64Type));
    /// ```
    UInt64Type, u64, 64, UInt64, "uint64"
);
