//! [`BytesConverter<T>`] — a primitive value to/from its little-endian bytes.

use core::marker::PhantomData;

use yggdryl_buffer::IoPrimitive;

use crate::{ConvertError, Converter, TypedConverter};

/// Converts one primitive value `T` to its little-endian bytes, and back.
///
/// [`encode`](TypedConverter::encode) is [`IoPrimitive::to_le_vec`];
/// [`decode`](TypedConverter::decode) rebuilds the value from exactly
/// [`WIDTH`](IoPrimitive::WIDTH) bytes, rejecting any other length with a guided
/// [`ConvertError::InvalidByteLength`]. The byte-level [`Converter`] methods validate
/// that the input is a whole number of `T` values and pass the packed little-endian
/// bytes through unchanged.
///
/// ```
/// use yggdryl_converter::{BytesConverter, TypedConverter};
///
/// let codec = BytesConverter::<i32>::new();
/// assert_eq!(codec.encode(1_i32).unwrap(), vec![1, 0, 0, 0]);
/// assert_eq!(codec.decode(vec![1, 0, 0, 0]).unwrap(), 1);
/// assert!(codec.decode(vec![1, 0]).unwrap_err().to_string().contains("multiple of 4"));
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct BytesConverter<T> {
    _marker: PhantomData<T>,
}

impl<T> BytesConverter<T> {
    /// Creates the value-to-bytes converter.
    pub const fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

/// Validates that `len` is a whole number of `width`-byte elements.
fn check_width(len: usize, width: usize) -> Result<(), ConvertError> {
    if len.is_multiple_of(width) {
        Ok(())
    } else {
        Err(ConvertError::InvalidByteLength { len, width })
    }
}

impl<T: IoPrimitive> Converter for BytesConverter<T> {
    fn convert_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, ConvertError> {
        check_width(bytes.len(), T::WIDTH)?;
        Ok(bytes.to_vec())
    }

    fn invert_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, ConvertError> {
        check_width(bytes.len(), T::WIDTH)?;
        Ok(bytes.to_vec())
    }
}

impl<T: IoPrimitive> TypedConverter<T, Vec<u8>> for BytesConverter<T> {
    fn encode(&self, value: T) -> Result<Vec<u8>, ConvertError> {
        Ok(value.to_le_vec())
    }

    fn decode(&self, value: Vec<u8>) -> Result<T, ConvertError> {
        if value.len() != T::WIDTH {
            return Err(ConvertError::InvalidByteLength {
                len: value.len(),
                width: T::WIDTH,
            });
        }
        Ok(T::from_le_slice(&value))
    }
}
