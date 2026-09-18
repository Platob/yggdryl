//! Enum: a value stored as a code that stands for it.
//!
//! One family, one file. Dictionary encoding is its first leaf - a key column
//! over a value column - and the family exists around it because every other
//! way of storing a value as a code for it is a leaf beside that one, not a
//! new variant on [`DataType`] and not a new spelling at every call site.
//!
//! The name is about what the column means, not how it is laid out: the
//! values are drawn from a closed set and the rows carry codes into it. That
//! is a different fact from [`crate::Vocabulary`], which is the closed set a
//! *name* is drawn from and whose datatype is `string`.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};

use smol_str::format_smolstr;
use crate::types::family::FamilyType;
use crate::types::invalid;
use crate::types::typed::define_field_types;
use crate::{DataType, DataTypeId, DataTypeKind, Result};

/// Shared dictionary key and value types.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize)]
pub struct DictionaryType {
    pub(crate) key: DataType,
    pub(crate) value: DataType,
}

impl DictionaryType {
    /// Returns the integer key type without allocating.
    pub const fn key(&self) -> &DataType {
        &self.key
    }

    /// Returns the encoded value type without allocating.
    pub const fn value(&self) -> &DataType {
        &self.value
    }
}

impl<'de> Deserialize<'de> for DictionaryType {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            key: DataType,
            value: DataType,
        }
        let repr = Repr::deserialize(deserializer)?;
        validate_dictionary_key(&repr.key).map_err(serde::de::Error::custom)?;
        Ok(Self {
            key: repr.key,
            value: repr.value,
        })
    }
}

/// The enum family's datatype payload.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
#[non_exhaustive]
pub enum EnumType {
    /// A key column over a value column.
    ///
    /// Behind a shared pointer, because the two datatypes are wider than this
    /// enum and a datatype clone must not walk them.
    Dictionary(Arc<DictionaryType>),
}

impl EnumType {
    /// Returns the code datatype the rows carry.
    pub fn key(&self) -> &DataType {
        match self {
            Self::Dictionary(dictionary) => &dictionary.key,
        }
    }

    /// Returns the datatype the codes stand for.
    pub fn value(&self) -> &DataType {
        match self {
            Self::Dictionary(dictionary) => &dictionary.value,
        }
    }

    /// Returns the dictionary parameters when this is that leaf.
    pub fn as_dictionary(&self) -> Option<&DictionaryType> {
        match self {
            Self::Dictionary(dictionary) => Some(dictionary),
        }
    }

    /// Returns whether both payloads are the same leaf over one allocation.
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Dictionary(left), Self::Dictionary(right)) => Arc::ptr_eq(left, right),
        }
    }
}

impl FamilyType for EnumType {
    const FAMILY: &'static str = "enum";

    fn id(&self) -> DataTypeId {
        match self {
            Self::Dictionary(_) => DataTypeId::Dictionary,
        }
    }

    fn kind(&self) -> DataTypeKind {
        DataTypeKind::Nested
    }

    fn validate(&self) -> Result<()> {
        validate_dictionary_key(self.key())
    }

    fn into_dtype(self) -> DataType {
        DataType::Enum(self)
    }

    fn from_dtype(dtype: &DataType) -> Option<&Self> {
        match dtype {
            DataType::Enum(family) => Some(family),
            _ => None,
        }
    }
}

impl fmt::Display for EnumType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.id().as_str())
    }
}

impl From<EnumType> for DataType {
    fn from(value: EnumType) -> Self {
        Self::Enum(value)
    }
}

impl DataType {

    /// Creates a dictionary and validates its integer key type.
    pub fn dictionary(key: Self, value: Self) -> Result<Self> {
        validate_dictionary_key(&key)?;
        Ok(Self::Enum(EnumType::Dictionary(Arc::new(DictionaryType {
            key,
            value,
        }))))
    }
}

pub(crate) fn validate_dictionary_key(key: &DataType) -> Result<()> {
    if is_valid_dictionary_key(key) {
        Ok(())
    } else {
        Err(invalid(
            "Dictionary",
            format_smolstr!(
                "expected an integer key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got {key}"
            ),
        ))
    }
}

define_field_types!(
    DictionaryTypeMarker,
    Dictionary,
    crate::DataType::Enum(crate::types::EnumType::Dictionary(_))
);

fn is_valid_dictionary_key(key: &DataType) -> bool {
    key.is_integer()
}
