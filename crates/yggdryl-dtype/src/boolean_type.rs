//! [`BooleanType`] — the `boolean` primitive data type.

use crate::{DTypeError, DataType, PrimitiveType, TypedDataType};

/// The `boolean` primitive data type (Arrow `Boolean`, native `bool`).
///
/// The one hand-written member of the primitive family: it is **bit-packed** (Arrow
/// stores 8 booleans per byte), so it has no fixed byte width
/// ([`byte_width`](DataType::byte_width) is `None`) and no core
/// [`PrimitiveType`](yggdryl_core::PrimitiveType) tag (that enum covers only the ten
/// numerics), exactly as `BooleanBuffer` (in the `yggdryl-buffer` crate) is the
/// hand-written member of the buffer family. A single value still encodes as one
/// `0`/`1` byte through the [`TypedDataType`] codec.
///
/// ```
/// use yggdryl_dtype::{BooleanType, DataType, PrimitiveType, TypedDataType};
/// use arrow_schema::DataType as ArrowDataType;
///
/// let dt = BooleanType::new();
/// assert_eq!(dt.name(), "boolean");
/// assert_eq!(dt.byte_width(), None);         // bit-packed
/// assert_eq!(dt.primitive_tag(), None);      // outside the core numeric tags
/// assert_eq!(dt.to_arrow(), ArrowDataType::Boolean);
/// assert_eq!(BooleanType::from_arrow(&dt.to_arrow()).unwrap(), dt);
/// // Value codec: one 0/1 byte.
/// assert_eq!(dt.value_to_bytes(true), vec![1]);
/// assert_eq!(dt.value_from_bytes(&[0]).unwrap(), false);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BooleanType;

impl BooleanType {
    /// Creates the `boolean` data type.
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
                ty: "boolean",
                len: bytes.len(),
            })
        }
    }

    /// Builds the type from an Arrow [`DataType`](arrow_schema::DataType), validating it
    /// is `Boolean`.
    ///
    /// # Errors
    /// [`DTypeError::ArrowTypeMismatch`] if `arrow` is a different variant.
    pub fn from_arrow(arrow: &arrow_schema::DataType) -> Result<Self, DTypeError> {
        if matches!(arrow, arrow_schema::DataType::Boolean) {
            Ok(Self)
        } else {
            Err(DTypeError::ArrowTypeMismatch {
                expected: "boolean",
                got: format!("{arrow:?}"),
            })
        }
    }
}

impl DataType for BooleanType {
    fn name(&self) -> &'static str {
        "boolean"
    }

    fn byte_width(&self) -> Option<usize> {
        None
    }

    fn to_arrow(&self) -> arrow_schema::DataType {
        arrow_schema::DataType::Boolean
    }

    fn serialize_bytes(&self) -> Vec<u8> {
        Vec::new()
    }
}

impl TypedDataType<bool> for BooleanType {
    fn native_default(&self) -> bool {
        false
    }

    fn value_to_bytes(&self, value: bool) -> Vec<u8> {
        vec![u8::from(value)]
    }

    fn value_from_bytes(&self, bytes: &[u8]) -> Result<bool, DTypeError> {
        if bytes.len() != 1 {
            return Err(DTypeError::InvalidValueLength {
                ty: "boolean",
                len: bytes.len(),
                width: 1,
            });
        }
        Ok(bytes[0] != 0)
    }
}

impl PrimitiveType for BooleanType {
    fn primitive_tag(&self) -> Option<yggdryl_core::PrimitiveType> {
        None
    }
}
