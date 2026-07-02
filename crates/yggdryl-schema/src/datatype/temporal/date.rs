//! The abstract base every date implementation satisfies.

use crate::{DataType, TimeUnit};

/// A date as a count of a unit since the UNIX epoch: the abstract base
/// implemented by [`Date32`](crate::Date32) (days) and
/// [`Date64`](crate::Date64) (milliseconds) — the two resolutions the Arrow
/// columnar spec defines for dates.
///
/// ```
/// use yggdryl_schema::{Date, Date32, Date64, Day, Millisecond};
///
/// assert_eq!(Date32.unit(), Day);
/// assert_eq!(Date64.unit(), Millisecond);
/// ```
pub trait Date: DataType {
    /// The resolution a date value counts since the epoch.
    type Unit: TimeUnit;

    /// The resolution a date value counts since the epoch.
    fn unit(&self) -> Self::Unit;
}
