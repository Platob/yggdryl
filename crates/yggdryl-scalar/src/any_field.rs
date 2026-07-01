//! The [`AnyField`] dynamic field.

use yggdryl_schema::{Field, Metadata};

use crate::{Any, AnyType};

/// A field of any type, resolved at run time — the child-field node of a
/// [`StructType`](crate::StructType). It pairs a `name` with an [`AnyType`], a
/// nullability flag and optional [`Metadata`], and is a [`Field`] over the dynamic
/// [`Any`]. The `with_*` / [`copy`](AnyField::copy) updates are non-mutating.
///
/// ```
/// use yggdryl_scalar::{AnyField, DataTypeId};
///
/// // A `DataTypeId` redirects to the correct `AnyType`…
/// let field = AnyField::new("id", DataTypeId::Int64);
/// assert_eq!(field.name(), "id");
/// assert_eq!(field.any_type().as_primitive(), Some(DataTypeId::Int64));
/// // …or use a typed constructor directly.
/// assert_eq!(AnyField::int64("id"), field);
/// assert!(!field.nullable());
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AnyField {
    name: String,
    dtype: AnyType,
    nullable: bool,
    metadata: Option<Metadata>,
}

/// Generates the typed field constructors (e.g. [`AnyField::int64`]).
macro_rules! primitive_field_ctors {
    ($($method:ident),+ $(,)?) => {$(
        #[doc = concat!("A non-nullable `", stringify!($method), "` field named `name`.")]
        pub fn $method(name: impl Into<String>) -> Self {
            Self::new(name, AnyType::$method())
        }
    )+};
}

impl AnyField {
    /// A non-nullable field named `name` of type `dtype` (anything that converts into
    /// an [`AnyType`] — an [`AnyType`], a [`DataTypeId`], or a [`StructType`](crate::StructType)),
    /// with no metadata.
    pub fn new(name: impl Into<String>, dtype: impl Into<AnyType>) -> Self {
        Self {
            name: name.into(),
            dtype: dtype.into(),
            nullable: false,
            metadata: None,
        }
    }

    primitive_field_ctors! {
        null, boolean, int8, int16, int32, int64, int128, int256, uint8, uint16, uint32, uint64,
        uint128, uint256, utf8,
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

impl Field<Any> for AnyField {
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
