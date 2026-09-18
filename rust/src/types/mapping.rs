//! Mapping: keys to values, on all three sides.
//!
//! One family, one file. The datatype, the field and the value of every
//! mapping shape live here, and the roots that redirect to them - [`DataType`],
//! [`Field`] and [`Scalar`] - hold one variant each:
//!
//! | side | root variant | family | leaf |
//! | --- | --- | --- | --- |
//! | datatype | `DataType::Mapping` | [`MappingType`] | [`MapType`] |
//! | field | `Field::Mapping` | [`MappingField`] | [`MapField`] |
//! | value | `Scalar::Mapping` | [`Mapping`] | [`Map`] |
//!
//! Arrow's map is the family's one leaf today: a list of non-null key-value
//! entry structs, with a flag for whether the keys are ordered. The family
//! exists around it because a second key-to-value layout is a leaf beside it,
//! not a new root variant and not a new spelling at every call site.

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};
use smol_str::SmolStr;

use crate::types::family::DataTypeValue;
use crate::types::structure::StructureType;
use crate::types::family::Children;
use crate::types::family::NestedValue;
use crate::types::structure::cmp_fields;
use crate::types::scalar::Value;
use crate::types::invalid;
use crate::{
    DataType, DataTypeId, DataTypeKind, Error, Field, Result, Scalar,
};

// ------------------------------------------------------------------------
// Datatype side: the family and its one leaf.
// ------------------------------------------------------------------------

/// The entries of one mapping leaf: a non-null key-value struct field.
///
/// Key order is not here. It is the leaf - [`MappingType::Map`] or
/// [`MappingType::SortedMap`] - so a reader that needs ordered keys asks the
/// type, and a value that claims them cannot be built under the other one.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
pub struct MapType {
    pub(crate) entries: Field,
}

impl MapType {
    /// Returns the non-null entries struct field without allocating.
    pub const fn entries(&self) -> &Field {
        &self.entries
    }
}

impl Ord for MapType {
    fn cmp(&self, other: &Self) -> Ordering {
        cmp_fields(&self.entries, &other.entries)
    }
}

impl PartialOrd for MapType {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<'de> Deserialize<'de> for MapType {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            entries: Field,
        }
        let repr = Repr::deserialize(deserializer)?;
        validate_map_entries(&repr.entries).map_err(serde::de::Error::custom)?;
        Ok(Self {
            entries: repr.entries,
        })
    }
}

/// The mapping family's datatype payload.
///
/// The two leaves are the same entries under a different promise about key
/// order, and that promise is the leaf rather than a flag beside it: a sorted
/// map is a `SortedMap`, so nothing downstream carries a boolean it has to
/// remember to honour.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
#[non_exhaustive]
pub enum MappingType {
    /// Arrow's map with keys in no particular order.
    ///
    /// Behind a shared pointer, because the entries field is a whole nested
    /// schema and a datatype clone must not walk it.
    Map(Arc<MapType>),
    /// Arrow's map with keys ordered within each row.
    SortedMap(Arc<MapType>),
}

impl MappingType {
    /// Returns the entries field of whichever leaf this is.
    ///
    /// Every mapping leaf has entries; only the promise around them differs,
    /// so a caller walking children never branches on the leaf.
    pub fn entries(&self) -> &Field {
        match self {
            Self::Map(parameters) | Self::SortedMap(parameters) => &parameters.entries,
        }
    }

    /// Returns the parameters of whichever leaf this is.
    pub fn parameters(&self) -> &MapType {
        match self {
            Self::Map(parameters) | Self::SortedMap(parameters) => parameters,
        }
    }

    /// Returns whether this leaf promises ordered keys.
    ///
    /// This reads the leaf; there is no stored flag that could disagree
    /// with it.
    pub const fn keys_sorted(&self) -> bool {
        matches!(self, Self::SortedMap(_))
    }

    /// Returns whether both payloads are the same leaf over one allocation.
    ///
    /// The cheap identity check the diff walk takes before comparing shapes;
    /// two different leaves never share storage however equal their entries.
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Map(left), Self::Map(right))
            | (Self::SortedMap(left), Self::SortedMap(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    /// Consumes this payload and returns its shared parameters.
    ///
    /// The leaf is dropped, so the caller must read [`Self::keys_sorted`]
    /// first if it still needs the promise.
    pub fn into_parameters(self) -> Arc<MapType> {
        match self {
            Self::Map(parameters) | Self::SortedMap(parameters) => parameters,
        }
    }

    /// Builds the leaf that carries `keys_sorted`.
    pub fn with_keys_sorted(parameters: Arc<MapType>, keys_sorted: bool) -> Self {
        if keys_sorted {
            Self::SortedMap(parameters)
        } else {
            Self::Map(parameters)
        }
    }
}

impl DataTypeValue for MappingType {
    const FAMILY: &'static str = "mapping";

    type Sidecar = ();

    fn id(&self) -> DataTypeId {
        match self {
            Self::Map(_) => DataTypeId::Map,
            Self::SortedMap(_) => DataTypeId::SortedMap,
        }
    }

    fn kind(&self) -> DataTypeKind {
        DataTypeKind::Nested
    }

    fn validate(&self) -> Result<()> {
        validate_map_entries(self.entries())
    }

    fn into_dtype(self) -> DataType {
        DataType::Mapping(self)
    }

    fn from_dtype(dtype: &DataType) -> Option<Self> {
        match dtype {
            DataType::Mapping(family) => Some(family.clone()),
            _ => None,
        }
    }
}

impl fmt::Display for MappingType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.id().as_str())
    }
}

impl From<MappingType> for DataType {
    fn from(value: MappingType) -> Self {
        Self::Mapping(value)
    }
}

impl DataType {
    /// Creates a map from a non-null entries field holding a key and a value.
    ///
    /// `keys_sorted` picks the leaf; after this it is the type, not a flag.
    /// The entries are stored as [`crate::Struct2Type`], so a caller may hand
    /// this either a pair or the two-child struct Arrow spells it with, and
    /// what comes back is always the pair.
    pub fn map(entries: Field, keys_sorted: bool) -> Result<Self> {
        let entries = pair_entries(entries)?;
        Ok(Self::Mapping(MappingType::with_keys_sorted(
            Arc::new(MapType { entries }),
            keys_sorted,
        )))
    }

    /// Creates a map from logical key and value types using Arrow names.
    pub fn map_of(key: Self, value: Self, keys_sorted: bool) -> Result<Self> {
        Self::map(
            Field::new(
                "entries",
                Self::struct2(
                    Field::new("key", key, false),
                    Field::new("value", value, true),
                ),
                false,
            ),
            keys_sorted,
        )
    }

    /// Returns the mapping family payload of a mapping datatype.
    pub fn as_mapping(&self) -> Option<MappingType> {
        MappingType::from_dtype(self)
    }
}

/// Reads one entries field as the key-value pair a mapping stores.
///
/// Arrow spells map entries as a struct of exactly two children, and callers
/// coming from Arrow, Iceberg and Avro build them that way; the pair is what
/// this crate stores, so the struct spelling is folded into it here, once,
/// rather than re-checked at every reader.
fn pair_entries(mut entries: Field) -> Result<Field> {
    if entries.is_nullable() {
        return Err(Error::InvalidDataType {
            kind: "map",
            reason: SmolStr::new_static("entries field must be non-null"),
        });
    }
    let pair = match entries.dtype() {
        DataType::Structure(StructureType::Struct2(_)) => None,
        DataType::Structure(structure) if structure.len() == 2 => Some(DataType::struct2(
            structure[0].clone(),
            structure[1].clone(),
        )),
        _ => {
            return Err(Error::InvalidDataType {
                kind: "map",
                reason: SmolStr::new_static(
                    "entries field must hold a key and a value",
                ),
            });
        }
    };
    if let Some(pair) = pair {
        entries.set_dtype(pair)?;
    }
    validate_map_entries(&entries)?;
    Ok(entries)
}

// ------------------------------------------------------------------------
// Field side: the family and its one leaf.
// ------------------------------------------------------------------------

// ------------------------------------------------------------------------
// Value side: the family and its one leaf.
// ------------------------------------------------------------------------

/// One insertion-ordered mapping with arbitrary scalar keys.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Map(Arc<[(Scalar, Scalar)]>);

impl Map {
    /// Construct a mapping from already unique entries.
    pub fn new(entries: impl Into<Arc<[(Scalar, Scalar)]>>) -> Self {
        Self(entries.into())
    }

    /// Borrow the ordered entries.
    pub fn as_slice(&self) -> &[(Scalar, Scalar)] {
        self.0.as_ref()
    }

    /// Consume this value and return its shared entries.
    pub fn into_inner(self) -> Arc<[(Scalar, Scalar)]> {
        self.0
    }
}

impl fmt::Display for Map {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.as_slice())
    }
}

impl NestedValue for Map {
    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn children(&self) -> Children<'_> {
        Children::Mapping(self.as_slice().iter())
    }
}

impl Value for Map {
    fn dtype(&self) -> Result<DataType> {
        Scalar::Mapping(Mapping::Map(self.clone())).dtype()
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Mapping(Mapping::Map(self))
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Mapping(Mapping::Map(value)) => Some(value),
            _ => None,
        }
    }
}

/// The mapping family's value payload.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Mapping {
    /// An insertion-ordered mapping with arbitrary scalar keys.
    Map(Map),
}

impl Mapping {
    /// Borrow the ordered entries of whichever leaf this is.
    pub fn as_slice(&self) -> &[(Scalar, Scalar)] {
        match self {
            Self::Map(value) => value.as_slice(),
        }
    }

    /// Returns the map value when this is that leaf.
    pub const fn as_map(&self) -> Option<&Map> {
        match self {
            Self::Map(value) => Some(value),
        }
    }
}

impl Default for Mapping {
    fn default() -> Self {
        Self::Map(Map::default())
    }
}

impl fmt::Display for Mapping {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Map(value) => value.fmt(formatter),
        }
    }
}

impl NestedValue for Mapping {
    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn children(&self) -> Children<'_> {
        Children::Mapping(self.as_slice().iter())
    }
}

impl Value for Mapping {
    fn dtype(&self) -> Result<DataType> {
        Scalar::Mapping(self.clone()).dtype()
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Mapping(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Mapping(value) => Some(value),
            _ => None,
        }
    }
}

impl From<Map> for Mapping {
    fn from(value: Map) -> Self {
        Self::Map(value)
    }
}

impl Serialize for Mapping {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Map(value) => value.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Mapping {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Map::deserialize(deserializer).map(Self::Map)
    }
}

pub(crate) fn validate_map_entries(entries: &Field) -> Result<()> {
    if entries.is_nullable() {
        return Err(invalid("Map", "entries field must be non-null"));
    }
    let DataType::Structure(StructureType::Struct2(pair)) = entries.dtype() else {
        return Err(invalid(
            "Map",
            "entries field must contain a key and a value",
        ));
    };
    if pair.first().is_nullable() {
        return Err(invalid("Map", "key field must be non-null"));
    }
    Ok(())
}

