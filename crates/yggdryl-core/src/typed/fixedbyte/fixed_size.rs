//! The **fixed-size byte** element types — [`FixedBinary`] and [`FixedUtf8`] — each element a fixed
//! byte width (the parameterized length), packed at that stride with **no offsets** buffer.
//!
//! They share the [`VarType`](crate::typed::VarType) bytes↔value descriptor with the variable-length
//! [`Binary`](crate::typed::Binary) / [`Utf8`](crate::typed::Utf8), but the carrier is
//! [`FixedSizeSerie`] (a single data buffer at a fixed stride) rather than the offsets+data
//! [`VarSerie`](crate::typed::VarSerie). A shorter value is **zero-padded** to the width, a longer
//! one truncated; the width lives in the [`Field`](crate::typed::Field) metadata.

use core::marker::PhantomData;

use crate::datatype_id::DataTypeId;
use crate::io::memory::{Heap, IOBase, IoError};
use crate::typed::nested::set_any_dtype_error;
use crate::typed::{FromValue, HeaderField, Scalar, Serie, Value, VarType};

/// Fixed-size **binary** — each element is exactly the column's byte width (`Vec<u8>`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FixedBinary;

impl VarType for FixedBinary {
    type Owned = Vec<u8>;
    const DATA_TYPE_ID: DataTypeId = DataTypeId::FixedBinary;
    fn to_owned(bytes: &[u8]) -> Option<Vec<u8>> {
        Some(bytes.to_vec())
    }
    fn owned_bytes(value: &Vec<u8>) -> &[u8] {
        value
    }
}

/// Fixed-size **UTF-8** — each element is exactly the column's byte width (`String`), zero-padded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FixedUtf8;

impl VarType for FixedUtf8 {
    type Owned = String;
    const DATA_TYPE_ID: DataTypeId = DataTypeId::FixedUtf8;
    fn to_owned(bytes: &[u8]) -> Option<String> {
        core::str::from_utf8(bytes).ok().map(str::to_string)
    }
    fn owned_bytes(value: &String) -> &[u8] {
        value.as_bytes()
    }
}

/// A **fixed-size column** over one data buffer `D` (default [`Heap`]) at a fixed byte `width` per
/// element, plus an optional validity buffer. Element `i` is `data[i * width..(i + 1) * width]`; a
/// shorter pushed value is zero-padded, a longer one truncated.
///
// DESIGN: there is **no separate "large fixed" type**. Unlike the variable-length family — where
// `LargeBinary` / `LargeUtf8` add a distinct marker for the wider (`i64`) offset element — a
// fixed-size column has no offsets buffer, so a large fixed-width column is simply a `FixedBinary` /
// `FixedUtf8` constructed with a big `width` (the `usize` stride is already unbounded). No new
// marker is warranted.
#[derive(Clone)]
pub struct FixedSizeSerie<T: VarType, D: IOBase = Heap> {
    data: D,
    validity: Option<D>,
    len: usize,
    width: usize,
    name: Option<Box<str>>,
    _type: PhantomData<T>,
}

impl<T: VarType> FixedSizeSerie<T, Heap> {
    /// An empty non-nullable column of the given fixed element `width` (bytes).
    pub fn new(width: usize) -> Self {
        FixedSizeSerie {
            data: Heap::new(),
            validity: None,
            len: 0,
            width,
            name: None,
            _type: PhantomData,
        }
    }

    /// An empty non-nullable column of fixed `width` pre-sized for `capacity` elements — the data
    /// buffer reserves `capacity * width` bytes, so a bounded `push` / [`append`](FixedSizeSerie::append)
    /// run does not reallocate.
    pub fn with_capacity(width: usize, capacity: usize) -> Self {
        FixedSizeSerie {
            data: Heap::with_capacity(capacity * width),
            validity: None,
            len: 0,
            width,
            name: None,
            _type: PhantomData,
        }
    }

    /// A non-nullable column of fixed `width` holding `values` (each zero-padded / truncated).
    pub fn from_values(width: usize, values: &[T::Owned]) -> Self {
        let mut column = Self::new(width);
        for value in values {
            column.push(value);
        }
        column
    }

    /// A column of fixed `width` from options. A collection with **no `None`** is **non-nullable**
    /// (no validity buffer, [`field().nullable()`](FixedSizeSerie::field) is `false`): the validity
    /// buffer is created lazily on the **first** null (via [`push_null`](FixedSizeSerie::push_null)),
    /// so a null-free build never allocates it.
    pub fn from_options(width: usize, values: &[Option<T::Owned>]) -> Self {
        let mut column = Self::new(width);
        for value in values {
            match value {
                Some(value) => column.push(value),
                None => column.push_null(),
            }
        }
        column
    }

    /// Sets the column **name** (reported by [`field`](FixedSizeSerie::field)).
    pub fn with_name(mut self, name: &str) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Appends a value's **raw bytes**, zero-padded (or truncated) to the fixed width.
    pub fn push_bytes(&mut self, bytes: &[u8]) {
        let mut slot = vec![0u8; self.width];
        let take = bytes.len().min(self.width);
        slot[..take].copy_from_slice(&bytes[..take]);
        self.data
            .pwrite_byte_array(self.len as u64 * self.width as u64, &slot);
        if let Some(validity) = self.validity.as_mut() {
            validity
                .pwrite_bit(self.len as u64, true)
                .expect("bit write into a heap");
        }
        self.len += 1;
    }

    /// Appends a **non-null** value.
    pub fn push(&mut self, value: &T::Owned) {
        self.push_bytes(T::owned_bytes(value));
    }

    /// Appends a **null** — a zero-filled slot, validity bit clear.
    pub fn push_null(&mut self) {
        self.ensure_validity();
        self.data
            .pwrite_byte_array(self.len as u64 * self.width as u64, &vec![0u8; self.width]);
        self.validity
            .as_mut()
            .expect("validity ensured")
            .pwrite_bit(self.len as u64, false)
            .expect("bit write into a heap");
        self.len += 1;
    }

    fn ensure_validity(&mut self) {
        if self.validity.is_none() {
            let mut validity = Heap::new();
            for index in 0..self.len as u64 {
                validity
                    .pwrite_bit(index, true)
                    .expect("bit write into a heap");
            }
            self.validity = Some(validity);
        }
    }

    /// A fresh sub-column copying elements `[start, start + len)` into a new in-heap
    /// [`FixedSizeSerie`] — one contiguous copy of the fixed-stride block — carrying the validity
    /// bits. The window is **clamped** to the column's length (an out-of-range window yields a
    /// shorter/empty column, never an error); the data copy is pre-sized to the exact block length.
    pub fn slice(&self, start: usize, len: usize) -> Self {
        let start = start.min(self.len);
        let count = len.min(self.len - start);
        let mut out = Self::new(self.width);
        if count > 0 {
            // One contiguous read of the fixed-stride block, one write into the fresh heap.
            let block = self
                .data
                .pread_vec(start as u64 * self.width as u64, count * self.width);
            out.data.pwrite_byte_array(0, &block);
        }
        out.len = count;
        if let Some(bits) = self.validity.as_ref() {
            let mut out_bits = Heap::with_capacity(count.div_ceil(8));
            for offset in 0..count as u64 {
                let valid = bits.pread_bit(start as u64 + offset).unwrap_or(false);
                out_bits
                    .pwrite_bit(offset, valid)
                    .expect("bit write into a fresh heap");
            }
            out.validity = Some(out_bits);
        }
        out
    }

    /// Appends `values` at the end (each zero-padded / truncated to the fixed width) — the batch
    /// counterpart of [`push`](FixedSizeSerie::push), pre-reserving the data block. All appended
    /// elements are non-null.
    pub fn append(&mut self, values: &[T::Owned]) {
        self.data.reserve((values.len() * self.width) as u64);
        for value in values {
            self.push(value);
        }
    }

    /// Appends **another column's** elements (values **and** validity). When both columns share the
    /// same fixed width the whole data block transfers in **one**
    /// [`pwrite_from`](crate::io::memory::IOBase::pwrite_from) (zero-copy when `other` is contiguous);
    /// a differing width re-pads each element. The source's per-element null-ness is reflected.
    pub fn extend<D2: IOBase>(&mut self, other: &FixedSizeSerie<T, D2>) {
        let count = other.len;
        if count == 0 {
            return;
        }
        let start = self.len;
        if other.width == self.width {
            // One bulk block copy of the contiguous fixed-stride data.
            self.data
                .pwrite_from(
                    start as u64 * self.width as u64,
                    &other.data,
                    0,
                    count as u64 * self.width as u64,
                )
                .expect("copy other's data block");
            match other.validity.as_ref() {
                Some(src_bits) => {
                    self.ensure_validity();
                    let dst_bits = self.validity.as_mut().expect("validity ensured");
                    for k in 0..count as u64 {
                        dst_bits
                            .pwrite_bit(start as u64 + k, src_bits.pread_bit(k).unwrap_or(false))
                            .expect("bit write into a heap");
                    }
                }
                None => {
                    if let Some(dst_bits) = self.validity.as_mut() {
                        for k in 0..count as u64 {
                            dst_bits
                                .pwrite_bit(start as u64 + k, true)
                                .expect("bit write into a heap");
                        }
                    }
                }
            }
            self.len += count;
        } else {
            // Widths differ — re-pad each element into this column's stride.
            self.data.reserve((count * self.width) as u64);
            for index in 0..count {
                if other.valid(index) {
                    self.push_bytes(&other.bytes_at(index).unwrap_or_default());
                } else {
                    self.push_null();
                }
            }
        }
    }

    /// Appends `count` copies of `value` at the end (zero-padded / truncated to the fixed width) —
    /// the **repeated-value fill**: the padded slot is written `count` times, never a materialized
    /// `count`-element array.
    pub fn push_repeat(&mut self, value: &T::Owned, count: usize) {
        if count == 0 {
            return;
        }
        let bytes = T::owned_bytes(value);
        let mut slot = vec![0u8; self.width];
        let take = bytes.len().min(self.width);
        slot[..take].copy_from_slice(&bytes[..take]);
        self.data.reserve((count * self.width) as u64);
        for k in 0..count {
            self.data
                .pwrite_byte_array((self.len + k) as u64 * self.width as u64, &slot);
        }
        if let Some(validity) = self.validity.as_mut() {
            for k in 0..count as u64 {
                validity
                    .pwrite_bit(self.len as u64 + k, true)
                    .expect("bit write into a heap");
            }
        }
        self.len += count;
    }

    /// A **non-nullable** column of fixed `width` holding `count` copies of `value` — the builder
    /// counterpart of [`push_repeat`](FixedSizeSerie::push_repeat).
    pub fn repeat(width: usize, value: &T::Owned, count: usize) -> Self {
        let mut column = Self::with_capacity(width, count);
        column.push_repeat(value, count);
        column
    }

    /// **Reverses element order in place** — rebuilds the fixed-stride data in reverse (see
    /// [`reverse`](FixedSizeSerie::reverse)).
    pub fn reverse_in_place(&mut self) {
        *self = self.reverse();
    }

    /// **Sorts the column ascending (lexicographically) in place** — see [`sort`](FixedSizeSerie::sort).
    pub fn sort_in_place(&mut self) {
        *self = self.sort();
    }
}

impl<T: VarType, D: IOBase> FixedSizeSerie<T, D> {
    /// Wraps an existing `data` buffer (+ optional validity) as a `len`-element fixed-`width` column.
    pub fn from_parts(data: D, validity: Option<D>, len: usize, width: usize) -> Self {
        FixedSizeSerie {
            data,
            validity,
            len,
            width,
            name: None,
            _type: PhantomData,
        }
    }

    /// The fixed byte **width** of one element.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Replaces the element at `index` **in place** with `bytes`, zero-padded (or truncated) to the
    /// fixed width (must be `< len`, else a guided [`IoError::SliceOutOfBounds`] naming the window and
    /// length). The fixed stride makes this a **direct slot write** — no tail rewrite (unlike a
    /// variable-length [`VarSerie`](crate::typed::VarSerie), which is append-only); when the column is
    /// nullable it also **marks the slot valid**.
    pub fn set(&mut self, index: usize, bytes: &[u8]) -> Result<(), IoError> {
        if index >= self.len {
            return Err(IoError::SliceOutOfBounds {
                offset: index as u64,
                len: 1,
                available: self.len as u64,
            });
        }
        self.set_checked(index, bytes);
        Ok(())
    }

    /// The **unchecked fast path** of [`set`](FixedSizeSerie::set): the same slot write with **no
    /// bounds check** (the caller guarantees `index < len`) and validity mark. An out-of-range `index`
    /// is a **silent logic error** here — it would write past the column.
    pub fn set_checked(&mut self, index: usize, bytes: &[u8]) {
        let mut slot = vec![0u8; self.width];
        let take = bytes.len().min(self.width);
        slot[..take].copy_from_slice(&bytes[..take]);
        self.data
            .pwrite_byte_array(index as u64 * self.width as u64, &slot);
        if let Some(validity) = self.validity.as_mut() {
            validity
                .pwrite_bit(index as u64, true)
                .expect("bit write into a heap");
        }
    }

    fn valid(&self, index: usize) -> bool {
        index < self.len
            && self
                .validity
                .as_ref()
                .is_none_or(|bits| bits.pread_bit(index as u64).unwrap_or(false))
    }

    /// The **raw** `width` bytes of the element at `index`, ignoring validity.
    pub fn bytes_at(&self, index: usize) -> Option<Vec<u8>> {
        (index < self.len).then(|| {
            self.data
                .pread_vec(index as u64 * self.width as u64, self.width)
        })
    }

    /// The backing data buffer.
    pub fn data(&self) -> &D {
        &self.data
    }

    /// The validity bit buffer, when nullable.
    pub fn validity(&self) -> Option<&D> {
        self.validity.as_ref()
    }

    /// The column **name**, if set — the lightweight accessor (the same value
    /// [`field`](FixedSizeSerie::field) reports), read without building a [`HeaderField`].
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Every element as its owned value, ignoring validity (a byte slot that does not decode — an
    /// invalid-UTF-8 `FixedUtf8` — is skipped). Mirrors [`VarSerie::values`](crate::typed::VarSerie).
    pub fn values(&self) -> Vec<T::Owned> {
        (0..self.len)
            .filter_map(|index| self.bytes_at(index).and_then(|bytes| T::to_owned(&bytes)))
            .collect()
    }

    /// The column's [`Field`](crate::typed::Field) — `name`, `type_id`, `nullable`, and the fixed
    /// byte `width`.
    pub fn field(&self) -> HeaderField {
        HeaderField::fixed_size(
            self.name.as_deref(),
            T::DATA_TYPE_ID,
            self.width as u32,
            self.validity.is_some(),
        )
    }

    /// **Gathers** the elements at `indices` (a permutation or any selection) into a fresh in-heap
    /// column of the same fixed width, carrying validity. An index past the column length (or a
    /// selected null) becomes a null in the result. The shared dense back end of
    /// [`mask_filter`](FixedSizeSerie::mask_filter) / [`reverse`](FixedSizeSerie::reverse) /
    /// [`sort`](FixedSizeSerie::sort).
    pub fn take(&self, indices: &[usize]) -> FixedSizeSerie<T, Heap> {
        let mut out = FixedSizeSerie::<T, Heap>::with_capacity(self.width, indices.len());
        for &index in indices {
            if index < self.len && self.valid(index) {
                out.push_bytes(&self.bytes_at(index).unwrap_or_default());
            } else {
                out.push_null();
            }
        }
        out
    }

    /// **Filters** the column by a boolean `mask` (an LSB-first bit source, `1` = keep), returning a
    /// fresh compacted in-heap column carrying the surviving elements' validity.
    pub fn mask_filter<M: IOBase>(&self, mask: &M) -> FixedSizeSerie<T, Heap> {
        let indices: Vec<usize> = (0..self.len)
            .filter(|&index| mask.pread_bit(index as u64).unwrap_or(false))
            .collect();
        self.take(&indices)
    }

    /// A fresh **reversed** copy — the fixed-stride data rebuilt in reverse element order (the copy
    /// front door of [`reverse_in_place`](FixedSizeSerie::reverse_in_place)).
    pub fn reverse(&self) -> FixedSizeSerie<T, Heap> {
        let indices: Vec<usize> = (0..self.len).rev().collect();
        self.take(&indices)
    }

    /// The **permutation that sorts** the column **lexicographically** over the element bytes.
    /// **Stable**, with **nulls last** in both directions.
    pub fn sort_indices(&self, ascending: bool) -> Vec<usize> {
        let elements: Vec<Vec<u8>> = (0..self.len)
            .map(|index| self.bytes_at(index).unwrap_or_default())
            .collect();
        let valid: Vec<bool> = (0..self.len).map(|index| self.valid(index)).collect();
        let mut indices: Vec<usize> = (0..self.len).collect();
        indices.sort_by(|&i, &j| {
            cmp_bytes_slot(valid[i], &elements[i], valid[j], &elements[j], ascending)
        });
        indices
    }

    /// A fresh **ascending-sorted** (lexicographic) copy — `take(sort_indices(true))`, the copy front
    /// door of [`sort_in_place`](FixedSizeSerie::sort_in_place). Nulls sort last.
    pub fn sort(&self) -> FixedSizeSerie<T, Heap> {
        self.take(&self.sort_indices(true))
    }
}

/// The null-aware **lexicographic** comparison of two fixed-size slots for
/// [`FixedSizeSerie::sort_indices`] — **nulls sort last** (both directions); among the non-null
/// values the `ascending` flag picks the direction over the raw element bytes.
fn cmp_bytes_slot(
    a_valid: bool,
    a: &[u8],
    b_valid: bool,
    b: &[u8],
    ascending: bool,
) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    match (a_valid, b_valid) {
        (false, false) => Ordering::Equal,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (true, true) => {
            let base = a.cmp(b);
            if ascending {
                base
            } else {
                base.reverse()
            }
        }
    }
}

impl<T: VarType, D: IOBase> Scalar for FixedSizeSerie<T, D> {
    type Value = T::Owned;

    fn data_type_id(&self) -> DataTypeId {
        T::DATA_TYPE_ID
    }

    fn len(&self) -> usize {
        self.len
    }

    fn is_valid(&self, index: usize) -> bool {
        self.valid(index)
    }

    fn get(&self, index: usize) -> Option<T::Owned> {
        if self.valid(index) {
            self.bytes_at(index).and_then(|bytes| T::to_owned(&bytes))
        } else {
            None
        }
    }
}

impl<T: VarType, D: IOBase> Serie for FixedSizeSerie<T, D> {
    /// Sets the element at `index` from an erased [`Value`] — extracts the owned value via
    /// [`FromValue`] (guided error on a dtype mismatch, naming both sides), then the fixed-stride
    /// in-place [`set`](FixedSizeSerie::set) (zero-padded / truncated to the width). Overrides the
    /// append-only default (a fixed-stride column has no tail to rewrite).
    fn set_any_scalar_at(&mut self, index: usize, value: &Value) -> Result<(), IoError>
    where
        Self::Value: FromValue,
    {
        let owned = <T::Owned as FromValue>::from_value(value)
            .ok_or_else(|| set_any_dtype_error(T::DATA_TYPE_ID, value))?;
        self.set(index, T::owned_bytes(&owned))
    }
}
