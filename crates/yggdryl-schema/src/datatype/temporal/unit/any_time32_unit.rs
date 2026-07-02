//! The erased 32-bit time unit.

use core::fmt;

use crate::{DataTypeError, TimeUnit, TimeUnitId};

/// The erased [`Time32Unit`](crate::Time32Unit): a value-level unit choice
/// validated to the units a 32-bit time can hold (second or millisecond), so
/// erased code paths carry the unit as data without ever representing an
/// invalid one.
///
/// ```
/// use yggdryl_schema::{AnyTime32Unit, TimeUnit, TimeUnitId};
///
/// let unit = AnyTime32Unit::from_unit_id(TimeUnitId::Second).unwrap();
/// assert_eq!(unit.unit_id(), TimeUnitId::Second);
/// assert!(AnyTime32Unit::from_unit_id(TimeUnitId::Nanosecond).is_err());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "RawAnyTime32Unit")
)]
pub struct AnyTime32Unit {
    unit_id: TimeUnitId,
}

impl TimeUnit for AnyTime32Unit {
    fn from_unit_id(unit_id: TimeUnitId) -> Result<Self, DataTypeError> {
        match unit_id {
            TimeUnitId::Second | TimeUnitId::Millisecond => Ok(Self { unit_id }),
            other => Err(DataTypeError::TimeUnitMismatch {
                expected: "s or ms",
                actual: other,
            }),
        }
    }

    fn unit_id(&self) -> TimeUnitId {
        self.unit_id
    }
}

impl fmt::Display for AnyTime32Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.unit_id.fmt(f)
    }
}

/// Mirror of the serialized fields, deserialized first so `try_from`
/// re-validates on the way in.
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct RawAnyTime32Unit {
    unit_id: TimeUnitId,
}

#[cfg(feature = "serde")]
impl TryFrom<RawAnyTime32Unit> for AnyTime32Unit {
    type Error = DataTypeError;

    fn try_from(raw: RawAnyTime32Unit) -> Result<Self, Self::Error> {
        Self::from_unit_id(raw.unit_id)
    }
}
