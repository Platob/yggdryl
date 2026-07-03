//! The [`SerieField`] field.

use crate::{Field, FieldFactory, TypedField};
use yggdryl_dtype::{DataError, DataType, SerieType, TypedDataType};

/// A nullable `list` field: a name paired with the
/// [`SerieType`](yggdryl_dtype::SerieType) of the value type `D`.
///
/// It carries both trait layers: the raw [`Field`] surface (its associated
/// [`DataType`](Field::DataType) is [`SerieType<D>`](SerieType)), and the typed
/// [`TypedField<SerieType<D>, Vec<T>>`] whenever the value type has a
/// [`TypedDataType<T>`] codec.
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{DataType, Int64Type, Serie, SerieType};
/// use yggdryl_field::{Field, FieldFactory, SerieField};
///
/// let scores = SerieField::<Int64Type>::new("scores", true);
/// assert_eq!(scores.name(), "scores");
/// assert_eq!(scores.data_type().name(), "list");
/// assert_eq!(scores.data_type().value_type().name(), "int64");
/// assert!(scores.is_nullable());
/// assert_eq!(SerieField::from_arrow(&scores.to_arrow()).unwrap(), scores);
/// assert_eq!(SerieType::new(Int64Type).field("scores", true), scores);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerieField<D> {
    name: String,
    data_type: SerieType<D>,
    nullable: bool,
}

impl<D: DataType + Default> SerieField<D> {
    /// A `list` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: SerieType::default(),
            nullable,
        }
    }
}

impl<D: DataType> Field for SerieField<D> {
    type DataType = SerieType<D>;
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &SerieType<D> {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        let data_type = SerieType::from_arrow(field.data_type())?;
        crate::field::validate_field_metadata(field, "SerieType")?;
        Ok(Self {
            name: field.name().to_string(),
            data_type,
            nullable: field.is_nullable(),
        })
    }
}

impl<T, D: TypedDataType<T>> TypedField<SerieType<D>, Vec<T>> for SerieField<D> {}

impl<T, D: TypedDataType<T> + Default> FieldFactory<Vec<T>> for SerieType<D> {
    type Field = SerieField<D>;
    fn field(&self, name: impl Into<String>, nullable: bool) -> SerieField<D> {
        SerieField::new(name, nullable)
    }
}
