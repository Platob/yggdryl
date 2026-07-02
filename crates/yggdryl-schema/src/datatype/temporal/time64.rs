//! The 64-bit time-of-day data type, one per 64-bit unit.

use core::fmt;

use arrow_schema::DataType as ArrowDataType;

use crate::{
    DataType, DataTypeError, DataTypeId, Int64Type, LogicalType, PrimitiveType, TemporalType, Time,
    Time64Unit, TimeUnitId,
};

/// A time of day as a 64-bit offset since midnight, mapping to Arrow
/// `Time64Type(unit)` and anchored on [`Int64Type`] — one type per [`Time64Unit`]
/// (`Time64Type<Microsecond>`, `Time64Type<Nanosecond>`, and the erased
/// `Time64Type<AnyTime64Unit>`), the resolutions the Arrow spec needs 64 bits
/// for.
///
/// ```
/// use yggdryl_schema::{DataType, Nanosecond, Time, Time64Type};
///
/// let time = Time64Type::from_parts(Nanosecond);
/// assert_eq!(Time64Type::from_arrow(&time.to_arrow()), Ok(time));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Time64Type<U: Time64Unit> {
    unit: U,
}

impl<U: Time64Unit> TemporalType for Time64Type<U> {
    type Unit = U;

    fn unit(&self) -> U {
        self.unit.clone()
    }
}

impl<U: Time64Unit> Time for Time64Type<U> {
    fn from_parts(unit: U) -> Self {
        Self { unit }
    }
}

impl<U: Time64Unit> DataType for Time64Type<U> {
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
        let mut out = vec![DataTypeId::Time64.to_u8()];
        out.extend(self.unit.unit_id().to_bytes());
        out
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError> {
        let payload = DataTypeId::Time64.strip_tag(bytes)?;
        Ok(Self::from_parts(U::from_unit_id(TimeUnitId::from_bytes(
            payload,
        )?)?))
    }
}

impl<U: Time64Unit> PrimitiveType for Time64Type<U> {
    type Native = i64;
    const BIT_WIDTH: usize = 64;
}

impl<U: Time64Unit> LogicalType for Time64Type<U> {
    type Physical = Int64Type;

    fn physical(&self) -> Int64Type {
        Int64Type
    }
}

impl<U: Time64Unit> fmt::Display for Time64Type<U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "time64({})", self.unit)
    }
}
