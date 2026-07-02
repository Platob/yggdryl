//! The 32-bit date data type.

use crate::datatype::macros::primitive_data_type;
use crate::{Int32, LogicalType};

primitive_data_type!(
    /// A date as days since the UNIX epoch, mapping to Arrow `Date32` and
    /// anchored on [`Int32`].
    ///
    /// ```
    /// use yggdryl_schema::{DataType, Date32, Int32, LogicalType};
    ///
    /// assert_eq!(Date32.physical(), Int32);
    /// assert_eq!(Date32::from_arrow(&Date32.to_arrow()), Ok(Date32));
    /// ```
    Date32, i32, 32, Date32, "date32"
);

impl LogicalType for Date32 {
    type Physical = Int32;

    fn physical(&self) -> Int32 {
        Int32
    }
}
