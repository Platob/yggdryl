//! [`MapSerie`] — a nullable **map column**: the optimized alias of `List<Struct<{key, value}>>`.
//! It reuses a two-column [`StructSerie`](crate::io::nested::StructSerie) (`key` non-null, `value`
//! nullable) as its flattened **entries** child, plus `i32` offsets, an optional top-level validity
//! mask, and a `keys_sorted` flag. Row `i` is the entries `key[j] -> value[j]` for `j` in
//! `[offsets[i], offsets[i + 1])`. It builds entirely on the root `Any*` primitives — it is itself an
//! [`AnySerie`] (so maps nest) — and bridges to Arrow's `MapArray`.

use core::any::Any;

use super::scalar::MapScalar;
use super::{MapField, MapType};
use crate::io::bitmap::Bitmap;
use crate::io::field_carrier::{any_serie_field_forwarding, field_accessors};
use crate::io::fixed::Field;
use crate::io::nested::{StructField, StructSerie};
use crate::io::{
    AnyField, AnyScalar, AnySerie, Bytes, DataTypeId, Headers, IOCursor, IoError, SerieType,
};

/// A **nullable map column** — the optimized alias of `List<Struct<{key non-null, value}>>`. It holds
/// a two-column [`StructSerie`] of `entries` (column 0 = keys, column 1 = values), `i32` offsets over
/// those entries (`len + 1` entries: `offsets[0] == 0`, non-decreasing, `offsets[len]` equals the
/// entries length), an optional top-level validity mask, and a `keys_sorted` flag. Row `i` is the
/// entries `key[j] -> value[j]` for `j` in `[offsets[i], offsets[i + 1])`.
///
/// ```
/// use yggdryl_core::io::fixed::Serie;
/// use yggdryl_core::io::var::Utf8Serie;
/// use yggdryl_core::io::{AnyScalar, AnySerie};
/// use yggdryl_core::io::nested::MapSerie;
///
/// // Two rows over 3 entries: row 0 = {"a"->1, "b"->2}, row 1 = {"c"->3}.
/// let keys = Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("c")]).named("key");
/// let values = Serie::from_values(&[1i64, 2, 3]).named("value");
/// let map = MapSerie::from_entries(keys, values, &[0, 2, 3], None, false).unwrap();
/// assert_eq!(map.len(), 2);
/// assert_eq!(map.row_scalar(0).len(), 2);
/// // Per-row key lookup: the value mapped to the first key in row 0.
/// let first_key = map.keys().value(0);
/// assert_eq!(map.get_value(0, &first_key), Some(map.values().value(0)));
/// ```
#[derive(Debug, Clone)]
pub struct MapSerie {
    // The flattened `key -> value` entries as a two-column struct (`key` non-null, `value` nullable).
    entries: StructSerie,
    offsets: Vec<i32>,
    validity: Option<Bitmap>,
    len: usize,
    keys_sorted: bool,
    /// The map column's **own-header** field (`Map` type_id) — its name, declared nullability, and
    /// metadata. Excluded from value identity and never written to the standalone frame; the
    /// key/value fields are derived from the entries struct's children.
    field: Field,
}

// A manual `PartialEq` (not a derive): two map columns are equal iff same length, `keys_sorted`,
// offsets, null positions, and flattened entries (the `StructSerie`, which carries the key/value
// field identity). Offsets and validity are canonicalized at construction, so equal logical columns
// compare equal (and serialize byte-equal).
impl PartialEq for MapSerie {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len
            && self.keys_sorted == other.keys_sorted
            && self.offsets == other.offsets
            && self.validity == other.validity
            && self.entries == other.entries
    }
}

impl Eq for MapSerie {}

impl MapSerie {
    /// A map column from its self-describing `keys` and `values` child columns, the row `offsets`, an
    /// optional per-row **present** mask (`present[i] == false` marks row `i` a null map), and whether
    /// the entries are sorted by key. The two columns become the flattened `entries` struct (column 0
    /// = keys, column 1 = values); the key/value fields are the columns' inferred fields.
    ///
    /// A map key is never null (Arrow's `Map` invariant), so the `keys` column **must not** contain
    /// nulls — a guided [`Unsupported`](IoError::Unsupported) error otherwise; this also makes the
    /// inferred key field non-nullable. The `offsets` must have `len + 1` entries with
    /// `offsets[0] == 0`, be non-decreasing, and end at the entries length
    /// (`offsets[len] == keys.len() == values.len()`); otherwise a guided error names the offending
    /// value and the requirement.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    /// use yggdryl_core::io::var::Utf8Serie;
    /// use yggdryl_core::io::AnySerie;
    /// use yggdryl_core::io::nested::MapSerie;
    ///
    /// // 3 rows: {"a"->1}, {} (empty), {"b"->2, "c"->3} — offsets partition the flat entries.
    /// let keys = Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("c")]).named("key");
    /// let values = Serie::from_values(&[1i64, 2, 3]).named("value");
    /// let map = MapSerie::from_entries(keys, values, &[0, 1, 1, 3], None, false).unwrap();
    /// assert_eq!(map.len(), 3);
    /// assert_eq!(map.row_scalar(1).len(), 0); // the empty row
    /// ```
    pub fn from_entries(
        keys: Box<dyn AnySerie>,
        values: Box<dyn AnySerie>,
        offsets: &[i32],
        present: Option<&[bool]>,
        keys_sorted: bool,
    ) -> Result<Self, IoError> {
        // A map key is never null: reject a key column that carries nulls, so the entries struct is a
        // valid Arrow map child and the inferred key field is non-nullable.
        let key_nulls = keys.null_count();
        if key_nulls > 0 {
            return Err(IoError::Unsupported {
                what: format!(
                    "a map key column must not contain nulls, but {:?} has {key_nulls} null \
                     key(s); a map key is never null (Arrow's Map invariant)",
                    keys.name()
                ),
            });
        }
        // Reuse the struct machinery for the two-column entries storage (this also validates that the
        // keys and values columns are the same length).
        let entries = StructSerie::from_series(vec![keys, values])?;
        let len = validate_offsets(offsets, entries.len())?;
        let validity = validity_from_present(present, len);
        Ok(Self {
            entries,
            offsets: offsets.to_vec(),
            validity,
            len,
            keys_sorted,
            field: Field::of("", DataTypeId::Map, 0, false),
        })
    }

    /// An empty (zero-row) map column of the given schema — `offsets = [0]` over empty key/value
    /// entries of the schema's key/value types.
    pub fn empty(schema: &MapField) -> Self {
        let entries = StructSerie::empty(&StructField::new(
            "entries",
            // A map key is never null (Arrow's Map invariant) — force the key field non-nullable
            // even if the schema's key field is nullable, so the stored entries key field matches
            // both Arrow export paths (the field descriptor and the array data type).
            vec![schema.key().with_nullable(false), schema.value().clone()],
            false,
        ));
        Self {
            entries,
            offsets: vec![0],
            validity: None,
            len: 0,
            keys_sorted: schema.keys_sorted(),
            field: Field::of("", DataTypeId::Map, 0, false),
        }
    }

    // DESIGN: no `from_scalars(&[MapScalar])`. Like a list column (see `ListSerie`), a map column is
    // built from *flattened key/value child columns* + offsets (`from_entries`), or reconstructed
    // whole via `deserialize_bytes` / `from_arrow_array`, not transposed from row scalars — a
    // row-scalar factory would have to concatenate each row's erased entries sub-column into one flat
    // entries struct and re-derive the offsets, an erased-column concatenation primitive that does not
    // exist.

    /// The number of rows.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the column has no rows.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The number of null map rows.
    pub fn null_count(&self) -> usize {
        self.validity.as_ref().map_or(0, Bitmap::null_count)
    }

    /// Whether any map row is null.
    pub fn has_nulls(&self) -> bool {
        self.null_count() > 0
    }

    /// The flattened key column (entries column 0), as an erased [`AnySerie`](crate::io::AnySerie)
    /// (downcast with `.as_serie::<T>()`). The `'static` object bound lets the borrow call the
    /// downcast helpers.
    pub fn keys(&self) -> &(dyn AnySerie + 'static) {
        self.entries
            .column(0)
            .expect("a map's entries always has a key column")
    }

    /// The flattened value column (entries column 1), as an erased [`AnySerie`](crate::io::AnySerie).
    pub fn values(&self) -> &(dyn AnySerie + 'static) {
        self.entries
            .column(1)
            .expect("a map's entries always has a value column")
    }

    /// The flattened `key -> value` entries as a two-column [`StructSerie`].
    pub fn entries(&self) -> &StructSerie {
        &self.entries
    }

    /// The row offsets (`len + 1` entries into the flattened entries).
    pub fn offsets(&self) -> &[i32] {
        &self.offsets
    }

    field_accessors!();

    /// The key field descriptor — **derived on demand** from the entries struct's first column's own
    /// header. Owned (there is no cached field to borrow).
    pub fn key_field(&self) -> AnyField {
        self.entries
            .field(0)
            .expect("a map's entries always has a key field")
    }

    /// The value field descriptor — **derived on demand** from the entries struct's second column's
    /// own header. Owned.
    pub fn value_field(&self) -> AnyField {
        self.entries
            .field(1)
            .expect("a map's entries always has a value field")
    }

    /// Whether the entries are sorted by key.
    pub fn keys_sorted(&self) -> bool {
        self.keys_sorted
    }

    /// The entries sub-range `[start, end)` of row `index`, or `None` if out of range. Returns the
    /// range even for a null row (its logical span in the entries); use [`row`](MapSerie::row) to get
    /// a null-aware value.
    pub fn value_range(&self, index: usize) -> Option<(usize, usize)> {
        (index < self.len).then(|| {
            (
                self.offsets[index] as usize,
                self.offsets[index + 1] as usize,
            )
        })
    }

    /// The row at `index` as an erased [`AnyScalar::Map`] — [`AnyScalar::Null`] if the row is null or
    /// out of range. The entries are the entries sub-column for the row.
    pub fn row(&self, index: usize) -> AnyScalar {
        if index >= self.len || self.validity.as_ref().is_some_and(|v| !v.get(index)) {
            return AnyScalar::Null;
        }
        let (start, end) = (
            self.offsets[index] as usize,
            self.offsets[index + 1] as usize,
        );
        AnyScalar::map(
            Box::new(self.entries.slice(start, end - start)),
            self.keys_sorted,
        )
    }

    /// The row at `index` as a [`MapScalar`] — its `is_null` flag reflects the top-level validity, but
    /// its entries are always populated (the entries sub-range). Out of range yields a null scalar
    /// over empty entries.
    pub fn row_scalar(&self, index: usize) -> MapScalar {
        let key = self.key_field();
        let value = self.value_field();
        if index >= self.len {
            return MapScalar::null(
                key,
                value,
                Box::new(self.entries.slice(0, 0)),
                self.keys_sorted,
            );
        }
        let (start, end) = (
            self.offsets[index] as usize,
            self.offsets[index + 1] as usize,
        );
        let entries: Box<dyn AnySerie> = Box::new(self.entries.slice(start, end - start));
        if self.validity.as_ref().is_some_and(|v| !v.get(index)) {
            MapScalar::null(key, value, entries, self.keys_sorted)
        } else {
            MapScalar::new(key, value, entries, self.keys_sorted)
        }
    }

    /// The value mapped to `key` in row `row`, or `None` if the row is null / out of range or the key
    /// is absent. A **linear scan** of the row's entries: each stored key is compared to `key` via the
    /// **allocation-free** [`AnySerie::cell_eq`](crate::io::AnySerie::cell_eq), and the first matching
    /// value is returned. The scan materializes **no** per-key scalar — it compares the key column's
    /// *borrowed* cell bytes (or a stack scratch for a fixed key) against the probe — so a lookup
    /// allocates nothing beyond the single returned value cell. The first **positional** match wins,
    /// so with duplicate keys the earliest value is returned.
    ///
    /// DESIGN: when [`keys_sorted`](MapSerie::keys_sorted) holds *and* the key type is totally
    /// ordered, a binary search over the row's entries would be `O(log n)`; the linear scan stays the
    /// correct default (now allocation-free) because the erased key comparison is bit-canonical
    /// equality only, not a total order — so the sorted fast path waits on an ordered-key comparator.
    pub fn get_value(&self, row: usize, key: &AnyScalar) -> Option<AnyScalar> {
        if row >= self.len || self.validity.as_ref().is_some_and(|v| !v.get(row)) {
            return None;
        }
        let (start, end) = (self.offsets[row] as usize, self.offsets[row + 1] as usize);
        let keys = self.keys();
        let values = self.values();
        (start..end).find_map(|j| keys.cell_eq(j, key).then(|| values.value(j)))
    }

    /// The typed [`MapType`] descriptor (its key/value fields + keys_sorted flag).
    pub fn data_type(&self) -> MapType {
        MapType::new(self.key_field(), self.value_field(), self.keys_sorted)
    }

    /// A [`MapField`] naming this map column, its nullability inferred from whether it holds any null
    /// rows.
    pub fn to_field(&self, name: &str) -> MapField {
        MapField::new(
            name,
            self.key_field(),
            self.value_field(),
            self.has_nulls(),
            self.keys_sorted,
        )
    }

    /// A **new** map column holding rows `[offset, offset + len)` — the range is clamped to the column
    /// (an out-of-range or overlong request yields the in-bounds sub-window, never a panic). The
    /// entries are windowed to exactly the sliced rows' entries and the offsets are rebased to start
    /// at `0`; the top-level validity is sliced to the same window; `keys_sorted` is preserved. The
    /// result is a fresh column; the original is untouched.
    pub fn slice(&self, offset: usize, len: usize) -> Self {
        let start = offset.min(self.len);
        let count = len.min(self.len - start);
        let child_start = self.offsets[start] as usize;
        let child_end = self.offsets[start + count] as usize;
        let entries = self.entries.slice(child_start, child_end - child_start);
        let base = self.offsets[start];
        let offsets: Vec<i32> = self.offsets[start..=start + count]
            .iter()
            .map(|&o| o - base)
            .collect();
        let validity = self.validity.as_ref().map(|mask| {
            let mut sliced = Bitmap::all_present(count);
            for index in 0..count {
                if !mask.get(start + index) {
                    sliced.set(index, false);
                }
            }
            sliced
        });
        Self {
            entries,
            offsets,
            validity: normalize(validity),
            len: count,
            keys_sorted: self.keys_sorted,
            field: self.field.clone(),
        }
    }

    // ---- serialization: the map schema, then validity + offsets, then the entries struct -------

    /// This map column's canonical bytes — a self-contained
    /// `[schema][len][validity?][offsets][entries]` frame. The exact inverse of
    /// [`deserialize_bytes`](MapSerie::deserialize_bytes).
    pub fn serialize_bytes(&self) -> Vec<u8> {
        let mut sink = Bytes::new();
        self.write_frame(&mut sink)
            .expect("writing to an in-memory buffer is infallible");
        sink.as_slice().to_vec()
    }

    /// Reconstructs a map column from [`serialize_bytes`](MapSerie::serialize_bytes) bytes.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        Self::read_frame(&mut Bytes::from_slice(bytes))
    }

    /// Writes the self-contained frame to a byte sink (shared by `serialize_bytes` and the
    /// [`AnySerie`](crate::io::AnySerie) impl, so a map child serializes recursively). The map schema,
    /// header, top-level validity, and offsets are packed into **one** pre-sized buffer and written in
    /// a single call; then the entries struct column serializes itself.
    fn write_frame(&self, sink: &mut Bytes) -> Result<(), IoError> {
        // Encode the schema (a map field over the **derived** key/value fields). Its name / metadata
        // are empty and its nullability is `has_nulls()` (not the own-header flag), so equal-in-data
        // maps serialize byte-identical regardless of the map's own name/metadata.
        let key = self.key_field();
        let value = self.value_field();
        let mut schema = Vec::new();
        AnyField::encode_map(
            "",
            self.has_nulls(),
            &Headers::new(),
            self.keys_sorted,
            &key,
            &value,
            &mut schema,
        );

        let has_validity = self.has_nulls();
        let validity_bytes = if has_validity {
            self.len.div_ceil(8)
        } else {
            0
        };
        let mut header =
            Vec::with_capacity(8 + schema.len() + 8 + 1 + validity_bytes + (self.len + 1) * 4);
        header.extend_from_slice(&(schema.len() as u64).to_le_bytes());
        header.extend_from_slice(&schema);
        header.extend_from_slice(&(self.len as u64).to_le_bytes());
        header.push(u8::from(has_validity));
        if has_validity {
            // `has_nulls` implies `validity` is `Some`.
            header.extend_from_slice(self.validity.as_ref().unwrap().as_bytes());
        }
        for &offset in &self.offsets {
            header.extend_from_slice(&offset.to_le_bytes());
        }
        sink.write_all(&header)?;
        // The entries are self-describing (a two-column struct); they serialize their own schema+data.
        self.entries.write_to(sink)?;
        Ok(())
    }

    /// Reads a frame written by [`write_frame`](MapSerie::write_frame). Crate-visible so the shared
    /// recursive [`read_any_column`](crate::io::nested::read_any_column) dispatch can read a map child.
    pub(crate) fn read_frame(source: &mut Bytes) -> Result<Self, IoError> {
        let schema_len = read_u64(source)? as usize;
        let schema_bytes = source.read_exact_vec(schema_len)?;
        let schema = AnyField::deserialize_bytes(&schema_bytes)?;
        let keys_sorted = match schema {
            AnyField::Map { keys_sorted, .. } => keys_sorted,
            AnyField::Leaf(_) | AnyField::Struct { .. } | AnyField::List { .. } => {
                return Err(IoError::Unsupported {
                    what: "serialized map schema did not decode to a map".to_string(),
                })
            }
        };
        let len = read_u64(source)? as usize;
        let validity = read_validity(source, len)?;
        // `len + 1` i32 offsets. Guard the size against a corrupt/hostile length before reading.
        let offset_count = len.checked_add(1).ok_or(IoError::CorruptLength {
            len: len as u64,
            width: 4,
        })?;
        let byte_len = offset_count.checked_mul(4).ok_or(IoError::CorruptLength {
            len: offset_count as u64,
            width: 4,
        })?;
        let offset_bytes = source.read_exact_vec(byte_len)?;
        let offsets: Vec<i32> = offset_bytes
            .chunks_exact(4)
            .map(|chunk| i32::from_le_bytes(chunk.try_into().unwrap()))
            .collect();
        // The entries are always a two-column struct column; read them through the struct frame reader
        // (which recurses each key/value child through the central `read_any_column` dispatch, so a
        // map-of-list / map-of-struct / map-of-map all round-trip).
        let entries = StructSerie::read_frame(source)?;
        if entries.num_columns() != 2 {
            return Err(IoError::Unsupported {
                what: format!(
                    "a map's entries must be a struct of exactly [key, value], got {} columns",
                    entries.num_columns()
                ),
            });
        }
        validate_offsets(&offsets, entries.len())?;
        Ok(Self {
            entries,
            offsets,
            validity: normalize(validity),
            len,
            keys_sorted,
            field: Field::of("", DataTypeId::Map, 0, false),
        })
    }
}

impl SerieType for MapSerie {
    type Elem = AnyScalar;

    fn len(&self) -> usize {
        self.len
    }

    fn null_count(&self) -> usize {
        self.null_count()
    }

    fn get(&self, index: usize) -> Option<AnyScalar> {
        match self.row(index) {
            AnyScalar::Null => None,
            value => Some(value),
        }
    }
}

impl AnySerie for MapSerie {
    fn len(&self) -> usize {
        self.len
    }

    fn null_count(&self) -> usize {
        MapSerie::null_count(self)
    }

    fn type_id(&self) -> DataTypeId {
        DataTypeId::Map
    }

    any_serie_field_forwarding!();

    fn field(&self, name: &str) -> AnyField {
        AnyField::map_(
            name,
            self.key_field(),
            self.value_field(),
            self.nullable() || self.has_nulls(),
            self.keys_sorted,
        )
        .with_metadata_overlay(self.metadata())
    }

    fn value(&self, index: usize) -> AnyScalar {
        self.row(index)
    }

    fn slice(&self, offset: usize, len: usize) -> Box<dyn AnySerie> {
        Box::new(MapSerie::slice(self, offset, len))
    }

    fn write_to(&self, sink: &mut Bytes) -> Result<(), IoError> {
        self.write_frame(sink)
    }

    #[cfg(feature = "arrow")]
    fn to_arrow_array(&self) -> Result<arrow_array::ArrayRef, IoError> {
        Ok(std::sync::Arc::new(MapSerie::to_arrow_array(self)?))
    }

    fn clone_box(&self) -> Box<dyn AnySerie> {
        Box::new(self.clone())
    }

    fn eq_any(&self, other: &dyn AnySerie) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| self == other)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Validates a map column's `offsets` against `entries_len` entries, returning the row count
/// (`offsets.len() - 1`) on success. Guided [`Unsupported`](IoError::Unsupported) on any violation.
fn validate_offsets(offsets: &[i32], entries_len: usize) -> Result<usize, IoError> {
    let Some((&first, rest)) = offsets.split_first() else {
        return Err(IoError::Unsupported {
            what: "a map column needs at least one offset (offsets = [0] for an empty column); \
                   the offsets slice was empty"
                .to_string(),
        });
    };
    if first != 0 {
        return Err(IoError::Unsupported {
            what: format!(
                "a map column's first offset must be 0, got {first}; offsets are cumulative \
                 entry counts into the flattened entries, starting at 0"
            ),
        });
    }
    let mut prev = first;
    for &offset in rest {
        if offset < prev {
            return Err(IoError::Unsupported {
                what: format!(
                    "a map column's offsets must be non-decreasing, but {offset} follows {prev}; \
                     each offset is a cumulative entry count into the flattened entries"
                ),
            });
        }
        prev = offset;
    }
    if prev as i64 != entries_len as i64 {
        return Err(IoError::Unsupported {
            what: format!(
                "a map column's last offset ({prev}) must equal the flattened entries length \
                 ({entries_len}); the offsets must cover exactly the entries"
            ),
        });
    }
    Ok(offsets.len() - 1)
}

/// Builds a top-level validity mask from a per-row `present` slice (canonical: `None` if fully
/// present). Mirrors [`ListSerie`](crate::io::nested::ListSerie)'s mask handling.
fn validity_from_present(present: Option<&[bool]>, len: usize) -> Option<Bitmap> {
    present.and_then(|flags| {
        let mut bitmap = Bitmap::all_present(len);
        for (index, &is_present) in flags.iter().take(len).enumerate() {
            if !is_present {
                bitmap.set(index, false);
            }
        }
        (bitmap.null_count() > 0).then_some(bitmap)
    })
}

/// Reads the map's top-level validity for `len` rows (the mask read is length-bounded).
fn read_validity<R: IOCursor>(source: &mut R, len: usize) -> Result<Option<Bitmap>, IoError> {
    let mut flag = [0u8; 1];
    source.read_exact(&mut flag)?;
    if flag[0] == 0 {
        return Ok(None);
    }
    let bits = source.read_exact_vec(len.div_ceil(8))?;
    Ok(Some(Bitmap::from_bytes(&bits, len)))
}

/// Drops an all-present mask to `None` so equality/serialization stay canonical.
fn normalize(validity: Option<Bitmap>) -> Option<Bitmap> {
    validity.filter(|bitmap| bitmap.null_count() > 0)
}

/// Reads a little-endian `u64`.
fn read_u64<R: IOCursor>(source: &mut R) -> Result<u64, IoError> {
    let mut bytes = [0u8; 8];
    source.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

// -------------------------------------------------------------------------------------
// Arrow interop (feature `arrow`): map column <-> MapArray.
// -------------------------------------------------------------------------------------

#[cfg(feature = "arrow")]
impl MapSerie {
    /// This map column as an Arrow [`MapArray`](arrow_array::MapArray) — **recursive**: the entries
    /// struct mapped by [`StructSerie::to_arrow_array`](crate::io::nested::StructSerie), the offsets as
    /// an `OffsetBuffer`, the top-level validity as a `NullBuffer`, and the `keys_sorted` flag. The
    /// non-nullable `entries` field is built from the struct array's own data type so it matches
    /// exactly (Arrow requires `field.data_type() == entries.data_type()`). Fallible because an entry
    /// child Arrow cannot express (a temporal resolution `Minute`…`Year`) has no Arrow array.
    pub fn to_arrow_array(&self) -> Result<arrow_array::MapArray, IoError> {
        use arrow_array::Array;
        use std::sync::Arc;
        let entries_array = self.entries.to_arrow_array()?;
        // DESIGN: the `entries` field must be non-nullable and its data type must equal the struct
        // array's exactly — build it straight from the array's data type.
        let entries_field = Arc::new(arrow_schema::Field::new(
            "entries",
            entries_array.data_type().clone(),
            false,
        ));
        let offsets =
            arrow_buffer::OffsetBuffer::new(arrow_buffer::ScalarBuffer::from(self.offsets.clone()));
        let nulls = self.validity.as_ref().map(|bitmap| {
            let buffer = arrow_buffer::Buffer::from(bitmap.as_bytes());
            arrow_buffer::NullBuffer::new(arrow_buffer::BooleanBuffer::new(buffer, 0, self.len))
        });
        arrow_array::MapArray::try_new(
            entries_field,
            offsets,
            entries_array,
            nulls,
            self.keys_sorted,
        )
        .map_err(|error| IoError::Unsupported {
            what: format!("could not build an Arrow MapArray from the map column: {error}"),
        })
    }

    /// Builds a map column from an Arrow [`MapArray`](arrow_array::MapArray) and its
    /// [`Field`](arrow_schema::Field) (of `Map` type), recovering the key/value fields and
    /// `keys_sorted` flag from the field's `Map(entries, keys_sorted)` type and importing the entries
    /// struct recursively. Reads the array's **logical** window, so a *sliced* map array converts
    /// correctly (the offsets index into the full entries; the entries are windowed to
    /// `[offsets[0], offsets[len])` and the offsets rebased to `0`).
    pub fn from_arrow_array(
        array: &dyn arrow_array::Array,
        field: &arrow_schema::Field,
    ) -> Result<Self, IoError> {
        use arrow_array::Array;
        let arrow_schema::DataType::Map(entries_field, keys_sorted) = field.data_type() else {
            return Err(IoError::Unsupported {
                what: format!("expected an Arrow Map field, got {:?}", field.data_type()),
            });
        };
        let map = array
            .as_any()
            .downcast_ref::<arrow_array::MapArray>()
            .ok_or_else(|| IoError::Unsupported {
                what: format!(
                    "expected an Arrow MapArray for field {:?}, got {:?}",
                    field.name(),
                    array.data_type()
                ),
            })?;
        // The entries field is a Struct of exactly [key, value].
        let arrow_schema::DataType::Struct(entry_fields) = entries_field.data_type() else {
            return Err(IoError::Unsupported {
                what: format!(
                    "an Arrow Map's entries must be a Struct of [key, value], got {:?}",
                    entries_field.data_type()
                ),
            });
        };
        if entry_fields.len() != 2 {
            return Err(IoError::Unsupported {
                what: format!(
                    "an Arrow Map's entries struct must have exactly 2 fields (key, value), got {}",
                    entry_fields.len()
                ),
            });
        }

        let len = map.len();
        let raw_offsets = map.value_offsets(); // `len + 1` offsets into the FULL entries
        let first = raw_offsets[0];
        let last = raw_offsets[len];
        // Window the full entries to exactly the used range, then import it recursively as a struct.
        let entries_window = map.entries().slice(first as usize, (last - first) as usize);
        // A map key is never null (Arrow's Map invariant); a foreign entries struct may declare the
        // key field nullable, so force it non-null on import — otherwise the two Arrow export paths
        // (the field descriptor forces it, the array data type would not) disagree and nesting the
        // map in a list/struct panics in arrow-rs.
        let entries = force_non_null_key(StructSerie::from_arrow_array(
            &entries_window,
            entries_field.as_ref(),
        )?);
        let offsets: Vec<i32> = raw_offsets.iter().map(|&offset| offset - first).collect();
        let validity = map_validity_from_arrow(map);
        Ok(Self {
            entries,
            offsets,
            validity,
            len,
            keys_sorted: *keys_sorted,
            field: Field::of("", DataTypeId::Map, 0, false),
        })
    }
}

/// Enforces the "a map key is never null" invariant on a freshly imported `entries` struct: if its
/// key field (column 0) is nullable, rebuild the struct with a non-null key field so the field
/// descriptor and the array data type agree on export. A no-op (no copy) when the key is already
/// non-null, which every yggdryl-built entries struct is; the clone only happens for a foreign array
/// that declared a nullable key.
#[cfg(feature = "arrow")]
fn force_non_null_key(entries: StructSerie) -> StructSerie {
    match entries.field(0) {
        Some(key) if key.nullable() => {
            let mut fields = entries.fields().to_vec();
            fields[0] = key.with_nullable(false);
            let columns = (0..entries.num_columns())
                .map(|index| {
                    entries
                        .column(index)
                        .expect("index < num_columns")
                        .clone_box()
                })
                .collect();
            StructSerie::from_columns(fields, columns, None)
                .expect("rebuilding entries with a non-null key preserves the column lengths")
        }
        _ => entries,
    }
}

/// The map's top-level validity from a `MapArray`, offset-aware, canonicalized (`None` if dense).
#[cfg(feature = "arrow")]
fn map_validity_from_arrow(array: &arrow_array::MapArray) -> Option<Bitmap> {
    use arrow_array::Array;
    if array.null_count() == 0 {
        return None;
    }
    let mut bitmap = Bitmap::all_present(array.len());
    for index in 0..array.len() {
        if array.is_null(index) {
            bitmap.set(index, false);
        }
    }
    Some(bitmap)
}
