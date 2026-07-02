//! The map data type.

use core::fmt;
use std::sync::Arc;

use arrow_schema::DataType as ArrowDataType;

use crate::{
    AnyDataType, DataType, DataTypeError, DataTypeId, Field, NestedType, StructType, TypedField,
    TypedFieldRef,
};

/// Key–value pairs stored as a list of entry structs, mapping to Arrow
/// `MapType(entries, sorted)`. The entries field is a [`StructType`] of exactly two
/// children — a non-nullable key field and a value field — preserved as-is so
/// the Arrow round-trip is lossless whatever the fields are named.
///
/// ```
/// use std::sync::Arc;
/// use yggdryl_schema::{DataType, Field, Int32Type, MapType, StructType, TypedField, Utf8Type};
///
/// let entries = StructType::from_parts(vec![
///     Arc::new(TypedField::from_parts("key", Utf8Type.into(), false, Default::default())),
///     Arc::new(TypedField::from_parts("value", Int32Type.into(), true, Default::default())),
/// ]);
/// let entries = Arc::new(TypedField::from_parts("entries", entries, false, Default::default()));
/// let map = MapType::from_parts(entries, false).unwrap();
/// assert_eq!(MapType::from_arrow(&map.to_arrow()), Ok(map.clone()));
/// assert_eq!(map.to_string(), "map<key: utf8, value: int32?>");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "RawMap")
)]
pub struct MapType {
    entries: TypedFieldRef<StructType>,
    sorted: bool,
}

impl MapType {
    /// Builds the map type from its entries field and key-ordering flag,
    /// validating that the entries struct has exactly two children and a
    /// non-nullable key.
    ///
    /// ```
    /// use std::sync::Arc;
    /// use yggdryl_schema::{Field, MapType, StructType, TypedField};
    ///
    /// let empty = Arc::new(TypedField::from_parts(
    ///     "entries",
    ///     StructType::from_parts(vec![]),
    ///     false,
    ///     Default::default(),
    /// ));
    /// assert!(MapType::from_parts(empty, false).is_err()); // expected key and value
    /// ```
    pub fn from_parts(
        entries: TypedFieldRef<StructType>,
        sorted: bool,
    ) -> Result<Self, DataTypeError> {
        let fields = entries.data_type().fields();
        if fields.len() != 2 {
            return Err(DataTypeError::InvalidMapEntries {
                message: format!(
                    "map entries must be a struct of exactly 2 fields, got {} — \
                     build entries as struct<key, value>",
                    fields.len()
                ),
            });
        }
        if fields[0].nullable() {
            return Err(DataTypeError::InvalidMapEntries {
                message: format!(
                    "map key field \"{}\" must be non-nullable — set its nullable flag to false",
                    fields[0].name()
                ),
            });
        }
        Ok(Self { entries, sorted })
    }

    /// The entries field: a [`StructType`] of the key and value fields.
    pub fn entries(&self) -> &TypedFieldRef<StructType> {
        &self.entries
    }

    /// The key field (the entries struct's first, non-nullable child).
    pub fn key(&self) -> &TypedFieldRef<AnyDataType> {
        &self.entries.data_type().fields()[0]
    }

    /// The value field (the entries struct's second child).
    pub fn value(&self) -> &TypedFieldRef<AnyDataType> {
        &self.entries.data_type().fields()[1]
    }

    /// Whether the keys of every map value are sorted.
    pub fn sorted(&self) -> bool {
        self.sorted
    }

    /// Returns a copy with any of the parts overridden; omitted parts come
    /// from `self`.
    pub fn copy(
        &self,
        entries: Option<TypedFieldRef<StructType>>,
        sorted: Option<bool>,
    ) -> Result<Self, DataTypeError> {
        Self::from_parts(
            entries.unwrap_or_else(|| self.entries.clone()),
            sorted.unwrap_or(self.sorted),
        )
    }

    /// Returns a copy with the entries field replaced.
    pub fn with_entries(&self, entries: TypedFieldRef<StructType>) -> Result<Self, DataTypeError> {
        self.copy(Some(entries), None)
    }

    /// Returns a copy with the key-ordering flag replaced.
    pub fn with_sorted(&self, sorted: bool) -> Result<Self, DataTypeError> {
        self.copy(None, Some(sorted))
    }
}

impl DataType for MapType {
    fn type_id(&self) -> DataTypeId {
        DataTypeId::Map
    }

    fn to_arrow(&self) -> ArrowDataType {
        ArrowDataType::Map(Arc::new(self.entries.to_arrow()), self.sorted)
    }

    fn from_arrow(data_type: &ArrowDataType) -> Result<Self, DataTypeError> {
        match data_type {
            ArrowDataType::Map(entries, sorted) => {
                Self::from_parts(Arc::new(TypedField::from_arrow(entries)?), *sorted)
            }
            other => Err(DataTypeError::ArrowTypeMismatch {
                expected: "map",
                actual: other.clone(),
            }),
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![DataTypeId::Map.to_u8(), u8::from(self.sorted)];
        out.extend(self.entries.to_bytes());
        out
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError> {
        let payload = DataTypeId::Map.strip_tag(bytes)?;
        let [sorted, entries @ ..] = payload else {
            return Err(DataTypeError::InvalidByteLength {
                expected: 1,
                actual: payload.len(),
            });
        };
        let sorted = match sorted {
            0 => false,
            1 => true,
            other => {
                return Err(DataTypeError::InvalidBytes {
                    message: format!("unknown sorted flag {other}, expected 0 or 1"),
                })
            }
        };
        Self::from_parts(Arc::new(TypedField::from_bytes(entries)?), sorted)
    }
}

impl NestedType for MapType {
    fn num_children(&self) -> usize {
        1
    }
}

impl fmt::Display for MapType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "map<{}, {}", self.key(), self.value())?;
        if self.sorted {
            f.write_str(", sorted")?;
        }
        f.write_str(">")
    }
}

/// Mirror of the serialized fields, deserialized first so `try_from`
/// re-validates on the way in.
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct RawMap {
    entries: TypedFieldRef<StructType>,
    sorted: bool,
}

#[cfg(feature = "serde")]
impl TryFrom<RawMap> for MapType {
    type Error = DataTypeError;

    fn try_from(raw: RawMap) -> Result<Self, Self::Error> {
        Self::from_parts(raw.entries, raw.sorted)
    }
}
