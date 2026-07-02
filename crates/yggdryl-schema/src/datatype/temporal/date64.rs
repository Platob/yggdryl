//! The 64-bit date data type.

use crate::datatype::macros::primitive_data_type;
use crate::{Date, Int64, LogicalType, Millisecond};

primitive_data_type!(
    /// A date as milliseconds since the UNIX epoch, mapping to Arrow `Date64`
    /// and anchored on [`Int64`].
    ///
    /// ```
    /// use yggdryl_schema::{DataType, Date64, Int64, LogicalType};
    ///
    /// assert_eq!(Date64.physical(), Int64);
    /// assert_eq!(Date64::from_arrow(&Date64.to_arrow()), Ok(Date64));
    /// ```
    Date64, i64, 64, Date64, "date64"
);

impl LogicalType for Date64 {
    type Physical = Int64;

    fn physical(&self) -> Int64 {
        Int64
    }
}

impl Date for Date64 {
    type Unit = Millisecond;

    fn unit(&self) -> Millisecond {
        Millisecond
    }
}
