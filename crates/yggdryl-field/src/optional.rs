//! The [`OptionalField`] field.

use crate::Field;
use yggdryl_dtype::{DataError, DataType, OptionalType};

/// A nullable `optional` field: a name paired with a dynamic logical
/// [`OptionalType`](yggdryl_dtype::OptionalType).
///
/// Like [`StructField`](crate::StructField) / [`UnionField`](crate::UnionField), the
/// data type carries its value type as an Arrow field, so
/// [`new`](OptionalField::new) takes the
/// [`OptionalType`](yggdryl_dtype::OptionalType) rather than defaulting it, and there
/// is no [`FieldFactory`](crate::FieldFactory). The statically-typed
/// [`TypedOptionalField<D>`](crate::TypedOptionalField) carries the value type's
/// codec.
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{DataType, Int64Type, OptionalType};
/// use yggdryl_field::{Field, OptionalField};
///
/// let score = OptionalField::new("score", OptionalType::new(&Int64Type), true);
/// assert_eq!(score.name(), "score");
/// assert_eq!(score.data_type().name(), "optional");
/// assert!(score.is_nullable());
/// assert_eq!(OptionalField::from_arrow(&score.to_arrow()).unwrap(), score);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionalField {
    name: String,
    data_type: OptionalType,
    nullable: bool,
}

impl OptionalField {
    /// An `optional` field named `name` of the optional type `data_type`.
    pub fn new(name: impl Into<String>, data_type: OptionalType, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type,
            nullable,
        }
    }
}

impl Field for OptionalField {
    type DataType = OptionalType;
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &OptionalType {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        let data_type = OptionalType::from_arrow(field.data_type())?;
        crate::field::validate_field_metadata(field, "OptionalType")?;
        Ok(Self::new(field.name(), data_type, field.is_nullable()))
    }
}
