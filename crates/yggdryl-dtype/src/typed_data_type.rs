//! [`TypedDataType<T>`] — the value-typed extension of [`DataType`].

use crate::{DTypeError, DataType};

/// Ties a [`DataType`] to its Rust native value type `T`, adding the value↔bytes codec
/// the [`scalar`](https://docs.rs/yggdryl-scalar) layer builds on.
///
/// [`value_to_bytes`](TypedDataType::value_to_bytes) serialises one value to its
/// little-endian bytes and [`value_from_bytes`](TypedDataType::value_from_bytes) is its
/// exact inverse (validating the byte length against the type's width). The numeric
/// primitives delegate to [`yggdryl_buffer::IoPrimitive`]; `Boolean` encodes as a single
/// `0`/`1` byte.
///
/// The trait carries the generic parameter `T`, so — like `TypedConverter<S, T>` in the
/// core — it is **Rust-only**; the bindings expose the concrete data types (which fix
/// `T`) and the byte-level [`DataType`] surface.
///
/// ```
/// use yggdryl_dtype::{I32Type, TypedDataType};
///
/// let dt = I32Type::new();
/// let bytes = dt.value_to_bytes(-5);
/// assert_eq!(bytes, (-5_i32).to_le_bytes());
/// assert_eq!(dt.value_from_bytes(&bytes).unwrap(), -5);
/// assert_eq!(dt.default_value(), 0);
/// ```
pub trait TypedDataType<T>: DataType {
    /// The zero / default native value of this type (`0`, `false`, `()` for `null`). This
    /// is what a null substitutes to when building a buffer from possibly-null values.
    fn default_value(&self) -> T;

    /// Serialises one value to its little-endian bytes.
    fn value_to_bytes(&self, value: T) -> Vec<u8>;

    /// Decodes one value from its little-endian `bytes`, the inverse of
    /// [`value_to_bytes`](TypedDataType::value_to_bytes).
    ///
    /// # Errors
    /// [`DTypeError::InvalidValueLength`](crate::DTypeError::InvalidValueLength) if
    /// `bytes.len()` is not the type's value width.
    fn value_from_bytes(&self, bytes: &[u8]) -> Result<T, DTypeError>;
}
