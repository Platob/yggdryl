//! The UTF-8 string data type with 32-bit offsets.

use crate::datatype::macros::unit_data_type;

unit_data_type!(
    /// A variable-size UTF-8 string with 32-bit offsets, mapping to Arrow
    /// `Utf8Type`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, Utf8Type};
    ///
    /// assert_eq!(Utf8Type::from_arrow(&Utf8Type.to_arrow()), Ok(Utf8Type));
    /// ```
    Utf8Type, Utf8, "utf8"
);
