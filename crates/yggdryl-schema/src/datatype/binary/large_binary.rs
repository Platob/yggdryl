//! The opaque binary data type with 64-bit offsets.

use crate::datatype::macros::unit_data_type;

unit_data_type!(
    /// Variable-size opaque bytes with 64-bit offsets, mapping to Arrow
    /// `LargeBinary`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, LargeBinary};
    ///
    /// assert_eq!(LargeBinary::from_arrow(&LargeBinary.to_arrow()), Ok(LargeBinary));
    /// ```
    LargeBinary, LargeBinary, "large_binary"
);
