//! The 16-bit unsigned integer data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The 16-bit unsigned integer type, mapping to Arrow `UInt16`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, PrimitiveType, UInt16};
    ///
    /// assert_eq!(UInt16::BIT_WIDTH, 16);
    /// assert_eq!(UInt16::from_arrow(&UInt16.to_arrow()), Ok(UInt16));
    /// ```
    UInt16, u16, 16, UInt16, "uint16"
);
