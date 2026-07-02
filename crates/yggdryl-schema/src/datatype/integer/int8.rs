//! The 8-bit signed integer data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The 8-bit signed integer type, mapping to Arrow `Int8`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, Int8Type, PrimitiveType};
    ///
    /// assert_eq!(Int8Type::BIT_WIDTH, 8);
    /// assert_eq!(Int8Type::from_arrow(&Int8Type.to_arrow()), Ok(Int8Type));
    /// ```
    Int8Type, i8, 8, Int8, "int8"
);
