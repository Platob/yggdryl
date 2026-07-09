//! [`Utf8Converter`] — a string to/from its UTF-8 bytes.

use crate::{ConvertError, Converter, TypedConverter};

/// Converts a [`String`] to its UTF-8 bytes, and validated UTF-8 bytes back to a
/// [`String`].
///
/// [`encode`](TypedConverter::encode) is [`String::into_bytes`] (infallible);
/// [`decode`](TypedConverter::decode) validates the bytes, rejecting invalid UTF-8
/// with a guided [`ConvertError::InvalidUtf8`] that names the failing offset. The
/// byte-level [`Converter`] methods validate and pass the bytes through.
///
/// Unlike the numeric converters this fixes both ends (`String` ↔ `Vec<u8>`), so it
/// is a single concrete type rather than a generic.
///
/// ```
/// use yggdryl_core::{TypedConverter, Utf8Converter};
///
/// let codec = Utf8Converter::new();
/// assert_eq!(codec.encode("é".to_string()).unwrap(), vec![0xC3, 0xA9]);
/// assert_eq!(codec.decode(vec![0xC3, 0xA9]).unwrap(), "é");
/// assert!(codec.decode(vec![0xFF]).unwrap_err().to_string().contains("UTF-8"));
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct Utf8Converter;

impl Utf8Converter {
    /// Creates the UTF-8 converter.
    pub const fn new() -> Self {
        Self
    }
}

/// Validates `bytes` as UTF-8, returning a guided error at the failing offset.
fn validate_utf8(bytes: &[u8]) -> Result<(), ConvertError> {
    core::str::from_utf8(bytes)
        .map(|_| ())
        .map_err(|error| ConvertError::InvalidUtf8 {
            valid_up_to: error.valid_up_to(),
        })
}

impl Converter for Utf8Converter {
    fn convert_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, ConvertError> {
        validate_utf8(bytes)?;
        Ok(bytes.to_vec())
    }

    fn invert_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, ConvertError> {
        validate_utf8(bytes)?;
        Ok(bytes.to_vec())
    }
}

impl TypedConverter<String, Vec<u8>> for Utf8Converter {
    fn encode(&self, value: String) -> Result<Vec<u8>, ConvertError> {
        Ok(value.into_bytes())
    }

    fn decode(&self, value: Vec<u8>) -> Result<String, ConvertError> {
        String::from_utf8(value).map_err(|error| ConvertError::InvalidUtf8 {
            valid_up_to: error.utf8_error().valid_up_to(),
        })
    }
}
