//! The [`MapField`] field of the [`MapType`](super::MapType) data type.

use super::MapType;
use crate::{DataError, DataType, Field, RawDataType, RawField};

/// A nullable `map` field: a name paired with the [`MapType`] from the key type
/// `K` to the value type `V`.
///
/// It carries both trait layers: the raw [`RawField<MapType<K, V>>`] surface, and
/// the typed [`Field<Vec<(TK, TV)>>`] whenever both types have codecs.
///
/// ```
/// use yggdryl_data::{Int64, MapField, RawDataType, RawField, RawMap, UInt8};
///
/// let ranks = MapField::<UInt8, Int64>::new("ranks", true);
/// assert_eq!(ranks.name(), "ranks");
/// assert_eq!(ranks.data_type().name(), "map");
/// assert_eq!(ranks.data_type().key_type().name(), "uint8");
/// assert!(ranks.is_nullable());
/// assert_eq!(MapField::from_arrow(&ranks.to_arrow()).unwrap(), ranks);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapField<K, V> {
    name: String,
    data_type: MapType<K, V>,
    nullable: bool,
}

impl<K: RawDataType + Default, V: RawDataType + Default> MapField<K, V> {
    /// A `map` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: MapType::default(),
            nullable,
        }
    }
}

impl<K: RawDataType, V: RawDataType> RawField<MapType<K, V>> for MapField<K, V> {
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &MapType<K, V> {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        let data_type = MapType::from_arrow(field.data_type())?;
        crate::raw_field::validate_field_metadata(field, "MapType")?;
        Ok(Self {
            name: field.name().to_string(),
            data_type,
            nullable: field.is_nullable(),
        })
    }
}

impl<TK, TV, K, V> Field<Vec<(TK, TV)>> for MapField<K, V>
where
    K: DataType<TK> + Default,
    V: DataType<TV> + Default,
    K::Scalar: crate::RawScalar<K>,
    V::Scalar: crate::RawScalar<V>,
{
    type Type = MapType<K, V>;
}
