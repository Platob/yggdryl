//! [`StructField`] — the **centralized struct schema**: a validated struct-shaped
//! [`AnyField`](crate::io::AnyField) (its children hold the ordered child fields), which maps to
//! **both** an Arrow [`Field`](arrow_schema::Field) (as a `Struct` column) *and* an Arrow
//! [`Schema`](arrow_schema::Schema) (its children as a top-level schema). This is the one place a
//! struct's shape is described; `StructType`, `StructScalar`, and `StructSerie` take their schema
//! from here.

use super::StructType;
use crate::io::{AnyField, DataTypeId, FieldType, Headers};

/// A **named, nullable struct** column descriptor — the schema of a struct. It is a thin, validated
/// wrapper over an [`AnyField`] (always the `Struct` variant), so the recursive Arrow mapping lives
/// once on `AnyField` and this type adds only the struct-specific surface (the `Schema` mapping,
/// `with_*` builders, field lookups).
///
/// ```
/// use yggdryl_core::io::FieldType;
/// use yggdryl_core::io::fixed::{Field, PrimitiveType};
/// use yggdryl_core::io::AnyField;
/// use yggdryl_core::io::nested::StructField;
///
/// let schema = StructField::new(
///     "point",
///     vec![
///         AnyField::leaf(Field::new("x", &PrimitiveType::<f64>::new(), false)),
///         AnyField::leaf(Field::new("y", &PrimitiveType::<f64>::new(), false)),
///     ],
///     true,
/// );
/// assert_eq!(schema.name(), "point");
/// assert_eq!(schema.type_name(), "struct");
/// assert!(schema.is_struct() && schema.nullable());
/// assert_eq!(schema.num_fields(), 2);
/// assert_eq!(schema.field(1).unwrap().name(), "y");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StructField {
    inner: AnyField,
}

impl StructField {
    /// A struct schema from a name, its ordered child fields, and its nullability (empty metadata).
    pub fn new(name: &str, children: Vec<AnyField>, nullable: bool) -> Self {
        Self {
            inner: AnyField::struct_(name, children, nullable),
        }
    }

    /// The struct's name.
    pub fn name(&self) -> &str {
        self.inner.name()
    }

    /// Whether the struct column admits nulls.
    pub fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// The ordered child fields.
    pub fn fields(&self) -> &[AnyField] {
        self.inner.children()
    }

    /// The number of child fields.
    pub fn num_fields(&self) -> usize {
        self.inner.children().len()
    }

    /// The child field at `index`, or `None` if out of range.
    pub fn field(&self, index: usize) -> Option<&AnyField> {
        self.inner.children().get(index)
    }

    /// The child field named `name` (first match), or `None`.
    pub fn field_named(&self, name: &str) -> Option<&AnyField> {
        self.inner.children().iter().find(|f| f.name() == name)
    }

    /// The 0-based index of the child field named `name` (first match), or `None`.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.inner.children().iter().position(|f| f.name() == name)
    }

    /// The struct's metadata [`Headers`].
    pub fn metadata(&self) -> &Headers {
        self.inner.metadata()
    }

    /// The typed [`StructType`] descriptor (its child fields).
    pub fn data_type(&self) -> StructType {
        StructType::new(self.inner.children().to_vec())
    }

    /// This schema as an [`AnyField`] (its `Struct` form) — the erased, recursive field.
    pub fn as_any_field(&self) -> &AnyField {
        &self.inner
    }

    /// Builds a struct schema from an [`AnyField`], or `None` if it is not a struct field.
    pub fn from_any_field(field: AnyField) -> Option<Self> {
        field.is_struct().then_some(Self { inner: field })
    }

    // ---- ergonomic immutable updates: `with_*` builders ----------------------------------

    fn parts(&self) -> (&str, bool, &Headers, &[AnyField]) {
        match &self.inner {
            AnyField::Struct {
                name,
                nullable,
                metadata,
                children,
            } => (name, *nullable, metadata, children),
            // A `StructField` is always a struct-shaped `AnyField` by construction.
            AnyField::Leaf(_) | AnyField::List { .. } | AnyField::Map { .. } => {
                unreachable!("StructField always wraps AnyField::Struct")
            }
        }
    }

    /// A fresh struct schema renamed to `name`.
    pub fn with_name(&self, name: &str) -> Self {
        let (_, nullable, metadata, children) = self.parts();
        Self {
            inner: AnyField::Struct {
                name: name.to_string(),
                nullable,
                metadata: metadata.clone(),
                children: children.to_vec(),
            },
        }
    }

    /// A fresh struct schema with `nullable` set.
    pub fn with_nullable(&self, nullable: bool) -> Self {
        let (name, _, metadata, children) = self.parts();
        Self {
            inner: AnyField::Struct {
                name: name.to_string(),
                nullable,
                metadata: metadata.clone(),
                children: children.to_vec(),
            },
        }
    }

    /// A fresh struct schema with one more child field appended.
    pub fn with_field(&self, child: AnyField) -> Self {
        let (name, nullable, metadata, children) = self.parts();
        let mut children = children.to_vec();
        children.push(child);
        Self {
            inner: AnyField::Struct {
                name: name.to_string(),
                nullable,
                metadata: metadata.clone(),
                children,
            },
        }
    }

    /// A fresh struct schema with the given metadata [`Headers`] attached (replacing any existing).
    pub fn with_metadata(&self, metadata: Headers) -> Self {
        let (name, nullable, _, children) = self.parts();
        Self {
            inner: AnyField::Struct {
                name: name.to_string(),
                nullable,
                metadata,
                children: children.to_vec(),
            },
        }
    }

    /// A fresh struct schema with one extra `key = value` metadata entry.
    pub fn with_metadata_entry(&self, key: &str, value: &str) -> Self {
        let (name, nullable, metadata, children) = self.parts();
        let mut metadata = metadata.clone();
        metadata.insert(key, value);
        Self {
            inner: AnyField::Struct {
                name: name.to_string(),
                nullable,
                metadata,
                children: children.to_vec(),
            },
        }
    }

    /// An explicit copy (the cross-language clone).
    pub fn copy(&self) -> Self {
        self.clone()
    }

    // ---- Arrow interop: a struct schema is BOTH an Arrow Field and an Arrow Schema -------

    /// This struct as an Arrow [`Field`](arrow_schema::Field) of `Struct` type (feature `arrow`) —
    /// name, nullability, metadata, and the recursively-mapped child fields (via [`AnyField::to_arrow`]).
    #[cfg(feature = "arrow")]
    pub fn to_arrow_field(&self) -> arrow_schema::Field {
        self.inner.to_arrow()
    }

    /// This struct's **children as a top-level Arrow [`Schema`](arrow_schema::Schema)** (feature
    /// `arrow`) — the schema of a [`RecordBatch`](arrow_array::RecordBatch) whose columns are the
    /// struct's fields. The struct's own name/nullability are not part of a schema; its metadata
    /// becomes the schema metadata.
    #[cfg(feature = "arrow")]
    pub fn to_arrow_schema(&self) -> arrow_schema::Schema {
        let fields: Vec<arrow_schema::Field> = self
            .inner
            .children()
            .iter()
            .map(AnyField::to_arrow)
            .collect();
        arrow_schema::Schema::new_with_metadata(fields, self.inner.metadata().to_arrow_metadata())
    }

    /// Builds a struct schema from an Arrow [`Field`](arrow_schema::Field) of `Struct` type (feature
    /// `arrow`), or `None` if the field is not a struct (or a child type is not modeled).
    #[cfg(feature = "arrow")]
    pub fn from_arrow_field(field: &arrow_schema::Field) -> Option<Self> {
        Self::from_any_field(AnyField::from_arrow(field)?)
    }

    /// Builds a struct schema from a top-level Arrow [`Schema`](arrow_schema::Schema) (feature
    /// `arrow`) — the inverse of [`to_arrow_schema`](StructField::to_arrow_schema). The result is a
    /// non-nullable, unnamed (`name = ""`) struct whose children are the schema's fields; the schema
    /// metadata becomes the struct metadata. `None` if a field's type is not modeled.
    #[cfg(feature = "arrow")]
    pub fn from_arrow_schema(schema: &arrow_schema::Schema) -> Option<Self> {
        let children = schema
            .fields()
            .iter()
            .map(|child| AnyField::from_arrow(child))
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            inner: AnyField::Struct {
                name: String::new(),
                nullable: false,
                metadata: Headers::from_arrow_metadata(schema.metadata()),
                children,
            },
        })
    }
}

impl FieldType for StructField {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn type_name(&self) -> &'static str {
        "struct"
    }

    fn byte_width(&self) -> usize {
        0
    }

    fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    fn type_id(&self) -> DataTypeId {
        DataTypeId::Struct
    }
}

impl From<StructField> for AnyField {
    fn from(field: StructField) -> Self {
        field.inner
    }
}
