//! The minute time unit.

use crate::datatype::temporal::unit::macros::time_unit;

time_unit!(
    /// Sixty seconds, anchoring on a physical type plus `ygg.*` metadata.
    ///
    /// ```
    /// use yggdryl_schema::{TimeUnit, TimeUnitId, Minute};
    ///
    /// assert_eq!(Minute.unit_id(), TimeUnitId::Minute);
    /// assert_eq!(Minute::from_unit_id(TimeUnitId::Minute), Ok(Minute));
    /// ```
    Minute
);
