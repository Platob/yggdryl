//! The opaque binary data type with 32-bit offsets.

use crate::datatype::macros::unit_data_type;

unit_data_type!(
    /// Variable-size opaque bytes with 32-bit offsets, mapping to Arrow
    /// `BinaryType`.
    ///
    /// ```
    /// use yggdryl_schema::{BinaryType, DataType};
    ///
    /// assert_eq!(BinaryType::from_arrow(&BinaryType.to_arrow()), Ok(BinaryType));
    /// ```
    BinaryType, Binary, "binary"
);
