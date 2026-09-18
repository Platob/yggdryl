//! Map: a list of key-value entries.

use std::cmp::Ordering;

use serde::{Deserialize, Deserializer, Serialize};

use crate::types::nested::cmp_fields;
use crate::types::nested::validate_map_entries;
use crate::{
    DataType, Field, Result,
};

/// Shared Arrow map parameters.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
pub struct MapType {
    pub(crate) entries: Field,
    pub(crate) keys_sorted: bool,
}

impl MapType {
    /// Returns the non-null entries struct field without allocating.
    pub const fn entries(&self) -> &Field {
        &self.entries
    }

    /// Returns whether map keys are ordered.
    pub const fn keys_sorted(&self) -> bool {
        self.keys_sorted
    }
}

impl Ord for MapType {
    fn cmp(&self, other: &Self) -> Ordering {
        cmp_fields(&self.entries, &other.entries)
            .then_with(|| self.keys_sorted.cmp(&other.keys_sorted))
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
            keys_sorted: bool,
        }
        let repr = Repr::deserialize(deserializer)?;
        validate_map_entries(&repr.entries).map_err(serde::de::Error::custom)?;
        Ok(Self {
            entries: repr.entries,
            keys_sorted: repr.keys_sorted,
        })
    }
}

impl DataType {

    /// Creates a map from logical key and value types using Arrow names.
    pub fn map_of(key: Self, value: Self, keys_sorted: bool) -> Result<Self> {
        Self::map(
            Field::new(
                "entries",
                Self::from_fields([
                    Field::new("key", key, false),
                    Field::new("value", value, true),
                ])?,
                false,
            ),
            keys_sorted,
        )
    }
}
