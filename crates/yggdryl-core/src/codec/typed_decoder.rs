//! [`TypedDecoder<T>`] — decoding into arrays of an arbitrary element type.

use crate::{DecodeError, Decoder};

/// Decodes an encoded byte array back into a vector of `T` values.
///
/// `TypedDecoder<T>` generalises [`Decoder`] from raw bytes to arrays of an
/// arbitrary element type `T`; the `T = u8` case coincides with
/// [`Decoder::decode_byte_array`]. Compression codecs such as `Gzip` (in
/// `yggdryl-compression`) operate on raw bytes, so they implement `TypedDecoder<u8>`.
///
/// ```
/// use yggdryl_core::{Decoder, DecodeError, TypedDecoder};
///
/// // A tiny example; real codecs like `Gzip` live in `yggdryl-compression`.
/// struct LeBytes;
/// impl Decoder for LeBytes {
///     fn decode_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, DecodeError> {
///         Ok(bytes.to_vec())
///     }
/// }
/// impl TypedDecoder<i32> for LeBytes {
///     fn decode(&self, bytes: &[u8]) -> Result<Vec<i32>, DecodeError> {
///         Ok(bytes.chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap())).collect())
///     }
/// }
/// assert_eq!(LeBytes.decode(&[1, 0, 0, 0]).unwrap(), vec![1_i32]);
/// ```
pub trait TypedDecoder<T>: Decoder {
    /// Decodes `bytes`, returning the recovered vector of `T` values.
    fn decode(&self, bytes: &[u8]) -> Result<Vec<T>, DecodeError>;
}
