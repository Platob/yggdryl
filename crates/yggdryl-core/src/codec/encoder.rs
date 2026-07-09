//! [`Encoder`] — the base byte-array encoding contract.

use crate::EncodeError;

/// Transforms a byte array into an encoded byte array.
///
/// This is the base of the codec hierarchy: every encoder — compression and
/// otherwise — implements [`encode_byte_array`](Encoder::encode_byte_array).
/// [`TypedEncoder<T>`](crate::TypedEncoder) generalises it to arrays of an
/// arbitrary element type, of which `T = u8` is exactly this trait.
///
/// The trait is FFI-opaque (no lifetimes, object-safe); concrete implementors
/// such as [`Gzip`](crate::Gzip) are what the Python and Node bindings expose.
///
/// ```
/// use yggdryl_core::{Encoder, Gzip};
///
/// let gzip = Gzip::new(6).unwrap();
/// let encoded = gzip.encode_byte_array(b"hello hello hello").unwrap();
/// assert!(!encoded.is_empty());
/// ```
pub trait Encoder {
    /// Encodes `bytes`, returning the encoded output.
    fn encode_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, EncodeError>;
}
