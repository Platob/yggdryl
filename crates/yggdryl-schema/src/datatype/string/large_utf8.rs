//! The UTF-8 string data type with 64-bit offsets.

use crate::datatype::macros::unit_data_type;

unit_data_type!(
    /// A variable-size UTF-8 string with 64-bit offsets, mapping to Arrow
    /// `LargeUtf8`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, LargeUtf8};
    ///
    /// assert_eq!(LargeUtf8::from_arrow(&LargeUtf8.to_arrow()), Ok(LargeUtf8));
    /// ```
    LargeUtf8, LargeUtf8, "large_utf8"
);
