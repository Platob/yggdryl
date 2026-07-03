//! The [`Union`] field.

use crate::RawField;
use yggdryl_dtype::{DataError, RawDataType};

/// A nullable `union` field: a name paired with a [`union`](yggdryl_dtype::Union)
/// data type.
///
/// Unlike the fixed-width fields, a union field carries a *parameterised* data type
/// (its children and mode), so [`new`](Union::new) takes the
/// [`Union`](yggdryl_dtype::Union) rather than defaulting it.
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{self, Int64, RawDataType};
/// use yggdryl_field::{RawField, Union};
///
/// let value = Union::new("value", yggdryl_dtype::Union::optional(&Int64), false);
/// assert_eq!(value.name(), "value");
/// assert_eq!(value.data_type().name(), "union");
/// assert!(!value.is_nullable());
/// assert_eq!(Union::from_arrow(&value.to_arrow()).unwrap(), value);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Union {
    name: String,
    data_type: yggdryl_dtype::Union,
    nullable: bool,
}

impl Union {
    /// A field named `name` of the union type `data_type`.
    pub fn new(name: impl Into<String>, data_type: yggdryl_dtype::Union, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type,
            nullable,
        }
    }
}

impl RawField<yggdryl_dtype::Union> for Union {
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &yggdryl_dtype::Union {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        let data_type = yggdryl_dtype::Union::from_arrow(field.data_type())?;
        crate::raw_field::validate_field_metadata(field, "Union")?;
        Ok(Self::new(field.name(), data_type, field.is_nullable()))
    }
}
