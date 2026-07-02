//! The 16-bit unsigned integer data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The 16-bit unsigned integer type, mapping to Arrow `UInt16`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, PrimitiveType, UInt16Type};
    ///
    /// assert_eq!(UInt16Type::BIT_WIDTH, 16);
    /// assert_eq!(UInt16Type::from_arrow(&UInt16Type.to_arrow()), Ok(UInt16Type));
    /// ```
    UInt16Type, u16, 16, UInt16, "uint16"
);
