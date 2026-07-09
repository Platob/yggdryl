//! [`TypedDecoder<T>`] — decoding into arrays of an arbitrary element type.

use crate::{DecodeError, Decoder};

/// Decodes an encoded byte array back into a vector of `T` values.
///
/// `TypedDecoder<T>` generalises [`Decoder`] from raw bytes to arrays of an
/// arbitrary element type `T`; the `T = u8` case coincides with
/// [`Decoder::decode_byte_array`]. Compression codecs such as [`Gzip`](crate::Gzip)
/// operate on raw bytes, so they implement `TypedDecoder<u8>`.
///
/// ```
/// use yggdryl_core::{Gzip, TypedDecoder, TypedEncoder};
///
/// let gzip = Gzip::new(6).unwrap();
/// let encoded = gzip.encode(b"payload").unwrap();
/// assert_eq!(gzip.decode(&encoded).unwrap(), b"payload");
/// ```
pub trait TypedDecoder<T>: Decoder {
    /// Decodes `bytes`, returning the recovered vector of `T` values.
    fn decode(&self, bytes: &[u8]) -> Result<Vec<T>, DecodeError>;
}
