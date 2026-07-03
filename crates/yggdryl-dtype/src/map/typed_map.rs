//! The typed [`TypedMap`] trait: a [`Map`](super::Map) whose key and value types
//! have codecs.

use super::Map;
use crate::TypedDataType;

/// A [`Map`](super::Map) whose key and value types are typed
/// [`TypedDataType`]s — the map's values have native Rust representation
/// `Vec<(TK, TV)>`.
///
/// The concrete key and value types are [`Map`](super::Map)'s associated
/// [`KeyType`](super::Map::KeyType) / [`ValueType`](super::Map::ValueType), here
/// refined to typed [`TypedDataType`]s; the accessors are inherited from
/// [`Map`](super::Map). It also carries the [`TypedDataType<Vec<(TK, TV)>>`] surface
/// itself: the codec concatenates each entry's key and value bytes, and the default
/// is the empty map.
///
/// ```
/// use yggdryl_dtype::{Int64Type, MapType, TypedDataType, TypedMap, UInt8Type};
///
/// fn default_of<TK, TV, M: TypedMap<TK, TV>>(map: &M) -> Vec<(TK, TV)> {
///     map.default_value() // the empty map
/// }
///
/// let map = MapType::new(UInt8Type, Int64Type);
/// assert_eq!(default_of(&map), Vec::<(u8, i64)>::new());
/// ```
pub trait TypedMap<TK, TV>:
    Map<KeyType: TypedDataType<TK>, ValueType: TypedDataType<TV>> + TypedDataType<Vec<(TK, TV)>>
{
}
