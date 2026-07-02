//! The map data type.

use core::fmt;
use std::sync::Arc;

use arrow_schema::DataType as ArrowDataType;

use crate::{
    AnyDataType, DataType, DataTypeError, DataTypeId, Field, NestedType, Struct, TypedField,
    TypedFieldRef,
};

/// Key–value pairs stored as a list of entry structs, mapping to Arrow
/// `Map(entries, sorted)`. The entries field is a [`Struct`] of exactly two
/// children — a non-nullable key field and a value field — preserved as-is so
/// the Arrow round-trip is lossless whatever the fields are named.
///
/// ```
/// use std::sync::Arc;
/// use yggdryl_schema::{DataType, Field, Int32, Map, Struct, TypedField, Utf8};
///
/// let entries = Struct::from_parts(vec![
///     Arc::new(TypedField::from_parts("key", Utf8.into(), false, Default::default())),
///     Arc::new(TypedField::from_parts("value", Int32.into(), true, Default::default())),
/// ]);
/// let entries = Arc::new(TypedField::from_parts("entries", entries, false, Default::default()));
/// let map = Map::from_parts(entries, false).unwrap();
/// assert_eq!(Map::from_arrow(&map.to_arrow()), Ok(map.clone()));
/// assert_eq!(map.to_string(), "map<key: utf8, value: int32?>");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "RawMap")
)]
pub struct Map {
    entries: TypedFieldRef<Struct>,
    sorted: bool,
}

impl Map {
    /// Builds the map type from its entries field and key-ordering flag,
    /// validating that the entries struct has exactly two children and a
    /// non-nullable key.
    ///
    /// ```
    /// use std::sync::Arc;
    /// use yggdryl_schema::{Field, Map, Struct, TypedField};
    ///
    /// let empty = Arc::new(TypedField::from_parts(
    ///     "entries",
    ///     Struct::from_parts(vec![]),
    ///     false,
    ///     Default::default(),
    /// ));
    /// assert!(Map::from_parts(empty, false).is_err()); // expected key and value
    /// ```
    pub fn from_parts(entries: TypedFieldRef<Struct>, sorted: bool) -> Result<Self, DataTypeError> {
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

    /// The entries field: a [`Struct`] of the key and value fields.
    pub fn entries(&self) -> &TypedFieldRef<Struct> {
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
        entries: Option<TypedFieldRef<Struct>>,
        sorted: Option<bool>,
    ) -> Result<Self, DataTypeError> {
        Self::from_parts(
            entries.unwrap_or_else(|| self.entries.clone()),
            sorted.unwrap_or(self.sorted),
        )
    }

    /// Returns a copy with the entries field replaced.
    pub fn with_entries(&self, entries: TypedFieldRef<Struct>) -> Result<Self, DataTypeError> {
        self.copy(Some(entries), None)
    }

    /// Returns a copy with the key-ordering flag replaced.
    pub fn with_sorted(&self, sorted: bool) -> Result<Self, DataTypeError> {
        self.copy(None, Some(sorted))
    }
}

impl DataType for Map {
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
        let mut out = vec![u8::from(self.sorted)];
        out.extend(self.entries.to_bytes());
        out
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError> {
        let [sorted, entries @ ..] = bytes else {
            return Err(DataTypeError::InvalidByteLength {
                expected: 1,
                actual: bytes.len(),
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

impl NestedType for Map {
    fn num_children(&self) -> usize {
        1
    }
}

impl fmt::Display for Map {
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
    entries: TypedFieldRef<Struct>,
    sorted: bool,
}

#[cfg(feature = "serde")]
impl TryFrom<RawMap> for Map {
    type Error = DataTypeError;

    fn try_from(raw: RawMap) -> Result<Self, Self::Error> {
        Self::from_parts(raw.entries, raw.sorted)
    }
}
