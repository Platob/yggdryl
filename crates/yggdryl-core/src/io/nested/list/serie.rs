//! [`ListSerie`] — a nullable **list column**: `i32` offsets over one flattened child column (itself
//! an erased [`AnySerie`](crate::io::AnySerie)), plus an optional top-level validity mask. Row `i` is
//! the child sub-range `child[offsets[i] .. offsets[i + 1]]`. It builds entirely on the root `Any*`
//! primitives — it is itself an [`AnySerie`] (so lists nest) — and bridges to Arrow's `ListArray`.

use core::any::Any;

use super::scalar::ListScalar;
use super::{ListField, ListType};
use crate::io::any_serie::apply_field_header;
use crate::io::bitmap::Bitmap;
use crate::io::field_carrier::{any_serie_field_forwarding, field_accessors};
use crate::io::fixed::Field;
use crate::io::{
    AnyField, AnyScalar, AnySerie, Bytes, DataTypeId, Headers, IOCursor, IoError, SerieType,
};

/// A **nullable list column** — `i32` offsets over one flattened child [`AnySerie`](crate::io::AnySerie)
/// (all rows share the single child column), plus an optional top-level validity mask. The offsets
/// have `len + 1` entries: `offsets[0] == 0`, they are non-decreasing, and `offsets[len]` equals the
/// child length; row `i` is `child[offsets[i] .. offsets[i + 1]]`.
///
/// ```
/// use yggdryl_core::io::fixed::Serie;
/// use yggdryl_core::io::AnySerie;
/// use yggdryl_core::io::nested::ListSerie;
///
/// // Two rows over the flat child [10, 20, 30, 40]: row 0 = [10, 20, 30], row 1 = [40].
/// let items = Serie::from_values(&[10i32, 20, 30, 40]).named("item");
/// let list = ListSerie::from_values(items, &[0, 3, 4], None).unwrap();
/// assert_eq!(list.len(), 2);
/// assert_eq!(list.row_scalar(0).len(), 3);
/// // The flat child is downcastable back to its concrete Serie.
/// let items: &Serie<i32> = list.values().as_serie::<i32>().unwrap();
/// assert_eq!(items.get(3), Some(40));
/// ```
#[derive(Debug, Clone)]
pub struct ListSerie {
    values: Box<dyn AnySerie>,
    offsets: Vec<i32>,
    validity: Option<Bitmap>,
    len: usize,
    /// The list column's **own-header** field (`List` type_id) — its name, declared nullability, and
    /// metadata. Excluded from value identity and never written to the standalone frame; the item
    /// field is derived from the flat child column.
    field: Field,
}

// A manual `PartialEq` (not a derive): the flat child is compared through `eq_any` (equal type *and*
// value), and its DERIVED item field ([`field_self`], which carries the item NAME — a list is
// unreconstructable without it) pairwise. The list's OWN name / nullability / metadata are schema
// intent, excluded. Offsets and validity are canonicalized at construction, so equal logical columns
// compare equal (and serialize byte-equal).
impl PartialEq for ListSerie {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len
            && self.offsets == other.offsets
            && self.validity == other.validity
            && self.values.field_self() == other.values.field_self()
            && self.values.eq_any(other.values.as_ref())
    }
}

impl Eq for ListSerie {}

impl ListSerie {
    /// A list column from a **self-describing** flattened child column (an erased
    /// [`AnySerie`](crate::io::AnySerie), typically named with [`named`](crate::io::AnySerie::named)),
    /// the row `offsets`, and an optional per-row **present** mask (`present[i] == false` marks row
    /// `i` a null list). The element (item) field is the child's own derived
    /// [`field_self`](crate::io::AnySerie::field_self) (its inferred type + header name); the child
    /// is stored as-is.
    ///
    /// The `offsets` must have `len + 1` entries with `offsets[0] == 0`, be non-decreasing, and end
    /// at the child length (`offsets[len] == child.len()`); otherwise a guided
    /// [`Unsupported`](IoError::Unsupported) error names the offending value and the requirement.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    /// use yggdryl_core::io::AnySerie;
    /// use yggdryl_core::io::nested::ListSerie;
    ///
    /// // 3 rows: [1, 2], [] (empty), [3] — offsets partition the flat child [1, 2, 3].
    /// let items = Serie::from_values(&[1i32, 2, 3]).named("item");
    /// let list = ListSerie::from_values(items, &[0, 2, 2, 3], None).unwrap();
    /// assert_eq!(list.len(), 3);
    /// assert_eq!(list.row_scalar(1).len(), 0); // the empty row
    /// ```
    pub fn from_values(
        items: Box<dyn AnySerie>,
        offsets: &[i32],
        present: Option<&[bool]>,
    ) -> Result<Self, IoError> {
        let len = validate_offsets(offsets, items.len())?;
        let validity = validity_from_present(present, len);
        Ok(Self {
            values: items,
            offsets: offsets.to_vec(),
            validity,
            len,
            field: Field::of("", DataTypeId::List, 0, false),
        })
    }

    /// An empty (zero-row) list column of the given schema — `offsets = [0]` over an empty child of
    /// the schema's element type.
    pub fn empty(schema: &ListField) -> Self {
        let item = schema.item().clone();
        let mut values = crate::io::nested::empty_any_column(&item);
        apply_field_header(&mut values, &item);
        Self {
            values,
            offsets: vec![0],
            validity: None,
            len: 0,
            field: Field::of("", DataTypeId::List, 0, false),
        }
    }

    // DESIGN: no `from_scalars(&[ListScalar])`. Like a struct column (see `StructSerie`), a list
    // column is built from a *flattened child column* + offsets (`from_values`), or reconstructed
    // whole via `deserialize_bytes` / `from_arrow_array`, not transposed from row scalars. A
    // row-scalar factory would have to concatenate each row's erased sub-column into one flat child
    // and re-derive the offsets — an erased-column concatenation primitive that does not exist —
    // duplicating per-family dispatch, so it is intentionally omitted here.

    /// The number of rows.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the column has no rows.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The number of null list rows.
    pub fn null_count(&self) -> usize {
        self.validity.as_ref().map_or(0, Bitmap::null_count)
    }

    /// Whether any list row is null.
    pub fn has_nulls(&self) -> bool {
        self.null_count() > 0
    }

    /// The flattened child column (as an erased [`AnySerie`](crate::io::AnySerie), downcast with
    /// `.as_serie::<T>()`). The `'static` object bound lets the borrow call the downcast helpers.
    pub fn values(&self) -> &(dyn AnySerie + 'static) {
        self.values.as_ref()
    }

    /// The row offsets (`len + 1` entries into the flattened child).
    pub fn offsets(&self) -> &[i32] {
        &self.offsets
    }

    field_accessors!();

    /// The element (item) field descriptor — **derived on demand** from the flat child column's own
    /// header (its [`field_self`](crate::io::AnySerie::field_self)); the child column is the single
    /// source of truth. Owned (there is no cached item field to borrow).
    pub fn item_field(&self) -> AnyField {
        self.values.field_self()
    }

    /// The child sub-range `[start, end)` of row `index`, or `None` if out of range. Returns the
    /// range even for a null row (its logical span in the child); use [`row`](ListSerie::row) to get
    /// a null-aware value.
    pub fn value_range(&self, index: usize) -> Option<(usize, usize)> {
        (index < self.len).then(|| {
            (
                self.offsets[index] as usize,
                self.offsets[index + 1] as usize,
            )
        })
    }

    /// The row at `index` as an erased [`AnyScalar::List`] — [`AnyScalar::Null`] if the row is null
    /// or out of range. The elements are the child sub-column for the row.
    pub fn row(&self, index: usize) -> AnyScalar {
        if index >= self.len || self.validity.as_ref().is_some_and(|v| !v.get(index)) {
            return AnyScalar::Null;
        }
        let (start, end) = (
            self.offsets[index] as usize,
            self.offsets[index + 1] as usize,
        );
        AnyScalar::list(self.values.slice(start, end - start))
    }

    /// The row at `index` as a [`ListScalar`] — its `is_null` flag reflects the top-level validity,
    /// but its elements are always populated (the child sub-range). Out of range yields a null
    /// scalar over an empty child.
    pub fn row_scalar(&self, index: usize) -> ListScalar {
        if index >= self.len {
            return ListScalar::null(self.item_field(), self.values.slice(0, 0));
        }
        let (start, end) = (
            self.offsets[index] as usize,
            self.offsets[index + 1] as usize,
        );
        let items = self.values.slice(start, end - start);
        if self.validity.as_ref().is_some_and(|v| !v.get(index)) {
            ListScalar::null(self.item_field(), items)
        } else {
            ListScalar::new(self.item_field(), items)
        }
    }

    /// The typed [`ListType`] descriptor (its element field).
    pub fn data_type(&self) -> ListType {
        ListType::new(self.item_field())
    }

    /// A [`ListField`] naming this list column, its nullability inferred from whether it holds any
    /// null rows.
    pub fn to_field(&self, name: &str) -> ListField {
        ListField::new(name, self.item_field(), self.has_nulls())
    }

    /// A **new** list column holding rows `[offset, offset + len)` — the range is clamped to the
    /// column (an out-of-range or overlong request yields the in-bounds sub-window, never a panic).
    /// The child is windowed to exactly the sliced rows' elements and the offsets are rebased to
    /// start at `0`; the top-level validity is sliced to the same window. The result is a fresh
    /// column; the original is untouched.
    pub fn slice(&self, offset: usize, len: usize) -> Self {
        let start = offset.min(self.len);
        let count = len.min(self.len - start);
        let child_start = self.offsets[start] as usize;
        let child_end = self.offsets[start + count] as usize;
        // A freshly-sliced child carries an empty header; restore the item field (its name /
        // metadata) so the derived item field survives the slice.
        let item = self.item_field();
        let mut values = self.values.slice(child_start, child_end - child_start);
        apply_field_header(&mut values, &item);
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
            values,
            offsets,
            validity: normalize(validity),
            len: count,
            field: self.field.clone(),
        }
    }

    // ---- serialization: the list schema, then validity + offsets, then the child column --------

    /// This list column's canonical bytes — a self-contained
    /// `[schema][len][validity?][offsets][child]` frame. The exact inverse of
    /// [`deserialize_bytes`](ListSerie::deserialize_bytes).
    pub fn serialize_bytes(&self) -> Vec<u8> {
        let mut sink = Bytes::new();
        self.write_frame(&mut sink)
            .expect("writing to an in-memory buffer is infallible");
        sink.as_slice().to_vec()
    }

    /// Reconstructs a list column from [`serialize_bytes`](ListSerie::serialize_bytes) bytes.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        Self::read_frame(&mut Bytes::from_slice(bytes))
    }

    /// Writes the self-contained frame to a byte sink (shared by `serialize_bytes` and the
    /// [`AnySerie`](crate::io::AnySerie) impl, so a list child serializes recursively). The schema,
    /// header, top-level validity, and offsets are packed into **one** pre-sized buffer and written
    /// in a single call; then the child column serializes itself.
    fn write_frame(&self, sink: &mut Bytes) -> Result<(), IoError> {
        // Encode the schema (a list field over the **derived** item field). Its name / metadata are
        // empty and its nullability is `has_nulls()` (not the own-header flag), so equal-in-data lists
        // serialize byte-identical regardless of the list's own name/metadata.
        let item = self.item_field();
        let mut schema = Vec::new();
        AnyField::encode_list("", self.has_nulls(), &Headers::new(), &item, &mut schema);

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
        self.values.write_to(sink)?;
        Ok(())
    }

    /// Reads a frame written by [`write_frame`](ListSerie::write_frame). Crate-visible so the shared
    /// recursive [`read_any_column`](crate::io::nested::read_any_column) dispatch can read a list
    /// child.
    pub(crate) fn read_frame(source: &mut Bytes) -> Result<Self, IoError> {
        let schema_len = read_u64(source)? as usize;
        let schema_bytes = source.read_exact_vec(schema_len)?;
        let schema = AnyField::deserialize_bytes(&schema_bytes)?;
        let item = match schema {
            AnyField::List { item, .. } => *item,
            AnyField::Leaf(_) | AnyField::Struct { .. } | AnyField::Map { .. } => {
                return Err(IoError::Unsupported {
                    what: "serialized list schema did not decode to a list".to_string(),
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
        // The child column is self-describing; read it through the shared recursive dispatch so a
        // leaf, struct, or nested list child all round-trip.
        let mut values = crate::io::nested::read_any_column(&item, source)?;
        validate_offsets(&offsets, values.len())?;
        // Restore the child's header from the item field so the derived item field round-trips.
        apply_field_header(&mut values, &item);
        Ok(Self {
            values,
            offsets,
            validity: normalize(validity),
            len,
            field: Field::of("", DataTypeId::List, 0, false),
        })
    }
}

impl SerieType for ListSerie {
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

impl AnySerie for ListSerie {
    fn len(&self) -> usize {
        self.len
    }

    fn null_count(&self) -> usize {
        ListSerie::null_count(self)
    }

    fn type_id(&self) -> DataTypeId {
        DataTypeId::List
    }

    any_serie_field_forwarding!();

    fn field(&self, name: &str) -> AnyField {
        AnyField::list_(name, self.item_field(), self.nullable() || self.has_nulls())
            .with_metadata_overlay(self.metadata())
    }

    fn value(&self, index: usize) -> AnyScalar {
        self.row(index)
    }

    fn slice(&self, offset: usize, len: usize) -> Box<dyn AnySerie> {
        Box::new(ListSerie::slice(self, offset, len))
    }

    fn write_to(&self, sink: &mut Bytes) -> Result<(), IoError> {
        self.write_frame(sink)
    }

    #[cfg(feature = "arrow")]
    fn to_arrow_array(&self) -> Result<arrow_array::ArrayRef, IoError> {
        Ok(std::sync::Arc::new(ListSerie::to_arrow_array(self)?))
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

/// Validates a list column's `offsets` against a child of `child_len` elements, returning the row
/// count (`offsets.len() - 1`) on success. Guided [`Unsupported`](IoError::Unsupported) on any
/// violation.
fn validate_offsets(offsets: &[i32], child_len: usize) -> Result<usize, IoError> {
    let Some((&first, rest)) = offsets.split_first() else {
        return Err(IoError::Unsupported {
            what: "a list column needs at least one offset (offsets = [0] for an empty column); \
                   the offsets slice was empty"
                .to_string(),
        });
    };
    if first != 0 {
        return Err(IoError::Unsupported {
            what: format!(
                "a list column's first offset must be 0, got {first}; offsets are cumulative \
                 element counts into the flattened child, starting at 0"
            ),
        });
    }
    let mut prev = first;
    for &offset in rest {
        if offset < prev {
            return Err(IoError::Unsupported {
                what: format!(
                    "a list column's offsets must be non-decreasing, but {offset} follows {prev}; \
                     each offset is a cumulative element count into the flattened child"
                ),
            });
        }
        prev = offset;
    }
    if prev as i64 != child_len as i64 {
        return Err(IoError::Unsupported {
            what: format!(
                "a list column's last offset ({prev}) must equal the flattened child length \
                 ({child_len}); the offsets must cover exactly the child column"
            ),
        });
    }
    Ok(offsets.len() - 1)
}

/// Builds a top-level validity mask from a per-row `present` slice (canonical: `None` if fully
/// present). Mirrors [`StructSerie::from_columns`](super::super::StructSerie)'s mask handling.
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

/// Reads the list's top-level validity for `len` rows (the mask read is length-bounded).
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
// Arrow interop (feature `arrow`): list column <-> ListArray.
// -------------------------------------------------------------------------------------

#[cfg(feature = "arrow")]
impl ListSerie {
    /// This list column as an Arrow [`ListArray`](arrow_array::ListArray) — **recursive**, the
    /// flattened child mapped by its [`AnySerie::to_arrow_array`](crate::io::AnySerie), the offsets
    /// as an `OffsetBuffer`, and the top-level validity as a `NullBuffer`. Fallible because a child
    /// Arrow cannot express (a temporal resolution `Minute`…`Year`) has no Arrow array.
    pub fn to_arrow_array(&self) -> Result<arrow_array::ListArray, IoError> {
        use std::sync::Arc;
        let item_field = Arc::new(self.item_field().to_arrow());
        let offsets =
            arrow_buffer::OffsetBuffer::new(arrow_buffer::ScalarBuffer::from(self.offsets.clone()));
        let values = self.values.to_arrow_array()?;
        let nulls = self.validity.as_ref().map(|bitmap| {
            let buffer = arrow_buffer::Buffer::from(bitmap.as_bytes());
            arrow_buffer::NullBuffer::new(arrow_buffer::BooleanBuffer::new(buffer, 0, self.len))
        });
        Ok(arrow_array::ListArray::new(
            item_field, offsets, values, nulls,
        ))
    }

    /// Builds a list column from an Arrow [`ListArray`](arrow_array::ListArray) and its
    /// [`Field`](arrow_schema::Field) (of `List` type), recovering the item field from the field's
    /// `List(item)` type and importing the child recursively. Reads the array's **logical** window,
    /// so a *sliced* list array converts correctly (the offsets index into the full child; the child
    /// is windowed to `[offsets[0], offsets[len])` and the offsets rebased to `0`).
    pub fn from_arrow_array(
        array: &dyn arrow_array::Array,
        field: &arrow_schema::Field,
    ) -> Result<Self, IoError> {
        use arrow_array::Array;
        let arrow_schema::DataType::List(item_field) = field.data_type() else {
            return Err(IoError::Unsupported {
                what: format!("expected an Arrow List field, got {:?}", field.data_type()),
            });
        };
        let list = array
            .as_any()
            .downcast_ref::<arrow_array::ListArray>()
            .ok_or_else(|| IoError::Unsupported {
                what: format!(
                    "expected an Arrow ListArray for field {:?}, got {:?}",
                    field.name(),
                    array.data_type()
                ),
            })?;
        let item = AnyField::from_arrow(item_field).ok_or_else(|| IoError::Unsupported {
            what: format!(
                "Arrow list item {:?} of type {:?} is not a yggdryl-modeled column type",
                item_field.name(),
                item_field.data_type()
            ),
        })?;

        let len = list.len();
        let raw_offsets = list.value_offsets(); // `len + 1` offsets into the FULL child
        let first = raw_offsets[0];
        let last = raw_offsets[len];
        // Window the full child to exactly the used range, then import it recursively.
        let child_window = list.values().slice(first as usize, (last - first) as usize);
        let mut values =
            crate::io::nested::from_arrow_any_column(child_window.as_ref(), item_field.as_ref())?;
        apply_field_header(&mut values, &item);
        let offsets: Vec<i32> = raw_offsets.iter().map(|&offset| offset - first).collect();
        let validity = list_validity_from_arrow(list);
        Ok(Self {
            values,
            offsets,
            validity,
            len,
            field: Field::of("", DataTypeId::List, 0, false),
        })
    }
}

/// The list's top-level validity from a `ListArray`, offset-aware, canonicalized (`None` if dense).
#[cfg(feature = "arrow")]
fn list_validity_from_arrow(array: &arrow_array::ListArray) -> Option<Bitmap> {
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
