//! The elapsed-time data type.

use core::fmt;

use arrow_schema::DataType as ArrowDataType;

use crate::{DataType, DataTypeError, DataTypeId, Int64, LogicalType, PrimitiveType, TimeUnitId};

/// An elapsed time as a 64-bit count of a unit, mapping to Arrow
/// `Duration(unit)` and anchored on [`Int64`]. Arrow durations only exist at
/// the four sub-second resolutions, so coarser units are rejected.
///
/// ```
/// use yggdryl_schema::{DataType, Duration, TimeUnitId};
///
/// let duration = Duration::from_parts(TimeUnitId::Second).unwrap();
/// assert_eq!(Duration::from_arrow(&duration.to_arrow()), Ok(duration));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "RawDuration")
)]
pub struct Duration {
    unit: TimeUnitId,
}

impl Duration {
    /// Builds the type from its resolution, rejecting units Arrow's
    /// `Duration` lacks.
    ///
    /// ```
    /// use yggdryl_schema::{Duration, TimeUnitId};
    ///
    /// assert!(Duration::from_parts(TimeUnitId::Minute).is_err()); // expected s, ms, us or ns
    /// ```
    pub fn from_parts(unit: TimeUnitId) -> Result<Self, DataTypeError> {
        if unit.to_arrow().is_none() {
            return Err(DataTypeError::TimeUnitMismatch {
                expected: "s, ms, us or ns",
                actual: unit,
            });
        }
        Ok(Self { unit })
    }

    /// The resolution of the count.
    pub fn unit(&self) -> TimeUnitId {
        self.unit
    }

    /// Returns a copy with any of the parts overridden; omitted parts come
    /// from `self`.
    pub fn copy(&self, unit: Option<TimeUnitId>) -> Result<Self, DataTypeError> {
        Self::from_parts(unit.unwrap_or(self.unit))
    }

    /// Returns a copy with the resolution replaced.
    pub fn with_unit(&self, unit: TimeUnitId) -> Result<Self, DataTypeError> {
        self.copy(Some(unit))
    }
}

impl DataType for Duration {
    fn type_id(&self) -> DataTypeId {
        DataTypeId::Duration
    }

    fn to_arrow(&self) -> ArrowDataType {
        ArrowDataType::Duration(self.unit.to_arrow().expect("validated sub-second unit"))
    }

    fn from_arrow(data_type: &ArrowDataType) -> Result<Self, DataTypeError> {
        match data_type {
            ArrowDataType::Duration(unit) => Self::from_parts(TimeUnitId::from_arrow(*unit)),
            other => Err(DataTypeError::ArrowTypeMismatch {
                expected: "duration",
                actual: other.clone(),
            }),
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        self.unit.to_bytes()
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError> {
        Self::from_parts(TimeUnitId::from_bytes(bytes)?)
    }
}

impl PrimitiveType for Duration {
    type Native = i64;
    const BIT_WIDTH: usize = 64;
}

impl LogicalType for Duration {
    type Physical = Int64;

    fn physical(&self) -> Int64 {
        Int64
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "duration({})", self.unit)
    }
}

/// Mirror of the serialized fields, deserialized first so `try_from`
/// re-validates on the way in.
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct RawDuration {
    unit: TimeUnitId,
}

#[cfg(feature = "serde")]
impl TryFrom<RawDuration> for Duration {
    type Error = DataTypeError;

    fn try_from(raw: RawDuration) -> Result<Self, Self::Error> {
        Self::from_parts(raw.unit)
    }
}
