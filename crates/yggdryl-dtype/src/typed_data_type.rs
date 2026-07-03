//! The typed [`TypedDataType`] trait: a [`DataType`] with a native Rust representation.

use super::{DataError, DataType};

/// A [`DataType`] whose values have a native Rust representation `T`, with the codec
/// that bridges a `T` to and from its Arrow physical bytes and the type's default
/// value.
///
/// This is where the physical type meets a concrete Rust type: [`Int64Type`](crate::Int64Type)
/// implements `TypedDataType<i64>`, `Utf8Type` would implement `TypedDataType<String>`,
/// and so on. The codec is the per-type byte surface (Rust value ↔ Arrow bytes);
/// streaming and transfers stay on the core IO traits.
/// [`default_value`](TypedDataType::default_value) is the type's default native value
/// (`0` for the integers, an empty sequence for lists and maps; a union's is its
/// *first* data type's default). This trait is the generic *factory*: it builds the
/// default value here, and — via the `yggdryl-field` / `yggdryl-scalar` extension
/// traits (`FieldFactory`, `ScalarFactory`) — its field and scalar.
///
/// ```
/// use yggdryl_dtype::{Int64Type, TypedDataType};
///
/// let int64 = Int64Type;
/// let bytes = int64.native_to_bytes(&42);
/// assert_eq!(bytes, vec![42, 0, 0, 0, 0, 0, 0, 0]); // little-endian i64
/// assert_eq!(int64.native_from_bytes(&bytes).unwrap(), 42);
///
/// // The wrong number of bytes is an error, not a wrap.
/// assert!(int64.native_from_bytes(&[1, 2, 3]).is_err());
///
/// // The type knows its default value.
/// assert_eq!(int64.default_value(), 0);
/// ```
pub trait TypedDataType<T>: DataType {
    /// Serialize a native `T` value into this type's Arrow physical bytes.
    fn native_to_bytes(&self, value: &T) -> Vec<u8>;

    /// Deserialize this type's Arrow physical bytes into a native `T`. The exact
    /// inverse of [`native_to_bytes`](TypedDataType::native_to_bytes); a length
    /// mismatch returns [`DataError::InvalidByteLength`].
    fn native_from_bytes(&self, bytes: &[u8]) -> Result<T, DataError>;

    /// The fixed size of one *encoded* native value, in bytes, or `None` when the
    /// codec is variable-width. Defaults to the physical
    /// [`byte_width`](crate::DataType::byte_width); a logical type whose codec
    /// delegates (the optional writes plain value bytes while its storage is a
    /// union) overrides it to the delegate's codec width. Sequence codecs split
    /// their elements by this width.
    fn codec_byte_width(&self) -> Option<usize> {
        crate::DataType::byte_width(self)
    }

    /// The type's default native value — `0` for the integers, an empty sequence
    /// for lists and maps, the first data type's default for a union.
    fn default_value(&self) -> T;
}
