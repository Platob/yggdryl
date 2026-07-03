//! The [`StructType`] data type.

use crate::{DataError, RawDataType, RawNested};
use arrow_schema::Fields;

/// The Apache Arrow `struct` data type: an ordered set of named child fields.
///
/// It carries its [`Fields`] exactly as Arrow models them, so
/// [`to_arrow`](RawDataType::to_arrow) / [`from_arrow`](RawDataType::from_arrow)
/// round-trip losslessly — like the dynamic [`UnionType`](crate::UnionType), whose
/// children are only known at runtime, it stays raw-only (a statically-shaped
/// struct also implements the typed [`TypedStruct`](crate::TypedStruct)).
///
/// ```
/// use yggdryl_data::{arrow_schema, RawDataType, RawNested, RawStruct, StructType};
///
/// let point = StructType::new(arrow_schema::Fields::from(vec![
///     arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
///     arrow_schema::Field::new("y", arrow_schema::DataType::Int64, false),
/// ]));
/// assert_eq!(point.name(), "struct");
/// assert_eq!(point.arrow_format(), "+s");
/// assert_eq!(point.byte_width(), None);
/// assert_eq!(point.child_count(), 2);
///
/// // to_arrow / from_arrow are lossless.
/// assert_eq!(StructType::from_arrow(&point.to_arrow()).unwrap(), point);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructType {
    fields: Fields,
}

impl StructType {
    /// This type's [`DataTypeId`](crate::DataTypeId).
    pub const ID: crate::DataTypeId = crate::DataTypeId::Struct;

    /// A struct of the given child `fields`.
    pub fn new(fields: Fields) -> Self {
        Self { fields }
    }
}

impl super::RawStruct for StructType {
    fn fields(&self) -> &Fields {
        &self.fields
    }
}

impl RawDataType for StructType {
    fn name(&self) -> &str {
        "struct"
    }

    fn arrow_format(&self) -> String {
        "+s".to_string()
    }

    fn byte_width(&self) -> Option<usize> {
        None
    }

    fn to_arrow(&self) -> arrow_schema::DataType {
        arrow_schema::DataType::Struct(self.fields.clone())
    }

    fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError> {
        match data_type {
            arrow_schema::DataType::Struct(fields) => Ok(Self::new(fields.clone())),
            other => Err(DataError::IncompatibleArrowType {
                expected: "TypedStruct".to_string(),
                got: other.to_string(),
            }),
        }
    }
}

impl RawNested for StructType {
    fn child_count(&self) -> usize {
        self.fields.len()
    }
}
