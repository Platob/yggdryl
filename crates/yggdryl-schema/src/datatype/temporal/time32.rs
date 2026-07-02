//! The 32-bit time-of-day data type.

use core::fmt;

use arrow_schema::DataType as ArrowDataType;

use crate::{DataType, DataTypeError, DataTypeId, Int32, LogicalType, PrimitiveType, TimeUnitId};

/// A time of day as a 32-bit offset since midnight, mapping to Arrow
/// `Time32(unit)` and anchored on [`Int32`]. Only second and millisecond
/// resolutions fit 32 bits.
///
/// ```
/// use yggdryl_schema::{DataType, Time32, TimeUnitId};
///
/// let time = Time32::from_parts(TimeUnitId::Millisecond).unwrap();
/// assert_eq!(Time32::from_arrow(&time.to_arrow()), Ok(time));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "RawTime32")
)]
pub struct Time32 {
    unit: TimeUnitId,
}

impl Time32 {
    /// Builds the type from its resolution, rejecting units finer than a
    /// millisecond.
    ///
    /// ```
    /// use yggdryl_schema::{Time32, TimeUnitId};
    ///
    /// assert!(Time32::from_parts(TimeUnitId::Nanosecond).is_err()); // expected s or ms
    /// ```
    pub fn from_parts(unit: TimeUnitId) -> Result<Self, DataTypeError> {
        match unit {
            TimeUnitId::Second | TimeUnitId::Millisecond => Ok(Self { unit }),
            other => Err(DataTypeError::TimeUnitMismatch {
                expected: "s or ms",
                actual: other,
            }),
        }
    }

    /// The resolution of the offset.
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

impl DataType for Time32 {
    fn type_id(&self) -> DataTypeId {
        DataTypeId::Time32
    }

    fn to_arrow(&self) -> ArrowDataType {
        ArrowDataType::Time32(self.unit.to_arrow().expect("validated sub-second unit"))
    }

    fn from_arrow(data_type: &ArrowDataType) -> Result<Self, DataTypeError> {
        match data_type {
            ArrowDataType::Time32(unit) => Self::from_parts(TimeUnitId::from_arrow(*unit)),
            other => Err(DataTypeError::ArrowTypeMismatch {
                expected: "time32",
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

impl PrimitiveType for Time32 {
    type Native = i32;
    const BIT_WIDTH: usize = 32;
}

impl LogicalType for Time32 {
    type Physical = Int32;

    fn physical(&self) -> Int32 {
        Int32
    }
}

impl fmt::Display for Time32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "time32({})", self.unit)
    }
}

/// Mirror of the serialized fields, deserialized first so `try_from`
/// re-validates on the way in.
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct RawTime32 {
    unit: TimeUnitId,
}

#[cfg(feature = "serde")]
impl TryFrom<RawTime32> for Time32 {
    type Error = DataTypeError;

    fn try_from(raw: RawTime32) -> Result<Self, Self::Error> {
        Self::from_parts(raw.unit)
    }
}
