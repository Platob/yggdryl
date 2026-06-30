//! The [`Binary`] scalar — a byte value carrying its binary data type.

use yggdryl_schema::DataType;

use crate::scalar::Scalar;

/// A binary scalar value: a byte payload tagged with its binary data type (e.g.
/// [`BinaryType`](yggdryl_schema::BinaryType) or
/// [`FixedSizeBinaryType`](yggdryl_schema::FixedSizeBinaryType)).
///
/// Generic over the concrete binary type `T`, so the same value type serves every
/// binary data type. When the type caps its size (a fixed- or max-size type), a
/// payload longer than [`max_byte_size`](yggdryl_schema::DataType::max_byte_size)
/// is truncated to that maximum.
///
/// ```
/// use yggdryl_schema::{DataType, DataTypeId, FixedSizeBinaryType, MaxedSizeBinaryType};
/// use yggdryl_scalar::{Binary, Scalar};
///
/// let value = Binary::new(FixedSizeBinaryType::new(2), vec![1u8, 2]);
/// assert_eq!(value.dtype().type_id(), DataTypeId::FixedSizeBinary);
/// assert_eq!(value.as_bytes(), b"\x01\x02");
/// assert!(value.is_fixed_size());
///
/// // A max-size type truncates an over-long payload.
/// let capped = Binary::new(MaxedSizeBinaryType::new(3), b"hello".to_vec());
/// assert_eq!(capped.as_bytes(), b"hel");
/// assert!(!capped.is_fixed_size());
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Binary<T: DataType> {
    dtype: T,
    bytes: Vec<u8>,
}

impl<T: DataType> Binary<T> {
    /// A binary value of type `dtype` holding `bytes`. If the type caps its size,
    /// an over-long payload is truncated to [`DataType::max_byte_size`].
    pub fn new(dtype: T, mut bytes: Vec<u8>) -> Self {
        if let Some(max) = dtype
            .max_byte_size()
            .and_then(|max| usize::try_from(max).ok())
        {
            if bytes.len() > max {
                crate::log_event!(
                    warn,
                    "Binary value of {} bytes truncated to the {} maximum of {}",
                    bytes.len(),
                    dtype.name(),
                    max
                );
                bytes.truncate(max);
            }
        }
        Self { dtype, bytes }
    }

    /// The value's bytes, borrowed (zero-copy).
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl<T: DataType> Scalar for Binary<T> {
    type Type = T;
    type Cast<D: DataType> = Binary<D>;

    fn dtype(&self) -> &T {
        &self.dtype
    }

    fn to_bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    fn from_bytes(dtype: T, bytes: &[u8]) -> Self {
        Self::new(dtype, bytes.to_vec())
    }

    /// Binary scalars hold raw bytes, so a cast never transforms the value — it
    /// re-tags the bytes as `dtype`, truncating them to that type's maximum byte
    /// size (a cast to the same type leaves them unchanged). Crossing into a
    /// non-byte-backed type is therefore the caller's responsibility to validate.
    fn cast<D: DataType>(&self, dtype: D) -> Binary<D> {
        Binary::new(dtype, self.bytes.clone())
    }
}
