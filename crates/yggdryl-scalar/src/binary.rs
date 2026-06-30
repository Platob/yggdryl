//! The [`Binary`] scalar — a byte value carrying its binary data type.

use yggdryl_core::Buffer;
use yggdryl_schema::DataType;

use crate::scalar::Scalar;

/// A binary scalar value: a byte payload tagged with its binary data type (e.g.
/// [`BinaryType`](yggdryl_schema::BinaryType) or
/// [`FixedSizeBinaryType`](yggdryl_schema::FixedSizeBinaryType)).
///
/// Generic over the concrete binary type `T`, so the same value type serves every
/// binary data type. The bytes live in a zero-copy [`Buffer`], so cloning,
/// slicing and [`cast`](Scalar::cast)ing share the allocation rather than
/// deep-copying — the property the view-backed types (`BinaryViewType`,
/// `StringViewType`, …) need. When the type caps its size (a fixed- or max-size
/// type), a payload longer than
/// [`max_byte_size`](yggdryl_schema::DataType::max_byte_size) is truncated to that
/// maximum (a zero-copy slice).
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
    bytes: Buffer,
}

impl<T: DataType> Binary<T> {
    /// A binary value of type `dtype` holding `bytes`. If the type caps its size,
    /// an over-long payload is truncated to [`DataType::max_byte_size`].
    pub fn new(dtype: T, bytes: Vec<u8>) -> Self {
        Self::from_buffer(dtype, Buffer::from_vec(bytes))
    }

    /// A binary value of type `dtype` holding `buffer`, sharing its allocation
    /// (zero-copy). Truncates — via a zero-copy slice — to the type's maximum.
    fn from_buffer(dtype: T, buffer: Buffer) -> Self {
        let bytes = match dtype
            .max_byte_size()
            .and_then(|max| usize::try_from(max).ok())
        {
            Some(max) if buffer.len() > max => {
                crate::log_event!(
                    warn,
                    "Binary value of {} bytes truncated to the {} maximum of {}",
                    buffer.len(),
                    dtype.name(),
                    max
                );
                buffer.slice(0..max)
            }
            _ => buffer,
        };
        Self { dtype, bytes }
    }

    /// The value's bytes, borrowed (zero-copy).
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }
}

impl<T: DataType> Scalar for Binary<T> {
    type Type = T;
    type Cast<D: DataType> = Binary<D>;

    fn dtype(&self) -> &T {
        &self.dtype
    }

    fn to_bytes(&self) -> Vec<u8> {
        self.bytes.as_slice().to_vec()
    }

    fn from_bytes(dtype: T, bytes: &[u8]) -> Self {
        Self::new(dtype, bytes.to_vec())
    }

    /// Binary scalars hold raw bytes, so a cast never transforms the value — it
    /// re-tags the (zero-copy shared) buffer as `dtype`, truncating it to that
    /// type's maximum via a zero-copy slice. A cast to the same type therefore
    /// shares the bytes unchanged; crossing into a non-byte-backed type is the
    /// caller's responsibility to validate.
    fn cast<D: DataType>(&self, dtype: D) -> Binary<D> {
        Binary::from_buffer(dtype, self.bytes.clone())
    }
}
