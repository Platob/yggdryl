//! The [`Optional`] field.

use crate::{Field, RawField};
use yggdryl_dtype::{DataError, DataType, RawDataType};

/// A nullable `optional` field: a name paired with the logical
/// [`optional`](yggdryl_dtype::Optional) of the value type `D`.
///
/// It carries both trait layers: the raw [`RawField<Optional<D>>`](RawField)
/// surface, and the typed [`Field<T>`] whenever the value type has a
/// [`DataType<T>`] codec.
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{Int64, RawDataType, RawOptional};
/// use yggdryl_field::{Optional, RawField};
///
/// let score = Optional::<Int64>::new("score", true);
/// assert_eq!(score.name(), "score");
/// assert_eq!(score.data_type().name(), "optional");
/// assert_eq!(score.data_type().value_type().name(), "int64");
/// assert!(score.is_nullable());
/// assert_eq!(Optional::from_arrow(&score.to_arrow()).unwrap(), score);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Optional<D> {
    name: String,
    data_type: yggdryl_dtype::Optional<D>,
    nullable: bool,
}

impl<D: RawDataType + Default> Optional<D> {
    /// An `optional` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: yggdryl_dtype::Optional::default(),
            nullable,
        }
    }
}

impl<D: RawDataType> RawField<yggdryl_dtype::Optional<D>> for Optional<D> {
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &yggdryl_dtype::Optional<D> {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        let data_type = yggdryl_dtype::Optional::from_arrow(field.data_type())?;
        crate::raw_field::validate_field_metadata(field, "Optional")?;
        Ok(Self {
            name: field.name().to_string(),
            data_type,
            nullable: field.is_nullable(),
        })
    }
}

impl<T, D: DataType<T>> Field<T> for Optional<D> {
    type Type = yggdryl_dtype::Optional<D>;
}
