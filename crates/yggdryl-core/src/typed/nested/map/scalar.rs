//! [`MapScalar`] — one **map element**: the key→value entries of a [`MapSerie`](super::MapSerie) at a
//! single index, materialized as owned parallel `Vec<`[`Value`]`>`s, plus the element-level validity.

use crate::typed::nested::Value;

/// A single map element — `keys[i]` and `values[i]` are the `i`-th entry (both erased to a
/// [`Value`]), and `valid` is the element-level null flag (a **null** map is distinct from an
/// **empty** one). It owns its entries, so it outlives the column it came from. `PartialEq` compares
/// the whole map; not `Eq` / `Hash`, because a [`Value`] can hold a float.
#[derive(Clone, Debug, PartialEq)]
pub struct MapScalar {
    keys: Vec<Value>,
    values: Vec<Value>,
    valid: bool,
}

impl MapScalar {
    /// A map element from its parallel `keys` / `values` entries and its element-level `valid` flag
    /// (`keys[i]` maps to `values[i]`).
    pub fn new(keys: Vec<Value>, values: Vec<Value>, valid: bool) -> Self {
        MapScalar {
            keys,
            values,
            valid,
        }
    }

    /// The number of key→value entries.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether the map has no entries (an empty — not necessarily null — map).
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The **key** of entry `index`, if present.
    pub fn get_key(&self, index: usize) -> Option<&Value> {
        self.keys.get(index)
    }

    /// The **value** of entry `index`, if present.
    pub fn get_value(&self, index: usize) -> Option<&Value> {
        self.values.get(index)
    }

    /// The value paired with the **first** entry whose key equals `key`, if any — the map lookup.
    pub fn get_by_key(&self, key: &Value) -> Option<&Value> {
        self.keys
            .iter()
            .position(|candidate| candidate == key)
            .and_then(|index| self.values.get(index))
    }

    /// Whether the **map element itself** is null (as opposed to a valid empty map).
    pub fn is_null(&self) -> bool {
        !self.valid
    }

    /// Whether the **map element itself** is valid (non-null).
    pub fn is_valid(&self) -> bool {
        self.valid
    }

    /// The entry keys in order (borrowed).
    pub fn keys(&self) -> &[Value] {
        &self.keys
    }

    /// The entry values in order (borrowed).
    pub fn values(&self) -> &[Value] {
        &self.values
    }
}
