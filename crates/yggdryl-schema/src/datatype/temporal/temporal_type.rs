//! The subtrait every temporal data type satisfies.

use crate::{LogicalType, TimeUnit, TimeUnitId};

/// A data type whose values count a [`TimeUnit`] — the shared root of the
/// [`Timestamp`](crate::Timestamp), [`Time`](crate::Time),
/// [`Date`](crate::Date) and [`Duration`](crate::Duration) bases, so code
/// that only cares about the resolution is written once against it.
///
/// Every temporal type is a [`LogicalType`] over an integer anchor; the unit
/// accessors live here and the per-kind traits add their own construction
/// and parts on top.
///
/// ```
/// use yggdryl_schema::{Date32Type, Minute, TemporalType, TimestampType, TimeUnitId};
///
/// assert_eq!(Date32Type.unit_id(), TimeUnitId::Day);
/// assert_eq!(
///     TimestampType::from_parts(Minute, None).unit_id(),
///     TimeUnitId::Minute,
/// );
/// # use yggdryl_schema::Timestamp;
/// ```
pub trait TemporalType: LogicalType {
    /// The resolution the values count.
    type Unit: TimeUnit;

    /// The resolution the values count.
    fn unit(&self) -> Self::Unit;

    /// The value-level identifier of the resolution.
    fn unit_id(&self) -> TimeUnitId {
        self.unit().unit_id()
    }
}
