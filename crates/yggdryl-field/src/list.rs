//! The [`List`] field.

use crate::{Field, RawField};
use yggdryl_dtype::{DataError, DataType, RawDataType};

/// A nullable `list` field: a name paired with the [`list`](yggdryl_dtype::List)
/// of the value type `D`.
///
/// It carries both trait layers: the raw [`RawField<List<D>>`](RawField) surface,
/// and the typed [`Field<Vec<T>>`] whenever the value type has a [`DataType<T>`]
/// codec.
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{Int64, RawDataType, RawList};
/// use yggdryl_field::{List, RawField};
///
/// let scores = List::<Int64>::new("scores", true);
/// assert_eq!(scores.name(), "scores");
/// assert_eq!(scores.data_type().name(), "list");
/// assert_eq!(scores.data_type().value_type().name(), "int64");
/// assert!(scores.is_nullable());
/// assert_eq!(List::from_arrow(&scores.to_arrow()).unwrap(), scores);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct List<D> {
    name: String,
    data_type: yggdryl_dtype::List<D>,
    nullable: bool,
}

impl<D: RawDataType + Default> List<D> {
    /// A `list` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: yggdryl_dtype::List::default(),
            nullable,
        }
    }
}

impl<D: RawDataType> RawField<yggdryl_dtype::List<D>> for List<D> {
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &yggdryl_dtype::List<D> {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        let data_type = yggdryl_dtype::List::from_arrow(field.data_type())?;
        crate::raw_field::validate_field_metadata(field, "List")?;
        Ok(Self {
            name: field.name().to_string(),
            data_type,
            nullable: field.is_nullable(),
        })
    }
}

impl<T, D: DataType<T>> Field<Vec<T>> for List<D> {
    type Type = yggdryl_dtype::List<D>;
}
