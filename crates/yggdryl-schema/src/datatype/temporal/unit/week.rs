//! The week time unit.

use crate::datatype::temporal::unit::macros::time_unit;

time_unit!(
    /// Seven days, anchoring on a physical type plus `ygg.*` metadata.
    ///
    /// ```
    /// use yggdryl_schema::{TimeUnit, TimeUnitId, Week};
    ///
    /// assert_eq!(Week.unit_id(), TimeUnitId::Week);
    /// assert_eq!(Week::from_unit_id(TimeUnitId::Week), Ok(Week));
    /// ```
    Week
);
