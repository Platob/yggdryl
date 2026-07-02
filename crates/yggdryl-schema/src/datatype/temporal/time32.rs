//! The 32-bit time-of-day data type, one per 32-bit unit.

use core::fmt;

use arrow_schema::DataType as ArrowDataType;

use crate::{
    DataType, DataTypeError, DataTypeId, Int32Type, LogicalType, PrimitiveType, TemporalType, Time,
    Time32Unit, TimeUnitId,
};

/// A time of day as a 32-bit offset since midnight, mapping to Arrow
/// `Time32Type(unit)` and anchored on [`Int32Type`] — one type per [`Time32Unit`]
/// (`Time32Type<Second>`, `Time32Type<Millisecond>`, and the erased
/// `Time32Type<AnyTime32Unit>`), the resolutions the Arrow spec fits in 32 bits.
///
/// ```
/// use yggdryl_schema::{DataType, Millisecond, Time, Time32Type};
///
/// let time = Time32Type::from_parts(Millisecond);
/// assert_eq!(Time32Type::from_arrow(&time.to_arrow()), Ok(time));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Time32Type<U: Time32Unit> {
    unit: U,
}

impl<U: Time32Unit> TemporalType for Time32Type<U> {
    type Unit = U;

    fn unit(&self) -> U {
        self.unit.clone()
    }
}

impl<U: Time32Unit> Time for Time32Type<U> {
    fn from_parts(unit: U) -> Self {
        Self { unit }
    }
}

impl<U: Time32Unit> DataType for Time32Type<U> {
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
        let mut out = vec![DataTypeId::Time32.to_u8()];
        out.extend(self.unit.unit_id().to_bytes());
        out
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError> {
        let payload = DataTypeId::Time32.strip_tag(bytes)?;
        Ok(Self::from_parts(U::from_unit_id(TimeUnitId::from_bytes(
            payload,
        )?)?))
    }
}

impl<U: Time32Unit> PrimitiveType for Time32Type<U> {
    type Native = i32;
    const BIT_WIDTH: usize = 32;
}

impl<U: Time32Unit> LogicalType for Time32Type<U> {
    type Physical = Int32Type;

    fn physical(&self) -> Int32Type {
        Int32Type
    }
}

impl<U: Time32Unit> fmt::Display for Time32Type<U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "time32({})", self.unit)
    }
}
