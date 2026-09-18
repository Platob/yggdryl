//! Dictionary encoding: a key column over a value column.

use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};

use smol_str::format_smolstr;
use crate::types::invalid;
use crate::types::typed::define_field_types;
use crate::{
    DataType, Result,
};

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

impl DataType {

    /// Creates a dictionary and validates its integer key type.
    pub fn dictionary(key: Self, value: Self) -> Result<Self> {
        validate_dictionary_key(&key)?;
        Ok(Self::Dictionary(Arc::new(DictionaryType { key, value })))
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
    crate::DataType::Dictionary(_)
);

fn is_valid_dictionary_key(key: &DataType) -> bool {
    key.is_integer()
}
