//! The [`Scalar`] trait — behaviour shared by every scalar value.

use yggdryl_schema::DataType;

/// A single typed value.
///
/// A scalar knows its [`dtype`](Scalar::dtype) and round-trips through its raw
/// byte form ([`to_bytes`](Scalar::to_bytes) / [`from_bytes`](Scalar::from_bytes)).
/// The data type is carried alongside the bytes, since the byte form holds only
/// the value, not the type.
///
/// ```
/// use yggdryl_schema::{BinaryType, DataType, DataTypeId};
/// use yggdryl_scalar::{Binary, Scalar};
///
/// let value = Binary::new(BinaryType, b"hi".to_vec());
/// assert_eq!(value.dtype().type_id(), DataTypeId::Binary);
/// assert_eq!(value.to_bytes(), b"hi".to_vec());
/// assert_eq!(Binary::from_bytes(BinaryType, &value.to_bytes()), value);
/// ```
pub trait Scalar {
    /// The value's data type.
    type Type: DataType;

    /// The value's data type (accessor).
    fn dtype(&self) -> &Self::Type;

    /// Whether the value's type has a fixed (exact) byte width. Defaults to the
    /// data type's category.
    fn is_fixed_size(&self) -> bool {
        self.dtype().type_id().is_fixed_size()
    }

    /// Serializes the value to its raw bytes.
    fn to_bytes(&self) -> Vec<u8>;

    /// Builds a value of `dtype` from its raw bytes.
    fn from_bytes(dtype: Self::Type, bytes: &[u8]) -> Self
    where
        Self: Sized;
}
