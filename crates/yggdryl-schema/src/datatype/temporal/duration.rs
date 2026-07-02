//! The abstract base every duration implementation satisfies.

use crate::{DataType, TimeUnit};

/// An elapsed time as a count of a unit: the abstract base implemented for
/// every [`TimeUnit`] by the generic [`TypedDuration`](crate::TypedDuration).
///
/// Implementors supply [`from_parts`](Duration::from_parts) and the
/// accessor; the functional updates come provided.
///
/// ```
/// use yggdryl_schema::{Duration, Second, TypedDuration, Week};
///
/// assert_eq!(TypedDuration::from_parts(Second).unit(), Second);
/// assert_eq!(TypedDuration::from_parts(Week).with_unit(Week).unit(), Week);
/// ```
pub trait Duration: DataType {
    /// The resolution of the count.
    type Unit: TimeUnit;

    /// Builds the duration type from its resolution.
    fn from_parts(unit: Self::Unit) -> Self;

    /// The resolution of the count.
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
