//! The boolean data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The boolean type, mapping to Arrow `Boolean`. Values are bit-packed,
    /// so the storage width is a single bit.
    ///
    /// ```
    /// use yggdryl_schema::{Boolean, DataType, PrimitiveType};
    ///
    /// assert_eq!(Boolean::BIT_WIDTH, 1);
    /// assert_eq!(Boolean::from_arrow(&Boolean.to_arrow()), Ok(Boolean));
    /// ```
    Boolean, bool, 1, Boolean, "boolean"
);
