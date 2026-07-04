//! The [`MapField`] field.

use crate::Field;
use yggdryl_dtype::{DataError, DataType, MapType};

/// A nullable `map` field: a name paired with a dynamic
/// [`MapType`](yggdryl_dtype::MapType).
///
/// Like [`StructField`](crate::StructField) / [`UnionField`](crate::UnionField), the
/// data type carries its key and value types as Arrow fields, so
/// [`new`](MapField::new) takes the [`MapType`](yggdryl_dtype::MapType) rather than
/// defaulting it, and there is no [`FieldFactory`](crate::FieldFactory). The
/// statically-typed [`TypedMapField<K, V>`](crate::TypedMapField) carries the key and
/// value codecs.
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{arrow_schema, DataType, MapType};
/// use yggdryl_field::{Field, MapField};
///
/// let ranks = MapField::new(
///     "ranks",
///     MapType::new(arrow_schema::DataType::UInt8, arrow_schema::DataType::Int64),
///     true,
/// );
/// assert_eq!(ranks.name(), "ranks");
/// assert_eq!(ranks.data_type().name(), "map");
/// assert!(ranks.is_nullable());
/// assert_eq!(MapField::from_arrow(&ranks.to_arrow()).unwrap(), ranks);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapField {
    name: String,
    data_type: MapType,
    nullable: bool,
}

impl MapField {
    /// A `map` field named `name` of the map type `data_type`.
    pub fn new(name: impl Into<String>, data_type: MapType, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type,
            nullable,
        }
    }
}

impl Field for MapField {
    type DataType = MapType;
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &MapType {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        let data_type = MapType::from_arrow(field.data_type())?;
        crate::field::validate_field_metadata(field, "MapType")?;
        Ok(Self::new(field.name(), data_type, field.is_nullable()))
    }
}
