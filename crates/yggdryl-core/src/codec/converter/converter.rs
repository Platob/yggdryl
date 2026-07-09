//! [`Converter`] — the base representation-conversion contract.

use crate::ConvertError;

/// Converts a source byte representation to a target byte representation, and back.
///
/// This is the FFI-opaque base of the converter hierarchy (no lifetimes,
/// object-safe): every converter — numeric cast, string parse, byte codec — turns a
/// source-encoded byte array into a target-encoded one via
/// [`convert_byte_array`](Converter::convert_byte_array), and its exact inverse via
/// [`invert_byte_array`](Converter::invert_byte_array). Each converter documents how
/// it encodes source and target (e.g. little-endian packed values, or UTF-8 text), so
/// converters chain by bytes without knowing each other's element types.
///
/// [`TypedConverter<S, T>`](crate::TypedConverter) generalises it to typed values;
/// concrete converters such as [`CastConverter`](crate::CastConverter) implement both
/// and are what the Python and Node bindings expose.
///
/// ```
/// use yggdryl_core::{CastConverter, Converter};
///
/// // Widen little-endian i32 bytes to i64 bytes, then narrow back.
/// let widen = CastConverter::<i32, i64>::new();
/// let wide = widen.convert_byte_array(&1_i32.to_le_bytes()).unwrap();
/// assert_eq!(wide, 1_i64.to_le_bytes());
/// assert_eq!(widen.invert_byte_array(&wide).unwrap(), 1_i32.to_le_bytes());
/// ```
pub trait Converter {
    /// Converts source-representation `bytes` to the target representation (forward).
    fn convert_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, ConvertError>;

    /// Converts target-representation `bytes` back to the source representation — the
    /// exact inverse of [`convert_byte_array`](Converter::convert_byte_array).
    fn invert_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, ConvertError>;
}
