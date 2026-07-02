//! The 32-bit signed integer data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The 32-bit signed integer type, mapping to Arrow `Int32`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, Int32, PrimitiveType};
    ///
    /// assert_eq!(Int32::BIT_WIDTH, 32);
    /// assert_eq!(Int32::from_arrow(&Int32.to_arrow()), Ok(Int32));
    /// ```
    Int32, i32, 32, Int32, "int32"
);
