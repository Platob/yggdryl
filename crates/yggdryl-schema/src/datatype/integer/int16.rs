//! The 16-bit signed integer data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The 16-bit signed integer type, mapping to Arrow `Int16`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, Int16, PrimitiveType};
    ///
    /// assert_eq!(Int16::BIT_WIDTH, 16);
    /// assert_eq!(Int16::from_arrow(&Int16.to_arrow()), Ok(Int16));
    /// ```
    Int16, i16, 16, Int16, "int16"
);
