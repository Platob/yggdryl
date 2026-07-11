//! [`NullScalar`] — the single value of the `null` data type.

use yggdryl_dtype::{NullType, TypedDataType};

use crate::{Scalar, ScalarError, TypedScalar};

/// The one value of the [`NullType`](yggdryl_dtype::NullType) data type — a scalar whose
/// value is "null".
///
/// A scalar is always present, so `NullScalar` is not a nullable wrapper: it is the plain
/// value of the sui-generis null type, its native value the unit `()`. It carries no data
/// and serialises to **zero bytes**. This is how a "null" is represented as a value (a
/// column's or union's nullability is built from these, not from an optional scalar). It
/// joins no category trait ([`PrimitiveScalar`](crate::PrimitiveScalar) / logical / nested),
/// mirroring [`NullType`](yggdryl_dtype::NullType) and [`NullField`](https://docs.rs/yggdryl-field).
///
/// ```
/// use yggdryl_scalar::{NullScalar, Scalar, TypedScalar};
///
/// let value = NullScalar::new();
/// assert_eq!(value.value(), ());
/// assert!(value.serialize_bytes().is_empty());
/// assert_eq!(NullScalar::deserialize_bytes(&value.serialize_bytes()).unwrap(), value);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NullScalar;

impl NullScalar {
    /// Creates the `null` scalar.
    pub const fn new() -> Self {
        Self
    }

    /// Reconstructs the scalar from its serialised bytes (which must be empty).
    ///
    /// # Errors
    /// [`ScalarError`] if `bytes` is non-empty (the null value carries no bytes).
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, ScalarError> {
        NullType::new().value_from_bytes(bytes)?;
        Ok(Self)
    }
}

impl Scalar for NullScalar {
    fn arrow_data_type(&self) -> arrow_schema::DataType {
        <NullType as yggdryl_dtype::DataType>::to_arrow(&NullType::new())
    }

    fn serialize_bytes(&self) -> Vec<u8> {
        Vec::new()
    }

    fn default_any_scalar(&self) -> Box<dyn Scalar> {
        Box::new(NullScalar::new())
    }
}

impl TypedScalar<NullType, ()> for NullScalar {
    fn value(&self) {}

    fn data_type(&self) -> NullType {
        NullType::new()
    }

    fn default_scalar() -> Self {
        Self::new()
    }
}
