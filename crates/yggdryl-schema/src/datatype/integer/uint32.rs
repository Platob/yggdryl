//! The 32-bit unsigned integer data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The 32-bit unsigned integer type, mapping to Arrow `UInt32`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, PrimitiveType, UInt32Type};
    ///
    /// assert_eq!(UInt32Type::BIT_WIDTH, 32);
    /// assert_eq!(UInt32Type::from_arrow(&UInt32Type.to_arrow()), Ok(UInt32Type));
    /// ```
    UInt32Type, u32, 32, UInt32, "uint32"
);
