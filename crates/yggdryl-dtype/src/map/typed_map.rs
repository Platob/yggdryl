//! The typed [`TypedMap`] trait: a [`Map`](super::Map) whose key and value types
//! have codecs.

use super::Map;
use crate::TypedDataType;

/// A [`Map`](super::Map) whose key and value types are typed
/// [`TypedDataType`]s — the map's values have native Rust representation
/// `Vec<(TK, TV)>`.
///
/// It names the concrete key and value types as the associated
/// [`KeyType`](TypedMap::KeyType) / [`ValueType`](TypedMap::ValueType) (both typed
/// [`TypedDataType`]s) so they are preserved for zero-cost access. It also carries
/// the [`TypedDataType<Vec<(TK, TV)>>`] surface itself: the codec concatenates each
/// entry's key and value bytes, and the default is the empty map. The untyped
/// [`Map`](super::Map) is implemented by both the dynamic
/// [`MapType`](crate::MapType) and the typed
/// [`TypedMapType<K, V>`](crate::TypedMapType); this typed layer is only the latter.
///
/// ```
/// use yggdryl_dtype::{DataType, Int64Type, TypedDataType, TypedMap, TypedMapType, UInt8Type};
///
/// fn default_of<TK, TV, M: TypedMap<TK, TV>>(map: &M) -> Vec<(TK, TV)> {
///     map.default_value() // the empty map
/// }
///
/// let map = TypedMapType::new(UInt8Type, Int64Type);
/// assert_eq!((map.key_type().name(), map.value_type().name()), ("uint8", "int64"));
/// assert_eq!(default_of(&map), Vec::<(u8, i64)>::new());
/// ```
pub trait TypedMap<TK, TV>: Map + TypedDataType<Vec<(TK, TV)>> {
    /// The type of the entries' keys.
    type KeyType: TypedDataType<TK>;

    /// The type of the entries' values.
    type ValueType: TypedDataType<TV>;

    /// The type of the entries' keys.
    fn key_type(&self) -> &Self::KeyType;

    /// The type of the entries' values.
    fn value_type(&self) -> &Self::ValueType;
}
