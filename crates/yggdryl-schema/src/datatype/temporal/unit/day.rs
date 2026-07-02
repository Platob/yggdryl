//! The day time unit.

use crate::datatype::temporal::unit::macros::time_unit;

time_unit!(
    /// Twenty-four hours, anchoring on a physical type plus `ygg.*` metadata.
    ///
    /// ```
    /// use yggdryl_schema::{TimeUnit, TimeUnitId, Day};
    ///
    /// assert_eq!(Day.unit_id(), TimeUnitId::Day);
    /// assert_eq!(Day::from_unit_id(TimeUnitId::Day), Ok(Day));
    /// ```
    Day
);
