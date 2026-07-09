//! [`Decoder`] — the base byte-array decoding contract.

use crate::DecodeError;

/// Reverses an [`Encoder`](crate::Encoder), turning an encoded byte array back
/// into the original bytes.
///
/// This is the base of the decoding hierarchy: every decoder implements
/// [`decode_byte_array`](Decoder::decode_byte_array).
/// [`TypedDecoder<T>`](crate::TypedDecoder) generalises it to arrays of an
/// arbitrary element type, of which `T = u8` is exactly this trait.
///
/// ```
/// use yggdryl_core::{Decoder, Encoder, Gzip};
///
/// let gzip = Gzip::new(6).unwrap();
/// let encoded = gzip.encode_byte_array(b"round trip").unwrap();
/// assert_eq!(gzip.decode_byte_array(&encoded).unwrap(), b"round trip");
/// ```
pub trait Decoder {
    /// Decodes `bytes`, returning the recovered output.
    fn decode_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, DecodeError>;
}
