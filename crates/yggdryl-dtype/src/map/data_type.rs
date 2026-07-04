//! The [`MapType`] data type.

use crate::{DataError, DataType, Nested};
use arrow_schema::Fields;

/// The Apache Arrow `map` data type: a variable-length sequence of key–value entries
/// (unsorted keys).
///
/// It carries its single Arrow child — the non-nullable `"entries"` struct of the
/// non-nullable `"key"` and nullable `"value"` fields — exactly as Arrow models it,
/// so [`to_arrow`](DataType::to_arrow) / [`from_arrow`](DataType::from_arrow)
/// round-trip losslessly, like the dynamic [`StructType`](crate::StructType) /
/// [`UnionType`](crate::UnionType). It stays *untyped* (the key/value native types
/// are erased); a statically-typed map carrying the byte codec is
/// [`TypedMapType<K, V>`](crate::TypedMapType), whose
/// [`erase`](crate::TypedMapType::erase) drops back to this dynamic type.
///
/// ```
/// use yggdryl_dtype::{arrow_schema, DataType, Map, MapType, Nested};
///
/// let map = MapType::new(arrow_schema::DataType::UInt8, arrow_schema::DataType::Int64);
/// assert_eq!(map.name(), "map");
/// assert_eq!(map.arrow_format(), "+m");
/// assert_eq!(map.byte_width(), None);
/// assert_eq!(map.child_count(), 1);
/// assert_eq!(map.entry_fields().len(), 2);
///
/// // to_arrow / from_arrow are lossless.
/// assert!(matches!(map.to_arrow(), arrow_schema::DataType::Map(..)));
/// assert_eq!(MapType::from_arrow(&map.to_arrow()).unwrap(), map);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapType {
    entries: Fields,
}

impl MapType {
    /// A map from `key_type` to `value_type`, wrapping them in the non-nullable
    /// `"key"` and nullable `"value"` entry fields Arrow models a map around.
    pub fn new(key_type: arrow_schema::DataType, value_type: arrow_schema::DataType) -> Self {
        Self {
            entries: Fields::from(vec![
                arrow_schema::Field::new("key", key_type, false),
                arrow_schema::Field::new("value", value_type, true),
            ]),
        }
    }
}

impl super::Map for MapType {
    fn entry_fields(&self) -> Fields {
        self.entries.clone()
    }
}

impl DataType for MapType {
    fn name(&self) -> &str {
        "map"
    }

    fn arrow_format(&self) -> String {
        "+m".to_string()
    }

    fn byte_width(&self) -> Option<usize> {
        None
    }

    fn to_arrow(&self) -> arrow_schema::DataType {
        arrow_schema::DataType::Map(super::Map::entries_field(self), false)
    }

    fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError> {
        let incompatible = || DataError::IncompatibleArrowType {
            expected: "an unsorted map of an \"entries\" struct of \"key\" and \"value\""
                .to_string(),
            got: data_type.to_string(),
        };
        let arrow_schema::DataType::Map(entries, false) = data_type else {
            return Err(incompatible());
        };
        if entries.name() != "entries" || entries.is_nullable() || !entries.metadata().is_empty() {
            return Err(incompatible());
        }
        let arrow_schema::DataType::Struct(fields) = entries.data_type() else {
            return Err(incompatible());
        };
        let [key, value] = fields.iter().collect::<Vec<_>>()[..] else {
            return Err(incompatible());
        };
        if key.name() != "key"
            || key.is_nullable()
            || !key.metadata().is_empty()
            || value.name() != "value"
            || !value.is_nullable()
            || !value.metadata().is_empty()
        {
            return Err(incompatible());
        }
        Ok(Self {
            entries: Fields::from(vec![key.clone(), value.clone()]),
        })
    }
}

impl Nested for MapType {
    fn child_count(&self) -> usize {
        1
    }
}
