//! The 32-bit date data type.

use crate::datatype::macros::primitive_data_type;
use crate::{Date, Day, Int32Type, LogicalType, TemporalType};

primitive_data_type!(
    /// A date as days since the UNIX epoch, mapping to Arrow `Date32` and
    /// anchored on [`Int32Type`].
    ///
    /// ```
    /// use yggdryl_schema::{DataType, Date32Type, Int32Type, LogicalType};
    ///
    /// assert_eq!(Date32Type.physical(), Int32Type);
    /// assert_eq!(Date32Type::from_arrow(&Date32Type.to_arrow()), Ok(Date32Type));
    /// ```
    Date32Type, i32, 32, Date32, "date32"
);

impl LogicalType for Date32Type {
    type Physical = Int32Type;

    fn physical(&self) -> Int32Type {
        Int32Type
    }
}

impl TemporalType for Date32Type {
    type Unit = Day;

    fn unit(&self) -> Day {
        Day
    }
}

impl Date for Date32Type {}
