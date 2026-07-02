//! The 32-bit time-of-day data type, one per 32-bit unit.

use core::fmt;

use arrow_schema::DataType as ArrowDataType;

use crate::{
    DataType, DataTypeError, DataTypeId, Int32, LogicalType, PrimitiveType, Time, Time32Unit,
    TimeUnitId,
};

/// A time of day as a 32-bit offset since midnight, mapping to Arrow
/// `Time32(unit)` and anchored on [`Int32`] — one type per [`Time32Unit`]
/// (`Time32<Second>`, `Time32<Millisecond>`, and the erased
/// `Time32<AnyTime32Unit>`), the resolutions the Arrow spec fits in 32 bits.
///
/// ```
/// use yggdryl_schema::{DataType, Millisecond, Time, Time32};
///
/// let time = Time32::from_parts(Millisecond);
/// assert_eq!(Time32::from_arrow(&time.to_arrow()), Ok(time));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Time32<U: Time32Unit> {
    unit: U,
}

impl<U: Time32Unit> Time for Time32<U> {
    type Unit = U;

    fn from_parts(unit: U) -> Self {
        Self { unit }
    }

    fn unit(&self) -> U {
        self.unit.clone()
    }
}

impl<U: Time32Unit> DataType for Time32<U> {
    fn type_id(&self) -> DataTypeId {
        DataTypeId::Time32
    }

    fn to_arrow(&self) -> ArrowDataType {
        // `Time32Unit` restricts the unit to second/millisecond, which Arrow
        // has natively.
        ArrowDataType::Time32(self.unit.to_arrow().expect("validated 32-bit time unit"))
    }

    fn from_arrow(data_type: &ArrowDataType) -> Result<Self, DataTypeError> {
        match data_type {
            ArrowDataType::Time32(unit) => Ok(Self::from_parts(U::from_unit_id(
                TimeUnitId::from_arrow(*unit),
            )?)),
            other => Err(DataTypeError::ArrowTypeMismatch {
                expected: "time32",
                actual: other.clone(),
            }),
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        self.unit.unit_id().to_bytes()
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError> {
        Ok(Self::from_parts(U::from_unit_id(TimeUnitId::from_bytes(
            bytes,
        )?)?))
    }
}

impl<U: Time32Unit> PrimitiveType for Time32<U> {
    type Native = i32;
    const BIT_WIDTH: usize = 32;
}

impl<U: Time32Unit> LogicalType for Time32<U> {
    type Physical = Int32;

    fn physical(&self) -> Int32 {
        Int32
    }
}

impl<U: Time32Unit> fmt::Display for Time32<U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "time32({})", self.unit)
    }
}
