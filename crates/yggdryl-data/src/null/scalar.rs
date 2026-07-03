//! The [`Null`] scalar of the [`NullType`](super::NullType) data type.

use super::NullType;
use crate::{DataError, RawScalar, Scalar};

/// The `null` scalar: always null, holding no value.
///
/// Its [`Value`](RawScalar::Value) is `()` — there is nothing to access — so
/// [`value`](RawScalar::value) is always `None` and every `as_*` accessor errors
/// with [`DataError::NullValue`](crate::DataError::NullValue): the scalar is
/// always null, and the shared accessor contract puts nullness first.
///
/// ```
/// use yggdryl_data::{DataError, Null, RawDataType, RawScalar};
///
/// let nothing = Null::new();
/// assert!(nothing.is_null());
/// assert_eq!(nothing.value(), None);
/// assert!(matches!(nothing.as_i64(), Err(DataError::NullValue)));
/// assert_eq!(nothing.data_type().name(), "null");
///
/// // Arrow's form is a one-element NullArray.
/// let arrow = nothing.to_arrow();
/// assert_eq!(arrow.len(), 1);
/// assert_eq!(Null::from_arrow(arrow.as_ref()).unwrap(), nothing);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Null {
    data_type: NullType,
}

impl Null {
    /// The null scalar.
    pub fn new() -> Self {
        Self {
            data_type: NullType,
        }
    }
}

impl RawScalar<NullType> for Null {
    type Value = ();

    fn data_type(&self) -> &NullType {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        true
    }

    fn value(&self) -> Option<&()> {
        None
    }

    fn to_arrow(&self) -> arrow_array::ArrayRef {
        std::sync::Arc::new(arrow_array::NullArray::new(1))
    }

    fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError> {
        if array.len() != 1 {
            return Err(DataError::InvalidScalarLength { got: array.len() });
        }
        if array
            .as_any()
            .downcast_ref::<arrow_array::NullArray>()
            .is_none()
        {
            return Err(DataError::IncompatibleArrowType {
                expected: "Null".to_string(),
                got: array.data_type().to_string(),
            });
        }
        Ok(Self::new())
    }

    // Always null, so per the shared contract nullness answers first: every
    // accessor errors with `NullValue`, not the `UnsupportedConversion` default.
    fn as_i8(&self) -> Result<i8, DataError> {
        Err(DataError::NullValue)
    }
    fn as_i16(&self) -> Result<i16, DataError> {
        Err(DataError::NullValue)
    }
    fn as_i32(&self) -> Result<i32, DataError> {
        Err(DataError::NullValue)
    }
    fn as_i64(&self) -> Result<i64, DataError> {
        Err(DataError::NullValue)
    }
    fn as_u8(&self) -> Result<u8, DataError> {
        Err(DataError::NullValue)
    }
    fn as_u16(&self) -> Result<u16, DataError> {
        Err(DataError::NullValue)
    }
    fn as_u32(&self) -> Result<u32, DataError> {
        Err(DataError::NullValue)
    }
    fn as_u64(&self) -> Result<u64, DataError> {
        Err(DataError::NullValue)
    }
    fn as_f32(&self) -> Result<f32, DataError> {
        Err(DataError::NullValue)
    }
    fn as_f64(&self) -> Result<f64, DataError> {
        Err(DataError::NullValue)
    }
    fn as_bool(&self) -> Result<bool, DataError> {
        Err(DataError::NullValue)
    }
    fn as_str(&self) -> Result<&str, DataError> {
        Err(DataError::NullValue)
    }
    fn as_bytes(&self) -> Result<&[u8], DataError> {
        Err(DataError::NullValue)
    }
}

impl Scalar<()> for Null {
    type Type = NullType;
}
