//! The [`TypedOptionalField`] field.

use crate::{Field, FieldFactory, TypedField};
use yggdryl_dtype::{DataError, DataType, TypedDataType, TypedOptionalType};

/// A nullable, statically-typed `optional` field: a name paired with the logical
/// [`TypedOptionalType`](yggdryl_dtype::TypedOptionalType) of the value type `D`.
///
/// It is the typed counterpart of the dynamic
/// [`OptionalField`](crate::OptionalField): it carries both trait layers — the raw
/// [`Field`] surface (its associated [`DataType`](Field::DataType) is
/// [`TypedOptionalType<D>`](TypedOptionalType)) and the typed
/// [`TypedField<TypedOptionalType<D>, T>`] whenever the value type has a
/// [`TypedDataType<T>`] codec.
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{DataType, Int64Type, TypedOptional, TypedOptionalType};
/// use yggdryl_field::{Field, FieldFactory, TypedOptionalField};
///
/// let score = TypedOptionalField::<Int64Type>::new("score", true);
/// assert_eq!(score.name(), "score");
/// assert_eq!(score.data_type().name(), "optional");
/// assert_eq!(score.data_type().value_type().name(), "int64");
/// assert!(score.is_nullable());
/// assert_eq!(TypedOptionalField::from_arrow(&score.to_arrow()).unwrap(), score);
/// assert_eq!(TypedOptionalType::new(Int64Type).field("score", true), score);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedOptionalField<D> {
    name: String,
    data_type: TypedOptionalType<D>,
    nullable: bool,
}

impl<D: DataType + Default> TypedOptionalField<D> {
    /// An `optional` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: TypedOptionalType::default(),
            nullable,
        }
    }
}

impl<D: DataType> Field for TypedOptionalField<D> {
    type DataType = TypedOptionalType<D>;
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &TypedOptionalType<D> {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        let data_type = TypedOptionalType::from_arrow(field.data_type())?;
        crate::field::validate_field_metadata(field, "TypedOptionalType")?;
        Ok(Self {
            name: field.name().to_string(),
            data_type,
            nullable: field.is_nullable(),
        })
    }
}

impl<T, D: TypedDataType<T>> TypedField<TypedOptionalType<D>, T> for TypedOptionalField<D> {}

impl<T, D: TypedDataType<T> + Default> FieldFactory<T> for TypedOptionalType<D> {
    type Field = TypedOptionalField<D>;
    fn field(&self, name: impl Into<String>, nullable: bool) -> TypedOptionalField<D> {
        TypedOptionalField::new(name, nullable)
    }
}
