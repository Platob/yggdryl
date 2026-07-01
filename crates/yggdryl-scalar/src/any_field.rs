//! The [`AnyField`] dynamic field.

use yggdryl_schema::{Field, Metadata};

use crate::{AnyType, AnyValue};

/// A field of any type, resolved at run time — the child-field node of a
/// [`StructType`](crate::StructType). It pairs a `name` with an [`AnyType`], a
/// nullability flag and optional [`Metadata`], and is a [`Field`] over the dynamic
/// [`AnyValue`]. The `with_*` / [`copy`](AnyField::copy) updates are non-mutating.
///
/// ```
/// use yggdryl_scalar::{AnyField, AnyType};
/// use yggdryl_schema::{DataType, DataTypeId};
///
/// let field = AnyField::new("id", AnyType::primitive(DataTypeId::Int64));
/// assert_eq!(field.name(), "id");
/// assert_eq!(field.any_type().type_id(), DataTypeId::Int64);
/// assert!(!field.nullable());
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AnyField {
    name: String,
    dtype: AnyType,
    nullable: bool,
    metadata: Option<Metadata>,
}

impl AnyField {
    /// A non-nullable field named `name` of type `dtype`, with no metadata.
    pub fn new(name: impl Into<String>, dtype: AnyType) -> Self {
        Self {
            name: name.into(),
            dtype,
            nullable: false,
            metadata: None,
        }
    }

    /// The field from its explicit parts.
    pub fn from_parts(
        name: String,
        dtype: AnyType,
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

    /// The field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The field's dynamic type.
    pub fn any_type(&self) -> &AnyType {
        &self.dtype
    }

    /// Whether this field admits null values.
    pub fn nullable(&self) -> bool {
        self.nullable
    }

    /// The field's metadata, if any.
    pub fn metadata(&self) -> Option<&Metadata> {
        self.metadata.as_ref()
    }

    /// A copy with the given parts overridden; omitted parts come from `self`.
    pub fn copy(
        &self,
        name: Option<String>,
        dtype: Option<AnyType>,
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

impl Field<AnyValue> for AnyField {
    type DType = AnyType;

    fn name(&self) -> &str {
        &self.name
    }

    fn dtype(&self) -> &AnyType {
        &self.dtype
    }

    fn nullable(&self) -> bool {
        self.nullable
    }

    fn metadata(&self) -> Option<&Metadata> {
        self.metadata.as_ref()
    }
}
