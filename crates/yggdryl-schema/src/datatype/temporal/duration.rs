//! The abstract base every duration implementation satisfies.

use crate::TemporalType;

/// An elapsed time as a count of a unit: the abstract base implemented for
/// every [`TimeUnit`] by the generic [`DurationType`](crate::DurationType).
///
/// Implementors supply [`from_parts`](Duration::from_parts) and the
/// accessor; the functional updates come provided.
///
/// ```
/// use yggdryl_schema::{Duration, DurationType, Second, TemporalType, Week};
///
/// assert_eq!(DurationType::from_parts(Second).unit(), Second);
/// assert_eq!(DurationType::from_parts(Week).with_unit(Week).unit(), Week);
/// ```
pub trait Duration: TemporalType {
    /// Builds the duration type from its resolution.
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
