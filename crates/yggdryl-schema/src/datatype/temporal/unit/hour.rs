//! The hour time unit.

use crate::datatype::temporal::unit::macros::time_unit;

time_unit!(
    /// Sixty minutes, anchoring on a physical type plus `ygg.*` metadata.
    ///
    /// ```
    /// use yggdryl_schema::{TimeUnit, TimeUnitId, Hour};
    ///
    /// assert_eq!(Hour.unit_id(), TimeUnitId::Hour);
    /// assert_eq!(Hour::from_unit_id(TimeUnitId::Hour), Ok(Hour));
    /// ```
    Hour
);
