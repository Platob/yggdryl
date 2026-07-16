//! [`ListField`] — the **centralized list schema**: a validated list-shaped
//! [`AnyField`](crate::io::AnyField) (its single child holds the element (item) field), which maps
//! to an Arrow [`Field`](arrow_schema::Field) (a `List` column). This is the one place a list's
//! shape is described; `ListType`, `ListScalar`, and `ListSerie` take their schema from here.

use super::ListType;
use crate::io::{AnyField, DataTypeId, FieldType, Headers};

/// A **named, nullable list** column descriptor — the schema of a list. It is a thin, validated
/// wrapper over an [`AnyField`] (always the `List` variant), so the recursive Arrow mapping lives
/// once on `AnyField` and this type adds only the list-specific surface (`with_*` builders, the
/// element lookup).
///
/// ```
/// use yggdryl_core::io::FieldType;
/// use yggdryl_core::io::fixed::{Field, PrimitiveType};
/// use yggdryl_core::io::AnyField;
/// use yggdryl_core::io::nested::ListField;
///
/// let schema = ListField::new(
///     "scores",
///     AnyField::leaf(Field::new("item", &PrimitiveType::<i32>::new(), true)),
///     true,
/// );
/// assert_eq!(schema.name(), "scores");
/// assert_eq!(schema.type_name(), "list");
/// assert!(schema.is_list() && schema.nullable());
/// assert_eq!(schema.item().name(), "item");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ListField {
    inner: AnyField,
}

impl ListField {
    /// A list schema from a name, its element (item) field, and its nullability (empty metadata).
    pub fn new(name: &str, item: AnyField, nullable: bool) -> Self {
        Self {
            inner: AnyField::list_(name, item, nullable),
        }
    }

    /// The list's name.
    pub fn name(&self) -> &str {
        self.inner.name()
    }

    /// Whether the list column admits nulls.
    pub fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// The element (item) field.
    pub fn item(&self) -> &AnyField {
        self.parts().3
    }

    /// The list's metadata [`Headers`].
    pub fn metadata(&self) -> &Headers {
        self.inner.metadata()
    }

    /// The typed [`ListType`] descriptor (its element field).
    pub fn data_type(&self) -> ListType {
        ListType::new(self.item().clone())
    }

    /// This schema as an [`AnyField`] (its `List` form) — the erased, recursive field.
    pub fn as_any_field(&self) -> &AnyField {
        &self.inner
    }

    /// Builds a list schema from an [`AnyField`], or `None` if it is not a list field.
    pub fn from_any_field(field: AnyField) -> Option<Self> {
        field.is_list().then_some(Self { inner: field })
    }

    // ---- ergonomic immutable updates: `with_*` builders ----------------------------------

    fn parts(&self) -> (&str, bool, &Headers, &AnyField) {
        match &self.inner {
            AnyField::List {
                name,
                nullable,
                metadata,
                item,
            } => (name, *nullable, metadata, item),
            // A `ListField` is always a list-shaped `AnyField` by construction.
            AnyField::Leaf(_) | AnyField::Struct { .. } | AnyField::Map { .. } => {
                unreachable!("ListField always wraps AnyField::List")
            }
        }
    }

    /// A fresh list schema renamed to `name`.
    pub fn with_name(&self, name: &str) -> Self {
        let (_, nullable, metadata, item) = self.parts();
        Self {
            inner: AnyField::List {
                name: name.to_string(),
                nullable,
                metadata: metadata.clone(),
                item: Box::new(item.clone()),
            },
        }
    }

    /// A fresh list schema with `nullable` set.
    pub fn with_nullable(&self, nullable: bool) -> Self {
        let (name, _, metadata, item) = self.parts();
        Self {
            inner: AnyField::List {
                name: name.to_string(),
                nullable,
                metadata: metadata.clone(),
                item: Box::new(item.clone()),
            },
        }
    }

    /// A fresh list schema with a new element (item) field.
    pub fn with_item(&self, item: AnyField) -> Self {
        let (name, nullable, metadata, _) = self.parts();
        Self {
            inner: AnyField::List {
                name: name.to_string(),
                nullable,
                metadata: metadata.clone(),
                item: Box::new(item),
            },
        }
    }

    /// A fresh list schema with the given metadata [`Headers`] attached (replacing any existing).
    pub fn with_metadata(&self, metadata: Headers) -> Self {
        let (name, nullable, _, item) = self.parts();
        Self {
            inner: AnyField::List {
                name: name.to_string(),
                nullable,
                metadata,
                item: Box::new(item.clone()),
            },
        }
    }

    /// A fresh list schema with one extra `key = value` metadata entry.
    pub fn with_metadata_entry(&self, key: &str, value: &str) -> Self {
        let (name, nullable, metadata, item) = self.parts();
        let mut metadata = metadata.clone();
        metadata.insert(key, value);
        Self {
            inner: AnyField::List {
                name: name.to_string(),
                nullable,
                metadata,
                item: Box::new(item.clone()),
            },
        }
    }

    /// An explicit copy (the cross-language clone).
    pub fn copy(&self) -> Self {
        self.clone()
    }

    // ---- Arrow interop: a list schema is an Arrow `List` Field --------------------------

    /// This list as an Arrow [`Field`](arrow_schema::Field) of `List` type (feature `arrow`) — name,
    /// nullability, metadata, and the recursively-mapped item field (via [`AnyField::to_arrow`]).
    #[cfg(feature = "arrow")]
    pub fn to_arrow_field(&self) -> arrow_schema::Field {
        self.inner.to_arrow()
    }

    /// Builds a list schema from an Arrow [`Field`](arrow_schema::Field) of `List` type (feature
    /// `arrow`), or `None` if the field is not a list (or the item type is not modeled).
    #[cfg(feature = "arrow")]
    pub fn from_arrow_field(field: &arrow_schema::Field) -> Option<Self> {
        Self::from_any_field(AnyField::from_arrow(field)?)
    }
}

impl FieldType for ListField {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn type_name(&self) -> &'static str {
        "list"
    }

    fn byte_width(&self) -> usize {
        0
    }

    fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    fn type_id(&self) -> DataTypeId {
        DataTypeId::List
    }
}

impl From<ListField> for AnyField {
    fn from(field: ListField) -> Self {
        field.inner
    }
}
