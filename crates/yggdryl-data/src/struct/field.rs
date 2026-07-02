//! The [`StructField`] field of the [`StructType`](super::StructType) data type.

use super::StructType;
use crate::{DataError, RawField};

/// A nullable `struct` field: a name paired with a [`StructType`].
///
/// Like [`UnionField`](crate::UnionField), the data type is *parameterised* (its
/// children), so [`new`](StructField::new) takes the [`StructType`] rather than
/// defaulting it.
///
/// ```
/// use yggdryl_data::{arrow_schema, RawDataType, RawField, StructField, StructType};
///
/// let point = StructType::new(arrow_schema::Fields::from(vec![
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

impl RawField<StructType> for StructField {
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
        use crate::RawDataType;
        let data_type = StructType::from_arrow(field.data_type())?;
        crate::raw_field::validate_field_metadata(field, "Struct")?;
        Ok(Self::new(field.name(), data_type, field.is_nullable()))
    }
}
