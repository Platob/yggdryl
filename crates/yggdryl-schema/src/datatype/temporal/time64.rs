//! The 64-bit time-of-day data type.

use core::fmt;

use arrow_schema::DataType as ArrowDataType;

use crate::{DataType, DataTypeError, DataTypeId, Int64, LogicalType, PrimitiveType, TimeUnitId};

/// A time of day as a 64-bit offset since midnight, mapping to Arrow
/// `Time64(unit)` and anchored on [`Int64`]. Only microsecond and nanosecond
/// resolutions need 64 bits.
///
/// ```
/// use yggdryl_schema::{DataType, Time64, TimeUnitId};
///
/// let time = Time64::from_parts(TimeUnitId::Nanosecond).unwrap();
/// assert_eq!(Time64::from_arrow(&time.to_arrow()), Ok(time));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "RawTime64")
)]
pub struct Time64 {
    unit: TimeUnitId,
}

impl Time64 {
    /// Builds the type from its resolution, rejecting units coarser than a
    /// microsecond.
    ///
    /// ```
    /// use yggdryl_schema::{Time64, TimeUnitId};
    ///
    /// assert!(Time64::from_parts(TimeUnitId::Second).is_err()); // expected us or ns
    /// ```
    pub fn from_parts(unit: TimeUnitId) -> Result<Self, DataTypeError> {
        match unit {
            TimeUnitId::Microsecond | TimeUnitId::Nanosecond => Ok(Self { unit }),
            other => Err(DataTypeError::TimeUnitMismatch {
                expected: "us or ns",
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

impl DataType for Time64 {
    fn type_id(&self) -> DataTypeId {
        DataTypeId::Time64
    }

    fn to_arrow(&self) -> ArrowDataType {
        ArrowDataType::Time64(self.unit.to_arrow().expect("validated sub-second unit"))
    }

    fn from_arrow(data_type: &ArrowDataType) -> Result<Self, DataTypeError> {
        match data_type {
            ArrowDataType::Time64(unit) => Self::from_parts(TimeUnitId::from_arrow(*unit)),
            other => Err(DataTypeError::ArrowTypeMismatch {
                expected: "time64",
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

impl PrimitiveType for Time64 {
    type Native = i64;
    const BIT_WIDTH: usize = 64;
}

impl LogicalType for Time64 {
    type Physical = Int64;

    fn physical(&self) -> Int64 {
        Int64
    }
}

impl fmt::Display for Time64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "time64({})", self.unit)
    }
}

/// Mirror of the serialized fields, deserialized first so `try_from`
/// re-validates on the way in.
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct RawTime64 {
    unit: TimeUnitId,
}

#[cfg(feature = "serde")]
impl TryFrom<RawTime64> for Time64 {
    type Error = DataTypeError;

    fn try_from(raw: RawTime64) -> Result<Self, Self::Error> {
        Self::from_parts(raw.unit)
    }
}
