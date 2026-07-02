//! The [`ListField`] field of the [`ListType`](super::ListType) data type.

use super::ListType;
use crate::{DataError, DataType, Field, RawDataType, RawField};

/// A nullable `list` field: a name paired with the [`ListType`] of the value type
/// `D`.
///
/// It carries both trait layers: the raw [`RawField<ListType<D>>`] surface, and
/// the typed [`Field<Vec<T>>`] whenever the value type has a [`DataType<T>`] codec.
///
/// ```
/// use yggdryl_data::{Int64, ListField, RawDataType, RawField, RawList};
///
/// let scores = ListField::<Int64>::new("scores", true);
/// assert_eq!(scores.name(), "scores");
/// assert_eq!(scores.data_type().name(), "list");
/// assert_eq!(scores.data_type().value_type().name(), "int64");
/// assert!(scores.is_nullable());
/// assert_eq!(ListField::from_arrow(&scores.to_arrow()).unwrap(), scores);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListField<D> {
    name: String,
    data_type: ListType<D>,
    nullable: bool,
}

impl<D: RawDataType + Default> ListField<D> {
    /// A `list` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: ListType::default(),
            nullable,
        }
    }
}

impl<D: RawDataType> RawField<ListType<D>> for ListField<D> {
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
        crate::raw_field::validate_field_metadata(field, "ListType")?;
        Ok(Self {
            name: field.name().to_string(),
            data_type,
            nullable: field.is_nullable(),
        })
    }
}

impl<T, D: DataType<T> + Default> Field<Vec<T>> for ListField<D>
where
    D::Scalar: crate::RawScalar<D>,
{
    type Type = ListType<D>;
}
