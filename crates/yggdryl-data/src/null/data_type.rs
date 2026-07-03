//! The [`NullType`] data type.

use crate::{DataError, RawDataType};

/// The Apache Arrow `null` data type: every value is null.
///
/// It is storage-free — no byte width, no codec — and is neither a
/// [`Primitive`](crate::Primitive) nor a [`Nested`](crate::Nested) type. Its main
/// structural role is as the null variant of a [`UnionType`](crate::UnionType) (see
/// [`Optional`](crate::Optional)).
///
/// ```
/// use yggdryl_data::{arrow_schema, DataTypeId, NullType, RawDataType};
///
/// assert_eq!(NullType.name(), "null");
/// assert_eq!(NullType.arrow_format(), "n");
/// assert_eq!((NullType.byte_width(), NullType.bit_width()), (None, None));
/// assert_eq!(NullType::ID, DataTypeId::Null);
///
/// assert_eq!(NullType.to_arrow(), arrow_schema::DataType::Null);
/// assert!(NullType::from_arrow(&arrow_schema::DataType::Int64).is_err());
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct NullType;

impl NullType {
    /// This type's [`DataTypeId`](crate::DataTypeId).
    pub const ID: crate::DataTypeId = crate::DataTypeId::Null;
}

impl RawDataType for NullType {
    fn name(&self) -> &str {
        "null"
    }

    fn arrow_format(&self) -> String {
        "n".to_string()
    }

    fn byte_width(&self) -> Option<usize> {
        None
    }

    fn to_arrow(&self) -> arrow_schema::DataType {
        arrow_schema::DataType::Null
    }

    fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError> {
        match data_type {
            arrow_schema::DataType::Null => Ok(Self),
            other => Err(DataError::IncompatibleArrowType {
                expected: "Null".to_string(),
                got: other.to_string(),
            }),
        }
    }
}
