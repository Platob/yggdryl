//! The [`NullScalar`] scalar of the [`Null`](super::Null) data type.

use super::Null;
use crate::{DataError, RawScalar, Scalar};

/// The `null` scalar: always null, holding no value.
///
/// Its [`Value`](RawScalar::Value) is `()` — there is nothing to access — so
/// [`value`](RawScalar::value) is always `None` and every `as_*` accessor answers
/// `None` (the trait defaults).
///
/// ```
/// use yggdryl_data::{NullScalar, RawDataType, RawScalar};
///
/// let nothing = NullScalar::new();
/// assert!(nothing.is_null());
/// assert_eq!(nothing.value(), None);
/// assert_eq!(nothing.as_i64(), None);
/// assert_eq!(nothing.data_type().name(), "null");
///
/// // Arrow's form is a one-element NullArray.
/// let arrow = nothing.to_arrow();
/// assert_eq!(arrow.len(), 1);
/// assert_eq!(NullScalar::from_arrow(arrow.as_ref()).unwrap(), nothing);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct NullScalar {
    data_type: Null,
}

impl NullScalar {
    /// The null scalar.
    pub fn new() -> Self {
        Self { data_type: Null }
    }
}

impl RawScalar<Null> for NullScalar {
    type Value = ();

    fn data_type(&self) -> &Null {
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
}

impl Scalar<()> for NullScalar {
    type Type = Null;
}
