//! The [`RawMap`] base trait: the untyped surface of a map data type.

use crate::{Nested, RawDataType};

/// The untyped surface every map data type carries: a variable-length sequence of
/// key–value entries, exposing the key and value types.
///
/// It refines [`Nested`] (the single child is the entries struct) and is
/// parameterised by the key and value data types so the concrete types are
/// preserved for zero-cost access, mirroring [`RawField`](crate::RawField) and
/// [`RawScalar`](crate::RawScalar). Key and value types with codecs also get the
/// typed [`Map`](crate::Map) layer.
///
/// ```
/// use yggdryl_data::{Int64, MapType, Nested, RawDataType, RawMap, UInt8};
///
/// let map = MapType::new(UInt8, Int64);
/// assert_eq!(map.key_type().name(), "uint8");
/// assert_eq!(map.value_type().name(), "int64");
/// assert_eq!(map.child_count(), 1);
/// ```
pub trait RawMap<K: RawDataType, V: RawDataType>: Nested {
    /// The type of the entries' keys.
    fn key_type(&self) -> &K;

    /// The type of the entries' values.
    fn value_type(&self) -> &V;
}
