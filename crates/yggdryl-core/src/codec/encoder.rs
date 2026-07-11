//! [`Encoder`] — the base byte-array encoding contract.

use crate::EncodeError;

/// Transforms a byte array into an encoded byte array.
///
/// This is the base of the codec hierarchy: every encoder — compression and
/// otherwise — implements [`encode_byte_array`](Encoder::encode_byte_array).
/// [`TypedEncoder<T>`](crate::TypedEncoder) generalises it to arrays of an
/// arbitrary element type, of which `T = u8` is exactly this trait.
///
/// The trait is FFI-opaque (no lifetimes, object-safe); concrete implementors — such as
/// `Gzip` in `yggdryl-compression` — are what the Python and Node bindings expose.
///
/// ```
/// use yggdryl_core::{Encoder, EncodeError};
///
/// // A tiny example codec; real codecs like `Gzip` live in `yggdryl-compression`.
/// struct Xor(u8);
/// impl Encoder for Xor {
///     fn encode_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, EncodeError> {
///         Ok(bytes.iter().map(|b| b ^ self.0).collect())
///     }
/// }
/// assert_eq!(Xor(0xFF).encode_byte_array(&[0, 1]).unwrap(), vec![0xFF, 0xFE]);
/// ```
pub trait Encoder {
    /// Encodes `bytes`, returning the encoded output.
    fn encode_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, EncodeError>;
}
