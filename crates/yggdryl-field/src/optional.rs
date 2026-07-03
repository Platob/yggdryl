//! The [`OptionalField`] field.

use crate::{Field, FieldFactory, TypedField};
use yggdryl_dtype::{DataError, DataType, OptionalType, TypedDataType};

/// A nullable `optional` field: a name paired with the logical
/// [`OptionalType`](yggdryl_dtype::OptionalType) of the value type `D`.
///
/// It carries both trait layers: the raw [`Field`] surface (its associated
/// [`DataType`](Field::DataType) is [`OptionalType<D>`](OptionalType)), and the typed
/// [`TypedField<OptionalType<D>, T>`] whenever the value type has a
/// [`TypedDataType<T>`] codec.
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{DataType, Int64Type, Optional, OptionalType};
/// use yggdryl_field::{Field, FieldFactory, OptionalField};
///
/// let score = OptionalField::<Int64Type>::new("score", true);
/// assert_eq!(score.name(), "score");
/// assert_eq!(score.data_type().name(), "optional");
/// assert_eq!(score.data_type().value_type().name(), "int64");
/// assert!(score.is_nullable());
/// assert_eq!(OptionalField::from_arrow(&score.to_arrow()).unwrap(), score);
/// assert_eq!(OptionalType::new(Int64Type).field("score", true), score);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionalField<D> {
    name: String,
    data_type: OptionalType<D>,
    nullable: bool,
}

impl<D: DataType + Default> OptionalField<D> {
    /// An `optional` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: OptionalType::default(),
            nullable,
        }
    }
}

impl<D: DataType> Field for OptionalField<D> {
    type DataType = OptionalType<D>;
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &OptionalType<D> {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        let data_type = OptionalType::from_arrow(field.data_type())?;
        crate::field::validate_field_metadata(field, "OptionalType")?;
        Ok(Self {
            name: field.name().to_string(),
            data_type,
            nullable: field.is_nullable(),
        })
    }
}

impl<T, D: TypedDataType<T>> TypedField<OptionalType<D>, T> for OptionalField<D> {}

impl<T, D: TypedDataType<T> + Default> FieldFactory<T> for OptionalType<D> {
    type Field = OptionalField<D>;
    fn field(&self, name: impl Into<String>, nullable: bool) -> OptionalField<D> {
        OptionalField::new(name, nullable)
    }
}
