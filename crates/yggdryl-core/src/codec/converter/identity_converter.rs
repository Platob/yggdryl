//! [`IdentityConverter<T>`] — the pass-through converter.

use core::marker::PhantomData;

use crate::{ConvertError, Converter, TypedConverter};

/// A converter that returns its input unchanged — the identity element of the
/// converter hierarchy.
///
/// Both directions are the identity: [`encode`](TypedConverter::encode) and
/// [`decode`](TypedConverter::decode) return the value, and the byte methods return
/// the bytes verbatim. Useful as the default step in a conversion pipeline, or to
/// satisfy a `Converter`-typed slot without transforming anything.
///
/// ```
/// use yggdryl_core::{Converter, IdentityConverter, TypedConverter};
///
/// let identity = IdentityConverter::<i64>::new();
/// assert_eq!(identity.encode(42_i64).unwrap(), 42);
/// assert_eq!(identity.convert_byte_array(b"abc").unwrap(), b"abc");
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct IdentityConverter<T> {
    _marker: PhantomData<T>,
}

impl<T> IdentityConverter<T> {
    /// Creates the pass-through converter.
    pub const fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<T> Converter for IdentityConverter<T> {
    fn convert_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, ConvertError> {
        Ok(bytes.to_vec())
    }

    fn invert_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, ConvertError> {
        Ok(bytes.to_vec())
    }
}

impl<T> TypedConverter<T, T> for IdentityConverter<T> {
    fn encode(&self, value: T) -> Result<T, ConvertError> {
        Ok(value)
    }

    fn decode(&self, value: T) -> Result<T, ConvertError> {
        Ok(value)
    }
}
