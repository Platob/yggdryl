//! The millisecond time unit.

use crate::datatype::temporal::unit::macros::time_unit;

time_unit!(
    /// One thousandth of a second, mapping natively to Arrow.
    ///
    /// ```
    /// use yggdryl_schema::{TimeUnit, TimeUnitId, Millisecond};
    ///
    /// assert_eq!(Millisecond.unit_id(), TimeUnitId::Millisecond);
    /// assert_eq!(Millisecond::from_unit_id(TimeUnitId::Millisecond), Ok(Millisecond));
    /// ```
    Millisecond
);
