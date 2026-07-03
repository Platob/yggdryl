//! The [`BinaryField`] field of the [`Binary`](super::Binary) data type.

use super::Binary;
use crate::{DataError, Field, RawField};

/// A nullable `binary` field: a name paired with the [`Binary`] data type.
///
/// ```
/// use yggdryl_data::{BinaryField, RawField};
///
/// let payload = BinaryField::new("payload", true);
/// assert_eq!(payload.name(), "payload");
/// assert_eq!(payload.data_type(), &yggdryl_data::Binary);
/// assert!(payload.is_nullable());
///
/// // from_arrow is the exact inverse of to_arrow.
/// assert_eq!(BinaryField::from_arrow(&payload.to_arrow()).unwrap(), payload);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BinaryField {
    name: String,
    data_type: Binary,
    nullable: bool,
}

impl BinaryField {
    /// A `binary` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: Binary,
            nullable,
        }
    }
}

impl RawField<Binary> for BinaryField {
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &Binary {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        <Binary as crate::RawDataType>::from_arrow(field.data_type())?;
        crate::raw_field::validate_field_metadata(field, "Binary")?;
        Ok(Self::new(field.name(), field.is_nullable()))
    }
}

impl Field<Vec<u8>> for BinaryField {
    type Type = Binary;
}
