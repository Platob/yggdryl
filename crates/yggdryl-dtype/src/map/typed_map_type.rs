//! The [`TypedMapType`] data type.

use crate::{DataError, DataType, MapType, Nested, TypedDataType};
use arrow_schema::Fields;

/// The statically-typed [`MapType`](crate::MapType): a map from a key type `K` to a
/// value type `V`, both known at compile time.
///
/// Where the dynamic [`MapType`](crate::MapType) carries its children as Arrow
/// fields, `TypedMapType<K, V>` keeps the concrete key and value types, so it adds
/// the [`TypedMap`](crate::TypedMap) surface — the key/value accessors and the
/// [`TypedDataType<Vec<(TK, TV)>>`] byte codec. The codec concatenates each entry's
/// key bytes then value bytes; splitting them back requires both fixed widths (a
/// variable-width side errors with [`DataError::IndeterminateElementWidth`] — decode
/// such maps from Arrow). [`erase`](TypedMapType::erase) drops the static types back
/// to a dynamic [`MapType`](crate::MapType).
///
/// ```
/// use yggdryl_dtype::{DataType, Int64Type, Map, MapType, TypedDataType, TypedMap, TypedMapType, UInt8Type};
///
/// let map = TypedMapType::new(UInt8Type, Int64Type);
/// assert_eq!(map.name(), "map");
/// assert_eq!((map.key_type().name(), map.value_type().name()), ("uint8", "int64"));
///
/// // The byte codec concatenates key bytes then value bytes per entry.
/// let bytes = map.native_to_bytes(&vec![(7, 42)]);
/// assert_eq!(bytes.len(), 9);
/// assert_eq!(map.native_from_bytes(&bytes).unwrap(), vec![(7, 42)]);
/// assert_eq!(map.default_value(), Vec::<(u8, i64)>::new());
///
/// // Erase to the dynamic map; from_arrow is the exact inverse of to_arrow.
/// assert_eq!(map.erase(), MapType::from_arrow(&map.to_arrow()).unwrap());
/// assert_eq!(TypedMapType::from_arrow(&map.to_arrow()).unwrap(), map);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TypedMapType<K, V> {
    key_type: K,
    value_type: V,
}

impl<K: DataType, V: DataType> TypedMapType<K, V> {
    /// The map from `key_type` to `value_type`.
    pub fn new(key_type: K, value_type: V) -> Self {
        Self {
            key_type,
            value_type,
        }
    }

    /// Drop the static key and value types, returning the dynamic [`MapType`].
    pub fn erase(&self) -> MapType {
        MapType::new(self.key_type.to_arrow(), self.value_type.to_arrow())
    }
}

impl<K: DataType, V: DataType> super::Map for TypedMapType<K, V> {
    fn entry_fields(&self) -> Fields {
        Fields::from(vec![
            arrow_schema::Field::new("key", self.key_type.to_arrow(), false),
            arrow_schema::Field::new("value", self.value_type.to_arrow(), true),
        ])
    }
}

impl<K: DataType, V: DataType> DataType for TypedMapType<K, V> {
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
        // Reuse the dynamic map's structural validation, then decode the children.
        let dynamic = MapType::from_arrow(data_type)?;
        let fields = super::Map::entry_fields(&dynamic);
        Ok(Self::new(
            K::from_arrow(fields[0].data_type())?,
            V::from_arrow(fields[1].data_type())?,
        ))
    }
}

impl<K: DataType, V: DataType> Nested for TypedMapType<K, V> {
    fn child_count(&self) -> usize {
        1
    }
}

impl<TK, TV, K: TypedDataType<TK>, V: TypedDataType<TV>> TypedDataType<Vec<(TK, TV)>>
    for TypedMapType<K, V>
{
    fn native_to_bytes(&self, entries: &Vec<(TK, TV)>) -> Vec<u8> {
        entries
            .iter()
            .flat_map(|(key, value)| {
                let mut bytes = self.key_type.native_to_bytes(key);
                bytes.extend(self.value_type.native_to_bytes(value));
                bytes
            })
            .collect()
    }

    fn native_from_bytes(&self, bytes: &[u8]) -> Result<Vec<(TK, TV)>, DataError> {
        let key_width = self
            .key_type
            .codec_byte_width()
            .filter(|width| *width > 0)
            .ok_or_else(|| DataError::IndeterminateElementWidth {
                data_type: self.key_type.name().to_string(),
            })?;
        let value_width = self
            .value_type
            .codec_byte_width()
            .filter(|width| *width > 0)
            .ok_or_else(|| DataError::IndeterminateElementWidth {
                data_type: self.value_type.name().to_string(),
            })?;
        let entry_width = key_width + value_width;
        if !bytes.len().is_multiple_of(entry_width) {
            return Err(DataError::InvalidByteLength {
                // The nearest valid length: a whole number of entries, rounded up.
                expected: bytes.len().div_ceil(entry_width) * entry_width,
                got: bytes.len(),
            });
        }
        bytes
            .chunks(entry_width)
            .map(|entry| {
                Ok((
                    self.key_type.native_from_bytes(&entry[..key_width])?,
                    self.value_type.native_from_bytes(&entry[key_width..])?,
                ))
            })
            .collect()
    }

    fn default_value(&self) -> Vec<(TK, TV)> {
        Vec::new()
    }
}

impl<TK, TV, K: TypedDataType<TK>, V: TypedDataType<TV>> crate::TypedNested<Vec<(TK, TV)>>
    for TypedMapType<K, V>
{
}

impl<TK, TV, K: TypedDataType<TK>, V: TypedDataType<TV>> super::TypedMap<TK, TV>
    for TypedMapType<K, V>
{
    type KeyType = K;
    type ValueType = V;

    fn key_type(&self) -> &K {
        &self.key_type
    }

    fn value_type(&self) -> &V {
        &self.value_type
    }
}
