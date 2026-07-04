//! The [`TypedSerieField`] field.

use crate::{Field, FieldFactory, TypedField};
use yggdryl_dtype::{DataError, DataType, TypedDataType, TypedSerieType};

/// A nullable, statically-typed `list` field: a name paired with the
/// [`TypedSerieType`](yggdryl_dtype::TypedSerieType) of the value type `D`.
///
/// It is the typed counterpart of the dynamic [`SerieField`](crate::SerieField):
/// it carries both trait layers — the raw [`Field`] surface (its associated
/// [`DataType`](Field::DataType) is [`TypedSerieType<D>`](TypedSerieType)) and the
/// typed [`TypedField<TypedSerieType<D>, Vec<T>>`] whenever the value type has a
/// [`TypedDataType<T>`] codec.
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{DataType, Int64Type, TypedSerie, TypedSerieType};
/// use yggdryl_field::{Field, FieldFactory, TypedSerieField};
///
/// let scores = TypedSerieField::<Int64Type>::new("scores", true);
/// assert_eq!(scores.name(), "scores");
/// assert_eq!(scores.data_type().name(), "list");
/// assert_eq!(scores.data_type().value_type().name(), "int64");
/// assert!(scores.is_nullable());
/// assert_eq!(TypedSerieField::from_arrow(&scores.to_arrow()).unwrap(), scores);
/// assert_eq!(TypedSerieType::new(Int64Type).field("scores", true), scores);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedSerieField<D> {
    name: String,
    data_type: TypedSerieType<D>,
    nullable: bool,
}

impl<D: DataType + Default> TypedSerieField<D> {
    /// A `list` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: TypedSerieType::default(),
            nullable,
        }
    }
}

impl<D: DataType> Field for TypedSerieField<D> {
    type DataType = TypedSerieType<D>;
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &TypedSerieType<D> {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        let data_type = TypedSerieType::from_arrow(field.data_type())?;
        crate::field::validate_field_metadata(field, "TypedSerieType")?;
        Ok(Self {
            name: field.name().to_string(),
            data_type,
            nullable: field.is_nullable(),
        })
    }
}

impl<T, D: TypedDataType<T>> TypedField<TypedSerieType<D>, Vec<T>> for TypedSerieField<D> {}

impl<T, D: TypedDataType<T> + Default> FieldFactory<Vec<T>> for TypedSerieType<D> {
    type Field = TypedSerieField<D>;
    fn field(&self, name: impl Into<String>, nullable: bool) -> TypedSerieField<D> {
        TypedSerieField::new(name, nullable)
    }
}
