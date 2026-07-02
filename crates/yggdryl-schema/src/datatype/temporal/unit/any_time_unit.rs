//! The erased time unit covering every identifier.

use core::fmt;

use crate::{DataTypeError, TimeUnit, TimeUnitId};

/// The erased [`TimeUnit`]: a value-level unit choice wrapping any
/// [`TimeUnitId`], so erased code paths — [`AnyDataType`](crate::AnyDataType)
/// timestamps, binding-held types — can carry the unit as data rather than as
/// a type parameter.
///
/// ```
/// use yggdryl_schema::{AnyTimeUnit, TimeUnit, TimeUnitId};
///
/// let unit = AnyTimeUnit::from(TimeUnitId::Minute);
/// assert_eq!(unit.unit_id(), TimeUnitId::Minute);
/// assert_eq!(AnyTimeUnit::from_unit_id(TimeUnitId::Year), Ok(TimeUnitId::Year.into()));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AnyTimeUnit {
    unit_id: TimeUnitId,
}

impl TimeUnit for AnyTimeUnit {
    fn from_unit_id(unit_id: TimeUnitId) -> Result<Self, DataTypeError> {
        Ok(Self { unit_id })
    }

    fn unit_id(&self) -> TimeUnitId {
        self.unit_id
    }
}

impl From<TimeUnitId> for AnyTimeUnit {
    fn from(unit_id: TimeUnitId) -> Self {
        Self { unit_id }
    }
}

impl fmt::Display for AnyTimeUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.unit_id.fmt(f)
    }
}
