//! The typed [`DataType`] trait: a [`RawDataType`] with a native Rust representation.

use super::{DataError, RawDataType};

/// A [`RawDataType`] whose values have a native Rust representation `T`, with the
/// codec that bridges a `T` to and from its Arrow physical bytes.
///
/// This is where the physical type meets a concrete Rust type: `Int64` implements
/// `DataType<i64>`, `Utf8` would implement `DataType<String>`, and so on. The codec is
/// the per-type byte surface (Rust value ↔ Arrow bytes); streaming and transfers stay
/// on the core IO traits.
///
/// ```
/// use yggdryl_data::{DataType, Int64};
///
/// let int64 = Int64;
/// let bytes = int64.native_to_bytes(&42);
/// assert_eq!(bytes, vec![42, 0, 0, 0, 0, 0, 0, 0]); // little-endian i64
/// assert_eq!(int64.native_from_bytes(&bytes).unwrap(), 42);
///
/// // The wrong number of bytes is an error, not a wrap.
/// assert!(int64.native_from_bytes(&[1, 2, 3]).is_err());
/// ```
pub trait DataType<T>: RawDataType {
    /// Serialize a native `T` value into this type's Arrow physical bytes.
    fn native_to_bytes(&self, value: &T) -> Vec<u8>;

    /// Deserialize this type's Arrow physical bytes into a native `T`. The exact
    /// inverse of [`native_to_bytes`](DataType::native_to_bytes); a length mismatch
    /// returns [`DataError::InvalidByteLength`].
    fn native_from_bytes(&self, bytes: &[u8]) -> Result<T, DataError>;
}
