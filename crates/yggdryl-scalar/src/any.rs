//! [`AnyScalar`] — the hashable, serializable carrier for any scalar value.

use yggdryl_core::ScalarError;
use yggdryl_dtype::{AnyType, DataType};

use crate::{Binary, Scalar, Utf8};

/// A concrete scalar value of any type — what [`Scalar::cast`] returns.
///
/// ```
/// use yggdryl_dtype::{BinaryType, Utf8Type};
/// use yggdryl_scalar::{AnyScalar, Binary, Scalar};
///
/// let bytes = Binary::from_bytes(b"hi");
/// let cast = bytes.cast(&Utf8Type::new()).unwrap();
/// assert!(matches!(cast, AnyScalar::Utf8(_)));
/// assert_eq!(cast.cast(&BinaryType::new()).unwrap(), AnyScalar::Binary(bytes));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AnyScalar {
    /// A binary buffer value.
    Binary(Binary),
    /// A UTF-8 string value.
    Utf8(Utf8),
}

impl AnyScalar {
    /// The contained binary value, if this is one.
    pub fn as_binary(&self) -> Option<&Binary> {
        match self {
            AnyScalar::Binary(inner) => Some(inner),
            AnyScalar::Utf8(_) => None,
        }
    }

    /// The contained string value, if this is one.
    pub fn as_utf8(&self) -> Option<&Utf8> {
        match self {
            AnyScalar::Utf8(inner) => Some(inner),
            AnyScalar::Binary(_) => None,
        }
    }
}

#[cfg(feature = "json")]
impl yggdryl_core::Jsonable for AnyScalar {}

impl From<Binary> for AnyScalar {
    fn from(inner: Binary) -> Self {
        AnyScalar::Binary(inner)
    }
}

impl From<Utf8> for AnyScalar {
    fn from(inner: Utf8) -> Self {
        AnyScalar::Utf8(inner)
    }
}

impl Scalar for AnyScalar {
    fn data_type(&self) -> AnyType {
        match self {
            AnyScalar::Binary(inner) => inner.data_type(),
            AnyScalar::Utf8(inner) => inner.data_type(),
        }
    }

    fn set_data_type(&mut self, data_type: &dyn DataType) -> Result<(), ScalarError> {
        match self {
            AnyScalar::Binary(inner) => inner.set_data_type(data_type),
            AnyScalar::Utf8(inner) => inner.set_data_type(data_type),
        }
    }

    fn cast(&self, data_type: &dyn DataType) -> Result<AnyScalar, ScalarError> {
        match self {
            AnyScalar::Binary(inner) => inner.cast(data_type),
            AnyScalar::Utf8(inner) => inner.cast(data_type),
        }
    }
}
