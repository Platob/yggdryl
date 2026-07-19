//! [`Serie`] — a **typed column**: many elements of one [`DataType`] over an [`IOBase`] data
//! buffer, with an optional validity bit buffer for nulls.
//!
//! `Serie: Scalar` — a series *is* a scalar generalized to `n` elements, so it shares the null-aware
//! indexed surface and adds the bulk one ([`to_options`](Serie::to_options)). The concrete
//! [`FixedSerie`] borrows the byte layer: it encodes through the source's **vectorized** typed array
//! write, decodes through the array read, and reduces through the source's
//! [`Aggregate`](crate::io::memory::Aggregate) kernels — no per-column loops of its own.
//!
//! Nulls: an absent validity buffer means **all valid** (a non-nullable column); a present one is
//! LSB-first, `1` = valid. Constructing from options builds it lazily on the first null.

use core::marker::PhantomData;

use super::{DataType, Decimal, Decoder, Encoder, HeaderField, Reduce, Scalar};
use crate::datatype_id::DataTypeId;
use crate::io::memory::{Heap, IOBase, IoError};

/// The bulk surface a [`Scalar`] gains as a column.
///
/// Beyond [`to_options`](Serie::to_options), a `Serie` inherits the **universal aggregations** that
/// depend only on [`len`](Scalar::len) / [`null_count`](Scalar::null_count) / [`get`](Scalar::get),
/// so they work for **every** element type — numeric, `bool`, byte, and utf8 alike (unlike the
/// numeric-only [`Reduce`](crate::typed::Reduce) reductions on [`FixedSerie`]). Two of them are
/// gated on the value type: [`n_unique`](Serie::n_unique) needs `Eq + Hash`, and the ordering-based
/// [`min_value`](Serie::min_value) / [`max_value`](Serie::max_value) need `Ord` (so a float column,
/// whose `f64` value is not `Ord`, has no `min_value`/`max_value` — it uses the NaN-safe numeric
/// `min`/`max` instead).
pub trait Serie: Scalar {
    /// Every element as an option, null-aware, decoded into a fresh `Vec`.
    fn to_options(&self) -> Vec<Option<Self::Value>> {
        (0..self.len()).map(|index| self.get(index)).collect()
    }

    /// The **total** element count (nulls included) — an alias of [`len`](Scalar::len).
    fn count(&self) -> usize {
        self.len()
    }

    /// The count of **non-null** elements — `len - null_count`.
    fn valid_count(&self) -> usize {
        self.len() - self.null_count()
    }

    /// The **first** element (null-aware, at index 0); `None` when empty or the element is null.
    fn first_value(&self) -> Option<Self::Value> {
        self.get(0)
    }

    /// The **last** element (null-aware, at `len - 1`); `None` when empty or the element is null.
    fn last_value(&self) -> Option<Self::Value> {
        if self.is_empty() {
            None
        } else {
            self.get(self.len() - 1)
        }
    }

    /// The count of **distinct non-null** values. Collects the valid values into a
    /// [`HashSet`](std::collections::HashSet) — the one allocation is inherent to distinct-counting.
    fn n_unique(&self) -> usize
    where
        Self::Value: Eq + core::hash::Hash,
    {
        (0..self.len())
            .filter_map(|index| self.get(index))
            .collect::<std::collections::HashSet<Self::Value>>()
            .len()
    }

    /// The **ordering-based** minimum over non-null values (a streamed fold, no sort); `None` when
    /// there are no non-null values. Available only for `Ord` value types — so it gives byte / utf8
    /// / integer / bool columns a lexicographic-or-numeric min, while a float column (not `Ord`)
    /// uses the NaN-safe numeric `min` instead.
    fn min_value(&self) -> Option<Self::Value>
    where
        Self::Value: Ord,
    {
        (0..self.len()).filter_map(|index| self.get(index)).min()
    }

    /// The **ordering-based** maximum over non-null values (a streamed fold, no sort); `None` when
    /// there are no non-null values. Available only for `Ord` value types (see
    /// [`min_value`](Serie::min_value)).
    fn max_value(&self) -> Option<Self::Value>
    where
        Self::Value: Ord,
    {
        (0..self.len()).filter_map(|index| self.get(index)).max()
    }
}

/// A **typed column** over an [`IOBase`] data buffer `D` (default [`Heap`]) plus an optional
/// validity buffer. Elements are packed at the type's stride; reads/writes/reductions all forward
/// to the byte layer's vectorized kernels.
pub struct FixedSerie<T: DataType, D: IOBase = Heap> {
    data: D,
    validity: Option<D>,
    len: usize,
    name: Option<Box<str>>,
    /// Decimal precision / scale metadata — set only for decimal columns (see
    /// [`with_precision_scale`](FixedSerie::with_precision_scale)).
    precision: Option<u32>,
    scale: Option<i32>,
    _type: PhantomData<T>,
}

impl<T: Encoder + Decoder> FixedSerie<T, Heap> {
    /// An empty non-nullable column.
    pub fn new() -> Self {
        // The type identity lives at the compile-time `T` and the `field()` metadata; the raw data
        // buffer stays untagged bytes so a build costs only its data allocation.
        FixedSerie {
            data: Heap::new(),
            validity: None,
            len: 0,
            name: None,
            precision: None,
            scale: None,
            _type: PhantomData,
        }
    }

    /// An empty non-nullable column with room for `capacity` elements before reallocating.
    pub fn with_capacity(capacity: usize) -> Self {
        FixedSerie {
            data: Heap::with_capacity(capacity * T::byte_width() as usize),
            validity: None,
            len: 0,
            name: None,
            precision: None,
            scale: None,
            _type: PhantomData,
        }
    }

    /// A non-nullable column holding `values` (encoded in one vectorized bulk write).
    pub fn from_values(values: &[T::Native]) -> Self {
        let mut column = Self::with_capacity(values.len());
        T::encode_slice(&mut column.data, 0, values).expect("encode into a fresh heap never fails");
        column.len = values.len();
        column
    }

    /// A column from options — builds the validity buffer, encoding a default in each null slot.
    /// Bulk: one vectorized data write (nulls → default) and one packed validity write, rather than
    /// an element-by-element push (which reallocated the growing validity buffer).
    pub fn from_options(values: &[Option<T::Native>]) -> Self {
        let mut column = Self::with_capacity(values.len());
        // The data buffer: every value (a null slot gets the type default), one vectorized write.
        let natives: Vec<T::Native> = values.iter().map(|v| v.unwrap_or_default()).collect();
        T::encode_slice(&mut column.data, 0, &natives)
            .expect("encode into a fresh heap never fails");
        column.len = values.len();
        // The validity bitmap: pre-packed LSB-first (1 = valid), one byte-array write (no per-bit growth).
        let mut bits = vec![0u8; values.len().div_ceil(8)];
        for (index, value) in values.iter().enumerate() {
            if value.is_some() {
                bits[index / 8] |= 1 << (index % 8);
            }
        }
        let mut validity = Heap::with_capacity(bits.len());
        validity.pwrite_byte_array(0, &bits);
        column.validity = Some(validity);
        column
    }

    /// Sets the column **name** (the metadata a [`field`](FixedSerie::field) reports).
    pub fn with_name(mut self, name: &str) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the **decimal precision + scale** metadata (for a decimal column) — reported by
    /// [`field`](FixedSerie::field) and used by
    /// [`to_decimal_string`](FixedSerie::to_decimal_string) to place the decimal point.
    pub fn with_precision_scale(mut self, precision: u32, scale: i32) -> Self {
        self.precision = Some(precision);
        self.scale = Some(scale);
        self
    }

    /// Appends a **non-null** `value`.
    pub fn push(&mut self, value: T::Native) {
        T::encode(&mut self.data, self.len as u64, value).expect("encode into a heap never fails");
        if let Some(validity) = self.validity.as_mut() {
            validity
                .pwrite_bit(self.len as u64, true)
                .expect("bit write never fails");
        }
        self.len += 1;
    }

    /// Appends a **null** (creating + back-filling the validity buffer on the first null).
    pub fn push_null(&mut self) {
        self.ensure_validity();
        T::encode(&mut self.data, self.len as u64, T::Native::default())
            .expect("encode into a heap never fails");
        self.validity
            .as_mut()
            .expect("validity ensured")
            .pwrite_bit(self.len as u64, false)
            .expect("bit write never fails");
        self.len += 1;
    }

    /// Appends an option — [`push`](FixedSerie::push) / [`push_null`](FixedSerie::push_null).
    pub fn push_option(&mut self, value: Option<T::Native>) {
        match value {
            Some(value) => self.push(value),
            None => self.push_null(),
        }
    }

    /// Appends `values` in **one vectorized bulk write** — the batch counterpart of
    /// [`push`](FixedSerie::push), avoiding the per-element call overhead when growing a column
    /// from a slice. All appended elements are non-null.
    pub fn extend(&mut self, values: &[T::Native]) {
        T::encode_slice(&mut self.data, self.len as u64, values)
            .expect("encode into a heap never fails");
        if let Some(validity) = self.validity.as_mut() {
            for offset in 0..values.len() as u64 {
                validity
                    .pwrite_bit(self.len as u64 + offset, true)
                    .expect("bit write never fails");
            }
        }
        self.len += values.len();
    }

    /// **Filters** the column by a boolean `mask` (an LSB-first bit source, `1` = keep), returning a
    /// fresh compacted column. DESIGN: the scaffold compacts element-by-element; the vectorized path
    /// is [`IOBase::mask_filter`](crate::io::memory::IOBase::mask_filter) over the data buffer with a
    /// rebuilt validity — wired here once the null-aware bitmap compaction lands.
    pub fn filter<M: IOBase>(&self, mask: &M) -> Self {
        let mut out = Self::new();
        if self.validity.is_some() {
            out.ensure_validity();
        }
        for index in 0..self.len {
            if mask.pread_bit(index as u64).unwrap_or(false) {
                out.push_option(self.get(index));
            }
        }
        out
    }
}

impl<T: Encoder + Decoder> Default for FixedSerie<T, Heap> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Decoder, D: IOBase> FixedSerie<T, D> {
    /// Wraps an existing `data` buffer (and optional `validity`) as a `len`-element column — the
    /// zero-copy **"view any [`IOBase`] as a typed column"** front door: `data` may be a mapped file
    /// ([`Mmap`](crate::io::local::Mmap)), a device buffer, or an in-heap [`Heap`], read in place.
    /// The caller guarantees `data` holds `len` encoded elements (and `validity`, if present, holds
    /// `len` bits).
    pub fn from_data(data: D, validity: Option<D>, len: usize) -> Self {
        FixedSerie {
            data,
            validity,
            len,
            name: None,
            precision: None,
            scale: None,
            _type: PhantomData,
        }
    }

    /// Whether the element at `index` is valid (non-null).
    fn valid(&self, index: usize) -> bool {
        index < self.len
            && self
                .validity
                .as_ref()
                .is_none_or(|bits| bits.pread_bit(index as u64).unwrap_or(false))
    }

    /// The **raw** values (validity ignored) decoded in one vectorized bulk read — null slots
    /// surface their stored default. Pair with [`is_valid`](Scalar::is_valid) for null-awareness.
    pub fn values(&self) -> Vec<T::Native> {
        let mut out = vec![T::Native::default(); self.len];
        if self.len > 0 {
            T::decode_slice(&self.data, 0, &mut out).expect("decode over a valid buffer");
        }
        out
    }

    /// The backing data buffer (borrowed).
    pub fn data(&self) -> &D {
        &self.data
    }

    /// The validity bit buffer, when the column is nullable.
    pub fn validity(&self) -> Option<&D> {
        self.validity.as_ref()
    }

    /// The column's [`Field`] metadata — its `name`, `type_id`, `nullable` flag, and (for a decimal
    /// column) its `precision` / `scale`.
    pub fn field(&self) -> HeaderField {
        let nullable = self.validity.is_some();
        match (self.precision, self.scale) {
            (Some(precision), Some(scale)) => HeaderField::decimal(
                self.name.as_deref(),
                T::DATA_TYPE_ID,
                precision,
                scale,
                nullable,
            ),
            _ => HeaderField::new(self.name.as_deref(), T::DATA_TYPE_ID, nullable),
        }
    }
}

/// Element + range **mutators** and a read-range slice — the in-place edit surface over the typed
/// column. Every write reuses the column's existing backing (no reallocation): a scalar `set` rewrites
/// one element slot, a bulk `set_range` rewrites a contiguous block through the source's **vectorized**
/// typed array write, and each keeps the validity bitmap in sync. The `*_checked` twins are the
/// **unchecked fast paths** — the caller pre-validated `index < len`, so they skip the bounds check.
impl<T: Encoder + Decoder, D: IOBase> FixedSerie<T, D> {
    /// The guided out-of-range error for a positioned/range write whose window `[offset, offset+len)`
    /// runs past the column length — states the offending window and length, and its fix ("request a
    /// window that fits").
    fn window_error(&self, offset: usize, len: usize) -> IoError {
        IoError::SliceOutOfBounds {
            offset: offset as u64,
            len: len as u64,
            available: self.len as u64,
        }
    }

    /// Ensures a validity buffer exists, back-filling every existing element as valid — the generic
    /// backing counterpart of the [`Heap`] builders' back-fill (creates an empty `D` on the first
    /// null). One implementation for every backing that can be constructed empty.
    fn ensure_validity(&mut self)
    where
        D: Default,
    {
        if self.validity.is_none() {
            let mut validity = D::default();
            for index in 0..self.len as u64 {
                validity
                    .pwrite_bit(index, true)
                    .expect("bit write never fails");
            }
            self.validity = Some(validity);
        }
    }

    /// Replaces the element at `index` **in place** (must be `< len`, else a guided
    /// [`IoError::SliceOutOfBounds`] naming the fix — set within `0..len`, or [`push`](FixedSerie::push)
    /// to append). Encodes `value` into the element's slot with **no reallocation**; when the column is
    /// nullable this also **marks the slot valid** (a previously-null slot becomes present).
    pub fn set(&mut self, index: usize, value: T::Native) -> Result<(), IoError> {
        if index >= self.len {
            return Err(self.window_error(index, 1));
        }
        self.set_checked(index, value);
        Ok(())
    }

    /// The **unchecked fast path** of [`set`](FixedSerie::set): rewrites the element slot with **no
    /// bounds check** (the caller guarantees `index < len`) and marks it valid. An out-of-range
    /// `index` is a **silent logic error** here — it would write past the column, corrupting length /
    /// element alignment; use [`set`](FixedSerie::set) unless the index is already validated.
    pub fn set_checked(&mut self, index: usize, value: T::Native) {
        T::encode(&mut self.data, index as u64, value).expect("encode into an existing slot");
        if let Some(validity) = self.validity.as_mut() {
            validity
                .pwrite_bit(index as u64, true)
                .expect("bit write never fails");
        }
    }

    /// **Nulls** the element at `index` (must be `< len`, else the guided
    /// [`IoError::SliceOutOfBounds`]) — ensures a validity buffer exists (back-filling existing
    /// elements as valid on the first null) and **clears** the bit at `index`. The stored data byte is
    /// left as-is; validity alone decides null-ness.
    pub fn set_null(&mut self, index: usize) -> Result<(), IoError>
    where
        D: Default,
    {
        if index >= self.len {
            return Err(self.window_error(index, 1));
        }
        self.ensure_validity();
        self.validity
            .as_mut()
            .expect("validity ensured")
            .pwrite_bit(index as u64, false)
            .expect("bit write never fails");
        Ok(())
    }

    /// A fresh sub-column copying elements `[start, start + len)` into a new in-heap
    /// [`FixedSerie`], carrying the matching validity bits. The window is **clamped** to the column's
    /// length — an out-of-range `start` or an over-long `len` yields a shorter (or empty) column, never
    /// an error. Both the data and (when nullable) the validity copy are **pre-sized** to the exact
    /// element count.
    pub fn slice(&self, start: usize, len: usize) -> FixedSerie<T, Heap> {
        let start = start.min(self.len);
        let count = len.min(self.len - start);
        let mut out = FixedSerie::<T, Heap>::with_capacity(count);
        if count > 0 {
            // One vectorized bulk read out of `self`, one vectorized bulk write into the fresh heap.
            let mut values = vec![T::Native::default(); count];
            T::decode_slice(&self.data, start as u64, &mut values)
                .expect("decode over a valid buffer");
            T::encode_slice(&mut out.data, 0, &values)
                .expect("encode into a fresh heap never fails");
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

    /// **Bulk in-place replace** of `values.len()` elements starting at `start` — requires
    /// `start + values.len() <= len` (else the guided [`IoError::SliceOutOfBounds`]). One **dense**
    /// vectorized typed-array write (no per-element loop); when the column is nullable the whole range
    /// is **marked valid**.
    pub fn set_range(&mut self, start: usize, values: &[T::Native]) -> Result<(), IoError> {
        if start
            .checked_add(values.len())
            .is_none_or(|end| end > self.len)
        {
            return Err(self.window_error(start, values.len()));
        }
        self.set_range_checked(start, values);
        Ok(())
    }

    /// The **unchecked bulk twin** of [`set_range`](FixedSerie::set_range): the same dense vectorized
    /// write with **no bounds check** (the caller guarantees `start + values.len() <= len`). An
    /// out-of-range window is a **silent logic error** — it would write past the column.
    pub fn set_range_checked(&mut self, start: usize, values: &[T::Native]) {
        T::encode_slice(&mut self.data, start as u64, values)
            .expect("encode into an existing range");
        if let Some(validity) = self.validity.as_mut() {
            for offset in 0..values.len() as u64 {
                validity
                    .pwrite_bit(start as u64 + offset, true)
                    .expect("bit write never fails");
            }
        }
    }

    /// Sets the range `[start, start + other.len())` from **another column's values and validity**
    /// (bounds-checked — requires `start + other.len() <= len`, else the guided
    /// [`IoError::SliceOutOfBounds`]). Reuses the vectorized bulk path for the values; the source's
    /// per-element null-ness is reflected across the range (a nullable `other` makes the target
    /// nullable, back-filling a validity buffer if it had none — an all-valid `other` marks the range
    /// valid).
    pub fn set_range_serie<D2: IOBase>(
        &mut self,
        start: usize,
        other: &FixedSerie<T, D2>,
    ) -> Result<(), IoError>
    where
        D: Default,
    {
        let count = other.len;
        if start.checked_add(count).is_none_or(|end| end > self.len) {
            return Err(self.window_error(start, count));
        }
        // Values: one bulk decode out of `other`, one bulk (vectorized) encode into `self`'s range.
        let values = other.values();
        T::encode_slice(&mut self.data, start as u64, &values)
            .expect("encode into an existing range");
        // Validity: reflect the source's per-element null-ness across the written range.
        match other.validity.as_ref() {
            Some(src_bits) => {
                self.ensure_validity();
                let dst_bits = self.validity.as_mut().expect("validity ensured");
                for offset in 0..count as u64 {
                    let valid = src_bits.pread_bit(offset).unwrap_or(false);
                    dst_bits
                        .pwrite_bit(start as u64 + offset, valid)
                        .expect("bit write never fails");
                }
            }
            None => {
                if let Some(dst_bits) = self.validity.as_mut() {
                    for offset in 0..count as u64 {
                        dst_bits
                            .pwrite_bit(start as u64 + offset, true)
                            .expect("bit write never fails");
                    }
                }
            }
        }
        Ok(())
    }
}

impl<T: Decoder, D: IOBase> Scalar for FixedSerie<T, D> {
    type Value = T::Native;

    fn data_type_id(&self) -> DataTypeId {
        T::DATA_TYPE_ID
    }

    fn len(&self) -> usize {
        self.len
    }

    fn is_valid(&self, index: usize) -> bool {
        self.valid(index)
    }

    fn get(&self, index: usize) -> Option<T::Native> {
        if self.valid(index) {
            T::decode(&self.data, index as u64).ok()
        } else {
            None
        }
    }
}

impl<T: Decoder, D: IOBase> Serie for FixedSerie<T, D> {}

/// Numeric reductions — routed to the data buffer's [`Aggregate`](crate::io::memory::Aggregate)
/// kernels. DESIGN: these reduce over the **physical** buffer (a null slot contributes its stored
/// default), so they are exact for a non-nullable column; the null-aware reduction (skip via the
/// validity bitmap) is the marked optimization seam.
impl<T: Reduce + Decoder, D: IOBase> FixedSerie<T, D> {
    /// The **sum** of every element.
    pub fn sum(&self) -> Result<T::Sum, IoError> {
        T::sum(&self.data, 0, self.len)
    }

    /// The **minimum** element (a float min ignores NaN); `None` when empty.
    pub fn min(&self) -> Result<Option<T::Native>, IoError> {
        T::min(&self.data, 0, self.len)
    }

    /// The **maximum** element (a float max ignores NaN); `None` when empty.
    pub fn max(&self) -> Result<Option<T::Native>, IoError> {
        T::max(&self.data, 0, self.len)
    }

    /// The **mean** as `f64`; `None` when empty.
    pub fn mean(&self) -> Result<Option<f64>, IoError> {
        T::mean(&self.data, 0, self.len)
    }

    /// The **population standard deviation** as `f64` (the `sqrt` of the variance); `None` when empty.
    pub fn std(&self) -> Result<Option<f64>, IoError> {
        T::std(&self.data, 0, self.len)
    }

    /// The **population variance** as `f64` (`std²`); `None` when empty.
    pub fn var(&self) -> Result<Option<f64>, IoError> {
        T::var(&self.data, 0, self.len)
    }

    /// The **median** as `f64`; `None` when empty. Materializes + sorts the values (an order
    /// statistic — the single allocation is inherent).
    pub fn median(&self) -> Result<Option<f64>, IoError> {
        T::median(&self.data, 0, self.len)
    }

    /// The **first** element (positional); `None` when empty.
    pub fn first(&self) -> Result<Option<T::Native>, IoError> {
        T::first(&self.data, 0, self.len)
    }

    /// The **last** element (positional); `None` when empty.
    pub fn last(&self) -> Result<Option<T::Native>, IoError> {
        T::last(&self.data, 0, self.len)
    }

    /// How many elements are `>= threshold`.
    pub fn count_ge(&self, threshold: T::Native) -> Result<usize, IoError> {
        T::count_ge(&self.data, 0, self.len, threshold)
    }
}

/// Decimal interoperability — the precision/scale metadata and scale-aware string formatting for a
/// decimal column (`FixedSerie<Decimal32|64|128|256>`).
impl<T: Decimal + Decoder, D: IOBase> FixedSerie<T, D>
where
    T::Native: core::fmt::Display,
{
    /// The decimal **scale** (decimal places) — the set value, else `0`.
    pub fn decimal_scale(&self) -> i32 {
        self.scale.unwrap_or(0)
    }

    /// The decimal **precision** (max significant digits) — the set value, else the type's max.
    pub fn decimal_precision(&self) -> u32 {
        self.precision.unwrap_or(T::MAX_PRECISION)
    }

    /// The decimal value at `index` formatted with the column's scale (e.g. `"123.45"`), or `None`
    /// when the element is null or out of range — the easy human-readable interop.
    pub fn to_decimal_string(&self, index: usize) -> Option<String> {
        self.get(index)
            .map(|value| T::format(value, self.decimal_scale()))
    }
}
