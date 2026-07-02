//! The 64-bit date data type.

use crate::datatype::macros::primitive_data_type;
use crate::{Date, Int64Type, LogicalType, Millisecond, TemporalType};

primitive_data_type!(
    /// A date as milliseconds since the UNIX epoch, mapping to Arrow `Date64`
    /// and anchored on [`Int64Type`].
    ///
    /// ```
    /// use yggdryl_schema::{DataType, Date64Type, Int64Type, LogicalType};
    ///
    /// assert_eq!(Date64Type.physical(), Int64Type);
    /// assert_eq!(Date64Type::from_arrow(&Date64Type.to_arrow()), Ok(Date64Type));
    /// ```
    Date64Type, i64, 64, Date64, "date64"
);

impl LogicalType for Date64Type {
    type Physical = Int64Type;

    fn physical(&self) -> Int64Type {
        Int64Type
    }
}

impl TemporalType for Date64Type {
    type Unit = Millisecond;

    fn unit(&self) -> Millisecond {
        Millisecond
    }
}

impl Date for Date64Type {}
