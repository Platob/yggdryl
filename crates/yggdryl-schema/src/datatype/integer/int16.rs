//! The 16-bit signed integer data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The 16-bit signed integer type, mapping to Arrow `Int16`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, Int16Type, PrimitiveType};
    ///
    /// assert_eq!(Int16Type::BIT_WIDTH, 16);
    /// assert_eq!(Int16Type::from_arrow(&Int16Type.to_arrow()), Ok(Int16Type));
    /// ```
    Int16Type, i16, 16, Int16, "int16"
);
