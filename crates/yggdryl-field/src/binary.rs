//! The [`BinaryField`] field.

use crate::{Field, FieldFactory, TypedField};
use yggdryl_dtype::{BinaryType, DataError, DataType};

/// A nullable `binary` field: a name paired with the
/// [`BinaryType`](yggdryl_dtype::BinaryType) data type.
///
/// ```
/// use yggdryl_field::{BinaryField, Field, FieldFactory};
/// use yggdryl_field::yggdryl_dtype::BinaryType;
///
/// let payload = BinaryField::new("payload", true);
/// assert_eq!(payload.name(), "payload");
/// assert_eq!(payload.data_type(), &BinaryType);
/// assert!(payload.is_nullable());
///
/// // from_arrow is the exact inverse of to_arrow; the data type is the factory.
/// assert_eq!(BinaryField::from_arrow(&payload.to_arrow()).unwrap(), payload);
/// assert_eq!(BinaryType.field("payload", true), payload);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BinaryField {
    name: String,
    data_type: BinaryType,
    nullable: bool,
}

impl BinaryField {
    /// A `binary` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: BinaryType,
            nullable,
        }
    }
}

impl Field<BinaryType> for BinaryField {
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &BinaryType {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        <BinaryType as DataType>::from_arrow(field.data_type())?;
        crate::field::validate_field_metadata(field, "BinaryType")?;
        Ok(Self::new(field.name(), field.is_nullable()))
    }
}

impl TypedField<BinaryType, Vec<u8>> for BinaryField {}

impl FieldFactory<Vec<u8>> for BinaryType {
    type Field = BinaryField;
    fn field(&self, name: impl Into<String>, nullable: bool) -> BinaryField {
        BinaryField::new(name, nullable)
    }
}
