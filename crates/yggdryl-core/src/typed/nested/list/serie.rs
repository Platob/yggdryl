//! [`ListSerie`] — the **variable-length list column**: an `i32`-offsets buffer + a flattened child
//! [`Column`] of the concatenated elements, plus an optional element-level validity buffer. List `i`
//! is the child sub-range `values[offsets[i]..offsets[i + 1]]` — the Arrow `List` layout over the
//! keystone.
//!
//! It mirrors the [`StructSerie`](super::super::StructSerie) shape: it implements
//! [`Scalar`] / [`Serie`] (its element is a [`ListScalar`]), so a list is itself a column and nests
//! inside a struct (or another list), and its [`values_mut`](ListSerie::values_mut) hands back a
//! `&mut Column` so a caller **deep-mutates the flattened child series in place, no copy** (matching
//! the public [`Column`] variant to recover the concrete series).
//!
//! ```
//! use yggdryl_core::typed::fixedbyte::Int64;
//! use yggdryl_core::typed::{Column, FixedSerie, ListSerie, Scalar, Value};
//!
//! // The flattened child [1, 2, 3, 4, 5]; three lists are demarcated by `push(child_len)`.
//! let child = FixedSerie::<Int64>::from_values(&[1, 2, 3, 4, 5]);
//! let mut list = ListSerie::new("nums", Column::from(child));
//! list.push(2); // [1, 2]
//! list.push(0); // []
//! list.push(3); // [3, 4, 5]
//!
//! assert_eq!(list.len(), 3);
//! assert_eq!(list.list_at(2), Some((2, 5)));
//! match list.get(2) {
//!     Value::List(scalar) => {
//!         assert_eq!(scalar.len(), 3);
//!         assert_eq!(scalar.get(0), Some(&Value::Int64(3)));
//!     }
//!     other => panic!("expected a list element, got {other:?}"),
//! }
//! ```

use crate::datatype_id::DataTypeId;
use crate::headers::Headers;
use crate::io::memory::{Heap, IOBase};
use crate::typed::nested::{Column, ListField, ListScalar, Value};
use crate::typed::{Scalar, Serie};

/// The i32 offset element width, in bytes.
const OFFSET_WIDTH: u64 = 4;

/// A **list column** over an `i32`-offsets [`Heap`] + a flattened child [`Column`], plus an optional
/// element-level validity buffer. `offsets` holds `len + 1` little-endian `i32`s with
/// `offsets[0] == 0`; list `i` occupies the child range `[offsets[i], offsets[i + 1])`. `validity`,
/// when present, is the element-level null bitmap (LSB-first, `1` = valid); `len` is the list count.
pub struct ListSerie {
    offsets: Heap,
    values: Box<Column>,
    validity: Option<Heap>,
    len: usize,
    name: Option<Box<str>>,
    metadata: Headers,
}

impl ListSerie {
    /// An **empty** list column named `name` over the flattened child `values` (its rows become the
    /// list elements as [`push`](ListSerie::push) demarcates them). `offsets[0]` is seeded to `0`.
    pub fn new(name: &str, values: Column) -> Self {
        let mut offsets = Heap::new();
        offsets
            .pwrite_i32(0, 0)
            .expect("offset[0] into a fresh heap never fails");
        ListSerie {
            offsets,
            values: Box::new(values),
            validity: None,
            len: 0,
            name: Some(name.into()),
            metadata: Headers::new(),
        }
    }

    /// Wraps an existing `offsets` buffer (`len + 1` little-endian `i32`s, `offsets[0] == 0`), the
    /// flattened `values` child, and an optional element-level `validity` bitmap as a `len`-element
    /// list column — the zero-copy "view existing buffers as a list" front door.
    pub fn from_offsets(
        name: &str,
        offsets: Heap,
        values: Column,
        validity: Option<Heap>,
        len: usize,
    ) -> Self {
        ListSerie {
            offsets,
            values: Box::new(values),
            validity,
            len,
            name: Some(name.into()),
            metadata: Headers::new(),
        }
    }

    /// Sets the list **name**, chainable.
    pub fn with_name(mut self, name: &str) -> Self {
        self.name = Some(name.into());
        self
    }

    /// The byte end of the current content — `offsets[len]` (the running child cursor).
    fn end_offset(&self) -> i32 {
        self.offsets
            .pread_i32(self.len as u64 * OFFSET_WIDTH)
            .unwrap_or(0)
    }

    /// Ensures an element-level validity buffer exists, back-filling every existing list as valid
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

    /// Appends a **non-null** list spanning the next `child_len` rows of the flattened
    /// [`values`](ListSerie::values) child. The caller has already staged those rows into the child
    /// (e.g. via [`values_mut`](ListSerie::values_mut)); this only advances the offset that
    /// demarcates the new sub-list.
    pub fn push(&mut self, child_len: usize) {
        let start = self.end_offset();
        let end = start + child_len as i32;
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

    /// Appends a **null** list — an empty span (`offsets[len + 1] == offsets[len]`) with the
    /// validity bit cleared (back-filling the validity buffer on the first null).
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

    /// The child range `[start, end)` of list `index` in the flattened
    /// [`values`](ListSerie::values) child — `None` when `index` is out of range.
    pub fn list_at(&self, index: usize) -> Option<(usize, usize)> {
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

    /// The **list element** at `index` — a [`ListScalar`] materializing the child sub-range as owned
    /// [`Value`]s, carrying the element-level validity — or `None` when `index` is out of range.
    pub fn list(&self, index: usize) -> Option<ListScalar> {
        let (start, end) = self.list_at(index)?;
        let mut values = Vec::with_capacity(end.saturating_sub(start));
        for child in start..end {
            values.push(self.values.get(child));
        }
        Some(ListScalar::new(values, self.is_valid(index)))
    }

    /// The list element at `index`, erased to a [`Value`] — [`Value::List`] when valid, else
    /// [`Value::Null`] (a null list or an out-of-range index).
    pub fn get(&self, index: usize) -> Value {
        match self.list(index) {
            Some(scalar) if !scalar.is_null() => Value::List(scalar),
            _ => Value::Null,
        }
    }

    /// The number of lists.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the column has no lists.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether the list at `index` is valid (non-null). Out of range is `false`.
    pub fn is_valid(&self, index: usize) -> bool {
        index < self.len
            && self
                .validity
                .as_ref()
                .is_none_or(|bits| bits.pread_bit(index as u64).unwrap_or(false))
    }

    /// How many lists are null.
    pub fn null_count(&self) -> usize {
        match &self.validity {
            None => 0,
            Some(bits) => (0..self.len)
                .filter(|&index| !bits.pread_bit(index as u64).unwrap_or(false))
                .count(),
        }
    }

    /// The flattened child [`Column`] holding every list's elements (borrowed) — the downward graph
    /// edge.
    pub fn values(&self) -> &Column {
        self.values.as_ref()
    }

    /// The flattened child [`Column`], **mutable** — the front door to a deep, in-place edit of the
    /// elements: match the returned `&mut Column`'s public variant to recover the concrete series and
    /// mutate it (e.g. `if let Column::Int64(serie) = column { serie.set(0, 99)?; }`), visible on the
    /// next [`get`](ListSerie::get) with **no copy**.
    pub fn values_mut(&mut self) -> &mut Column {
        self.values.as_mut()
    }

    /// The list's name, if set.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// The free-form metadata map (borrowed).
    pub fn metadata(&self) -> &Headers {
        &self.metadata
    }

    /// The free-form metadata map (mutable).
    pub fn metadata_mut(&mut self) -> &mut Headers {
        &mut self.metadata
    }

    /// The list's [`ListField`] schema — derived from the child column's field plus the list's name,
    /// nullability (whether a validity buffer is present), and metadata.
    pub fn field(&self) -> ListField {
        let mut field = ListField::new(self.name.as_deref(), self.values.field());
        field.set_nullable(self.validity.is_some());
        *field.metadata_mut() = self.metadata.clone();
        field
    }
}

// A list is itself a column whose element is a sub-list. The `Scalar` methods delegate to the
// inherent implementations (fully-qualified so the path resolves to the inherent method, never back
// to the trait — no recursion).
impl Scalar for ListSerie {
    type Value = ListScalar;

    fn data_type_id(&self) -> DataTypeId {
        DataTypeId::List
    }

    fn len(&self) -> usize {
        ListSerie::len(self)
    }

    fn is_valid(&self, index: usize) -> bool {
        ListSerie::is_valid(self, index)
    }

    fn null_count(&self) -> usize {
        ListSerie::null_count(self)
    }

    fn get(&self, index: usize) -> Option<ListScalar> {
        self.list(index)
    }
}

impl Serie for ListSerie {
    // The one graph edge is the flattened child column; nested owns its child **downward** (no
    // `parent()` up-pointer).
    fn children(&self) -> Vec<&Column> {
        vec![self.values.as_ref()]
    }
}
