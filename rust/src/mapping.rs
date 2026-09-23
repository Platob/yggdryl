//! Mapping: keys to values, on all three sides.
//!
//! One family, one file. The datatype, the field and the value of every
//! mapping shape live here, and the roots that redirect to them - [`DataType`],
//! [`Field`] and [`Scalar`] - hold one variant each:
//!
//! | side | root variant | family | leaf |
//! | --- | --- | --- | --- |
//! | datatype | `DataType::Map`, `DataType::SortedMap` | [`MappingType`], a view | [`MapType`] |
//! | field | `Field::Map`, `Field::SortedMap` | [`MappingField`](crate::MappingField) | - |
//! | value | `Scalar::Map`, `Scalar::SortedMap` | - | [`Map`] |
//!
//! Arrow's map is a list of non-null key-value entry structs with a flag for
//! whether the keys are ordered; the flag is the root variant, `Map` or
//! `SortedMap`, on all three sides, so a map is dispatched like every other
//! datatype. [`MappingType`] is the borrowed view both variants answer.

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};
use smol_str::SmolStr;

use crate::invalid;
use crate::structure::StructType;
use crate::structure::cmp_fields;
use crate::value::Children;
use crate::value::DataTypeValue;
use crate::value::NestedValue;
use crate::value::Value;
use crate::{DataType, DataTypeId, DataTypeKind, Error, Field, Result, Scalar};

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
        match self {
            Self::Map(parameters) => DataType::Map(parameters),
            Self::SortedMap(parameters) => DataType::SortedMap(parameters),
        }
    }

    fn from_dtype(dtype: &DataType) -> Option<Self> {
        dtype.as_mapping()
    }
}

impl fmt::Display for MappingType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.id().as_str())
    }
}

impl From<MappingType> for DataType {
    fn from(value: MappingType) -> Self {
        DataTypeValue::into_dtype(value)
    }
}

impl DataType {
    /// The entries field of a map or sorted map, borrowed.
    #[must_use]
    pub fn map_entries(&self) -> Option<&Field> {
        match self {
            Self::Map(parameters) | Self::SortedMap(parameters) => Some(&parameters.entries),
            _ => None,
        }
    }

    /// Creates a map from a non-null entries field holding a key and a value.
    ///
    /// `keys_sorted` picks the leaf; after this it is the type, not a flag.
    /// The entries are the struct of exactly two children Arrow spells them
    /// with, a key and a value, checked here once.
    pub fn map(entries: Field, keys_sorted: bool) -> Result<Self> {
        let entries = pair_entries(entries)?;
        Ok(MappingType::with_keys_sorted(Arc::new(MapType { entries }), keys_sorted).into())
    }

    /// Creates a map from logical key and value types using Arrow names.
    pub fn map_of(key: Self, value: Self, keys_sorted: bool) -> Result<Self> {
        let entries = StructType::from_unique_fields(vec![
            Field::new("key", key, false),
            Field::new("value", value, true),
        ]);
        Self::map(
            Field::new("entries", Self::Struct(entries), false),
            keys_sorted,
        )
    }

    /// The mapping family's view of a map or sorted map, `None` for every
    /// other datatype: a shared-pointer clone, never a walk of the entries.
    #[must_use]
    pub fn as_mapping(&self) -> Option<MappingType> {
        match self {
            Self::Map(parameters) => Some(MappingType::Map(Arc::clone(parameters))),
            Self::SortedMap(parameters) => Some(MappingType::SortedMap(Arc::clone(parameters))),
            _ => None,
        }
    }
}

/// Reads one entries field as the key-value struct a mapping stores.
///
/// Arrow spells map entries as a struct of exactly two children, the key
/// non-null, and that is what this crate stores too: the shape is checked
/// here, once, rather than at every reader.
fn pair_entries(entries: Field) -> Result<Field> {
    if entries.is_nullable() {
        return Err(Error::InvalidDataType {
            kind: "map",
            reason: SmolStr::new_static("entries field must be non-null"),
        });
    }
    match entries.dtype() {
        DataType::Struct(fields) if fields.len() == 2 => {
            validate_map_entries(&entries)?;
            Ok(entries)
        }
        _ => Err(Error::InvalidDataType {
            kind: "map",
            reason: SmolStr::new_static("entries field must hold a key and a value"),
        }),
    }
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
        Scalar::Map(self.clone()).dtype()
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Map(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Map(value) | Scalar::SortedMap(value) => Some(value),
            _ => None,
        }
    }
}

pub(crate) fn validate_map_entries(entries: &Field) -> Result<()> {
    if entries.is_nullable() {
        return Err(invalid("Map", "entries field must be non-null"));
    }
    let DataType::Struct(pair) = entries.dtype() else {
        return Err(invalid(
            "Map",
            "entries field must contain a key and a value",
        ));
    };
    if pair.len() != 2 {
        return Err(invalid(
            "Map",
            "entries field must contain a key and a value",
        ));
    }
    if pair[0].is_nullable() {
        return Err(invalid("Map", "key field must be non-null"));
    }
    Ok(())
}

// ------------------------------------------------------------------------
// Arrow projection: a map, and whether its keys are sorted within a row.
// ------------------------------------------------------------------------

mod arrow {
    use std::sync::Arc;

    use arrow_schema::DataType as ArrowDataType;
    use arrow_schema::ffi::Flags;

    use super::{MappingType, validate_map_entries};
    use crate::value::ArrowFfiParts;
    use crate::{DataType, Field, Result};

    impl MappingType {
        /// The Arrow storage this mapping lays out.
        ///
        /// Whether the keys are sorted is a leaf here and a flag there, so the
        /// two spellings never disagree.
        ///
        /// # Errors
        ///
        /// Returns an error when the entries are not the non-null struct of
        /// a key and a value a map declares, or have no Arrow projection.
        pub(crate) fn arrow_storage(&self) -> Result<ArrowDataType> {
            validate_map_entries(self.entries())?;
            Ok(ArrowDataType::Map(
                self.entries().clone().into_arrow_field_ref()?,
                self.keys_sorted(),
            ))
        }

        /// The same projection, consuming uniquely held entries.
        ///
        /// # Errors
        ///
        /// [`Self::arrow_storage`] carries the rule.
        pub(crate) fn into_arrow_storage(self) -> Result<ArrowDataType> {
            let keys_sorted = self.keys_sorted();
            let entries = match Arc::try_unwrap(self.into_parameters()) {
                Ok(parameters) => {
                    validate_map_entries(&parameters.entries)?;
                    parameters.entries.into_arrow_field_ref()?
                }
                Err(parameters) => {
                    validate_map_entries(&parameters.entries)?;
                    parameters.entries.clone().into_arrow_field_ref()?
                }
            };
            Ok(ArrowDataType::Map(entries, keys_sorted))
        }

        /// The C Data Interface node this mapping writes.
        ///
        /// # Errors
        ///
        /// [`Self::arrow_storage`] carries the rule.
        pub(crate) fn arrow_ffi_parts(&self) -> Result<ArrowFfiParts> {
            validate_map_entries(self.entries())?;
            Ok(ArrowFfiParts {
                format: "+m".to_owned(),
                children: vec![self.entries().clone().into_arrow_field_ffi()?],
                dictionary: None,
                flags: if self.keys_sorted() {
                    Flags::MAP_KEYS_SORTED
                } else {
                    Flags::empty()
                },
            })
        }

        /// The mapping datatype one Arrow map storage imports as.
        ///
        /// # Errors
        ///
        /// Returns an error when the entries cannot be imported or are not the
        /// non-null struct of a key and a value a map declares.
        pub(crate) fn from_arrow_storage_at_depth(
            entries: &arrow_schema::FieldRef,
            keys_sorted: bool,
            depth: usize,
        ) -> Result<DataType> {
            DataType::map(
                Field::from_arrow_field_ref_at_depth(Arc::clone(entries), depth)?,
                keys_sorted,
            )
        }

        /// The same import, consuming Arrow's shared entries field.
        ///
        /// # Errors
        ///
        /// [`Self::from_arrow_storage_at_depth`] carries the rule.
        pub(crate) fn from_arrow_storage_owned_at_depth(
            entries: arrow_schema::FieldRef,
            keys_sorted: bool,
            depth: usize,
        ) -> Result<DataType> {
            DataType::map(
                Field::from_arrow_field_ref_at_depth(entries, depth)?,
                keys_sorted,
            )
        }
    }
}
