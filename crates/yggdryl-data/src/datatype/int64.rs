//! The [`Int64`] data type: Apache Arrow's 64-bit signed integer.

use super::{DataError, DataType, Primitive, RawDataType};

/// Apache Arrow's `Int64`: a 64-bit signed integer, native Rust `i64`, stored
/// little-endian in eight bytes, Arrow C Data Interface format `"l"`.
///
/// It is the first concrete data type: a fixed-width [`Primitive`] whose
/// [`DataType<i64>`] codec round-trips an `i64` to and from its Arrow bytes.
///
/// ```
/// use yggdryl_data::{DataType, Int64, Primitive, RawDataType};
///
/// assert_eq!(Int64.name(), "int64");
/// assert_eq!(Int64.arrow_format(), "l");
/// assert_eq!((Int64.byte_width(), Int64.bit_width()), (Some(8), Some(64)));
///
/// // The DataType<i64> codec round-trips through Arrow bytes.
/// assert_eq!(Int64.native_to_bytes(&-1), vec![0xFF; 8]);
/// assert_eq!(Int64.native_from_bytes(&[0xFF; 8]).unwrap(), -1);
///
/// // It is a fixed-width primitive.
/// fn width<P: Primitive>(p: &P) -> Option<usize> { p.byte_width() }
/// assert_eq!(width(&Int64), Some(8));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Int64;

impl RawDataType for Int64 {
    fn name(&self) -> &str {
        "int64"
    }

    fn arrow_format(&self) -> String {
        "l".to_string()
    }

    fn byte_width(&self) -> Option<usize> {
        Some(8)
    }
}

impl DataType<i64> for Int64 {
    fn native_to_bytes(&self, value: &i64) -> Vec<u8> {
        value.to_le_bytes().to_vec()
    }

    fn native_from_bytes(&self, bytes: &[u8]) -> Result<i64, DataError> {
        let array: [u8; 8] = bytes.try_into().map_err(|_| DataError::InvalidByteLength {
            expected: 8,
            got: bytes.len(),
        })?;
        Ok(i64::from_le_bytes(array))
    }
}

impl Primitive for Int64 {}
