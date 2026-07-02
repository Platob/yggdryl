//! The 8-bit unsigned integer data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The 8-bit unsigned integer type, mapping to Arrow `UInt8`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, PrimitiveType, UInt8Type};
    ///
    /// assert_eq!(UInt8Type::BIT_WIDTH, 8);
    /// assert_eq!(UInt8Type::from_arrow(&UInt8Type.to_arrow()), Ok(UInt8Type));
    /// ```
    UInt8Type, u8, 8, UInt8, "uint8"
);
