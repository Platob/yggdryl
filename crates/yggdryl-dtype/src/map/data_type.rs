//! The [`MapType`] data type.

use crate::{DataError, DataType, Nested, TypedDataType};

/// The Apache Arrow `map` data type: a variable-length sequence of key–value
/// entries, keyed by `K` with values of `V` (unsorted keys).
///
/// Its single child is the non-nullable `"entries"` struct of the non-nullable
/// `"key"` and nullable `"value"` fields. The typed
/// [`TypedDataType<Vec<(TK, TV)>>`] byte codec concatenates each entry's key bytes
/// then value bytes; splitting them back requires both fixed widths (a
/// variable-width side errors with [`DataError::IndeterminateElementWidth`] — decode
/// such maps from Arrow).
///
/// ```
/// use yggdryl_dtype::{arrow_schema, DataType, Int64Type, Map, MapType, TypedDataType, UInt8Type};
///
/// let map = MapType::new(UInt8Type, Int64Type);
/// assert_eq!(map.name(), "map");
/// assert_eq!(map.arrow_format(), "+m");
/// assert_eq!(map.byte_width(), None);
/// assert_eq!((map.key_type().name(), map.value_type().name()), ("uint8", "int64"));
///
/// // The byte codec concatenates key bytes then value bytes per entry.
/// let bytes = map.native_to_bytes(&vec![(7, 42)]);
/// assert_eq!(bytes.len(), 9);
/// assert_eq!(map.native_from_bytes(&bytes).unwrap(), vec![(7, 42)]);
///
/// // The type knows its default: the empty map.
/// assert_eq!(map.default_value(), Vec::<(u8, i64)>::new());
///
/// // from_arrow is the exact inverse of to_arrow.
/// assert!(matches!(map.to_arrow(), arrow_schema::DataType::Map(..)));
/// assert_eq!(MapType::from_arrow(&map.to_arrow()).unwrap(), map);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct MapType<K, V> {
    key_type: K,
    value_type: V,
}

impl<K: DataType, V: DataType> MapType<K, V> {
    /// The map from `key_type` to `value_type`.
    pub fn new(key_type: K, value_type: V) -> Self {
        Self {
            key_type,
            value_type,
        }
    }

    /// The entry struct's fields: the non-nullable `"key"` and nullable `"value"`
    /// — the per-entry children the scalar layer assembles its Arrow form around.
    pub fn entry_fields(&self) -> arrow_schema::Fields {
        arrow_schema::Fields::from(vec![
            arrow_schema::Field::new("key", self.key_type.to_arrow(), false),
            arrow_schema::Field::new("value", self.value_type.to_arrow(), true),
        ])
    }

    /// The map's single Arrow child: the non-nullable `"entries"` struct field —
    /// the exact child [`to_arrow`](DataType::to_arrow) wraps.
    pub fn entries_field(&self) -> arrow_schema::FieldRef {
        std::sync::Arc::new(arrow_schema::Field::new(
            "entries",
            arrow_schema::DataType::Struct(self.entry_fields()),
            false,
        ))
    }
}

impl<K: DataType, V: DataType> super::Map<K, V> for MapType<K, V> {
    fn key_type(&self) -> &K {
        &self.key_type
    }

    fn value_type(&self) -> &V {
        &self.value_type
    }
}

impl<K: DataType, V: DataType> DataType for MapType<K, V> {
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
        arrow_schema::DataType::Map(self.entries_field(), false)
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
        // The key and value children redirect to their types' own from_arrow.
        Ok(Self::new(
            K::from_arrow(key.data_type())?,
            V::from_arrow(value.data_type())?,
        ))
    }
}

impl<K: DataType, V: DataType> Nested for MapType<K, V> {
    fn child_count(&self) -> usize {
        1
    }
}

impl<TK, TV, K: TypedDataType<TK>, V: TypedDataType<TV>> TypedDataType<Vec<(TK, TV)>>
    for MapType<K, V>
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
    for MapType<K, V>
{
}

impl<TK, TV, K: TypedDataType<TK>, V: TypedDataType<TV>> super::TypedMap<TK, TV> for MapType<K, V> {
    type KeyType = K;
    type ValueType = V;
}
