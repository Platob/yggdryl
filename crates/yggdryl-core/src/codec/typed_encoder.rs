//! [`TypedEncoder<T>`] — encoding of arrays of an arbitrary element type.

use crate::{EncodeError, Encoder};

/// Encodes a slice of `T` values into an encoded byte array.
///
/// `TypedEncoder<T>` generalises [`Encoder`] from raw bytes to arrays of an
/// arbitrary element type `T`; the `T = u8` case coincides with
/// [`Encoder::encode_byte_array`]. Compression codecs such as [`Gzip`](crate::Gzip)
/// operate on raw bytes, so they implement `TypedEncoder<u8>`.
///
/// ```
/// use yggdryl_core::{Gzip, TypedEncoder};
///
/// let gzip = Gzip::new(6).unwrap();
/// let encoded = gzip.encode(b"payload").unwrap();
/// assert!(!encoded.is_empty());
/// ```
pub trait TypedEncoder<T>: Encoder {
    /// Encodes `items`, returning the encoded output.
    fn encode(&self, items: &[T]) -> Result<Vec<u8>, EncodeError>;
}
