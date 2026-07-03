//! The [`Binary`] field.

use crate::{Field, RawField};
use yggdryl_dtype::{DataError, RawDataType};

/// A nullable `binary` field: a name paired with the
/// [`binary`](yggdryl_dtype::Binary) data type.
///
/// ```
/// use yggdryl_field::{Binary, RawField};
///
/// let payload = Binary::new("payload", true);
/// assert_eq!(payload.name(), "payload");
/// assert_eq!(payload.data_type(), &yggdryl_field::yggdryl_dtype::Binary);
/// assert!(payload.is_nullable());
///
/// // from_arrow is the exact inverse of to_arrow.
/// assert_eq!(Binary::from_arrow(&payload.to_arrow()).unwrap(), payload);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Binary {
    name: String,
    data_type: yggdryl_dtype::Binary,
    nullable: bool,
}

impl Binary {
    /// A `binary` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: yggdryl_dtype::Binary,
            nullable,
        }
    }
}

impl RawField<yggdryl_dtype::Binary> for Binary {
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &yggdryl_dtype::Binary {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        <yggdryl_dtype::Binary as RawDataType>::from_arrow(field.data_type())?;
        crate::raw_field::validate_field_metadata(field, "Binary")?;
        Ok(Self::new(field.name(), field.is_nullable()))
    }
}

impl Field<Vec<u8>> for Binary {
    type Type = yggdryl_dtype::Binary;
}
