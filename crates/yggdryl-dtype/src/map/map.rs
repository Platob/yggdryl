//! The [`Map`] base trait: the untyped surface of a map data type.

use crate::{DataType, Nested};

/// The untyped surface every map data type carries: a variable-length sequence of
/// key–value entries, exposing the key and value types.
///
/// It refines [`Nested`] (the single child is the entries struct) and names the key
/// and value data types as the associated [`KeyType`](Map::KeyType) and
/// [`ValueType`](Map::ValueType) so the concrete types are preserved for zero-cost
/// access, mirroring `yggdryl-field`'s `Field` and `yggdryl-scalar`'s `Scalar`. Key
/// and value types with codecs also get the typed [`TypedMap`](crate::TypedMap) layer.
///
/// ```
/// use yggdryl_dtype::{DataType, Int64Type, Map, MapType, Nested, UInt8Type};
///
/// let map = MapType::new(UInt8Type, Int64Type);
/// assert_eq!(map.key_type().name(), "uint8");
/// assert_eq!(map.value_type().name(), "int64");
/// assert_eq!(map.child_count(), 1);
/// ```
pub trait Map: Nested {
    /// The type of the entries' keys.
    type KeyType: DataType;

    /// The type of the entries' values.
    type ValueType: DataType;

    /// The type of the entries' keys.
    fn key_type(&self) -> &Self::KeyType;

    /// The type of the entries' values.
    fn value_type(&self) -> &Self::ValueType;
}
