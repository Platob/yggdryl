//! The 64-bit time-of-day data type, one per 64-bit unit.

use core::fmt;

use arrow_schema::DataType as ArrowDataType;

use crate::{
    DataType, DataTypeError, DataTypeId, Int64, LogicalType, PrimitiveType, Time, Time64Unit,
    TimeUnitId,
};

/// A time of day as a 64-bit offset since midnight, mapping to Arrow
/// `Time64(unit)` and anchored on [`Int64`] — one type per [`Time64Unit`]
/// (`Time64<Microsecond>`, `Time64<Nanosecond>`, and the erased
/// `Time64<AnyTime64Unit>`), the resolutions the Arrow spec needs 64 bits
/// for.
///
/// ```
/// use yggdryl_schema::{DataType, Nanosecond, Time, Time64};
///
/// let time = Time64::from_parts(Nanosecond);
/// assert_eq!(Time64::from_arrow(&time.to_arrow()), Ok(time));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Time64<U: Time64Unit> {
    unit: U,
}

impl<U: Time64Unit> Time for Time64<U> {
    type Unit = U;

    fn from_parts(unit: U) -> Self {
        Self { unit }
    }

    fn unit(&self) -> U {
        self.unit.clone()
    }
}

impl<U: Time64Unit> DataType for Time64<U> {
    fn type_id(&self) -> DataTypeId {
        DataTypeId::Time64
    }

    fn to_arrow(&self) -> ArrowDataType {
        // `Time64Unit` restricts the unit to microsecond/nanosecond, which
        // Arrow has natively.
        ArrowDataType::Time64(self.unit.to_arrow().expect("validated 64-bit time unit"))
    }

    fn from_arrow(data_type: &ArrowDataType) -> Result<Self, DataTypeError> {
        match data_type {
            ArrowDataType::Time64(unit) => Ok(Self::from_parts(U::from_unit_id(
                TimeUnitId::from_arrow(*unit),
            )?)),
            other => Err(DataTypeError::ArrowTypeMismatch {
                expected: "time64",
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

impl<U: Time64Unit> PrimitiveType for Time64<U> {
    type Native = i64;
    const BIT_WIDTH: usize = 64;
}

impl<U: Time64Unit> LogicalType for Time64<U> {
    type Physical = Int64;

    fn physical(&self) -> Int64 {
        Int64
    }
}

impl<U: Time64Unit> fmt::Display for Time64<U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "time64({})", self.unit)
    }
}
