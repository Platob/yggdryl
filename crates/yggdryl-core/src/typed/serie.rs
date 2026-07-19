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

use super::field::{cast_dtype_error, cast_null_error};
use super::{
    AnyScalar, Column, DataType, Decimal, Decoder, Encoder, Field, FlexibleFromStr, FlexibleToStr,
    FromValue, HeaderField, Reduce, Scalar, ToValue, Value,
};
use crate::datatype_id::DataTypeId;
use crate::headers::Headers;
use crate::io::memory::{Heap, IOBase, IoError};
use crate::typed::nested::set_any_dtype_error;

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

    /// The **child columns** of this series — the downward graph edge for nested navigation. A leaf
    /// series (numeric, byte, string, bool) has none, so the default returns an empty `Vec` (no
    /// allocation); a nested [`StructSerie`](crate::typed::StructSerie) overrides it to return its
    /// columns. There is no `parent()` up-pointer — nested owns its children downward, so a uniform
    /// `children()` is the whole graph surface.
    fn children(&self) -> Vec<&crate::typed::nested::Column> {
        Vec::new()
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

    /// The element at `index` **erased** into a [`Value`] — the generic bridge from this
    /// concrete typed column to the [`Any`](crate::typed::AnySerie) surface. It redirects to this
    /// series' **own** [`get`](Scalar::get) (its optimized decode) and wraps the native through
    /// [`ToValue`], so every carrier inherits it with no slow fallback. A null or out-of-range slot
    /// is [`Value::Null`].
    ///
    /// ```
    /// use yggdryl_core::typed::{FixedSerie, Serie, Value};
    /// use yggdryl_core::typed::fixedbyte::Int64;
    ///
    /// let col = FixedSerie::<Int64>::from_values(&[10, 20, 30]);
    /// assert_eq!(col.get_any_value_at(1), Value::Int64(20));
    /// assert_eq!(col.get_any_value_at(9), Value::Null); // out of range
    /// ```
    fn get_any_value_at(&self, index: usize) -> Value
    where
        Self::Value: ToValue,
    {
        self.get(index)
            .map(ToValue::to_value)
            .unwrap_or(Value::Null)
    }

    /// The element at `index` as an [`AnyScalar`] (the erased [`Value`]) — the `Any`-typed reading of
    /// [`get_any_value_at`](Serie::get_any_value_at).
    fn get_any_scalar_at(&self, index: usize) -> AnyScalar
    where
        Self::Value: ToValue,
    {
        self.get_any_value_at(index)
    }

    /// Sets the element at `index` from an **erased** [`Value`] — extracts this series' native via
    /// [`FromValue`] (a guided error, naming **both** the column's and the value's dtype, when they
    /// do not match), then applies the column's own `set`.
    ///
    /// The **default is a guided refusal** for an **append-only / read-only** carrier — a
    /// variable-length [`VarSerie`](crate::typed::VarSerie) (replacing an element would rewrite the
    /// whole tail) or a bufferless [`NullSerie`](crate::typed::NullSerie). The **settable** carriers
    /// ([`FixedSerie`], [`FixedSizeSerie`](crate::typed::FixedSizeSerie)) override it with their
    /// in-place `set`.
    fn set_any_scalar_at(&mut self, index: usize, value: &Value) -> Result<(), IoError>
    where
        Self::Value: FromValue,
    {
        let _ = (index, value);
        Err(IoError::TypedCast {
            detail: format!(
                "this {} column is append-only: rebuild it from values (its bytes cannot be \
                 replaced in place)",
                self.data_type_id()
            ),
        })
    }

    /// Erases this column into the [`Any`](crate::typed::AnySerie) carrier — the erased [`Column`]
    /// that wraps every concrete type. A thin `self.into()`, available for any series with a
    /// [`From`] into [`Column`].
    fn into_any(self) -> Column
    where
        Self: Sized + Into<Column>,
    {
        self.into()
    }
}

/// A **typed column** over an [`IOBase`] data buffer `D` (default [`Heap`]) plus an optional
/// validity buffer. Elements are packed at the type's stride; reads/writes/reductions all forward
/// to the byte layer's vectorized kernels.
#[derive(Clone)]
pub struct FixedSerie<T: DataType, D: IOBase = Heap> {
    data: D,
    validity: Option<D>,
    len: usize,
    name: Option<Box<str>>,
    /// Decimal precision / scale metadata — set only for decimal columns (see
    /// [`with_precision_scale`](FixedSerie::with_precision_scale)).
    precision: Option<u32>,
    scale: Option<i32>,
    /// Free-form annotations beyond the promoted name / type / nullable / precision / scale —
    /// carried onto the [`field`](FixedSerie::field) and set by a
    /// [`cast_field`](FixedSerie::cast_field). Empty for a plain column (an empty [`Headers`]
    /// allocates nothing).
    metadata: Headers,
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
            metadata: Headers::new(),
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
            metadata: Headers::new(),
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

    /// A column from options. A collection with **no `None`** is **non-nullable** — no validity
    /// buffer is built ([`field().nullable()`](FixedSerie::field) is `false`), equivalent to
    /// [`from_values`](FixedSerie::from_values). When at least one null is present it keeps the
    /// validity buffer, encoding a default in each null slot. Bulk: one vectorized data write (nulls
    /// → default) and, only when needed, one packed validity write — never an element-by-element push.
    pub fn from_options(values: &[Option<T::Native>]) -> Self {
        // Scan once: a null-free collection is non-nullable (skip the validity buffer entirely).
        let has_null = values.iter().any(Option::is_none);
        let mut column = Self::with_capacity(values.len());
        // The data buffer: every value (a null slot gets the type default), one vectorized write.
        let natives: Vec<T::Native> = values.iter().map(|v| v.unwrap_or_default()).collect();
        T::encode_slice(&mut column.data, 0, &natives)
            .expect("encode into a fresh heap never fails");
        column.len = values.len();
        if has_null {
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
        }
        column
    }

    /// A **non-nullable** in-heap column built by **flexibly parsing** `values` — each string via the
    /// tolerant [`parse_flexible`](super::FlexibleFromStr::parse_flexible) (thousands separators,
    /// `0x`/`0b`/`0o` radices, `1e3` scientific, `+`/whitespace) in **one vectorized**
    /// [`encode_str_slice`](Encoder::encode_str_slice) into a fresh [`Heap`]. A value the type cannot
    /// represent surfaces the guided [`IoError::ParseError`].
    pub fn from_strings(values: &[&str]) -> Result<FixedSerie<T, Heap>, IoError>
    where
        T::Native: FlexibleFromStr,
    {
        let mut column = Self::with_capacity(values.len());
        T::encode_str_slice(&mut column.data, 0, values)?;
        column.len = values.len();
        Ok(column)
    }

    /// The **strict** twin of [`from_strings`](FixedSerie::from_strings): parses each string with
    /// [`parse_exact`](super::FlexibleFromStr::parse_exact) (`str::parse`, no coercion) in one
    /// vectorized [`encode_str_exact_slice`](Encoder::encode_str_exact_slice).
    pub fn from_strings_exact(values: &[&str]) -> Result<FixedSerie<T, Heap>, IoError>
    where
        T::Native: FlexibleFromStr,
    {
        let mut column = Self::with_capacity(values.len());
        T::encode_str_exact_slice(&mut column.data, 0, values)?;
        column.len = values.len();
        Ok(column)
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
            metadata: Headers::new(),
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

    /// Every element decoded and **formatted** to its string in one vectorized
    /// [`decode_str_slice`](Decoder::decode_str_slice) — validity **ignored** (matching
    /// [`values`](FixedSerie::values); a null slot surfaces the rendering of its stored default).
    /// Pair with [`to_string_options`](FixedSerie::to_string_options) for the null-aware form.
    pub fn to_strings(&self) -> Result<Vec<String>, IoError>
    where
        T::Native: FlexibleToStr,
    {
        T::decode_str_slice(&self.data, 0, self.len)
    }

    /// Every element as an `Option<String>`, **null-aware**: `None` for a null slot, else the
    /// formatted value. One vectorized decode+format pass, then the invalid indices are nulled out.
    pub fn to_string_options(&self) -> Result<Vec<Option<String>>, IoError>
    where
        T::Native: FlexibleToStr,
    {
        let formatted = T::decode_str_slice(&self.data, 0, self.len)?;
        Ok(formatted
            .into_iter()
            .enumerate()
            .map(|(index, value)| if self.valid(index) { Some(value) } else { None })
            .collect())
    }

    /// The backing data buffer (borrowed).
    pub fn data(&self) -> &D {
        &self.data
    }

    /// The validity bit buffer, when the column is nullable.
    pub fn validity(&self) -> Option<&D> {
        self.validity.as_ref()
    }

    /// The column **name**, if set — the lightweight accessor (the same value
    /// [`field`](FixedSerie::field) reports), read without building a [`HeaderField`].
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// The column's [`Field`] metadata — its `name`, `type_id`, `nullable` flag, (for a decimal
    /// column) its `precision` / `scale`, and any free-form annotations carried by a
    /// [`cast_field`](FixedSerie::cast_field).
    pub fn field(&self) -> HeaderField {
        let nullable = self.validity.is_some();
        let mut field = match (self.precision, self.scale) {
            (Some(precision), Some(scale)) => HeaderField::decimal(
                self.name.as_deref(),
                T::DATA_TYPE_ID,
                precision,
                scale,
                nullable,
            ),
            _ => HeaderField::new(self.name.as_deref(), T::DATA_TYPE_ID, nullable),
        };
        // Overlay the free-form annotations (the promoted name / type / nullable stay structural).
        for (name, value) in self.metadata.iter() {
            field.metadata_mut().append_bytes(name, value);
        }
        field
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

/// Field-driven **cast** — reshape a column's *metadata* (nullability, name, annotations) to a
/// target [`HeaderField`] while **keeping the element type**. The copy front door
/// ([`cast_field`](FixedSerie::cast_field)) is a thin `clone → `[`cast_field_in_place`](FixedSerie::cast_field_in_place),
/// so the in-place form is the single implementation.
impl<T: Encoder + Decoder, D: IOBase> FixedSerie<T, D> {
    /// A fresh column reshaped to `field` — the non-mutating front door of
    /// [`cast_field_in_place`](FixedSerie::cast_field_in_place) (`clone → cast_field_in_place`).
    pub fn cast_field(&self, field: &HeaderField) -> Result<Self, IoError>
    where
        D: Default + Clone,
    {
        let mut out = self.clone();
        out.cast_field_in_place(field)?;
        Ok(out)
    }

    /// Reshapes this column **in place** to match `field`'s metadata, keeping the element type:
    ///
    /// - **No-op** when `field` already matches — same dtype, nullability, name, and annotations —
    ///   returning `Ok(())` without touching the backing.
    /// - **Same dtype** ([`data_type_id`](super::Field::data_type_id) equals `T`'s): applies the
    ///   target **nullability** (non-nullable → nullable adds an all-valid validity buffer;
    ///   nullable → non-nullable requires [`null_count`](Scalar::null_count) `== 0`, else the guided
    ///   [`IoError::TypedCast`]), the target **name**, and the target's free-form **annotations** —
    ///   reusing the data backing (no element copy, only validity / metadata change).
    /// - **Different dtype**: the guided [`IoError::TypedCast`] — the typed column keeps its
    ///   element type; a runtime dtype change belongs to the erased layer.
    // DESIGN: `FixedSerie<T>` is compile-time-typed, so `cast_field` deliberately cannot change the
    // element type — that is the erased `Serie.cast_field` (bindings) / `IOBase::resize_dtype`'s job.
    pub fn cast_field_in_place(&mut self, field: &HeaderField) -> Result<(), IoError>
    where
        D: Default,
    {
        let target = field.data_type_id();
        if target != T::DATA_TYPE_ID {
            return Err(cast_dtype_error("FixedSerie", T::DATA_TYPE_ID, target));
        }

        let to_nullable = field.nullable();
        let is_nullable = self.validity.is_some();
        let extra = field.extra_annotations();

        // Same dtype, nullability, name, and annotations — nothing to do (skip all work).
        if is_nullable == to_nullable
            && field.headers().name() == self.name.as_deref()
            && extra == self.metadata
        {
            return Ok(());
        }

        // Validate the one fallible step first, so a rejected cast leaves `self` untouched.
        if is_nullable && !to_nullable {
            let nulls = self.null_count();
            if nulls > 0 {
                return Err(cast_null_error(nulls));
            }
        }

        // Apply the (now infallible) changes: nullability, then name, then annotations.
        if !is_nullable && to_nullable {
            self.ensure_validity();
        } else if is_nullable && !to_nullable {
            self.validity = None; // verified null-free above
        }
        self.name = field.headers().name().map(Into::into);
        self.metadata = extra;
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

impl<T: Encoder + Decoder, D: IOBase> Serie for FixedSerie<T, D> {
    /// Sets the element at `index` from an erased [`Value`] — extracts the fixed-width native via
    /// [`FromValue`] (guided error on a dtype mismatch, naming both sides), then the vectorized
    /// in-place [`set`](FixedSerie::set). Overrides the append-only default with the fast path.
    fn set_any_scalar_at(&mut self, index: usize, value: &Value) -> Result<(), IoError>
    where
        Self::Value: FromValue,
    {
        let native = <T::Native as FromValue>::from_value(value)
            .ok_or_else(|| set_any_dtype_error(T::DATA_TYPE_ID, value))?;
        self.set(index, native)
    }
}

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
