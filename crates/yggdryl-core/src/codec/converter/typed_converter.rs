//! [`TypedConverter<S, T>`] — typed conversion between a source and target type.

use crate::{ConvertError, Converter};

/// Converts values of a source type `S` into a target type `T`, and back.
///
/// `TypedConverter<S, T>` generalises [`Converter`] from byte arrays to typed values:
/// [`encode`](TypedConverter::encode) maps `S` → `T` (the forward direction) and
/// [`decode`](TypedConverter::decode) maps `T` → `S` (its inverse). The batch
/// [`encode_slice`](TypedConverter::encode_slice) / [`decode_slice`] default to a
/// per-element loop but are overridden by converters that can go faster in bulk.
///
/// The trait carries the two generic parameters, so — like the buffer layer — it is
/// **Rust-only**; the bindings expose the concrete converters (which fix `S` and `T`)
/// and the byte-level [`Converter`] surface.
///
/// ```
/// use yggdryl_core::{CastConverter, TypedConverter};
///
/// let widen = CastConverter::<i32, i64>::new();
/// assert_eq!(widen.encode(7_i32).unwrap(), 7_i64);
/// assert_eq!(widen.decode(7_i64).unwrap(), 7_i32);
/// assert_eq!(widen.encode_slice(vec![1, 2, 3]).unwrap(), vec![1_i64, 2, 3]);
/// ```
pub trait TypedConverter<S, T>: Converter {
    /// Converts one `S` value to `T` (forward).
    fn encode(&self, value: S) -> Result<T, ConvertError>;

    /// Converts one `T` value back to `S` (the inverse of
    /// [`encode`](TypedConverter::encode)).
    fn decode(&self, value: T) -> Result<S, ConvertError>;

    /// Converts a batch of `S` values to `T`. Defaults to a per-element loop;
    /// override when a bulk path is cheaper.
    fn encode_slice(&self, values: Vec<S>) -> Result<Vec<T>, ConvertError> {
        values.into_iter().map(|value| self.encode(value)).collect()
    }

    /// Converts a batch of `T` values back to `S`. Defaults to a per-element loop;
    /// override when a bulk path is cheaper.
    fn decode_slice(&self, values: Vec<T>) -> Result<Vec<S>, ConvertError> {
        values.into_iter().map(|value| self.decode(value)).collect()
    }
}
