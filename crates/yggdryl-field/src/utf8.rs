//! The [`Utf8Field`] field.

use crate::{Field, FieldFactory, TypedField};
use yggdryl_dtype::{DataError, DataType, Utf8Type};

/// A nullable `utf8` field: a name paired with the
/// [`Utf8Type`](yggdryl_dtype::Utf8Type) data type.
///
/// ```
/// use yggdryl_field::{Field, FieldFactory, Utf8Field};
/// use yggdryl_field::yggdryl_dtype::Utf8Type;
///
/// let name = Utf8Field::new("name", true);
/// assert_eq!(name.name(), "name");
/// assert_eq!(name.data_type(), &Utf8Type);
/// assert!(name.is_nullable());
///
/// // from_arrow is the exact inverse of to_arrow; the data type is the factory.
/// assert_eq!(Utf8Field::from_arrow(&name.to_arrow()).unwrap(), name);
/// assert_eq!(Utf8Type.field("name", true), name);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Utf8Field {
    name: String,
    data_type: Utf8Type,
    nullable: bool,
}

impl Utf8Field {
    /// A `utf8` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: Utf8Type,
            nullable,
        }
    }
}

impl Field for Utf8Field {
    type DataType = Utf8Type;
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &Utf8Type {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        <Utf8Type as DataType>::from_arrow(field.data_type())?;
        crate::field::validate_field_metadata(field, "Utf8Type")?;
        Ok(Self::new(field.name(), field.is_nullable()))
    }
}

impl TypedField<Utf8Type, String> for Utf8Field {}

impl FieldFactory<String> for Utf8Type {
    type Field = Utf8Field;
    fn field(&self, name: impl Into<String>, nullable: bool) -> Utf8Field {
        Utf8Field::new(name, nullable)
    }
}
