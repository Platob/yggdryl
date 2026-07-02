//! The 64-bit signed integer data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The 64-bit signed integer type, mapping to Arrow `Int64`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, Int64, PrimitiveType};
    ///
    /// assert_eq!(Int64::BIT_WIDTH, 64);
    /// assert_eq!(Int64::from_arrow(&Int64.to_arrow()), Ok(Int64));
    /// ```
    Int64, i64, 64, Int64, "int64"
);
