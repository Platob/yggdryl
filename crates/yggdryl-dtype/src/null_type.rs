//! [`NullType`] — the `null` data type.

use crate::{DTypeError, DataType, TypedDataType};

/// The `null` data type (Arrow `Null`) — a type whose only value is the absence of a value.
///
/// It is **sui generis**: not a primitive, logical, or nested type, so it joins none of
/// those category traits and carries no core
/// [`PrimitiveType`](yggdryl_converter::PrimitiveType) tag. Its single value is the unit
/// `()` and encodes to **zero bytes** through the [`TypedDataType`] codec — this is how
/// `yggdryl` represents "null" now that a [`Scalar`](https://docs.rs/yggdryl-scalar) is
/// always present (nullability of a column/union is modelled from these null values, not
/// from an optional scalar).
///
/// ```
/// use yggdryl_dtype::{DataType, NullType, TypedDataType};
/// use arrow_schema::DataType as ArrowDataType;
///
/// let dt = NullType::new();
/// assert_eq!(dt.name(), "null");
/// assert_eq!(dt.byte_width(), Some(0));       // a null value is zero bytes
/// assert_eq!(dt.to_arrow(), ArrowDataType::Null);
/// assert_eq!(NullType::from_arrow(&dt.to_arrow()).unwrap(), dt);
/// // Value codec: the unit value is empty bytes.
/// assert!(dt.value_to_bytes(()).is_empty());
/// assert_eq!(dt.value_from_bytes(&[]).unwrap(), ());
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NullType;

impl NullType {
    /// Creates the `null` data type.
    pub const fn new() -> Self {
        Self
    }

    /// Reconstructs the type from its (empty) serialised payload.
    ///
    /// # Errors
    /// [`DTypeError::UnexpectedPayload`] if `bytes` is non-empty.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, DTypeError> {
        if bytes.is_empty() {
            Ok(Self)
        } else {
            Err(DTypeError::UnexpectedPayload {
                ty: "null",
                len: bytes.len(),
            })
        }
    }

    /// Builds the type from an Arrow [`DataType`](arrow_schema::DataType), validating it is
    /// `Null`.
    ///
    /// # Errors
    /// [`DTypeError::ArrowTypeMismatch`] if `arrow` is a different variant.
    pub fn from_arrow(arrow: &arrow_schema::DataType) -> Result<Self, DTypeError> {
        if matches!(arrow, arrow_schema::DataType::Null) {
            Ok(Self)
        } else {
            Err(DTypeError::ArrowTypeMismatch {
                expected: "null",
                got: format!("{arrow:?}"),
            })
        }
    }
}

impl DataType for NullType {
    fn name(&self) -> &'static str {
        "null"
    }

    fn byte_width(&self) -> Option<usize> {
        Some(0)
    }

    fn to_arrow(&self) -> arrow_schema::DataType {
        arrow_schema::DataType::Null
    }

    fn serialize_bytes(&self) -> Vec<u8> {
        Vec::new()
    }

    #[allow(clippy::unit_arg)] // the null type's default value *is* the unit `()`.
    fn default_any_value(&self) -> Box<dyn core::any::Any> {
        Box::new(<Self as TypedDataType<()>>::default_value(self))
    }
}

impl TypedDataType<()> for NullType {
    fn default_value(&self) {}

    fn value_to_bytes(&self, _value: ()) -> Vec<u8> {
        Vec::new()
    }

    fn value_from_bytes(&self, bytes: &[u8]) -> Result<(), DTypeError> {
        if bytes.is_empty() {
            Ok(())
        } else {
            Err(DTypeError::InvalidValueLength {
                ty: "null",
                len: bytes.len(),
                width: 0,
            })
        }
    }
}
