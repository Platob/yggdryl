//! [`VarSerie`] — a **variable-length typed column**: an `i32` **offsets** buffer + a **data**
//! buffer (element `i` is `data[offsets[i]..offsets[i + 1]]`), plus an optional validity bit buffer.
//!
//! This is the Arrow variable-length layout over the [`IOBase`] contract: the offsets and data are
//! each an `IOBase` source, so a `Binary` / `Utf8` column is in-heap, memory-mapped, or on device
//! memory with no change to its surface. It implements the same [`Scalar`] / [`Serie`] traits the
//! fixed families do — its `Value` is the type's owned form (`Vec<u8>` / `String`).

use core::marker::PhantomData;

use crate::datatype_id::DataTypeId;
use crate::io::memory::{Heap, IOBase, IoError};
use crate::typed::{HeaderField, Scalar, Serie, VarType};

/// A **variable-length column** over an `i32` offsets buffer + a data buffer (default [`Heap`]),
/// plus an optional validity buffer. Element `i` occupies `data[offsets[i]..offsets[i + 1]]`.
pub struct VarSerie<T: VarType, D: IOBase = Heap> {
    /// `len + 1` little-endian `i32` offsets; `offsets[0] == 0`.
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

impl<T: VarType> VarSerie<T, Heap> {
    /// An empty non-nullable column.
    pub fn new() -> Self {
        let mut offsets = Heap::new();
        offsets
            .pwrite_i32(0, 0)
            .expect("offset[0] into a fresh heap");
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

    /// A non-nullable column holding `values`.
    pub fn from_values(values: &[T::Owned]) -> Self {
        let mut column = Self::new();
        for value in values {
            column.push(value);
        }
        column
    }

    /// A column from options — pushing a null (an empty span) where a value is absent.
    pub fn from_options(values: &[Option<T::Owned>]) -> Self {
        let mut column = Self::new();
        column.ensure_validity();
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

    /// The byte end of the current content — `offsets[len]`.
    fn end_offset(&self) -> i32 {
        self.offsets.pread_i32(self.len as u64 * 4).unwrap_or(0)
    }

    /// Appends the **raw bytes** of a non-null element (the type-agnostic front door). This is the
    /// **fast, unchecked** path: it does **not** enforce [`max_width`](VarSerie::max_width) — a
    /// caller wanting the schema bound enforced uses [`try_push_bytes`](VarSerie::try_push_bytes).
    pub fn push_bytes(&mut self, bytes: &[u8]) {
        let start = self.end_offset();
        self.data.pwrite_byte_array(start as u64, bytes);
        let end = start + bytes.len() as i32;
        self.offsets
            .pwrite_i32((self.len as u64 + 1) * 4, end)
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
        self.offsets
            .pwrite_i32((self.len as u64 + 1) * 4, start) // empty span: offset[len+1] == offset[len]
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
        out.offsets.reserve(((count + 1) * 4) as u64);
        if count > 0 {
            let data_start = self.offsets.pread_i32(start as u64 * 4).unwrap_or(0).max(0) as u64;
            let data_end = self
                .offsets
                .pread_i32((start + count) as u64 * 4)
                .unwrap_or(0)
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
}

impl<T: VarType> Default for VarSerie<T, Heap> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: VarType, D: IOBase> VarSerie<T, D> {
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
                let start = self.offsets.pread_i32(index as u64 * 4).unwrap_or(0).max(0) as usize;
                let end = self
                    .offsets
                    .pread_i32((index as u64 + 1) * 4)
                    .unwrap_or(0)
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
        let start = self.offsets.pread_i32(index as u64 * 4).ok()?.max(0) as u64;
        let end = self.offsets.pread_i32((index as u64 + 1) * 4).ok()?.max(0) as u64;
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
}

impl<T: VarType, D: IOBase> Scalar for VarSerie<T, D> {
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

impl<T: VarType, D: IOBase> Serie for VarSerie<T, D> {}
