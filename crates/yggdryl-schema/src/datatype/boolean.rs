//! The boolean data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The boolean type, mapping to Arrow `Boolean`. Values are bit-packed,
    /// so the storage width is a single bit.
    ///
    /// ```
    /// use yggdryl_schema::{BooleanType, DataType, PrimitiveType};
    ///
    /// assert_eq!(BooleanType::BIT_WIDTH, 1);
    /// assert_eq!(BooleanType::from_arrow(&BooleanType.to_arrow()), Ok(BooleanType));
    /// ```
    BooleanType, bool, 1, Boolean, "boolean"
);
