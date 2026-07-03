//! The [`MapField`] field.

use crate::{Field, FieldFactory, TypedField};
use yggdryl_dtype::{DataError, DataType, MapType, TypedDataType};

/// A nullable `map` field: a name paired with the
/// [`MapType`](yggdryl_dtype::MapType) from the key type `K` to the value type `V`.
///
/// It carries both trait layers: the raw [`Field<MapType<K, V>>`](Field) surface,
/// and the typed [`TypedField<MapType<K, V>, Vec<(TK, TV)>>`] whenever both types
/// have codecs.
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{DataType, Int64Type, Map, MapType, UInt8Type};
/// use yggdryl_field::{Field, FieldFactory, MapField};
///
/// let ranks = MapField::<UInt8Type, Int64Type>::new("ranks", true);
/// assert_eq!(ranks.name(), "ranks");
/// assert_eq!(ranks.data_type().name(), "map");
/// assert_eq!(ranks.data_type().key_type().name(), "uint8");
/// assert!(ranks.is_nullable());
/// assert_eq!(MapField::from_arrow(&ranks.to_arrow()).unwrap(), ranks);
/// assert_eq!(MapType::new(UInt8Type, Int64Type).field("ranks", true), ranks);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapField<K, V> {
    name: String,
    data_type: MapType<K, V>,
    nullable: bool,
}

impl<K: DataType + Default, V: DataType + Default> MapField<K, V> {
    /// A `map` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: MapType::default(),
            nullable,
        }
    }
}

impl<K: DataType, V: DataType> Field<MapType<K, V>> for MapField<K, V> {
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
        crate::field::validate_field_metadata(field, "MapType")?;
        Ok(Self {
            name: field.name().to_string(),
            data_type,
            nullable: field.is_nullable(),
        })
    }
}

impl<TK, TV, K: TypedDataType<TK>, V: TypedDataType<TV>> TypedField<MapType<K, V>, Vec<(TK, TV)>>
    for MapField<K, V>
{
}

impl<TK, TV, K: TypedDataType<TK> + Default, V: TypedDataType<TV> + Default>
    FieldFactory<Vec<(TK, TV)>> for MapType<K, V>
{
    type Field = MapField<K, V>;
    fn field(&self, name: impl Into<String>, nullable: bool) -> MapField<K, V> {
        MapField::new(name, nullable)
    }
}
