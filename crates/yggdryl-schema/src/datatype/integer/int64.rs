//! The 64-bit signed integer data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The 64-bit signed integer type, mapping to Arrow `Int64`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, Int64Type, PrimitiveType};
    ///
    /// assert_eq!(Int64Type::BIT_WIDTH, 64);
    /// assert_eq!(Int64Type::from_arrow(&Int64Type.to_arrow()), Ok(Int64Type));
    /// ```
    Int64Type, i64, 64, Int64, "int64"
);
