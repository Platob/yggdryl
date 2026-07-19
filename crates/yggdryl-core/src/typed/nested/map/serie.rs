//! [`MapSerie`] — the **map column**: an `i32`-offsets buffer + a two-column
//! [`StructSerie`](crate::typed::StructSerie) of flattened `key` + `value` entries, plus an optional
//! element-level validity buffer. Map `i` is the entry rows `entries[offsets[i]..offsets[i + 1]]` —
//! the Arrow `Map` layout (a `List<Struct<key, value>>`) over the keystone.
//!
//! It mirrors the [`StructSerie`](crate::typed::StructSerie) / [`ListSerie`](super::super::ListSerie)
//! shape: it implements [`Scalar`] / [`Serie`] (its element is a [`MapScalar`]), so a map is itself a
//! column and nests inside a struct or a list, and its [`keys_mut`](MapSerie::keys_mut) /
//! [`values_mut`](MapSerie::values_mut) hand back a `&mut Column` so a caller **deep-mutates an entry
//! series in place, no copy** (matching the public [`Column`] variant to recover the concrete series).
//!
//! ```
//! use yggdryl_core::typed::fixedbyte::Int32;
//! use yggdryl_core::typed::varbyte::Utf8;
//! use yggdryl_core::typed::{Column, FixedSerie, MapSerie, Scalar, Value, VarSerie};
//!
//! // The flattened entries {"a", "b", "c"} -> {1, 2, 3}; two maps demarcated by `push(entry_count)`.
//! let keys = VarSerie::<Utf8>::from_values(&["a".into(), "b".into(), "c".into()]);
//! let vals = FixedSerie::<Int32>::from_values(&[1, 2, 3]);
//! let mut map = MapSerie::new("m", Column::from(keys), Column::from(vals)).unwrap();
//! map.push(2); // {"a": 1, "b": 2}
//! map.push(1); // {"c": 3}
//!
//! assert_eq!(map.len(), 2);
//! match map.get(0) {
//!     Value::Map(scalar) => {
//!         assert_eq!(scalar.len(), 2);
//!         assert_eq!(scalar.get_by_key(&Value::Utf8("b".into())), Some(&Value::Int32(2)));
//!     }
//!     other => panic!("expected a map element, got {other:?}"),
//! }
//! ```

use crate::datatype_id::DataTypeId;
use crate::io::memory::{Heap, IOBase, IoError};
use crate::typed::nested::{Column, MapField, MapScalar, StructSerie, Value};
use crate::typed::{Scalar, Serie};

/// The i32 offset element width, in bytes.
const OFFSET_WIDTH: u64 = 4;

/// The guided [`IoError`] for a map built over a **nullable key** column — Arrow forbids null map
/// keys, so the message names the constraint and the concrete fix. Shared by
/// [`new`](MapSerie::new) and [`from_offsets`](MapSerie::from_offsets) so both read identically.
fn nullable_keys_error() -> IoError {
    IoError::TypedCast {
        detail: "a map's key column must be non-nullable (map keys cannot be null): build the key \
                 column without a validity buffer, or fill its nulls before combining"
            .to_string(),
    }
}

/// A **map column** over an `i32`-offsets [`Heap`] + a two-column [`StructSerie`] of flattened
/// `key` + `value` entries, plus an optional element-level validity buffer. `offsets` holds
/// `len + 1` little-endian `i32`s with `offsets[0] == 0`; map `i` occupies the entry range
/// `[offsets[i], offsets[i + 1])`. `validity`, when present, is the element-level null bitmap
/// (LSB-first, `1` = valid); `len` is the map count.
#[derive(Clone)]
pub struct MapSerie {
    offsets: Heap,
    entries: StructSerie,
    validity: Option<Heap>,
    len: usize,
    name: Option<Box<str>>,
    keys_sorted: bool,
}

impl MapSerie {
    /// Builds the two-column `entries` struct from the flattened `key_col` + `value_col`, validating
    /// them (the key column must be **non-nullable**, else the guided [`nullable_keys_error`]; the
    /// columns must share a length, else the [`StructSerie`] length-mismatch error).
    fn build_entries(key_col: Column, value_col: Column) -> Result<StructSerie, IoError> {
        if key_col.field().nullable() {
            return Err(nullable_keys_error());
        }
        StructSerie::from_columns(vec![key_col, value_col]).map(|s| s.with_name("entries"))
    }

    /// An **empty** map column named `name` over the flattened `key_col` + `value_col` entries (their
    /// rows become the entries as [`push`](MapSerie::push) demarcates them). `offsets[0]` is seeded
    /// to `0`. **Errors** (guided) when the key column is nullable or the two columns differ in length.
    pub fn new(name: &str, key_col: Column, value_col: Column) -> Result<Self, IoError> {
        let entries = Self::build_entries(key_col, value_col)?;
        let mut offsets = Heap::new();
        offsets
            .pwrite_i32(0, 0)
            .expect("offset[0] into a fresh heap never fails");
        Ok(MapSerie {
            offsets,
            entries,
            validity: None,
            len: 0,
            name: Some(name.into()),
            keys_sorted: false,
        })
    }

    /// Wraps an existing `offsets` buffer (`len + 1` little-endian `i32`s, `offsets[0] == 0`), the
    /// flattened `key_col` + `value_col` entries, and an optional element-level `validity` bitmap as
    /// a `len`-element map column — the zero-copy "view existing buffers as a map" front door.
    /// **Errors** (guided) when the key column is nullable or the two columns differ in length.
    pub fn from_offsets(
        name: &str,
        offsets: Heap,
        key_col: Column,
        value_col: Column,
        validity: Option<Heap>,
        len: usize,
    ) -> Result<Self, IoError> {
        let entries = Self::build_entries(key_col, value_col)?;
        Ok(MapSerie {
            offsets,
            entries,
            validity,
            len,
            name: Some(name.into()),
            keys_sorted: false,
        })
    }

    /// Sets the map **name**, chainable.
    pub fn with_name(mut self, name: &str) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Records whether the keys are **sorted** within each map (an Arrow schema hint), chainable.
    pub fn with_keys_sorted(mut self, keys_sorted: bool) -> Self {
        self.keys_sorted = keys_sorted;
        self
    }

    /// The byte end of the current content — `offsets[len]` (the running entry cursor).
    fn end_offset(&self) -> i32 {
        self.offsets
            .pread_i32(self.len as u64 * OFFSET_WIDTH)
            .unwrap_or(0)
    }

    /// Ensures an element-level validity buffer exists, back-filling every existing map as valid
    /// (created lazily on the first null).
    fn ensure_validity(&mut self) {
        if self.validity.is_none() {
            let mut validity = Heap::new();
            for index in 0..self.len as u64 {
                validity
                    .pwrite_bit(index, true)
                    .expect("bit write into a fresh heap never fails");
            }
            self.validity = Some(validity);
        }
    }

    /// Appends a **non-null** map spanning the next `entry_count` rows of the flattened
    /// [`entries`](MapSerie::entries) (staged by the caller via
    /// [`keys_mut`](MapSerie::keys_mut) / [`values_mut`](MapSerie::values_mut)); this only advances
    /// the offset that demarcates the new map.
    pub fn push(&mut self, entry_count: usize) {
        let start = self.end_offset();
        let end = start + entry_count as i32;
        self.offsets
            .pwrite_i32((self.len as u64 + 1) * OFFSET_WIDTH, end)
            .expect("offset write into a heap never fails");
        if let Some(validity) = self.validity.as_mut() {
            validity
                .pwrite_bit(self.len as u64, true)
                .expect("bit write into a heap never fails");
        }
        self.len += 1;
    }

    /// Appends a **null** map — an empty span (`offsets[len + 1] == offsets[len]`) with the validity
    /// bit cleared (back-filling the validity buffer on the first null).
    pub fn push_null(&mut self) {
        self.ensure_validity();
        let start = self.end_offset();
        self.offsets
            .pwrite_i32((self.len as u64 + 1) * OFFSET_WIDTH, start)
            .expect("offset write into a heap never fails");
        self.validity
            .as_mut()
            .expect("validity ensured")
            .pwrite_bit(self.len as u64, false)
            .expect("bit write into a heap never fails");
        self.len += 1;
    }

    /// The entry range `[start, end)` of map `index` in the flattened [`entries`](MapSerie::entries)
    /// — `None` when `index` is out of range.
    pub fn map_at(&self, index: usize) -> Option<(usize, usize)> {
        if index >= self.len {
            return None;
        }
        let start = self
            .offsets
            .pread_i32(index as u64 * OFFSET_WIDTH)
            .unwrap_or(0)
            .max(0) as usize;
        let end = self
            .offsets
            .pread_i32((index as u64 + 1) * OFFSET_WIDTH)
            .unwrap_or(0)
            .max(0) as usize;
        Some((start, end))
    }

    /// The **map element** at `index` — a [`MapScalar`] materializing the entry sub-range's keys and
    /// values as owned parallel [`Value`]s, carrying the element-level validity — or `None` when
    /// `index` is out of range.
    pub fn map(&self, index: usize) -> Option<MapScalar> {
        let (start, end) = self.map_at(index)?;
        let keys = self.keys();
        let values = self.values();
        let count = end.saturating_sub(start);
        let mut key_values = Vec::with_capacity(count);
        let mut value_values = Vec::with_capacity(count);
        for entry in start..end {
            key_values.push(keys.get(entry));
            value_values.push(values.get(entry));
        }
        Some(MapScalar::new(
            key_values,
            value_values,
            self.is_valid(index),
        ))
    }

    /// The map element at `index`, erased to a [`Value`] — [`Value::Map`] when valid, else
    /// [`Value::Null`] (a null map or an out-of-range index).
    pub fn get(&self, index: usize) -> Value {
        match self.map(index) {
            Some(scalar) if !scalar.is_null() => Value::Map(scalar),
            _ => Value::Null,
        }
    }

    /// The number of maps.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the column has no maps.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether the map at `index` is valid (non-null). Out of range is `false`.
    pub fn is_valid(&self, index: usize) -> bool {
        index < self.len
            && self
                .validity
                .as_ref()
                .is_none_or(|bits| bits.pread_bit(index as u64).unwrap_or(false))
    }

    /// How many maps are null.
    pub fn null_count(&self) -> usize {
        match &self.validity {
            None => 0,
            Some(bits) => (0..self.len)
                .filter(|&index| !bits.pread_bit(index as u64).unwrap_or(false))
                .count(),
        }
    }

    /// Whether the keys are **sorted** within each map (an Arrow schema hint).
    pub fn keys_sorted(&self) -> bool {
        self.keys_sorted
    }

    /// The two-column `entries` [`StructSerie`] of flattened `key` + `value` pairs (borrowed).
    pub fn entries(&self) -> &StructSerie {
        &self.entries
    }

    /// The flattened **key** [`Column`] (entries' first column, borrowed).
    pub fn keys(&self) -> &Column {
        &self.entries.columns()[0]
    }

    /// The flattened **value** [`Column`] (entries' second column, borrowed).
    pub fn values(&self) -> &Column {
        &self.entries.columns()[1]
    }

    /// The flattened **key** [`Column`], **mutable** — the front door to a deep, in-place edit of the
    /// keys (match the returned `&mut Column`'s public variant to recover the concrete series),
    /// visible on the next [`get`](MapSerie::get) with **no copy**.
    pub fn keys_mut(&mut self) -> &mut Column {
        self.entries
            .column_mut(0)
            .expect("map entries always carry a key column")
    }

    /// The flattened **value** [`Column`], **mutable** — see [`keys_mut`](MapSerie::keys_mut).
    pub fn values_mut(&mut self) -> &mut Column {
        self.entries
            .column_mut(1)
            .expect("map entries always carry a value column")
    }

    /// The map's name, if set.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// The map's [`MapField`] schema — derived from the entries' key / value fields plus the map's
    /// name, nullability (whether a validity buffer is present), and `keys_sorted` flag.
    pub fn field(&self) -> MapField {
        let mut field = MapField::new(
            self.name.as_deref(),
            self.keys().field(),
            self.values().field(),
        );
        field.set_nullable(self.validity.is_some());
        field.set_keys_sorted(self.keys_sorted);
        field
    }
}

// A map is itself a column whose element is a set of entries. The `Scalar` methods delegate to the
// inherent implementations (fully-qualified so the path resolves to the inherent method, never back
// to the trait — no recursion).
impl Scalar for MapSerie {
    type Value = MapScalar;

    fn data_type_id(&self) -> DataTypeId {
        DataTypeId::Map
    }

    fn len(&self) -> usize {
        MapSerie::len(self)
    }

    fn is_valid(&self, index: usize) -> bool {
        MapSerie::is_valid(self, index)
    }

    fn null_count(&self) -> usize {
        MapSerie::null_count(self)
    }

    fn get(&self, index: usize) -> Option<MapScalar> {
        self.map(index)
    }
}

impl Serie for MapSerie {
    // The graph edges are the entries' two columns (key + value); nested owns them **downward** (no
    // `parent()` up-pointer).
    fn children(&self) -> Vec<&Column> {
        self.entries.columns().iter().collect()
    }
}
