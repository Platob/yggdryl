//! The [`StructField`] — a field of [`StructType`].

use crate::dtype::StructType;
use crate::field::{AnyField, Field, Metadata};
use crate::value::Struct;

/// A field whose data type is a [`StructType`] — a named, nullable struct with
/// optional [`Metadata`]. It is a [`Field`] over the [`Struct`] value. Because a
/// struct field is *the* recursive schema node, an Arrow schema is simply a
/// `StructField`. The `with_*` / [`copy`](StructField::copy) updates are
/// non-mutating.
///
/// ```
/// use yggdryl_schema::{AnyField, AnyType, DataTypeId, Field, StructField};
///
/// let schema = StructField::new(
///     "record",
///     vec![
///         AnyField::new("id", AnyType::primitive(DataTypeId::Int64)),
///         AnyField::new("name", AnyType::primitive(DataTypeId::Utf8)),
///     ],
/// );
/// assert_eq!(schema.name(), "record");
/// assert_eq!(schema.dtype().len(), 2);
/// assert_eq!(schema.dtype().field_by("id").map(AnyField::name), Some("id"));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct StructField {
    name: String,
    dtype: StructType,
    nullable: bool,
    metadata: Option<Metadata>,
}

impl StructField {
    /// A non-nullable struct field named `name` over `fields`, with no metadata.
    pub fn new(name: impl Into<String>, fields: Vec<AnyField>) -> Self {
        Self {
            name: name.into(),
            dtype: StructType::new(fields),
            nullable: false,
            metadata: None,
        }
    }

    /// The field from its explicit parts.
    pub fn from_parts(
        name: String,
        dtype: StructType,
        nullable: bool,
        metadata: Option<Metadata>,
    ) -> Self {
        Self {
            name,
            dtype,
            nullable,
            metadata,
        }
    }

    /// A copy with the given parts overridden; omitted parts come from `self`.
    pub fn copy(
        &self,
        name: Option<String>,
        dtype: Option<StructType>,
        nullable: Option<bool>,
        metadata: Option<Option<Metadata>>,
    ) -> Self {
        Self {
            name: name.unwrap_or_else(|| self.name.clone()),
            dtype: dtype.unwrap_or_else(|| self.dtype.clone()),
            nullable: nullable.unwrap_or(self.nullable),
            metadata: metadata.unwrap_or_else(|| self.metadata.clone()),
        }
    }

    /// A copy renamed to `name`.
    pub fn with_name(&self, name: String) -> Self {
        self.copy(Some(name), None, None, None)
    }

    /// A copy with the nullability set to `nullable`.
    pub fn with_nullable(&self, nullable: bool) -> Self {
        self.copy(None, None, Some(nullable), None)
    }

    /// A copy carrying `metadata`.
    pub fn with_metadata(&self, metadata: Metadata) -> Self {
        self.copy(None, None, None, Some(Some(metadata)))
    }

    /// A copy with the metadata cleared.
    pub fn without_metadata(&self) -> Self {
        self.copy(None, None, None, Some(None))
    }
}

impl Field<Struct> for StructField {
    type DType = StructType;

    fn name(&self) -> &str {
        &self.name
    }

    fn dtype(&self) -> &StructType {
        &self.dtype
    }

    fn nullable(&self) -> bool {
        self.nullable
    }

    fn metadata(&self) -> Option<&Metadata> {
        self.metadata.as_ref()
    }
}
