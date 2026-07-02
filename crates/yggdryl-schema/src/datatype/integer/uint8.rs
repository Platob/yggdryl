//! The 8-bit unsigned integer data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The 8-bit unsigned integer type, mapping to Arrow `UInt8`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, PrimitiveType, UInt8};
    ///
    /// assert_eq!(UInt8::BIT_WIDTH, 8);
    /// assert_eq!(UInt8::from_arrow(&UInt8.to_arrow()), Ok(UInt8));
    /// ```
    UInt8, u8, 8, UInt8, "uint8"
);
