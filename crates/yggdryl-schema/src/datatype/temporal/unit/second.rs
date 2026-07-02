//! The second time unit.

use crate::datatype::temporal::unit::macros::time_unit;

time_unit!(
    /// One second, mapping natively to Arrow.
    ///
    /// ```
    /// use yggdryl_schema::{TimeUnit, TimeUnitId, Second};
    ///
    /// assert_eq!(Second.unit_id(), TimeUnitId::Second);
    /// assert_eq!(Second::from_unit_id(TimeUnitId::Second), Ok(Second));
    /// ```
    Second
);
