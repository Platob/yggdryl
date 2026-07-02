//! The abstract base every time unit satisfies.

use core::fmt::{Debug, Display};
use core::hash::Hash;

use arrow_schema::TimeUnit as ArrowTimeUnit;

use crate::{DataTypeError, TimeUnitId};

/// A type-level time unit: the resolution parameter of temporal types such
/// as [`Timestamp<U>`](crate::Timestamp).
///
/// One unit struct implements this per [`TimeUnitId`] — from
/// [`Nanosecond`](crate::Nanosecond) up to [`Year`](crate::Year) — plus the
/// erased [`AnyTimeUnit`](crate::AnyTimeUnit) for value-level unit choices.
/// Implementors supply the identifier conversions; the Arrow mapping and the
/// fixed span come provided.
///
/// ```
/// use yggdryl_schema::{Minute, Nanosecond, TimeUnit, TimeUnitId, Year};
///
/// assert_eq!(Minute.unit_id(), TimeUnitId::Minute);
/// assert_eq!(Minute.fixed_nanoseconds(), Some(60_000_000_000));
/// assert_eq!(Nanosecond.to_arrow(), Some(arrow_schema::TimeUnit::Nanosecond));
/// assert_eq!(Year.to_arrow(), None); // anchors on a physical type instead
/// ```
pub trait TimeUnit: Clone + Debug + Display + Eq + Hash + Send + Sync + Sized + 'static {
    /// Builds the unit from its value-level identifier, rejecting the
    /// identifiers this unit type does not cover.
    fn from_unit_id(unit_id: TimeUnitId) -> Result<Self, DataTypeError>;

    /// The value-level identifier of this unit.
    fn unit_id(&self) -> TimeUnitId;

    /// The Arrow time unit this unit maps to, `None` for the units Arrow's
    /// type system lacks.
    fn to_arrow(&self) -> Option<ArrowTimeUnit> {
        self.unit_id().to_arrow()
    }

    /// The unit's fixed span in nanoseconds; `None` for calendar units,
    /// whose span depends on the date.
    fn fixed_nanoseconds(&self) -> Option<i64> {
        self.unit_id().fixed_nanoseconds()
    }
}
