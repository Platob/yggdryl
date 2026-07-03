//! The [`UnionField`] field.

use crate::Field;
use yggdryl_dtype::{DataError, DataType, UnionType};

/// A nullable `union` field: a name paired with a
/// [`UnionType`](yggdryl_dtype::UnionType) data type.
///
/// Unlike the fixed-width fields, a union field carries a *parameterised* dynamic
/// data type (its children and mode), so [`new`](UnionField::new) takes the
/// [`UnionType`](yggdryl_dtype::UnionType) rather than defaulting it, and there is no
/// [`FieldFactory`](crate::FieldFactory).
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{self as dtype, DataType, Int64Type};
/// use yggdryl_field::{Field, UnionField};
///
/// let value = UnionField::new("value", dtype::UnionType::optional(&Int64Type), false);
/// assert_eq!(value.name(), "value");
/// assert_eq!(value.data_type().name(), "union");
/// assert!(!value.is_nullable());
/// assert_eq!(UnionField::from_arrow(&value.to_arrow()).unwrap(), value);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnionField {
    name: String,
    data_type: UnionType,
    nullable: bool,
}

impl UnionField {
    /// A field named `name` of the union type `data_type`.
    pub fn new(name: impl Into<String>, data_type: UnionType, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type,
            nullable,
        }
    }
}

impl Field for UnionField {
    type DataType = UnionType;
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &UnionType {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        let data_type = UnionType::from_arrow(field.data_type())?;
        crate::field::validate_field_metadata(field, "UnionType")?;
        Ok(Self::new(field.name(), data_type, field.is_nullable()))
    }
}
