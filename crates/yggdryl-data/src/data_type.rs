//! The typed [`DataType`] trait: a [`RawDataType`] with a native Rust representation.

use super::{DataError, RawDataType};

/// A [`RawDataType`] whose values have a native Rust representation `T`, with the
/// codec that bridges a `T` to and from its Arrow physical bytes, the type's
/// [`Scalar`](DataType::Scalar), and its defaults.
///
/// This is where the physical type meets a concrete Rust type: `Int64` implements
/// `DataType<i64>`, `Utf8` would implement `DataType<String>`, and so on. The codec is
/// the per-type byte surface (Rust value ↔ Arrow bytes); streaming and transfers stay
/// on the core IO traits. [`default_value`](DataType::default_value) is the type's
/// default native value (`0` for the integers, an empty sequence for lists and maps;
/// a union's is its *first* data type's default), and
/// [`default_scalar`](DataType::default_scalar) the default
/// [`Scalar`](DataType::Scalar) — a scalar holding the default value, except where
/// the scalar itself models nullness (the optional defaults to its null variant,
/// matching the scalar's own `Default`).
///
/// ```
/// use yggdryl_data::{DataType, Int64, RawScalar};
///
/// let int64 = Int64;
/// let bytes = int64.native_to_bytes(&42);
/// assert_eq!(bytes, vec![42, 0, 0, 0, 0, 0, 0, 0]); // little-endian i64
/// assert_eq!(int64.native_from_bytes(&bytes).unwrap(), 42);
///
/// // The wrong number of bytes is an error, not a wrap.
/// assert!(int64.native_from_bytes(&[1, 2, 3]).is_err());
///
/// // The type knows its default value and default scalar.
/// assert_eq!(int64.default_value(), 0);
/// assert_eq!(int64.default_scalar().value(), Some(&0));
/// ```
pub trait DataType<T>: RawDataType {
    /// The scalar type this data type's defaults produce — conventionally a
    /// [`RawScalar`] *of* this data type; a typed [`Union`](crate::Union)'s is its
    /// first data type's scalar (the union defaults to its first variant).
    type Scalar;

    /// Serialize a native `T` value into this type's Arrow physical bytes.
    fn native_to_bytes(&self, value: &T) -> Vec<u8>;

    /// Deserialize this type's Arrow physical bytes into a native `T`. The exact
    /// inverse of [`native_to_bytes`](DataType::native_to_bytes); a length mismatch
    /// returns [`DataError::InvalidByteLength`].
    fn native_from_bytes(&self, bytes: &[u8]) -> Result<T, DataError>;

    /// The fixed size of one *encoded* native value, in bytes, or `None` when the
    /// codec is variable-width. Defaults to the physical
    /// [`byte_width`](crate::RawDataType::byte_width); a logical type whose codec
    /// delegates (the optional writes plain value bytes while its storage is a
    /// union) overrides it to the delegate's codec width. Sequence codecs split
    /// their elements by this width.
    fn codec_byte_width(&self) -> Option<usize> {
        crate::RawDataType::byte_width(self)
    }

    /// The type's default native value — `0` for the integers, an empty sequence
    /// for lists and maps, the first data type's default for a union.
    fn default_value(&self) -> T;

    /// The default [`Scalar`](DataType::Scalar) of this type: a scalar holding
    /// [`default_value`](DataType::default_value), except where the scalar itself
    /// models nullness (an optional's default scalar is its null variant, matching
    /// the scalar's own `Default`).
    fn default_scalar(&self) -> Self::Scalar;
}
