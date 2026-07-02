//! The [`NullField`] field of the [`Null`](super::Null) data type.

use super::Null;
use crate::{DataError, RawField};

/// A `null` field: a name paired with the [`Null`] data type.
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
    data_type: Null,
    nullable: bool,
}

impl NullField {
    /// A `null` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: Null,
            nullable,
        }
    }
}

impl RawField<Null> for NullField {
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &Null {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        use crate::RawDataType;
        Null::from_arrow(field.data_type())?;
        crate::raw_field::validate_field_metadata(field, "Null")?;
        Ok(Self::new(field.name(), field.is_nullable()))
    }
}
