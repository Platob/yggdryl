//! The [`UnionField`] field of the [`Union`](super::Union) data type.

use super::Union;
use crate::{DataError, RawField};

/// A nullable `union` field: a name paired with a [`Union`] data type.
///
/// Unlike the fixed-width fields, a union field carries a *parameterised* data type
/// (its children and mode), so [`new`](UnionField::new) takes the [`Union`] rather
/// than defaulting it.
///
/// ```
/// use yggdryl_data::{Int64, RawDataType, RawField, Union, UnionField};
///
/// let value = UnionField::new("value", Union::optional(&Int64), false);
/// assert_eq!(value.name(), "value");
/// assert_eq!(value.data_type().name(), "union");
/// assert!(!value.is_nullable());
/// assert_eq!(UnionField::from_arrow(&value.to_arrow()).unwrap(), value);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnionField {
    name: String,
    data_type: Union,
    nullable: bool,
}

impl UnionField {
    /// A field named `name` of the union type `data_type`.
    pub fn new(name: impl Into<String>, data_type: Union, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type,
            nullable,
        }
    }
}

impl RawField<Union> for UnionField {
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &Union {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        use crate::RawDataType;
        let data_type = Union::from_arrow(field.data_type())?;
        crate::raw_field::validate_field_metadata(field, "Union")?;
        Ok(Self::new(field.name(), data_type, field.is_nullable()))
    }
}
