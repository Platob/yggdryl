//! The elapsed-time data type.

use core::fmt;

use arrow_schema::DataType as ArrowDataType;

use crate::{DataType, DataTypeError, DataTypeId, Int64, LogicalType, PrimitiveType, TimeUnit};

/// An elapsed time as a 64-bit count of a unit, mapping to Arrow
/// `Duration(unit)` and anchored on [`Int64`].
///
/// ```
/// use yggdryl_schema::{DataType, Duration, TimeUnit};
///
/// let duration = Duration::from_parts(TimeUnit::Second);
/// assert_eq!(Duration::from_arrow(&duration.to_arrow()), Ok(duration));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Duration {
    unit: TimeUnit,
}

impl Duration {
    /// Builds the type from its resolution; every unit is valid.
    pub fn from_parts(unit: TimeUnit) -> Self {
        Self { unit }
    }

    /// The resolution of the count.
    pub fn unit(&self) -> TimeUnit {
        self.unit
    }

    /// Returns a copy with any of the parts overridden; omitted parts come
    /// from `self`.
    pub fn copy(&self, unit: Option<TimeUnit>) -> Self {
        Self::from_parts(unit.unwrap_or(self.unit))
    }

    /// Returns a copy with the resolution replaced.
    pub fn with_unit(&self, unit: TimeUnit) -> Self {
        self.copy(Some(unit))
    }
}

impl DataType for Duration {
    fn type_id(&self) -> DataTypeId {
        DataTypeId::Duration
    }

    fn to_arrow(&self) -> ArrowDataType {
        ArrowDataType::Duration(self.unit.to_arrow())
    }

    fn from_arrow(data_type: &ArrowDataType) -> Result<Self, DataTypeError> {
        match data_type {
            ArrowDataType::Duration(unit) => Ok(Self::from_parts(TimeUnit::from_arrow(*unit))),
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
        Ok(Self::from_parts(TimeUnit::from_bytes(bytes)?))
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
