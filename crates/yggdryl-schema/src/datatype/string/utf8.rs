//! The UTF-8 string data type with 32-bit offsets.

use crate::datatype::macros::unit_data_type;

unit_data_type!(
    /// A variable-size UTF-8 string with 32-bit offsets, mapping to Arrow
    /// `Utf8`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, Utf8};
    ///
    /// assert_eq!(Utf8::from_arrow(&Utf8.to_arrow()), Ok(Utf8));
    /// ```
    Utf8, Utf8, "utf8"
);
