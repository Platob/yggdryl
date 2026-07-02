//! The UTF-8 string data type with 64-bit offsets.

use crate::datatype::macros::unit_data_type;

unit_data_type!(
    /// A variable-size UTF-8 string with 64-bit offsets, mapping to Arrow
    /// `LargeUtf8Type`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, LargeUtf8Type};
    ///
    /// assert_eq!(LargeUtf8Type::from_arrow(&LargeUtf8Type.to_arrow()), Ok(LargeUtf8Type));
    /// ```
    LargeUtf8Type, LargeUtf8, "large_utf8"
);
