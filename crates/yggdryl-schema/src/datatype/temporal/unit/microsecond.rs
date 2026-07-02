//! The microsecond time unit.

use crate::datatype::temporal::unit::macros::time_unit;

time_unit!(
    /// One millionth of a second, mapping natively to Arrow.
    ///
    /// ```
    /// use yggdryl_schema::{TimeUnit, TimeUnitId, Microsecond};
    ///
    /// assert_eq!(Microsecond.unit_id(), TimeUnitId::Microsecond);
    /// assert_eq!(Microsecond::from_unit_id(TimeUnitId::Microsecond), Ok(Microsecond));
    /// ```
    Microsecond
);
