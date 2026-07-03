//! The typed [`TypedMap`] trait: a [`RawMap`](super::RawMap) whose key and value types
//! have codecs.

use super::RawMap;
use crate::DataType;

/// A [`RawMap`](super::RawMap) whose key and value types are typed
/// [`DataType`]s — the map's values have native Rust representation
/// `Vec<(TK, TV)>`.
///
/// The concrete key and value types are the associated
/// [`KeyType`](TypedMap::KeyType) / [`ValueType`](TypedMap::ValueType), so a map has exactly
/// one of each; the accessors are inherited from [`RawMap`](super::RawMap). It
/// also carries the [`DataType<Vec<(TK, TV)>>`] surface itself: the codec
/// concatenates each entry's key and value bytes, and the default is the empty
/// map.
///
/// ```
/// use yggdryl_dtype::{DataType, Int64, Map, TypedMap, UInt8};
///
/// fn default_of<TK, TV, M: TypedMap<TK, TV>>(map: &M) -> Vec<(TK, TV)> {
///     map.default_value() // the empty map
/// }
///
/// let map = Map::new(UInt8, Int64);
/// assert_eq!(default_of(&map), Vec::<(u8, i64)>::new());
/// ```
pub trait TypedMap<TK, TV>:
    RawMap<Self::KeyType, Self::ValueType> + DataType<Vec<(TK, TV)>>
{
    /// The concrete key type of this map.
    type KeyType: DataType<TK>;

    /// The concrete value type of this map.
    type ValueType: DataType<TV>;
}
