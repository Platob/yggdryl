//! The [`OptionalField`] field of the [`OptionalType`](super::OptionalType) data type.

use super::OptionalType;
use crate::{DataError, DataType, Field, RawDataType, RawField};

/// A nullable `optional` field: a name paired with the [`OptionalType`] of the value
/// type `D`.
///
/// It carries both trait layers: the raw [`RawField<OptionalType<D>>`] surface, and the
/// typed [`Field<T>`] whenever the value type has a [`DataType<T>`] codec.
///
/// ```
/// use yggdryl_data::{Int64, OptionalField, RawDataType, RawField, RawOptional};
///
/// let score = OptionalField::<Int64>::new("score", true);
/// assert_eq!(score.name(), "score");
/// assert_eq!(score.data_type().name(), "optional");
/// assert_eq!(score.data_type().value_type().name(), "int64");
/// assert!(score.is_nullable());
/// assert_eq!(OptionalField::from_arrow(&score.to_arrow()).unwrap(), score);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionalField<D> {
    name: String,
    data_type: OptionalType<D>,
    nullable: bool,
}

impl<D: RawDataType + Default> OptionalField<D> {
    /// An `optional` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: OptionalType::default(),
            nullable,
        }
    }
}

impl<D: RawDataType> RawField<OptionalType<D>> for OptionalField<D> {
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
        crate::raw_field::validate_field_metadata(field, "OptionalType")?;
        Ok(Self {
            name: field.name().to_string(),
            data_type,
            nullable: field.is_nullable(),
        })
    }
}

impl<T, D: DataType<T> + Default> Field<T> for OptionalField<D>
where
    D::Scalar: crate::RawScalar<D>,
{
    type Type = OptionalType<D>;
}
