//! The abstract base every time-of-day implementation satisfies.

use crate::TemporalType;

/// A time of day as an offset since midnight at a unit resolution: the
/// abstract base implemented by [`Time32Type<U>`](crate::Time32Type) for the 32-bit
/// units and [`Time64Type<U>`](crate::Time64Type) for the 64-bit units.
///
/// Implementors supply [`from_parts`](Time::from_parts) and the accessor;
/// the functional updates come provided.
///
/// ```
/// use yggdryl_schema::{Millisecond, Nanosecond, TemporalType, Time, Time32Type, Time64Type, TimeUnit};
///
/// assert_eq!(Time32Type::from_parts(Millisecond).unit(), Millisecond);
/// assert_eq!(Time64Type::from_parts(Nanosecond).unit().fixed_nanoseconds(), Some(1));
/// ```
pub trait Time: TemporalType {
    /// Builds the time type from its resolution.
    fn from_parts(unit: Self::Unit) -> Self;

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
