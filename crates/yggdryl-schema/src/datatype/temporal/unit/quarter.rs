//! The quarter time unit.

use crate::datatype::temporal::unit::macros::time_unit;

time_unit!(
    /// Three calendar months — not a fixed span of time, anchoring on a physical type plus `ygg.*` metadata.
    ///
    /// ```
    /// use yggdryl_schema::{TimeUnit, TimeUnitId, Quarter};
    ///
    /// assert_eq!(Quarter.unit_id(), TimeUnitId::Quarter);
    /// assert_eq!(Quarter::from_unit_id(TimeUnitId::Quarter), Ok(Quarter));
    /// ```
    Quarter
);
