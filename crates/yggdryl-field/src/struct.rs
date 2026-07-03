//! The [`StructField`] field.

use crate::Field;
use yggdryl_dtype::{DataError, DataType, StructType};

/// A nullable `struct` field: a name paired with a
/// [`StructType`](yggdryl_dtype::StructType) data type.
///
/// Like [`UnionField`](crate::UnionField), the data type is *parameterised* (its
/// children) and dynamic, so [`new`](StructField::new) takes the
/// [`StructType`](yggdryl_dtype::StructType) rather than defaulting it, and there is
/// no [`FieldFactory`](crate::FieldFactory).
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{self as dtype, arrow_schema, DataType};
/// use yggdryl_field::{Field, StructField};
///
/// let point = dtype::StructType::new(arrow_schema::Fields::from(vec![
///     arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
/// ]));
/// let field = StructField::new("point", point.clone(), false);
/// assert_eq!(field.name(), "point");
/// assert_eq!(field.data_type(), &point);
/// assert!(!field.is_nullable());
/// assert_eq!(StructField::from_arrow(&field.to_arrow()).unwrap(), field);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructField {
    name: String,
    data_type: StructType,
    nullable: bool,
}

impl StructField {
    /// A field named `name` of the struct type `data_type`.
    pub fn new(name: impl Into<String>, data_type: StructType, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type,
            nullable,
        }
    }
}

impl Field<StructType> for StructField {
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &StructType {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        let data_type = StructType::from_arrow(field.data_type())?;
        crate::field::validate_field_metadata(field, "StructType")?;
        Ok(Self::new(field.name(), data_type, field.is_nullable()))
    }
}
