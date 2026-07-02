//! The abstract base every timestamp implementation satisfies.

use std::sync::Arc;

use crate::TemporalType;

/// An instant as an offset since the UNIX epoch at a unit resolution, with
/// an optional timezone: the abstract base implemented for every
/// [`TimeUnit`] by the generic [`TimestampType`](crate::TimestampType).
///
/// Implementors supply [`from_parts`](Timestamp::from_parts) and the two
/// accessors; the functional updates come provided.
///
/// ```
/// use yggdryl_schema::{Minute, TemporalType, Timestamp, TimestampType};
///
/// let logged = TimestampType::from_parts(Minute, Some("UTC".into()));
/// assert_eq!(logged.unit(), Minute);
/// assert_eq!(logged.without_timezone().timezone(), None);
/// ```
pub trait Timestamp: TemporalType {
    /// Builds the timestamp type from its resolution and optional timezone.
    fn from_parts(unit: Self::Unit, timezone: Option<Arc<str>>) -> Self;

    /// The timezone the instant is rendered in, if any.
    fn timezone(&self) -> Option<&str>;

    /// Returns a copy with any of the parts overridden; omitted parts come
    /// from `self`.
    fn copy(&self, unit: Option<Self::Unit>, timezone: Option<Option<Arc<str>>>) -> Self {
        Self::from_parts(
            unit.unwrap_or_else(|| self.unit()),
            timezone.unwrap_or_else(|| self.timezone().map(Arc::from)),
        )
    }

    /// Returns a copy with the resolution replaced.
    fn with_unit(&self, unit: Self::Unit) -> Self {
        self.copy(Some(unit), None)
    }

    /// Returns a copy with the timezone replaced.
    fn with_timezone(&self, timezone: impl Into<Arc<str>>) -> Self {
        self.copy(None, Some(Some(timezone.into())))
    }

    /// Returns a copy with the timezone cleared.
    fn without_timezone(&self) -> Self {
        self.copy(None, Some(None))
    }
}
