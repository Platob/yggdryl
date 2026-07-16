//! [`MapField`] — the **centralized map schema**: a validated map-shaped
//! [`AnyField`](crate::io::AnyField) (its two children hold the `key` and `value` fields), which maps
//! to an Arrow [`Field`](arrow_schema::Field) (a `Map` column). This is the one place a map's shape is
//! described; `MapType`, `MapScalar`, and `MapSerie` take their schema from here.

use super::MapType;
use crate::io::{AnyField, DataTypeId, FieldType, Headers};

/// A **named, nullable map** column descriptor — the schema of a map. It is a thin, validated wrapper
/// over an [`AnyField`] (always the `Map` variant), so the recursive Arrow mapping lives once on
/// `AnyField` and this type adds only the map-specific surface (`with_*` builders, the key/value
/// lookups).
///
/// ```
/// use yggdryl_core::io::FieldType;
/// use yggdryl_core::io::fixed::{Field, PrimitiveType};
/// use yggdryl_core::io::AnyField;
/// use yggdryl_core::io::nested::MapField;
///
/// let schema = MapField::new(
///     "counts",
///     AnyField::leaf(Field::new("key", &PrimitiveType::<i32>::new(), false)),
///     AnyField::leaf(Field::new("value", &PrimitiveType::<i64>::new(), true)),
///     true,
///     false,
/// );
/// assert_eq!(schema.name(), "counts");
/// assert_eq!(schema.type_name(), "map");
/// assert!(schema.is_map() && schema.nullable());
/// assert_eq!(schema.key().name(), "key");
/// assert_eq!(schema.value().name(), "value");
/// assert!(!schema.keys_sorted());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MapField {
    inner: AnyField,
}

impl MapField {
    /// A map schema from a name, its `key` and `value` fields, its nullability, and whether the
    /// entries are sorted by key (empty metadata).
    pub fn new(
        name: &str,
        key: AnyField,
        value: AnyField,
        nullable: bool,
        keys_sorted: bool,
    ) -> Self {
        Self {
            inner: AnyField::map_(name, key, value, nullable, keys_sorted),
        }
    }

    /// The map's name.
    pub fn name(&self) -> &str {
        self.inner.name()
    }

    /// Whether the map column admits nulls.
    pub fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// The key field.
    pub fn key(&self) -> &AnyField {
        self.parts().4
    }

    /// The value field.
    pub fn value(&self) -> &AnyField {
        self.parts().5
    }

    /// Whether the entries are sorted by key.
    pub fn keys_sorted(&self) -> bool {
        self.parts().3
    }

    /// The map's metadata [`Headers`].
    pub fn metadata(&self) -> &Headers {
        self.inner.metadata()
    }

    /// The typed [`MapType`] descriptor (its key/value fields + keys_sorted flag).
    pub fn data_type(&self) -> MapType {
        MapType::new(self.key().clone(), self.value().clone(), self.keys_sorted())
    }

    /// This schema as an [`AnyField`] (its `Map` form) — the erased, recursive field.
    pub fn as_any_field(&self) -> &AnyField {
        &self.inner
    }

    /// Builds a map schema from an [`AnyField`], or `None` if it is not a map field.
    pub fn from_any_field(field: AnyField) -> Option<Self> {
        field.is_map().then_some(Self { inner: field })
    }

    // ---- ergonomic immutable updates: `with_*` builders ----------------------------------

    fn parts(&self) -> (&str, bool, &Headers, bool, &AnyField, &AnyField) {
        match &self.inner {
            AnyField::Map {
                name,
                nullable,
                metadata,
                keys_sorted,
                entries,
            } => (
                name,
                *nullable,
                metadata,
                *keys_sorted,
                &entries[0],
                &entries[1],
            ),
            // A `MapField` is always a map-shaped `AnyField` by construction.
            AnyField::Leaf(_) | AnyField::Struct { .. } | AnyField::List { .. } => {
                unreachable!("MapField always wraps AnyField::Map")
            }
        }
    }

    /// A fresh map schema renamed to `name`.
    pub fn with_name(&self, name: &str) -> Self {
        let (_, nullable, metadata, keys_sorted, key, value) = self.parts();
        Self {
            inner: AnyField::Map {
                name: name.to_string(),
                nullable,
                metadata: metadata.clone(),
                keys_sorted,
                entries: Box::new([key.clone(), value.clone()]),
            },
        }
    }

    /// A fresh map schema with `nullable` set.
    pub fn with_nullable(&self, nullable: bool) -> Self {
        let (name, _, metadata, keys_sorted, key, value) = self.parts();
        Self {
            inner: AnyField::Map {
                name: name.to_string(),
                nullable,
                metadata: metadata.clone(),
                keys_sorted,
                entries: Box::new([key.clone(), value.clone()]),
            },
        }
    }

    /// A fresh map schema with the `keys_sorted` flag set.
    pub fn with_keys_sorted(&self, keys_sorted: bool) -> Self {
        let (name, nullable, metadata, _, key, value) = self.parts();
        Self {
            inner: AnyField::Map {
                name: name.to_string(),
                nullable,
                metadata: metadata.clone(),
                keys_sorted,
                entries: Box::new([key.clone(), value.clone()]),
            },
        }
    }

    /// A fresh map schema with the given metadata [`Headers`] attached (replacing any existing).
    pub fn with_metadata(&self, metadata: Headers) -> Self {
        let (name, nullable, _, keys_sorted, key, value) = self.parts();
        Self {
            inner: AnyField::Map {
                name: name.to_string(),
                nullable,
                metadata,
                keys_sorted,
                entries: Box::new([key.clone(), value.clone()]),
            },
        }
    }

    /// A fresh map schema with one extra `key = value` metadata entry.
    pub fn with_metadata_entry(&self, key: &str, value: &str) -> Self {
        let (name, nullable, metadata, keys_sorted, key_field, value_field) = self.parts();
        let mut metadata = metadata.clone();
        metadata.insert(key, value);
        Self {
            inner: AnyField::Map {
                name: name.to_string(),
                nullable,
                metadata,
                keys_sorted,
                entries: Box::new([key_field.clone(), value_field.clone()]),
            },
        }
    }

    /// An explicit copy (the cross-language clone).
    pub fn copy(&self) -> Self {
        self.clone()
    }

    // ---- Arrow interop: a map schema is an Arrow `Map` Field ----------------------------

    /// This map as an Arrow [`Field`](arrow_schema::Field) of `Map` type (feature `arrow`) — name,
    /// nullability, metadata, and the recursively-mapped `Struct<{key non-null, value}>` entries (via
    /// [`AnyField::to_arrow`]).
    #[cfg(feature = "arrow")]
    pub fn to_arrow_field(&self) -> arrow_schema::Field {
        self.inner.to_arrow()
    }

    /// Builds a map schema from an Arrow [`Field`](arrow_schema::Field) of `Map` type (feature
    /// `arrow`), or `None` if the field is not a map (or the key/value type is not modeled).
    #[cfg(feature = "arrow")]
    pub fn from_arrow_field(field: &arrow_schema::Field) -> Option<Self> {
        Self::from_any_field(AnyField::from_arrow(field)?)
    }
}

impl FieldType for MapField {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn type_name(&self) -> &'static str {
        "map"
    }

    fn byte_width(&self) -> usize {
        0
    }

    fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    fn type_id(&self) -> DataTypeId {
        DataTypeId::Map
    }
}

impl From<MapField> for AnyField {
    fn from(field: MapField) -> Self {
        field.inner
    }
}
