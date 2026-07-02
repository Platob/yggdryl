//! The 8-bit signed integer data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The 8-bit signed integer type, mapping to Arrow `Int8`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, Int8, PrimitiveType};
    ///
    /// assert_eq!(Int8::BIT_WIDTH, 8);
    /// assert_eq!(Int8::from_arrow(&Int8.to_arrow()), Ok(Int8));
    /// ```
    Int8, i8, 8, Int8, "int8"
);
