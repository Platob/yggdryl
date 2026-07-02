//! The abstract base every time-of-day implementation satisfies.

use crate::{DataType, TimeUnit};

/// A time of day as an offset since midnight at a unit resolution: the
/// abstract base implemented by [`Time32<U>`](crate::Time32) for the 32-bit
/// units and [`Time64<U>`](crate::Time64) for the 64-bit units.
///
/// Implementors supply [`from_parts`](Time::from_parts) and the accessor;
/// the functional updates come provided.
///
/// ```
/// use yggdryl_schema::{Millisecond, Nanosecond, Time, Time32, Time64, TimeUnit};
///
/// assert_eq!(Time32::from_parts(Millisecond).unit(), Millisecond);
/// assert_eq!(Time64::from_parts(Nanosecond).unit().fixed_nanoseconds(), Some(1));
/// ```
pub trait Time: DataType {
    /// The resolution of the offset.
    type Unit: TimeUnit;

    /// Builds the time type from its resolution.
    fn from_parts(unit: Self::Unit) -> Self;

    /// The resolution of the offset.
    fn unit(&self) -> Self::Unit;

    /// Returns a copy with any of the parts overridden; omitted parts come
    /// from `self`.
    fn copy(&self, unit: Option<Self::Unit>) -> Self {
        Self::from_parts(unit.unwrap_or_else(|| self.unit()))
    }

    /// Returns a copy with the resolution replaced.
    fn with_unit(&self, unit: Self::Unit) -> Self {
        self.copy(Some(unit))
    }
}
