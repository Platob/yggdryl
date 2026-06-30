//! The [`Scalar`] trait — behaviour shared by every scalar value.

use yggdryl_schema::DataType;

use crate::codec::{Decode, Encode};
use crate::error::ScalarError;

/// A single typed value.
///
/// A scalar knows its [`dtype`](Scalar::dtype) and round-trips through its raw
/// byte form ([`to_bytes`](Scalar::to_bytes) / [`from_bytes`](Scalar::from_bytes)).
/// On top of that it [`encode`](Scalar::encode)s / [`decode`](Scalar::decode)s
/// native Rust values (Arrow scalar values) and [`cast`](Scalar::cast)s to another
/// data type.
///
/// ```
/// use yggdryl_schema::{BinaryType, DataType, DataTypeId, LargeBinaryType};
/// use yggdryl_scalar::{Binary, Scalar};
///
/// let value = Binary::encode(BinaryType, "hi");
/// assert_eq!(value.dtype().type_id(), DataTypeId::Binary);
/// assert_eq!(value.decode::<String>().unwrap(), "hi");
/// assert_eq!(value.to_bytes(), b"hi".to_vec());
///
/// // A cast re-tags the value as another type.
/// let large = value.cast(LargeBinaryType);
/// assert_eq!(large.dtype().type_id(), DataTypeId::LargeBinary);
/// assert_eq!(large.to_bytes(), b"hi".to_vec());
/// ```
pub trait Scalar {
    /// The value's data type.
    type Type: DataType;

    /// The scalar this value becomes when [`cast`](Scalar::cast) to data type `D`.
    type Cast<D: DataType>: Scalar<Type = D>;

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

    /// Casts the value to `dtype`, returning a scalar of the target type.
    fn cast<D: DataType>(&self, dtype: D) -> Self::Cast<D>;

    /// Builds a scalar by encoding a native value (an Arrow scalar value).
    fn encode<E: Encode + ?Sized>(dtype: Self::Type, value: &E) -> Self
    where
        Self: Sized,
    {
        Self::from_bytes(dtype, &value.encode())
    }

    /// Decodes this scalar's bytes into a native value (an Arrow scalar value).
    fn decode<V: Decode>(&self) -> Result<V, ScalarError> {
        V::decode(&self.to_bytes())
    }
}
