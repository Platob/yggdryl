//! The month time unit.

use crate::datatype::temporal::unit::macros::time_unit;

time_unit!(
    /// One calendar month — not a fixed span of time, anchoring on a physical type plus `ygg.*` metadata.
    ///
    /// ```
    /// use yggdryl_schema::{TimeUnit, TimeUnitId, Month};
    ///
    /// assert_eq!(Month.unit_id(), TimeUnitId::Month);
    /// assert_eq!(Month::from_unit_id(TimeUnitId::Month), Ok(Month));
    /// ```
    Month
);
