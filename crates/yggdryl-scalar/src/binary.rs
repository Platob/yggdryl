//! The [`Binary`] scalar — a byte value carrying its binary data type.

use yggdryl_schema::DataType;

use crate::scalar::Scalar;

/// A binary scalar value: a byte payload tagged with its binary data type (e.g.
/// [`BinaryType`](yggdryl_schema::BinaryType) or
/// [`FixedSizeBinaryType`](yggdryl_schema::FixedSizeBinaryType)).
///
/// Generic over the concrete binary type `T`, so the same value type serves every
/// binary data type.
///
/// ```
/// use yggdryl_schema::{DataType, DataTypeId, FixedSizeBinaryType};
/// use yggdryl_scalar::{Binary, Scalar};
///
/// let value = Binary::new(FixedSizeBinaryType::new(2), vec![1u8, 2]);
/// assert_eq!(value.dtype().type_id(), DataTypeId::FixedSizeBinary);
/// assert_eq!(value.as_bytes(), b"\x01\x02");
/// assert_eq!(value.to_bytes(), vec![1u8, 2]);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Binary<T: DataType> {
    dtype: T,
    bytes: Vec<u8>,
}

impl<T: DataType> Binary<T> {
    /// A binary value of type `dtype` holding `bytes`.
    pub fn new(dtype: T, bytes: Vec<u8>) -> Self {
        Self { dtype, bytes }
    }

    /// The value's bytes, borrowed (zero-copy).
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl<T: DataType> Scalar for Binary<T> {
    type Type = T;

    fn dtype(&self) -> &T {
        &self.dtype
    }

    fn to_bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    fn from_bytes(dtype: T, bytes: &[u8]) -> Self {
        Self {
            dtype,
            bytes: bytes.to_vec(),
        }
    }
}
