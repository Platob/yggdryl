//! [`TypedEncoder<T>`] — encoding of arrays of an arbitrary element type.

use crate::{EncodeError, Encoder};

/// Encodes a slice of `T` values into an encoded byte array.
///
/// `TypedEncoder<T>` generalises [`Encoder`] from raw bytes to arrays of an
/// arbitrary element type `T`; the `T = u8` case coincides with
/// [`Encoder::encode_byte_array`]. Compression codecs such as `Gzip` (in
/// `yggdryl-compression`) operate on raw bytes, so they implement `TypedEncoder<u8>`.
///
/// ```
/// use yggdryl_core::{Encoder, EncodeError, TypedEncoder};
///
/// // A tiny example; real codecs like `Gzip` live in `yggdryl-compression`.
/// struct LeBytes;
/// impl Encoder for LeBytes {
///     fn encode_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, EncodeError> {
///         Ok(bytes.to_vec())
///     }
/// }
/// impl TypedEncoder<i32> for LeBytes {
///     fn encode(&self, items: &[i32]) -> Result<Vec<u8>, EncodeError> {
///         Ok(items.iter().flat_map(|v| v.to_le_bytes()).collect())
///     }
/// }
/// assert_eq!(LeBytes.encode(&[1_i32]).unwrap(), vec![1, 0, 0, 0]);
/// ```
pub trait TypedEncoder<T>: Encoder {
    /// Encodes `items`, returning the encoded output.
    fn encode(&self, items: &[T]) -> Result<Vec<u8>, EncodeError>;
}
