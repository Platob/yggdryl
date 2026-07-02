//! The nanosecond time unit.

use crate::datatype::temporal::unit::macros::time_unit;

time_unit!(
    /// One billionth of a second, the finest resolution, mapping natively to Arrow.
    ///
    /// ```
    /// use yggdryl_schema::{TimeUnit, TimeUnitId, Nanosecond};
    ///
    /// assert_eq!(Nanosecond.unit_id(), TimeUnitId::Nanosecond);
    /// assert_eq!(Nanosecond::from_unit_id(TimeUnitId::Nanosecond), Ok(Nanosecond));
    /// ```
    Nanosecond
);
