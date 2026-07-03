//! The typed [`TypedMap`] trait: a [`Map`](super::Map) whose key and value types
//! have codecs.

use super::Map;
use crate::TypedDataType;

/// A [`Map`](super::Map) whose key and value types are typed
/// [`TypedDataType`]s — the map's values have native Rust representation
/// `Vec<(TK, TV)>`.
///
/// The concrete key and value types are the associated
/// [`KeyType`](TypedMap::KeyType) / [`ValueType`](TypedMap::ValueType), so a map has exactly
/// one of each; the accessors are inherited from [`Map`](super::Map). It also
/// carries the [`TypedDataType<Vec<(TK, TV)>>`] surface itself: the codec
/// concatenates each entry's key and value bytes, and the default is the empty map.
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
    Map<Self::KeyType, Self::ValueType> + TypedDataType<Vec<(TK, TV)>>
{
    /// The concrete key type of this map.
    type KeyType: TypedDataType<TK>;

    /// The concrete value type of this map.
    type ValueType: TypedDataType<TV>;
}
