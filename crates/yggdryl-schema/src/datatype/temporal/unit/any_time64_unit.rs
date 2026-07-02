//! The erased 64-bit time unit.

use core::fmt;

use crate::{DataTypeError, TimeUnit, TimeUnitId};

/// The erased [`Time64Unit`](crate::Time64Unit): a value-level unit choice
/// validated to the units a 64-bit time can hold (microsecond or nanosecond),
/// so erased code paths carry the unit as data without ever representing an
/// invalid one.
///
/// ```
/// use yggdryl_schema::{AnyTime64Unit, TimeUnit, TimeUnitId};
///
/// let unit = AnyTime64Unit::from_unit_id(TimeUnitId::Nanosecond).unwrap();
/// assert_eq!(unit.unit_id(), TimeUnitId::Nanosecond);
/// assert!(AnyTime64Unit::from_unit_id(TimeUnitId::Second).is_err());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "RawAnyTime64Unit")
)]
pub struct AnyTime64Unit {
    unit_id: TimeUnitId,
}

impl TimeUnit for AnyTime64Unit {
    fn from_unit_id(unit_id: TimeUnitId) -> Result<Self, DataTypeError> {
        match unit_id {
            TimeUnitId::Microsecond | TimeUnitId::Nanosecond => Ok(Self { unit_id }),
            other => Err(DataTypeError::TimeUnitMismatch {
                expected: "us or ns",
                actual: other,
            }),
        }
    }

    fn unit_id(&self) -> TimeUnitId {
        self.unit_id
    }
}

impl fmt::Display for AnyTime64Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.unit_id.fmt(f)
    }
}

/// Mirror of the serialized fields, deserialized first so `try_from`
/// re-validates on the way in.
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct RawAnyTime64Unit {
    unit_id: TimeUnitId,
}

#[cfg(feature = "serde")]
impl TryFrom<RawAnyTime64Unit> for AnyTime64Unit {
    type Error = DataTypeError;

    fn try_from(raw: RawAnyTime64Unit) -> Result<Self, Self::Error> {
        Self::from_unit_id(raw.unit_id)
    }
}
