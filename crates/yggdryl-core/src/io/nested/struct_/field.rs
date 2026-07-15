//! [`StructField`] — the **centralized struct schema**: a named, nullable struct column descriptor
//! that holds the ordered child fields, and maps to **both** an Arrow [`Field`](arrow_schema::Field)
//! (as a `Struct` column) *and* an Arrow [`Schema`](arrow_schema::Schema) (its children as a
//! top-level schema). This is the one place a struct's shape is described; `StructType`,
//! `StructScalar`, and `StructSerie` all take their schema from here.

use super::StructType;
use crate::io::nested::ColumnField;
use crate::io::{DataTypeId, FieldType, Headers};

/// A **named, nullable struct** column descriptor — the schema of a struct: its `name`, whether it
/// admits nulls, its ordered child [`ColumnField`]s, and [`Headers`] metadata.
///
/// It is the recursive, nested peer of the flat [`Field`](crate::io::fixed::Field), and the
/// **single source of truth** for a struct's shape: it maps to an Arrow `Field` of `Struct` type
/// ([`to_arrow_field`](StructField::to_arrow_field)) *and*, treating its children as a top-level
/// schema, to an Arrow [`Schema`](arrow_schema::Schema)
/// ([`to_arrow_schema`](StructField::to_arrow_schema)) — the natural bridge for
/// [`StructSerie`](super::StructSerie) ↔ `RecordBatch`.
///
/// ```
/// use yggdryl_core::io::FieldType;
/// use yggdryl_core::io::fixed::{Field, PrimitiveType};
/// use yggdryl_core::io::nested::{ColumnField, StructField};
///
/// let schema = StructField::new(
///     "point",
///     vec![
///         ColumnField::leaf(Field::new("x", &PrimitiveType::<f64>::new(), false)),
///         ColumnField::leaf(Field::new("y", &PrimitiveType::<f64>::new(), false)),
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
    name: String,
    nullable: bool,
    children: Vec<ColumnField>,
    metadata: Headers,
}

impl StructField {
    /// A struct schema from a name, its ordered child fields, and its nullability (empty metadata).
    pub fn new(name: &str, children: Vec<ColumnField>, nullable: bool) -> Self {
        Self {
            name: name.to_string(),
            nullable,
            children,
            metadata: Headers::new(),
        }
    }

    /// The struct's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the struct column admits nulls.
    pub fn nullable(&self) -> bool {
        self.nullable
    }

    /// The ordered child fields.
    pub fn fields(&self) -> &[ColumnField] {
        &self.children
    }

    /// The number of child fields.
    pub fn num_fields(&self) -> usize {
        self.children.len()
    }

    /// The child field at `index`, or `None` if out of range.
    pub fn field(&self, index: usize) -> Option<&ColumnField> {
        self.children.get(index)
    }

    /// The child field named `name` (first match), or `None`.
    pub fn field_named(&self, name: &str) -> Option<&ColumnField> {
        self.children.iter().find(|f| f.name() == name)
    }

    /// The 0-based index of the child field named `name` (first match), or `None`.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.children.iter().position(|f| f.name() == name)
    }

    /// The struct's metadata [`Headers`].
    pub fn metadata(&self) -> &Headers {
        &self.metadata
    }

    /// The typed [`StructType`] descriptor (its child fields).
    pub fn data_type(&self) -> StructType {
        StructType::new(self.children.clone())
    }

    // ---- ergonomic immutable updates: `with_*` builders ----------------------------------

    /// A fresh struct schema renamed to `name`.
    pub fn with_name(&self, name: &str) -> Self {
        let mut next = self.clone();
        next.name = name.to_string();
        next
    }

    /// A fresh struct schema with `nullable` set.
    pub fn with_nullable(&self, nullable: bool) -> Self {
        let mut next = self.clone();
        next.nullable = nullable;
        next
    }

    /// A fresh struct schema with one more child field appended.
    pub fn with_field(&self, child: ColumnField) -> Self {
        let mut next = self.clone();
        next.children.push(child);
        next
    }

    /// A fresh struct schema with the given metadata [`Headers`] attached (replacing any existing).
    pub fn with_metadata(&self, metadata: Headers) -> Self {
        let mut next = self.clone();
        next.metadata = metadata;
        next
    }

    /// A fresh struct schema with one extra `key = value` metadata entry.
    pub fn with_metadata_entry(&self, key: &str, value: &str) -> Self {
        let mut next = self.clone();
        next.metadata.insert(key, value);
        next
    }

    /// An explicit copy (the cross-language clone).
    pub fn copy(&self) -> Self {
        self.clone()
    }

    // ---- Arrow interop: a struct schema is BOTH an Arrow Field and an Arrow Schema -------

    /// This struct as an Arrow [`Field`](arrow_schema::Field) of `Struct` type (feature `arrow`) —
    /// name, nullability, metadata, and the recursively-mapped child fields.
    #[cfg(feature = "arrow")]
    pub fn to_arrow_field(&self) -> arrow_schema::Field {
        let data_type = crate::io::DataType::to_arrow(&self.data_type());
        arrow_schema::Field::new(&self.name, data_type, self.nullable)
            .with_metadata(self.metadata.to_arrow_metadata())
    }

    /// This struct's **children as a top-level Arrow [`Schema`](arrow_schema::Schema)** (feature
    /// `arrow`) — the schema of a [`RecordBatch`](arrow_array::RecordBatch) whose columns are the
    /// struct's fields. The struct's own name/nullability are not part of a schema; its metadata
    /// becomes the schema metadata.
    #[cfg(feature = "arrow")]
    pub fn to_arrow_schema(&self) -> arrow_schema::Schema {
        let fields: Vec<arrow_schema::Field> =
            self.children.iter().map(ColumnField::to_arrow).collect();
        arrow_schema::Schema::new_with_metadata(fields, self.metadata.to_arrow_metadata())
    }

    /// Builds a struct schema from an Arrow [`Field`](arrow_schema::Field) of `Struct` type (feature
    /// `arrow`), recovering each child recursively, or `None` if the field is not a struct (or a
    /// child type is not modeled).
    #[cfg(feature = "arrow")]
    pub fn from_arrow_field(field: &arrow_schema::Field) -> Option<Self> {
        let arrow_schema::DataType::Struct(children) = field.data_type() else {
            return None;
        };
        let children = children
            .iter()
            .map(|child| ColumnField::from_arrow(child))
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            name: field.name().clone(),
            nullable: field.is_nullable(),
            children,
            metadata: Headers::from_arrow_metadata(field.metadata()),
        })
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
            .map(|child| ColumnField::from_arrow(child))
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            name: String::new(),
            nullable: false,
            children,
            metadata: Headers::from_arrow_metadata(schema.metadata()),
        })
    }
}

impl FieldType for StructField {
    fn name(&self) -> &str {
        &self.name
    }

    fn type_name(&self) -> &'static str {
        "struct"
    }

    fn byte_width(&self) -> usize {
        0
    }

    fn nullable(&self) -> bool {
        self.nullable
    }

    fn type_id(&self) -> DataTypeId {
        DataTypeId::Struct
    }
}
