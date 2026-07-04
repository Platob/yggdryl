//! The [`Map`] base trait: the untyped surface of a map data type.

use crate::Nested;
use arrow_schema::{FieldRef, Fields};

/// The untyped surface every map data type carries: a variable-length sequence of
/// key–value entries, exposing the Arrow `"key"` and `"value"` entry fields.
///
/// It refines [`Nested`] (the single child is the entries struct). The dynamic
/// [`MapType`](crate::MapType) implements it over arbitrary key and value types; a
/// statically-typed map also implements the typed [`TypedMap`](crate::TypedMap) (via
/// [`TypedMapType<K, V>`](crate::TypedMapType)), which adds the concrete key/value
/// accessors and the byte codec. This mirrors the dynamic
/// [`StructType`](crate::StructType) / [`Struct`](crate::Struct) split.
///
/// ```
/// use yggdryl_dtype::{arrow_schema, Map, MapType, Nested};
///
/// let map = MapType::new(arrow_schema::DataType::UInt8, arrow_schema::DataType::Int64);
/// assert_eq!(map.entry_fields().len(), 2);
/// assert_eq!(map.entries_field().name(), "entries");
/// assert_eq!(map.child_count(), 1);
/// ```
pub trait Map: Nested {
    /// The entry struct's fields: the non-nullable `"key"` and nullable `"value"` —
    /// the per-entry children the scalar layer assembles its Arrow form around.
    ///
    /// Returned by value, since a typed map builds them from its key and value types
    /// rather than storing them.
    fn entry_fields(&self) -> Fields;

    /// The map's single Arrow child: the non-nullable `"entries"` struct field — the
    /// exact child [`to_arrow`](crate::DataType::to_arrow) wraps, built around
    /// [`entry_fields`](Map::entry_fields).
    fn entries_field(&self) -> FieldRef {
        std::sync::Arc::new(arrow_schema::Field::new(
            "entries",
            arrow_schema::DataType::Struct(self.entry_fields()),
            false,
        ))
    }
}
