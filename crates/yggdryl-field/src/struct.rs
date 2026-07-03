//! The [`Struct`] field.

use crate::RawField;
use yggdryl_dtype::{DataError, RawDataType};

/// A nullable `struct` field: a name paired with a
/// [`struct`](yggdryl_dtype::Struct) data type.
///
/// Like [`Union`](crate::Union), the data type is *parameterised* (its children),
/// so [`new`](Struct::new) takes the [`Struct`](yggdryl_dtype::Struct) rather than
/// defaulting it.
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{self, arrow_schema, RawDataType};
/// use yggdryl_field::{RawField, Struct};
///
/// let point = yggdryl_dtype::Struct::new(arrow_schema::Fields::from(vec![
///     arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
/// ]));
/// let field = Struct::new("point", point.clone(), false);
/// assert_eq!(field.name(), "point");
/// assert_eq!(field.data_type(), &point);
/// assert!(!field.is_nullable());
/// assert_eq!(Struct::from_arrow(&field.to_arrow()).unwrap(), field);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Struct {
    name: String,
    data_type: yggdryl_dtype::Struct,
    nullable: bool,
}

impl Struct {
    /// A field named `name` of the struct type `data_type`.
    pub fn new(name: impl Into<String>, data_type: yggdryl_dtype::Struct, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type,
            nullable,
        }
    }
}

impl RawField<yggdryl_dtype::Struct> for Struct {
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &yggdryl_dtype::Struct {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        let data_type = yggdryl_dtype::Struct::from_arrow(field.data_type())?;
        crate::raw_field::validate_field_metadata(field, "Struct")?;
        Ok(Self::new(field.name(), data_type, field.is_nullable()))
    }
}
