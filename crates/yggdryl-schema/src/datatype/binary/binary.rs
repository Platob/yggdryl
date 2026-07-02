//! The opaque binary data type with 32-bit offsets.

use crate::datatype::macros::unit_data_type;

unit_data_type!(
    /// Variable-size opaque bytes with 32-bit offsets, mapping to Arrow
    /// `Binary`.
    ///
    /// ```
    /// use yggdryl_schema::{Binary, DataType};
    ///
    /// assert_eq!(Binary::from_arrow(&Binary.to_arrow()), Ok(Binary));
    /// ```
    Binary, Binary, "binary"
);
