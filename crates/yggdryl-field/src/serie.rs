//! The [`SerieField`] field.

use crate::Field;
use yggdryl_dtype::{DataError, DataType, SerieType};

/// A nullable `list` field: a name paired with a dynamic
/// [`SerieType`](yggdryl_dtype::SerieType).
///
/// Like [`StructField`](crate::StructField) / [`UnionField`](crate::UnionField), the
/// data type carries its value type as an Arrow field, so [`new`](SerieField::new)
/// takes the [`SerieType`](yggdryl_dtype::SerieType) rather than defaulting it, and
/// there is no [`FieldFactory`](crate::FieldFactory). The statically-typed
/// [`TypedSerieField<D>`](crate::TypedSerieField) carries the value type's codec.
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{arrow_schema, DataType, SerieType};
/// use yggdryl_field::{Field, SerieField};
///
/// let scores = SerieField::new("scores", SerieType::new(arrow_schema::DataType::Int64), true);
/// assert_eq!(scores.name(), "scores");
/// assert_eq!(scores.data_type().name(), "list");
/// assert!(scores.is_nullable());
/// assert_eq!(SerieField::from_arrow(&scores.to_arrow()).unwrap(), scores);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerieField {
    name: String,
    data_type: SerieType,
    nullable: bool,
}

impl SerieField {
    /// A `list` field named `name` of the serie type `data_type`.
    pub fn new(name: impl Into<String>, data_type: SerieType, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type,
            nullable,
        }
    }
}

impl Field for SerieField {
    type DataType = SerieType;
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &SerieType {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        let data_type = SerieType::from_arrow(field.data_type())?;
        crate::field::validate_field_metadata(field, "SerieType")?;
        Ok(Self::new(field.name(), data_type, field.is_nullable()))
    }
}
