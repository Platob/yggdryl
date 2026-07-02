//! The opaque binary data type with 64-bit offsets.

use crate::datatype::macros::unit_data_type;

unit_data_type!(
    /// Variable-size opaque bytes with 64-bit offsets, mapping to Arrow
    /// `LargeBinaryType`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, LargeBinaryType};
    ///
    /// assert_eq!(LargeBinaryType::from_arrow(&LargeBinaryType.to_arrow()), Ok(LargeBinaryType));
    /// ```
    LargeBinaryType, LargeBinary, "large_binary"
);
