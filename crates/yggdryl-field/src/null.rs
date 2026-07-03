//! The [`NullField`] field.

use crate::Field;
use yggdryl_dtype::{DataError, DataType, NullType};

/// A `null` field: a name paired with the [`NullType`](yggdryl_dtype::NullType) data
/// type.
///
/// [`NullType`] has no native value, so it is not a typed data type and has no
/// [`FieldFactory`](crate::FieldFactory); a `NullField` is constructed directly.
///
/// ```
/// use yggdryl_field::yggdryl_dtype::DataType;
/// use yggdryl_field::{Field, NullField};
///
/// let gap = NullField::new("gap", true);
/// assert_eq!(gap.name(), "gap");
/// assert_eq!(gap.data_type().name(), "null");
/// assert!(gap.is_nullable());
/// assert_eq!(NullField::from_arrow(&gap.to_arrow()).unwrap(), gap);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NullField {
    name: String,
    data_type: NullType,
    nullable: bool,
}

impl NullField {
    /// A `null` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: NullType,
            nullable,
        }
    }
}

impl Field<NullType> for NullField {
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &NullType {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        <NullType as DataType>::from_arrow(field.data_type())?;
        crate::field::validate_field_metadata(field, "NullType")?;
        Ok(Self::new(field.name(), field.is_nullable()))
    }
}
