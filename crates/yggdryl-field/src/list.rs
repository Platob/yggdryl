//! The [`ListField`] field.

use crate::{Field, FieldFactory, TypedField};
use yggdryl_dtype::{DataError, DataType, ListType, TypedDataType};

/// A nullable `list` field: a name paired with the
/// [`ListType`](yggdryl_dtype::ListType) of the value type `D`.
///
/// It carries both trait layers: the raw [`Field<ListType<D>>`](Field) surface, and
/// the typed [`TypedField<ListType<D>, Vec<T>>`] whenever the value type has a
/// [`TypedDataType<T>`] codec.
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{DataType, Int64Type, List, ListType};
/// use yggdryl_field::{Field, FieldFactory, ListField};
///
/// let scores = ListField::<Int64Type>::new("scores", true);
/// assert_eq!(scores.name(), "scores");
/// assert_eq!(scores.data_type().name(), "list");
/// assert_eq!(scores.data_type().value_type().name(), "int64");
/// assert!(scores.is_nullable());
/// assert_eq!(ListField::from_arrow(&scores.to_arrow()).unwrap(), scores);
/// assert_eq!(ListType::new(Int64Type).field("scores", true), scores);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListField<D> {
    name: String,
    data_type: ListType<D>,
    nullable: bool,
}

impl<D: DataType + Default> ListField<D> {
    /// A `list` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: ListType::default(),
            nullable,
        }
    }
}

impl<D: DataType> Field<ListType<D>> for ListField<D> {
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &ListType<D> {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        let data_type = ListType::from_arrow(field.data_type())?;
        crate::field::validate_field_metadata(field, "ListType")?;
        Ok(Self {
            name: field.name().to_string(),
            data_type,
            nullable: field.is_nullable(),
        })
    }
}

impl<T, D: TypedDataType<T>> TypedField<ListType<D>, Vec<T>> for ListField<D> {}

impl<T, D: TypedDataType<T> + Default> FieldFactory<Vec<T>> for ListType<D> {
    type Field = ListField<D>;
    fn field(&self, name: impl Into<String>, nullable: bool) -> ListField<D> {
        ListField::new(name, nullable)
    }
}
