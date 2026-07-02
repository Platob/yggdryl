//! The 64-bit unsigned integer data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The 64-bit unsigned integer type, mapping to Arrow `UInt64`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, PrimitiveType, UInt64};
    ///
    /// assert_eq!(UInt64::BIT_WIDTH, 64);
    /// assert_eq!(UInt64::from_arrow(&UInt64.to_arrow()), Ok(UInt64));
    /// ```
    UInt64, u64, 64, UInt64, "uint64"
);
