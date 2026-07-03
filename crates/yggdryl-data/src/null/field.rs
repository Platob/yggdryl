//! The [`NullField`] field of the [`NullType`](super::NullType) data type.

use super::NullType;
use crate::{DataError, RawField};

/// A `null` field: a name paired with the [`NullType`] data type.
///
/// ```
/// use yggdryl_data::{NullField, RawDataType, RawField};
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

impl RawField<NullType> for NullField {
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
        use crate::RawDataType;
        NullType::from_arrow(field.data_type())?;
        crate::raw_field::validate_field_metadata(field, "Null")?;
        Ok(Self::new(field.name(), field.is_nullable()))
    }
}
