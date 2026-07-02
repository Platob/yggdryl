//! The 32-bit signed integer data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The 32-bit signed integer type, mapping to Arrow `Int32`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, Int32Type, PrimitiveType};
    ///
    /// assert_eq!(Int32Type::BIT_WIDTH, 32);
    /// assert_eq!(Int32Type::from_arrow(&Int32Type.to_arrow()), Ok(Int32Type));
    /// ```
    Int32Type, i32, 32, Int32, "int32"
);
