//! [`VarSerie`] — a **variable-length typed column**: an **offsets** buffer + a **data** buffer
//! (element `i` is `data[offsets[i]..offsets[i + 1]]`), plus an optional validity bit buffer.
//!
//! This is the Arrow variable-length layout over the [`IOBase`] contract: the offsets and data are
//! each an `IOBase` source, so a `Binary` / `Utf8` column is in-heap, memory-mapped, or on device
//! memory with no change to its surface. It implements the same [`Scalar`] / [`Serie`] traits the
//! fixed families do — its `Value` is the type's owned form (`Vec<u8>` / `String`).
//!
//! The **offset element width** is generic, chosen by the marker's [`VarLenType::Offset`]: the
//! default `Binary` / `Utf8` use `i32` offsets (4 bytes each), the `LargeBinary` / `LargeUtf8` use
//! `i64` offsets (8 bytes — Arrow's `Large*`), for data past the `i32` offset range. Every offset op
//! goes through [`VarOffset`], so the carrier is one implementation across both widths.

use core::marker::PhantomData;

use crate::datatype_id::DataTypeId;
use crate::io::memory::{Heap, IOBase, IoError};
use crate::typed::{HeaderField, Scalar, Serie, VarLenType, VarOffset};

/// A **variable-length column** over an offsets buffer + a data buffer (default [`Heap`]), plus an
/// optional validity buffer. Element `i` occupies `data[offsets[i]..offsets[i + 1]]`. The offset
/// element width (`i32` or `i64`) is the marker's [`VarLenType::Offset`].
#[derive(Clone)]
pub struct VarSerie<T: VarLenType, D: IOBase = Heap> {
    /// `len + 1` little-endian offsets of width `T::Offset::WIDTH`; `offsets[0] == 0`.
    offsets: D,
    /// The concatenated element bytes.
    data: D,
    validity: Option<D>,
    len: usize,
    name: Option<Box<str>>,
    /// The optional **max element width** (bytes) — a schema bound enforced on the checked append
    /// path ([`try_push`](VarSerie::try_push) / [`try_push_bytes`](VarSerie::try_push_bytes)) and
    /// reported as the field's [`byte_width`](HeaderField::byte_width). `None` means unbounded.
    max_width: Option<usize>,
    _type: PhantomData<T>,
}

/// The guided [`IoError::TypedCast`] for a value that overruns a variable-length column's declared
/// **max element width** — names the offending element `index`, its `width`, the `max`, and the fix.
/// Shared by [`set_max_width`](VarSerie::set_max_width) / [`with_max_width`](VarSerie::with_max_width)
/// (validating existing elements) and the checked appends so every message reads identically.
fn max_width_error(index: usize, width: usize, max: usize) -> IoError {
    IoError::TypedCast {
        detail: format!(
            "element {index} is {width} bytes, over the column's max width of {max}: shorten the \
             value or raise the max width"
        ),
    }
}

impl<T: VarLenType> VarSerie<T, Heap> {
    /// An empty non-nullable column.
    pub fn new() -> Self {
        let mut offsets = Heap::new();
        T::Offset::write(&mut offsets, 0, 0).expect("offset[0] into a fresh heap");
        VarSerie {
            offsets,
            data: Heap::new(),
            validity: None,
            len: 0,
            name: None,
            max_width: None,
            _type: PhantomData,
        }
    }

    /// An empty non-nullable column pre-sized for `capacity` elements: the **offsets** buffer holds
    /// `capacity + 1` entries and the **data** buffer reserves `capacity` bytes (a one-byte-per-element
    /// lower bound), so a bounded `push` / [`append`](VarSerie::append) run does not reallocate.
    pub fn with_capacity(capacity: usize) -> Self {
        let mut offsets = Heap::with_capacity((capacity + 1) * T::Offset::WIDTH as usize);
        T::Offset::write(&mut offsets, 0, 0).expect("offset[0] into a fresh heap");
        VarSerie {
            offsets,
            data: Heap::with_capacity(capacity),
            validity: None,
            len: 0,
            name: None,
            max_width: None,
            _type: PhantomData,
        }
    }

    /// A non-nullable column holding `values`.
    pub fn from_values(values: &[T::Owned]) -> Self {
        let mut column = Self::new();
        for value in values {
            column.push(value);
        }
        column
    }

    /// A column from options — pushing a null (an empty span) where a value is absent. A collection
    /// with **no `None`** is **non-nullable** (no validity buffer, [`field().nullable()`](VarSerie::field)
    /// is `false`): the validity buffer is created lazily on the **first** null (via
    /// [`push_null`](VarSerie::push_null)), so a null-free build never allocates it.
    pub fn from_options(values: &[Option<T::Owned>]) -> Self {
        let mut column = Self::new();
        for value in values {
            match value {
                Some(value) => column.push(value),
                None => column.push_null(),
            }
        }
        column
    }

    /// Sets the column **name** (reported by [`field`](VarSerie::field)).
    pub fn with_name(mut self, name: &str) -> Self {
        self.name = Some(name.into());
        self
    }

    /// The byte end of the current content — `offsets[len]` (widened to `i64`).
    fn end_offset(&self) -> i64 {
        T::Offset::read(&self.offsets, self.len as u64 * T::Offset::WIDTH)
    }

    /// Appends the **raw bytes** of a non-null element (the type-agnostic front door). This is the
    /// **fast, unchecked** path: it does **not** enforce [`max_width`](VarSerie::max_width) — a
    /// caller wanting the schema bound enforced uses [`try_push_bytes`](VarSerie::try_push_bytes).
    pub fn push_bytes(&mut self, bytes: &[u8]) {
        let start = self.end_offset();
        self.data.pwrite_byte_array(start as u64, bytes);
        let end = start + bytes.len() as i64;
        T::Offset::write(
            &mut self.offsets,
            (self.len as u64 + 1) * T::Offset::WIDTH,
            end,
        )
        .expect("offset write into a heap");
        if let Some(validity) = self.validity.as_mut() {
            validity
                .pwrite_bit(self.len as u64, true)
                .expect("bit write into a heap");
        }
        self.len += 1;
    }

    /// Appends a **non-null** value. Like [`push_bytes`](VarSerie::push_bytes) this is the fast,
    /// **unchecked** path and does **not** enforce [`max_width`](VarSerie::max_width) — use
    /// [`try_push`](VarSerie::try_push) for the checked append.
    pub fn push(&mut self, value: &T::Owned) {
        self.push_bytes(T::owned_bytes(value));
    }

    /// The **checked** append of raw bytes: enforces [`max_width`](VarSerie::max_width) when set —
    /// bytes longer than the column's max width are refused with a guided [`IoError`] (naming the
    /// element index, its length, the max, and the fix) and **nothing is appended** (the length is
    /// unchanged). Within the bound it delegates to the unchecked [`push_bytes`](VarSerie::push_bytes).
    /// The check is a length comparison — no copy.
    pub fn try_push_bytes(&mut self, bytes: &[u8]) -> Result<(), IoError> {
        if let Some(max) = self.max_width {
            if bytes.len() > max {
                return Err(max_width_error(self.len, bytes.len(), max));
            }
        }
        self.push_bytes(bytes);
        Ok(())
    }

    /// The **checked** append of a value — the [`try_push_bytes`](VarSerie::try_push_bytes) twin of
    /// [`push`](VarSerie::push), enforcing [`max_width`](VarSerie::max_width).
    pub fn try_push(&mut self, value: &T::Owned) -> Result<(), IoError> {
        self.try_push_bytes(T::owned_bytes(value))
    }

    /// Appends a **null** — a zero-length span, validity bit clear.
    pub fn push_null(&mut self) {
        self.ensure_validity();
        let start = self.end_offset();
        // empty span: offset[len+1] == offset[len]
        T::Offset::write(
            &mut self.offsets,
            (self.len as u64 + 1) * T::Offset::WIDTH,
            start,
        )
        .expect("offset write into a heap");
        self.validity
            .as_mut()
            .expect("validity ensured")
            .pwrite_bit(self.len as u64, false)
            .expect("bit write into a heap");
        self.len += 1;
    }

    /// Appends an option — [`push`](VarSerie::push) / [`push_null`](VarSerie::push_null).
    pub fn push_option(&mut self, value: Option<&T::Owned>) {
        match value {
            Some(value) => self.push(value),
            None => self.push_null(),
        }
    }

    /// Ensures a validity buffer exists, back-filling every existing element as valid.
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

    /// A fresh sub-column copying elements `[start, start + len)` into a new in-heap [`VarSerie`],
    /// **rebuilding** the offsets + data (element `i` of the copy re-packed from `self`'s bytes) and
    /// carrying the validity bits. The window is **clamped** to the column's length — an out-of-range
    /// `start` or over-long `len` yields a shorter (or empty) column, never an error. The offsets +
    /// data buffers are pre-reserved to the range's known extent.
    ///
    // DESIGN: an in-place `set` of a **variable-length** element is out of scope — replacing element
    // `i` with a value of a different length would rewrite the entire tail (every following offset and
    // the packed data after it), so variable-length columns are **append-only** for now: `slice` (a
    // read-range copy) is the only range operation, and growth goes through `push` / `push_bytes`. A
    // fixed-stride carrier (`FixedSizeSerie`) has no such tail and does get an in-place `set`.
    pub fn slice(&self, start: usize, len: usize) -> Self {
        let start = start.min(self.len);
        let count = len.min(self.len - start);
        let mut out = Self::new();
        if self.validity.is_some() {
            out.ensure_validity();
        }
        // Pre-size: the copy holds `count + 1` offsets and the range's byte span in data.
        out.offsets.reserve((count as u64 + 1) * T::Offset::WIDTH);
        if count > 0 {
            let data_start =
                T::Offset::read(&self.offsets, start as u64 * T::Offset::WIDTH).max(0) as u64;
            let data_end = T::Offset::read(&self.offsets, (start + count) as u64 * T::Offset::WIDTH)
                .max(0) as u64;
            out.data.reserve(data_end.saturating_sub(data_start));
        }
        for index in start..start + count {
            if self.valid(index) {
                out.push_bytes(&self.bytes_at(index).unwrap_or_default());
            } else {
                out.push_null();
            }
        }
        out
    }

    /// Appends `values` at the end — the batch counterpart of [`push`](VarSerie::push). Pre-reserves
    /// the offsets (`values.len()` more) and the data (the values' total byte length) in one pass, so
    /// the run never reallocates, then appends each element through the offsets+data path. All
    /// appended elements are non-null.
    pub fn append(&mut self, values: &[T::Owned]) {
        let total: usize = values.iter().map(|value| T::owned_bytes(value).len()).sum();
        self.data.reserve(total as u64);
        self.offsets.reserve(values.len() as u64 * T::Offset::WIDTH);
        for value in values {
            self.push(value);
        }
    }

    /// Appends **another column's** elements (values **and** validity) — a bulk column concatenation.
    /// The whole data block of `other` transfers in **one** [`pwrite_from`](crate::io::memory::IOBase::pwrite_from)
    /// (zero-copy when `other` is contiguous), its offsets are re-based onto this column's data end,
    /// and its per-element null-ness is reflected (a nullable `other` back-fills a validity buffer on
    /// `self` if it had none).
    pub fn extend<D2: IOBase>(&mut self, other: &VarSerie<T, D2>) {
        let count = other.len;
        if count == 0 {
            return;
        }
        let base = self.end_offset();
        // `other`'s used data byte length (its final offset) — bulk-copy that block in one pass.
        let other_len = T::Offset::read(&other.offsets, count as u64 * T::Offset::WIDTH).max(0);
        self.data
            .pwrite_from(base as u64, &other.data, 0, other_len as u64)
            .expect("copy other's data block");
        self.offsets.reserve(count as u64 * T::Offset::WIDTH);
        for k in 1..=count as u64 {
            let offset = T::Offset::read(&other.offsets, k * T::Offset::WIDTH);
            T::Offset::write(
                &mut self.offsets,
                (self.len as u64 + k) * T::Offset::WIDTH,
                base + offset,
            )
            .expect("offset write into a heap");
        }
        match other.validity.as_ref() {
            Some(src_bits) => {
                self.ensure_validity();
                let dst_bits = self.validity.as_mut().expect("validity ensured");
                for k in 0..count as u64 {
                    dst_bits
                        .pwrite_bit(self.len as u64 + k, src_bits.pread_bit(k).unwrap_or(false))
                        .expect("bit write into a heap");
                }
            }
            None => {
                if let Some(dst_bits) = self.validity.as_mut() {
                    for k in 0..count as u64 {
                        dst_bits
                            .pwrite_bit(self.len as u64 + k, true)
                            .expect("bit write into a heap");
                    }
                }
            }
        }
        self.len += count;
    }

    /// Appends `count` copies of `value` at the end — the **repeated-value fill** for a variable-length
    /// column: the element bytes are written `count` times into the data buffer and the `count` new
    /// offsets are filled by arithmetic (no materialized `count`-element array).
    pub fn push_repeat(&mut self, value: &T::Owned, count: usize) {
        if count == 0 {
            return;
        }
        let bytes = T::owned_bytes(value);
        let width = bytes.len();
        let base = self.end_offset();
        self.data.reserve((width * count) as u64);
        self.offsets.reserve(count as u64 * T::Offset::WIDTH);
        for k in 0..count {
            self.data
                .pwrite_byte_array(base as u64 + (k * width) as u64, bytes);
        }
        for k in 1..=count {
            T::Offset::write(
                &mut self.offsets,
                (self.len as u64 + k as u64) * T::Offset::WIDTH,
                base + (k * width) as i64,
            )
            .expect("offset write into a heap");
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

    /// A **non-nullable** column of `count` copies of `value` — the builder counterpart of
    /// [`push_repeat`](VarSerie::push_repeat).
    pub fn repeat(value: &T::Owned, count: usize) -> Self {
        let mut column = Self::with_capacity(count);
        column.push_repeat(value, count);
        column
    }

    /// **Reverses element order in place** — rebuilds the offsets + data in reverse (see
    /// [`reverse`](VarSerie::reverse)).
    pub fn reverse_in_place(&mut self) {
        *self = self.reverse();
    }

    /// **Sorts the column ascending (lexicographically) in place** — see [`sort`](VarSerie::sort).
    pub fn sort_in_place(&mut self) {
        *self = self.sort();
    }
}

impl<T: VarLenType> Default for VarSerie<T, Heap> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: VarLenType, D: IOBase> VarSerie<T, D> {
    /// Wraps existing offsets + data (+ optional validity) as a `len`-element column — the zero-copy
    /// "view any [`IOBase`] pair as a variable-length column" front door. The wrapped column is
    /// **unbounded** ([`max_width`](VarSerie::max_width) is `None`); declare a bound afterwards with
    /// [`set_max_width`](VarSerie::set_max_width) / [`with_max_width`](VarSerie::with_max_width), or
    /// wrap-and-bound in one step with [`from_parts_bounded`](VarSerie::from_parts_bounded).
    pub fn from_parts(offsets: D, data: D, validity: Option<D>, len: usize) -> Self {
        VarSerie {
            offsets,
            data,
            validity,
            len,
            name: None,
            max_width: None,
            _type: PhantomData,
        }
    }

    /// [`from_parts`](VarSerie::from_parts) that also records an optional **max element width** —
    /// the bound is validated against the wrapped elements (see
    /// [`set_max_width`](VarSerie::set_max_width)), so a value already over `max_width` returns a
    /// guided [`IoError`].
    pub fn from_parts_bounded(
        offsets: D,
        data: D,
        validity: Option<D>,
        len: usize,
        max_width: Option<usize>,
    ) -> Result<Self, IoError> {
        let mut serie = Self::from_parts(offsets, data, validity, len);
        serie.set_max_width(max_width)?;
        Ok(serie)
    }

    /// The column's optional **max element width** (bytes) — the schema bound enforced by the
    /// checked appends and reported as the field's [`byte_width`](HeaderField::byte_width). `None`
    /// means unbounded.
    pub fn max_width(&self) -> Option<usize> {
        self.max_width
    }

    /// Sets (or clears, with `None`) the **max element width**, **validating** every existing
    /// element against it first: a stored value longer than `max_width` yields a guided
    /// [`IoError`] naming its element index, its length, the max, and the fix — and the bound is
    /// **not** applied. Clearing to `None` always succeeds. The validation reads only the offsets
    /// (element width `= offsets[i + 1] - offsets[i]`) — no data copy.
    pub fn set_max_width(&mut self, max_width: Option<usize>) -> Result<(), IoError> {
        if let Some(max) = max_width {
            for index in 0..self.len {
                let start =
                    T::Offset::read(&self.offsets, index as u64 * T::Offset::WIDTH).max(0) as usize;
                let end = T::Offset::read(&self.offsets, (index as u64 + 1) * T::Offset::WIDTH)
                    .max(0) as usize;
                let width = end.saturating_sub(start);
                if width > max {
                    return Err(max_width_error(index, width, max));
                }
            }
        }
        self.max_width = max_width;
        Ok(())
    }

    /// [`set_max_width(Some(max_width))`](VarSerie::set_max_width), chainable on the `Ok` path — the
    /// clone-with-bound front door. Returns the guided [`IoError`] (and consumes `self`) when an
    /// existing element already exceeds `max_width`.
    pub fn with_max_width(mut self, max_width: usize) -> Result<Self, IoError> {
        self.set_max_width(Some(max_width))?;
        Ok(self)
    }

    /// Whether the element at `index` is valid (non-null).
    fn valid(&self, index: usize) -> bool {
        index < self.len
            && self
                .validity
                .as_ref()
                .is_none_or(|bits| bits.pread_bit(index as u64).unwrap_or(false))
    }

    /// The **raw bytes** of the element at `index`, ignoring validity — `None` when out of range.
    pub fn bytes_at(&self, index: usize) -> Option<Vec<u8>> {
        if index >= self.len {
            return None;
        }
        let start = T::Offset::read(&self.offsets, index as u64 * T::Offset::WIDTH).max(0) as u64;
        let end =
            T::Offset::read(&self.offsets, (index as u64 + 1) * T::Offset::WIDTH).max(0) as u64;
        Some(
            self.data
                .pread_vec(start, end.saturating_sub(start) as usize),
        )
    }

    /// The backing offsets buffer.
    pub fn offsets(&self) -> &D {
        &self.offsets
    }

    /// The backing data buffer.
    pub fn data(&self) -> &D {
        &self.data
    }

    /// The validity bit buffer, when the column is nullable.
    pub fn validity(&self) -> Option<&D> {
        self.validity.as_ref()
    }

    /// The column **name**, if set — the lightweight accessor (the same value
    /// [`field`](VarSerie::field) reports), read without building a [`HeaderField`].
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Every element as its owned value, ignoring validity (invalid-byte elements are skipped).
    pub fn values(&self) -> Vec<T::Owned> {
        (0..self.len)
            .filter_map(|index| self.bytes_at(index).and_then(|bytes| T::to_owned(&bytes)))
            .collect()
    }

    /// The column's [`Field`](crate::typed::Field) metadata — `name`, `type_id`, `nullable`, and
    /// (when a [`max_width`](VarSerie::max_width) is declared) the max recorded as the field's
    /// `byte_width`.
    ///
    // DESIGN: the field's `byte_width` metadata key is shared but its meaning is the carrier's — on
    // a variable-length `VarSerie` it records the OPTIONAL MAX element width (elements still vary,
    // none may exceed it), whereas on a `FixedSizeSerie` the same key records the EXACT (fixed)
    // element stride. `field().byte_width() == Some(max)` iff a max width is set; `None` otherwise.
    pub fn field(&self) -> HeaderField {
        match self.max_width {
            Some(max) => HeaderField::fixed_size(
                self.name.as_deref(),
                T::DATA_TYPE_ID,
                max as u32,
                self.validity.is_some(),
            ),
            None => HeaderField::new(
                self.name.as_deref(),
                T::DATA_TYPE_ID,
                self.validity.is_some(),
            ),
        }
    }

    /// **Gathers** the elements at `indices` (a permutation or any selection) into a fresh in-heap
    /// column, carrying validity by rebuilding the offsets + data. An index past the column length
    /// (or a selected null) becomes a null in the result. This is the shared dense back end of
    /// [`mask_filter`](VarSerie::mask_filter) / [`reverse`](VarSerie::reverse) / [`sort`](VarSerie::sort).
    pub fn take(&self, indices: &[usize]) -> VarSerie<T, Heap> {
        let mut out = VarSerie::<T, Heap>::with_capacity(indices.len());
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
    pub fn mask_filter<M: IOBase>(&self, mask: &M) -> VarSerie<T, Heap> {
        let indices: Vec<usize> = (0..self.len)
            .filter(|&index| mask.pread_bit(index as u64).unwrap_or(false))
            .collect();
        self.take(&indices)
    }

    /// A fresh **reversed** copy — the offsets + data rebuilt in reverse element order (the copy front
    /// door of [`reverse_in_place`](VarSerie::reverse_in_place)).
    pub fn reverse(&self) -> VarSerie<T, Heap> {
        let indices: Vec<usize> = (0..self.len).rev().collect();
        self.take(&indices)
    }

    /// The **permutation that sorts** the column **lexicographically** over the element bytes (so
    /// `Utf8` sorts by code point, `Binary` by byte order). **Stable**, with **nulls last** in both
    /// directions.
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
    /// door of [`sort_in_place`](VarSerie::sort_in_place). Nulls sort last.
    pub fn sort(&self) -> VarSerie<T, Heap> {
        self.take(&self.sort_indices(true))
    }
}

/// The null-aware **lexicographic** comparison of two variable-length slots for
/// [`VarSerie::sort_indices`] — **nulls sort last** (both directions); among the non-null values the
/// `ascending` flag picks the direction over the raw element bytes (`Utf8` code-point order, `Binary`
/// byte order). Total and stable-compatible.
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

impl<T: VarLenType, D: IOBase> Scalar for VarSerie<T, D> {
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

impl<T: VarLenType, D: IOBase> Serie for VarSerie<T, D> {}
