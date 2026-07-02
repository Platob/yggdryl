//! The abstract base every date implementation satisfies.

use crate::TemporalType;

/// A date as a count of a unit since the UNIX epoch: the abstract base
/// implemented by [`Date32Type`](crate::Date32Type) (days) and
/// [`Date64Type`](crate::Date64Type) (milliseconds) — the two resolutions the Arrow
/// columnar spec defines for dates.
///
/// The unit accessors come from [`TemporalType`]: [`Date32Type`] counts
/// [`Day`](crate::Day)s and [`Date64Type`] counts
/// [`Millisecond`](crate::Millisecond)s.
///
/// ```
/// use yggdryl_schema::{Date32Type, Date64Type, Day, Millisecond, TemporalType};
///
/// assert_eq!(Date32Type.unit(), Day);
/// assert_eq!(Date64Type.unit(), Millisecond);
/// ```
pub trait Date: TemporalType {}
