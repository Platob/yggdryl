//! The [`StringField`] field.

use crate::{Field, FieldFactory, TypedField};
use yggdryl_dtype::{DataError, DataType, StringType};

/// A nullable `utf8` field: a name paired with the
/// [`StringType`](yggdryl_dtype::StringType) data type.
///
/// ```
/// use yggdryl_field::{Field, FieldFactory, StringField};
/// use yggdryl_field::yggdryl_dtype::StringType;
///
/// let name = StringField::new("name", true);
/// assert_eq!(name.name(), "name");
/// assert_eq!(name.data_type(), &StringType);
/// assert!(name.is_nullable());
///
/// // from_arrow is the exact inverse of to_arrow; the data type is the factory.
/// assert_eq!(StringField::from_arrow(&name.to_arrow()).unwrap(), name);
/// assert_eq!(StringType.field("name", true), name);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StringField {
    name: String,
    data_type: StringType,
    nullable: bool,
}

impl StringField {
    /// A `utf8` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: StringType,
            nullable,
        }
    }
}

impl Field for StringField {
    type DataType = StringType;
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &StringType {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        <StringType as DataType>::from_arrow(field.data_type())?;
        crate::field::validate_field_metadata(field, "StringType")?;
        Ok(Self::new(field.name(), field.is_nullable()))
    }
}

impl TypedField<StringType, String> for StringField {}

impl FieldFactory<String> for StringType {
    type Field = StringField;
    fn field(&self, name: impl Into<String>, nullable: bool) -> StringField {
        StringField::new(name, nullable)
    }
}
