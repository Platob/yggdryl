//! [`MapScalar`] — one **map value**: a nullable row of a map column, its `key -> value` entries
//! carried as an erased `StructSerie(key, value)` sub-[`Serie`](crate::io::AnySerie). It is what
//! [`MapSerie::row_scalar`](super::MapSerie::row_scalar) yields.

use super::MapType;
use crate::io::{AnyField, AnySerie, DataTypeId, ScalarType};

/// A single **map value** — a row: the map's `key` and `value` fields, the row's `key -> value`
/// entries as an erased two-column struct sub-column (`Box<dyn AnySerie>` — a `StructSerie`), whether
/// the entries are sorted by key, and whether the map value itself is null. Like [`ListScalar`], a
/// map scalar "falls back on our [`Serie`](crate::io::AnySerie)": its entries *are* a (usually short)
/// erased column, so it needs no dependency on a bespoke value container.
///
/// It is a hashable value type: two map values are equal iff they have the same key/value fields, the
/// same `keys_sorted` flag, and either are both null, or hold equal entries. A **null** map's phantom
/// entries are ignored (two same-typed null maps are equal, like `Scalar::null() == Scalar::null()`).
///
/// DESIGN: equality is **POSITIONAL** — the entries compare in stored order (the `StructSerie` child
/// order), so two maps with the same logical `key -> value` pairs in a different order are *not*
/// equal. A positional identity keeps it in lock-step with the byte codec and is the only total
/// choice while the erased key type carries no canonical ordering.
///
/// ```
/// use yggdryl_core::io::fixed::Serie;
/// use yggdryl_core::io::var::Utf8Serie;
/// use yggdryl_core::io::AnySerie;
/// use yggdryl_core::io::nested::MapSerie;
///
/// let keys = Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("c")]).named("key");
/// let values = Serie::from_values(&[1i64, 2, 3]).named("value");
/// let map = MapSerie::from_entries(keys, values, &[0, 2, 3], None, false).unwrap();
/// let row = map.row_scalar(0);
/// assert!(!row.is_null());
/// assert_eq!(row.len(), 2); // two entries: "a"->1, "b"->2
/// assert!(!row.keys_sorted());
/// ```
#[derive(Debug, Clone)]
pub struct MapScalar {
    key: AnyField,
    value: AnyField,
    entries: Box<dyn AnySerie>,
    keys_sorted: bool,
    null: bool,
}

impl MapScalar {
    /// A present map value from its `key`/`value` fields, its `key -> value` entries as an erased
    /// `StructSerie(key, value)` sub-column, and whether the entries are sorted by key.
    pub fn new(
        key: AnyField,
        value: AnyField,
        entries: Box<dyn AnySerie>,
        keys_sorted: bool,
    ) -> Self {
        Self {
            key,
            value,
            entries,
            keys_sorted,
            null: false,
        }
    }

    /// A null map value carrying its (logically-absent) entries.
    pub fn null(
        key: AnyField,
        value: AnyField,
        entries: Box<dyn AnySerie>,
        keys_sorted: bool,
    ) -> Self {
        Self {
            key,
            value,
            entries,
            keys_sorted,
            null: true,
        }
    }

    /// Whether the map value is null.
    pub fn is_null(&self) -> bool {
        self.null
    }

    /// The number of `key -> value` entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the map value has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.len() == 0
    }

    /// The row's `key -> value` entries as an erased two-column struct sub-column
    /// ([`AnySerie`](crate::io::AnySerie); its columns are the keys and the values).
    pub fn entries(&self) -> &(dyn AnySerie + 'static) {
        self.entries.as_ref()
    }

    /// The key field descriptor.
    pub fn key_field(&self) -> &AnyField {
        &self.key
    }

    /// The value field descriptor.
    pub fn value_field(&self) -> &AnyField {
        &self.value
    }

    /// Whether the entries are sorted by key.
    pub fn keys_sorted(&self) -> bool {
        self.keys_sorted
    }

    /// The element [`DataTypeId`] — always [`Map`](DataTypeId::Map).
    pub fn type_id(&self) -> DataTypeId {
        DataTypeId::Map
    }

    /// The typed [`MapType`] descriptor of this value.
    pub fn data_type(&self) -> MapType {
        MapType::new(self.key.clone(), self.value.clone(), self.keys_sorted)
    }
}

impl PartialEq for MapScalar {
    fn eq(&self, other: &Self) -> bool {
        if self.null != other.null
            || self.keys_sorted != other.keys_sorted
            || self.key != other.key
            || self.value != other.value
        {
            return false;
        }
        // A null map's entries are logically absent, so they do not affect identity.
        self.null || self.entries.eq_any(other.entries.as_ref())
    }
}

impl Eq for MapScalar {}

impl core::hash::Hash for MapScalar {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.key.hash(state);
        self.value.hash(state);
        self.keys_sorted.hash(state);
        self.null.hash(state);
        if !self.null {
            // Stay in lock-step with `PartialEq`: equal erased columns are byte-canonical, so hashing
            // the entries' frame keeps "equal values hash equal". A map value is a whole (short)
            // column, so this one allocation is acceptable (see `AnyScalar::hash`).
            self.entries.serialize_bytes().hash(state);
        }
    }
}

impl ScalarType for MapScalar {
    type Data = MapType;

    fn data_type(&self) -> MapType {
        self.data_type()
    }

    fn is_null(&self) -> bool {
        self.null
    }
}
