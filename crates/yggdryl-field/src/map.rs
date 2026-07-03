//! The [`Map`] field.

use crate::{Field, RawField};
use yggdryl_dtype::{DataError, DataType, RawDataType};

/// A nullable `map` field: a name paired with the [`map`](yggdryl_dtype::Map)
/// from the key type `K` to the value type `V`.
///
/// It carries both trait layers: the raw [`RawField<Map<K, V>>`](RawField)
/// surface, and the typed [`Field<Vec<(TK, TV)>>`] whenever both types have
/// codecs.
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{Int64, RawDataType, RawMap, UInt8};
/// use yggdryl_field::{Map, RawField};
///
/// let ranks = Map::<UInt8, Int64>::new("ranks", true);
/// assert_eq!(ranks.name(), "ranks");
/// assert_eq!(ranks.data_type().name(), "map");
/// assert_eq!(ranks.data_type().key_type().name(), "uint8");
/// assert!(ranks.is_nullable());
/// assert_eq!(Map::from_arrow(&ranks.to_arrow()).unwrap(), ranks);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Map<K, V> {
    name: String,
    data_type: yggdryl_dtype::Map<K, V>,
    nullable: bool,
}

impl<K: RawDataType + Default, V: RawDataType + Default> Map<K, V> {
    /// A `map` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: yggdryl_dtype::Map::default(),
            nullable,
        }
    }
}

impl<K: RawDataType, V: RawDataType> RawField<yggdryl_dtype::Map<K, V>> for Map<K, V> {
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &yggdryl_dtype::Map<K, V> {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        let data_type = yggdryl_dtype::Map::from_arrow(field.data_type())?;
        crate::raw_field::validate_field_metadata(field, "Map")?;
        Ok(Self {
            name: field.name().to_string(),
            data_type,
            nullable: field.is_nullable(),
        })
    }
}

impl<TK, TV, K: DataType<TK>, V: DataType<TV>> Field<Vec<(TK, TV)>> for Map<K, V> {
    type Type = yggdryl_dtype::Map<K, V>;
}
