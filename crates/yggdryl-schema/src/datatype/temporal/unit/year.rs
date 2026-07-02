//! The year time unit.

use crate::datatype::temporal::unit::macros::time_unit;

time_unit!(
    /// One calendar year — not a fixed span of time, anchoring on a physical type plus `ygg.*` metadata.
    ///
    /// ```
    /// use yggdryl_schema::{TimeUnit, TimeUnitId, Year};
    ///
    /// assert_eq!(Year.unit_id(), TimeUnitId::Year);
    /// assert_eq!(Year::from_unit_id(TimeUnitId::Year), Ok(Year));
    /// ```
    Year
);
