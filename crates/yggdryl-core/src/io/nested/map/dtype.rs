//! [`MapType`] — the **map data-type descriptor**: the `key` and `value` fields (and whether the
//! entries are sorted by key) that define a map's shape, and the concrete implementor of the root
//! [`DataType`](crate::io::DataType) for the nested `map` family.

use crate::io::{AnyField, DataType, DataTypeId};

/// The **typed descriptor** of a map type — its `key` and `value` fields (each an [`AnyField`], leaf
/// or nested) plus its `keys_sorted` flag. A map is the optimized alias of Arrow's exact model,
/// `List<Struct<{key non-null, value}>>`, so `MapType` has no width of its own (it reports `0`; a map
/// is neither fixed-width nor variable-length). The named, nullable counterpart is
/// [`MapField`](super::MapField).
///
/// ```
/// use yggdryl_core::io::fixed::{Field, PrimitiveType};
/// use yggdryl_core::io::nested::MapType;
/// use yggdryl_core::io::{AnyField, DataType};
///
/// let key = AnyField::leaf(Field::new("key", &PrimitiveType::<i32>::new(), false));
/// let value = AnyField::leaf(Field::new("value", &PrimitiveType::<i64>::new(), true));
/// let dt = MapType::new(key, value, false);
/// assert_eq!(dt.name(), "map");
/// assert!(dt.is_map());
/// assert_eq!(dt.key().name(), "key");
/// assert_eq!(dt.value().name(), "value");
/// assert!(!dt.keys_sorted());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MapType {
    key: Box<AnyField>,
    value: Box<AnyField>,
    keys_sorted: bool,
}

impl MapType {
    /// A map type from its `key` and `value` fields and whether the entries are sorted by key.
    pub fn new(key: AnyField, value: AnyField, keys_sorted: bool) -> Self {
        Self {
            key: Box::new(key),
            value: Box::new(value),
            keys_sorted,
        }
    }

    /// The key field.
    pub fn key(&self) -> &AnyField {
        &self.key
    }

    /// The value field.
    pub fn value(&self) -> &AnyField {
        &self.value
    }

    /// Whether the entries are sorted by key.
    pub fn keys_sorted(&self) -> bool {
        self.keys_sorted
    }
}

impl DataType for MapType {
    fn name(&self) -> &'static str {
        "map"
    }

    fn byte_width(&self) -> usize {
        0
    }

    fn type_id(&self) -> DataTypeId {
        DataTypeId::Map
    }

    /// The Arrow `Map(entries, keys_sorted)` type (feature `arrow`) — **recursive**: the entries are
    /// a non-nullable `Struct<{key non-null, value}>`, the key and value mapped by their
    /// [`AnyField::to_arrow`]. Overrides the id-level shell default (which cannot supply the
    /// key/value).
    #[cfg(feature = "arrow")]
    fn to_arrow(&self) -> arrow_schema::DataType {
        use std::sync::Arc;
        // DESIGN: Arrow requires the map key field non-nullable (a map key is never null); force it
        // here regardless of the caller's key-field nullability.
        let key = self.key.to_arrow().with_nullable(false);
        let value = self.value.to_arrow();
        let entries = arrow_schema::Field::new(
            "entries",
            arrow_schema::DataType::Struct(arrow_schema::Fields::from(vec![key, value])),
            false,
        );
        arrow_schema::DataType::Map(Arc::new(entries), self.keys_sorted)
    }
}
