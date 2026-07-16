//! [`ByteSerie`] — a nullable, variable-length column (`Utf8Serie = ByteSerie<Utf8>`,
//! `BinarySerie = ByteSerie<Binary>`): Arrow-style **offsets + data** with an optional
//! validity bitmap.

use core::marker::PhantomData;

use super::dtype::OFFSET_WIDTH;
use super::{ByteField, ByteScalar, ByteType, VarElement};
use crate::io::any_serie::filter_len_mismatch;
use crate::io::bitmap::{extend_validity, Bitmap};
use crate::io::field_carrier::field_accessors;
use crate::io::{AnyField, Bytes, FieldType, IOCursor, IoError, SerieType};

/// The **variable-length column** sub-trait — the sibling of
/// [`FixedSerie`](crate::io::fixed::FixedSerie) for a column of byte slices (strings, binary),
/// addressed through an offsets buffer.
pub trait VarSerie: SerieType {
    /// The raw bytes of element `index`, or `None` if null or out of range.
    fn value_bytes(&self, index: usize) -> Option<&[u8]>;
}

/// A **nullable, variable-length column** of kind `E` — an `i32` offsets buffer over a
/// contiguous data buffer, plus an optional validity bitmap (absent when there are no nulls).
/// Value `i` is `data[offsets[i] .. offsets[i + 1]]`. `Utf8Serie = ByteSerie<Utf8>`.
///
/// ```
/// use yggdryl_core::io::var::Utf8Serie;
/// use yggdryl_core::io::{Bytes, IOCursor};
///
/// let mut col = Utf8Serie::new();
/// col.push_str(Some("a"));
/// col.push_str(None);
/// col.push_str(Some("cd"));
/// assert_eq!(col.len(), 3);
/// assert_eq!(col.null_count(), 1);
/// assert_eq!(col.get_str(0), Some("a"));
/// assert_eq!(col.get_str(1), None);
/// assert_eq!(col.get_str(2), Some("cd"));
///
/// let mut sink = Bytes::new();
/// col.write_to(&mut sink).unwrap();
/// sink.rewind();
/// assert_eq!(Utf8Serie::read_from(&mut sink).unwrap(), col);
/// ```
pub struct ByteSerie<E: VarElement> {
    /// `len + 1` offsets into `data` (`offsets[0] == 0`).
    offsets: Vec<i32>,
    /// The concatenated value bytes.
    data: Vec<u8>,
    /// `None` means "no nulls".
    validity: Option<Bitmap>,
    len: usize,
    /// The column's own [`ByteField`] descriptor — its name, declared nullability, and metadata.
    /// Excluded from value identity and the byte codec. The single source of truth for schema intent.
    field: ByteField<E>,
    _element: PhantomData<E>,
}

// A manual `Clone` (not a derive) so it does not spuriously require `E: Clone` — a var column holds
// only `PhantomData<E>`, so cloning never touches the marker (matches how the erased `AnySerie`
// clones a boxed column without an element-`Clone` bound).
impl<E: VarElement> Clone for ByteSerie<E> {
    fn clone(&self) -> Self {
        Self {
            offsets: self.offsets.clone(),
            data: self.data.clone(),
            validity: self.validity.clone(),
            len: self.len,
            field: self.field.clone(),
            _element: PhantomData,
        }
    }
}

impl<E: VarElement> ByteSerie<E> {
    /// An empty column.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty column that can hold `capacity` elements before its offsets reallocate.
    pub fn with_capacity(capacity: usize) -> Self {
        let mut offsets = Vec::with_capacity(capacity + 1);
        offsets.push(0);
        Self {
            offsets,
            data: Vec::new(),
            validity: None,
            len: 0,
            field: ByteField::<E>::new("", false),
            _element: PhantomData,
        }
    }

    /// Appends one value from raw bytes (`None` is a null), validating it for the kind. A null
    /// lazily materializes the validity mask.
    pub fn push_bytes(&mut self, value: Option<&[u8]>) -> Result<(), IoError> {
        match value {
            Some(bytes) => {
                E::validate(bytes)?;
                self.data.extend_from_slice(bytes);
                if let Some(validity) = &mut self.validity {
                    validity.push(true);
                }
            }
            None => {
                self.validity
                    .get_or_insert_with(|| Bitmap::all_present(self.len))
                    .push(false);
            }
        }
        self.offsets.push(self.data.len() as i32);
        self.len += 1;
        Ok(())
    }

    /// A column from **present** raw byte values (no nulls), each validated for the kind — the
    /// present-only twin of [`from_options`](ByteSerie::from_options) (mirrors
    /// [`extend_values`](ByteSerie::extend_values)). `from_values` = present, `from_options` = nullable,
    /// uniform across every family.
    pub fn from_values(values: &[&[u8]]) -> Result<Self, IoError> {
        let mut serie = Self::with_capacity(values.len());
        serie.extend_values(values)?;
        Ok(serie)
    }

    /// A column from **optional** raw byte values (a `None` is a null), each validated for the kind —
    /// the nullable twin of [`from_values`](ByteSerie::from_values).
    pub fn from_options(values: &[Option<&[u8]>]) -> Result<Self, IoError> {
        let mut serie = Self::with_capacity(values.len());
        for &value in values {
            serie.push_bytes(value)?;
        }
        Ok(serie)
    }

    /// A column from a slice of [`ByteScalar`]s — each scalar contributing its bytes (or a null).
    /// The bulk analogue of the in-place [`set_scalars`](ByteSerie::set_scalars), and the inverse of
    /// collecting a column's [`get_scalar`](ByteSerie::get_scalar)s.
    ///
    /// ```
    /// use yggdryl_core::io::var::{Utf8Scalar, Utf8Serie};
    ///
    /// let col =
    ///     Utf8Serie::from_scalars(&[Utf8Scalar::of("a"), Utf8Scalar::null(), Utf8Scalar::of("cd")])
    ///         .unwrap();
    /// assert_eq!(col.get_str(0), Some("a"));
    /// assert_eq!(col.get_str(1), None);
    /// assert_eq!(col.get_str(2), Some("cd"));
    /// ```
    pub fn from_scalars(scalars: &[ByteScalar<E>]) -> Result<Self, IoError> {
        Self::from_options(
            &scalars
                .iter()
                .map(ByteScalar::value_bytes)
                .collect::<Vec<_>>(),
        )
    }

    /// The raw bytes of element `index` — zero-copy — or `None` if null or out of range.
    pub fn get_bytes(&self, index: usize) -> Option<&[u8]> {
        if index >= self.len {
            return None;
        }
        if let Some(validity) = &self.validity {
            if !validity.get(index) {
                return None;
            }
        }
        let start = self.offsets[index] as usize;
        let end = self.offsets[index + 1] as usize;
        Some(&self.data[start..end])
    }

    /// Iterates each element's raw bytes as `Option<&[u8]>`, in order — **allocation-free** and
    /// zero-copy per element (a null yields `None`). The var-family analogue of
    /// [`Serie::iter`](crate::io::fixed::Serie::iter).
    ///
    /// ```
    /// use yggdryl_core::io::var::Utf8Serie;
    ///
    /// let col = Utf8Serie::from_strs(&[Some("a"), None, Some("c")]);
    /// let seen: Vec<Option<&[u8]>> = col.iter_bytes().collect();
    /// assert_eq!(seen, [Some(&b"a"[..]), None, Some(&b"c"[..])]);
    /// ```
    pub fn iter_bytes(&self) -> impl Iterator<Item = Option<&[u8]>> + '_ {
        (0..self.len).map(move |index| self.get_bytes(index))
    }

    /// Iterates only the **present** (non-null) elements' raw bytes, in order — allocation-free,
    /// zero-copy per element. The var-family analogue of
    /// [`Serie::iter_valid`](crate::io::fixed::Serie::iter_valid).
    pub fn iter_valid_bytes(&self) -> impl Iterator<Item = &[u8]> + '_ {
        self.iter_bytes().flatten()
    }

    /// The number of elements.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the column is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The number of null elements.
    pub fn null_count(&self) -> usize {
        self.validity.as_ref().map_or(0, Bitmap::null_count)
    }

    /// Whether the column carries any nulls.
    pub fn has_nulls(&self) -> bool {
        self.null_count() > 0
    }

    /// The total number of value bytes (excluding offsets / validity).
    pub fn data_len(&self) -> usize {
        self.data.len()
    }

    /// The typed data type of the column.
    pub fn data_type(&self) -> ByteType<E> {
        ByteType::new()
    }

    field_accessors!();

    /// The erased [`AnyField`] this column contributes — its **held field** (name + metadata) with
    /// **effective** nullability `self.nullable() || self.has_nulls()` (declared OR the column holds
    /// nulls) — a lenient, Arrow-standard over-approximation.
    pub fn field(&self) -> AnyField {
        let mut field = self.field.clone();
        field.set_nullable(self.nullable() || self.has_nulls());
        AnyField::leaf(field.erase())
    }

    /// Like [`field`](ByteSerie::field) but **consumes** the column.
    pub fn into_field(mut self) -> AnyField {
        let nullable = self.nullable() || self.has_nulls();
        self.field.set_nullable(nullable);
        AnyField::leaf(self.field.erase())
    }

    /// A [`ByteField`] naming a column of this serie's type with **explicit** nullability.
    pub fn typed_field(&self, name: &str, nullable: bool) -> ByteField<E> {
        ByteField::new(name, nullable)
    }

    /// A [`ByteField`] naming this column, its nullability inferred from whether it has nulls.
    pub fn to_field(&self, name: &str) -> ByteField<E> {
        ByteField::new(name, self.has_nulls())
    }

    /// Element `index` as a [`ByteScalar`] — null if the element is null or out of range.
    pub fn get_scalar(&self, index: usize) -> ByteScalar<E> {
        match self.get_bytes(index) {
            // The bytes are already validated (they entered through a checked path).
            Some(bytes) => ByteScalar::from_bytes(bytes).unwrap_or_else(|_| ByteScalar::null()),
            None => ByteScalar::null(),
        }
    }

    // ---- in-place set: single element + bulk (from a Serie / scalars / byte values) ----------

    /// Overwrites element `index` in place. Unlike the fixed-width columns, a variable-length
    /// value can change **length**, so this is deliberately **expensive**: when the new length
    /// differs it splices the data buffer and shifts every trailing offset (an O(n) rewrite). A
    /// `None` (or empty value) shrinks the slot to zero bytes. Errors
    /// [`IndexOutOfBounds`](IoError::IndexOutOfBounds) past the end, or [`InvalidUtf8`](IoError::InvalidUtf8)
    /// for a bad `utf8` value (the column is left unchanged on error).
    ///
    /// ```
    /// use yggdryl_core::io::var::Utf8Serie;
    ///
    /// let mut col = Utf8Serie::from_strs(&[Some("a"), Some("bb"), Some("ccc")]);
    /// col.set_str(1, Some("longer"));            // grows -> trailing offsets shift
    /// col.set_str(2, None);                       // -> null, slot shrinks
    /// assert_eq!(col.get_str(0), Some("a"));
    /// assert_eq!(col.get_str(1), Some("longer"));
    /// assert_eq!(col.get_str(2), None);
    /// ```
    pub fn set_bytes(&mut self, index: usize, value: Option<&[u8]>) -> Result<(), IoError> {
        if index >= self.len {
            return Err(IoError::IndexOutOfBounds {
                index,
                len: self.len,
            });
        }
        // Validate before mutating anything, so a bad value leaves the column unchanged.
        let (new_bytes, present): (&[u8], bool) = match value {
            Some(bytes) => {
                E::validate(bytes)?;
                (bytes, true)
            }
            None => (&[], false), // a null shrinks the slot to empty
        };
        let start = self.offsets[index] as usize;
        let end = self.offsets[index + 1] as usize;
        let old_len = end - start;

        if new_bytes.len() == old_len {
            // Same length — overwrite the bytes in place, no offset rewrite.
            self.data[start..end].copy_from_slice(new_bytes);
        } else {
            // Different length — splice the data and shift every trailing offset by the delta.
            let delta = new_bytes.len() as i64 - old_len as i64;
            self.data.splice(start..end, new_bytes.iter().copied());
            for offset in &mut self.offsets[index + 1..] {
                *offset = (*offset as i64 + delta) as i32;
            }
        }

        if present {
            if let Some(validity) = &mut self.validity {
                validity.set(index, true);
            }
        } else {
            self.validity
                .get_or_insert_with(|| Bitmap::all_present(self.len))
                .set(index, false);
        }
        Ok(())
    }

    /// Overwrites element `index` from a [`ByteScalar`].
    pub fn set_scalar(&mut self, index: usize, scalar: &ByteScalar<E>) -> Result<(), IoError> {
        self.set_bytes(index, scalar.value_bytes())
    }

    /// Bounds-checks a bulk range `[start, start + count)` against the column length.
    fn check_range(&self, start: usize, count: usize) -> Result<(), IoError> {
        match start.checked_add(count) {
            Some(end) if end <= self.len => Ok(()),
            _ => Err(IoError::IndexOutOfBounds {
                index: start.max(self.len),
                len: self.len,
            }),
        }
    }

    /// Bulk-overwrites `[start, start + source.len())` from another column (nulls included). Each
    /// element may change length, so this rewrites offsets per element — O(n·m); build a fresh
    /// column when replacing most of it.
    pub fn set_range(&mut self, start: usize, source: &ByteSerie<E>) -> Result<(), IoError> {
        self.check_range(start, source.len())?;
        for index in 0..source.len() {
            self.set_bytes(start + index, source.get_bytes(index))?;
        }
        Ok(())
    }

    /// Bulk-overwrites `[start, start + scalars.len())` from a slice of [`ByteScalar`]s.
    pub fn set_scalars(&mut self, start: usize, scalars: &[ByteScalar<E>]) -> Result<(), IoError> {
        self.check_range(start, scalars.len())?;
        for (offset, scalar) in scalars.iter().enumerate() {
            self.set_bytes(start + offset, scalar.value_bytes())?;
        }
        Ok(())
    }

    /// Bulk-overwrites `[start, start + values.len())` from present byte values.
    pub fn set_byte_values(&mut self, start: usize, values: &[&[u8]]) -> Result<(), IoError> {
        self.check_range(start, values.len())?;
        for (offset, &value) in values.iter().enumerate() {
            self.set_bytes(start + offset, Some(value))?;
        }
        Ok(())
    }

    // ---- grow: append single + bulk (the mutator vocabulary) ----------------------------

    /// Appends a slice of **optional** byte values (each validated for the kind) — the bulk grow twin
    /// of [`from_options`](ByteSerie::from_options). Unlike a fixed-width column, a var
    /// column's data / offsets are owned `Vec`s, so the grow is an amortized-`O(1)` append (one
    /// `reserve` + `extend`, not a per-element buffer re-seal). Every value is validated **up front**,
    /// so a bad value leaves the column unchanged; a null lazily materializes the validity mask.
    ///
    /// ```
    /// use yggdryl_core::io::var::Utf8Serie;
    ///
    /// let mut col = Utf8Serie::from_strs(&[Some("a")]);
    /// col.extend_options(&[Some(b"bc".as_slice()), None]).unwrap();
    /// assert_eq!(col.get_str(1), Some("bc"));
    /// assert_eq!(col.get_str(2), None);
    /// ```
    pub fn extend_options(&mut self, values: &[Option<&[u8]>]) -> Result<(), IoError> {
        // Validate up front so a bad value leaves the column unchanged.
        for value in values.iter().flatten() {
            E::validate(value)?;
        }
        let base = self.len;
        let extra: usize = values.iter().flatten().map(|bytes| bytes.len()).sum();
        self.data.reserve(extra);
        self.offsets.reserve(values.len());
        for (offset, value) in values.iter().enumerate() {
            match value {
                Some(bytes) => {
                    self.data.extend_from_slice(bytes);
                    if let Some(validity) = &mut self.validity {
                        validity.push(true);
                    }
                }
                None => {
                    self.validity
                        .get_or_insert_with(|| Bitmap::all_present(base + offset))
                        .push(false);
                }
            }
            self.offsets.push(self.data.len() as i32);
        }
        self.len += values.len();
        Ok(())
    }

    /// Appends a slice of **present** byte values (no nulls), each validated for the kind — the bulk
    /// grow twin of [`set_byte_values`](ByteSerie::set_byte_values).
    pub fn extend_values(&mut self, values: &[&[u8]]) -> Result<(), IoError> {
        for value in values {
            E::validate(value)?;
        }
        let extra: usize = values.iter().map(|bytes| bytes.len()).sum();
        self.data.reserve(extra);
        self.offsets.reserve(values.len());
        for bytes in values {
            self.data.extend_from_slice(bytes);
            if let Some(validity) = &mut self.validity {
                validity.push(true);
            }
            self.offsets.push(self.data.len() as i32);
        }
        self.len += values.len();
        Ok(())
    }

    /// Appends a slice of [`ByteScalar`]s (each its bytes or a null) — the bulk grow twin of
    /// [`from_scalars`](ByteSerie::from_scalars).
    pub fn extend_scalars(&mut self, scalars: &[ByteScalar<E>]) -> Result<(), IoError> {
        self.extend_options(
            &scalars
                .iter()
                .map(ByteScalar::value_bytes)
                .collect::<Vec<_>>(),
        )
    }

    /// Appends **another whole column** of the same kind to this one — the two columns concatenate.
    /// The source's whole data blob is memcpy'd in one `extend`, its offsets are rebased onto this
    /// column's data length, and its null positions carry over — all amortized `O(1)`. Errors
    /// [`Unsupported`](IoError::Unsupported) if the combined data would overflow the `i32` offset
    /// space (the column is left partially grown only up to the memcpy — callers should treat this as
    /// a hard corruption guard; it needs a `LargeUtf8`/`LargeBinary` 64-bit-offset kind).
    pub fn concat(&mut self, source: &ByteSerie<E>) -> Result<(), IoError> {
        if source.len == 0 {
            return Ok(());
        }
        let base = self.data.len();
        let combined = base as i64 + source.data.len() as i64;
        if combined > i32::MAX as i64 {
            return Err(IoError::Unsupported {
                what: format!(
                    "concatenating these var columns would overflow the i32 offset space \
                     ({combined} bytes exceeds i32::MAX); split the column (a 64-bit-offset \
                     LargeUtf8/LargeBinary kind is reserved but not yet shipped)"
                ),
            });
        }
        let prev_len = self.len;
        self.data.extend_from_slice(&source.data); // memcpy the whole value blob
        self.offsets.reserve(source.len);
        for &offset in &source.offsets[1..] {
            self.offsets.push(base as i32 + offset);
        }
        extend_validity(&mut self.validity, prev_len, source.len, |offset| {
            source.validity.as_ref().is_none_or(|mask| mask.get(offset))
        });
        self.len += source.len;
        Ok(())
    }

    // ---- reshape: filter (keep selected rows) + fill_null (replace nulls) -----------------

    /// A **new** column keeping only the elements where `mask[i]` is `true` — the row filter. Errors
    /// ([`Unsupported`](IoError::Unsupported)) if `mask.len() != self.len()`.
    ///
    /// OPTIMIZED: a first cheap pass popcounts the kept rows *and* sums their data bytes, so the
    /// rebuilt offsets and data buffers are **pre-sized**; a second pass copies each selected value's
    /// bytes (zero-copy per element) and rebuilds the offsets, keeping the selected rows' null-ness.
    ///
    /// ```
    /// use yggdryl_core::io::var::Utf8Serie;
    ///
    /// let col = Utf8Serie::from_strs(&[Some("a"), None, Some("cd"), Some("e")]);
    /// let kept = col.filter(&[true, true, false, true]).unwrap();
    /// assert_eq!(kept.to_strs(), [Some("a"), None, Some("e")]);
    /// ```
    pub fn filter(&self, mask: &[bool]) -> Result<ByteSerie<E>, IoError> {
        if mask.len() != self.len {
            return Err(filter_len_mismatch(mask.len(), self.len));
        }
        // First pass: count kept rows and their total data bytes, so both result buffers pre-size.
        let mut kept = 0usize;
        let mut data_len = 0usize;
        for (index, &keep) in mask.iter().enumerate() {
            if keep {
                kept += 1;
                if let Some(bytes) = self.get_bytes(index) {
                    data_len += bytes.len();
                }
            }
        }
        let mut offsets = Vec::with_capacity(kept + 1);
        offsets.push(0i32);
        let mut data = Vec::with_capacity(data_len);
        let mut validity: Option<Bitmap> = None;
        let mut out_len = 0;
        for (index, &keep) in mask.iter().enumerate() {
            if !keep {
                continue;
            }
            match self.get_bytes(index) {
                Some(bytes) => {
                    data.extend_from_slice(bytes);
                    if let Some(bitmap) = &mut validity {
                        bitmap.push(true);
                    }
                }
                None => {
                    validity
                        .get_or_insert_with(|| Bitmap::all_present(out_len))
                        .push(false);
                }
            }
            offsets.push(data.len() as i32);
            out_len += 1;
        }
        Ok(Self {
            offsets,
            data,
            validity,
            len: kept,
            field: self.field.clone(),
            _element: PhantomData,
        })
    }

    /// A **new** column with every null replaced by `value` (validated for the kind) — one pass. If
    /// the column has no nulls it is cloned; otherwise the offsets and data are rebuilt with `value`
    /// spliced into each null slot and the validity mask **dropped** (the result is fully present).
    /// The var-family twin of [`Serie::fill_null`](crate::io::fixed::Serie::fill_null).
    pub fn fill_null_bytes(&self, value: &[u8]) -> Result<ByteSerie<E>, IoError> {
        E::validate(value)?;
        if !self.has_nulls() {
            return Ok(self.clone());
        }
        // A null slot is zero-width in `data`, so `self.data.len()` is exactly the present bytes; the
        // fills add `null_count * value.len()` — both known up front, so `data` pre-sizes exactly.
        let data_len = self.data.len() + self.null_count() * value.len();
        let mut offsets = Vec::with_capacity(self.len + 1);
        offsets.push(0i32);
        let mut data = Vec::with_capacity(data_len);
        for index in 0..self.len {
            match self.get_bytes(index) {
                Some(bytes) => data.extend_from_slice(bytes),
                None => data.extend_from_slice(value),
            }
            offsets.push(data.len() as i32);
        }
        Ok(Self {
            offsets,
            data,
            validity: None,
            len: self.len,
            field: self.field.clone(),
            _element: PhantomData,
        })
    }

    /// Writes the column to `sink`: `[len:u64][flags:u8][validity?][offsets:(len+1)×i32]`
    /// `[data_len:u64][data]`.
    ///
    /// DESIGN: the small, numerous parts (header + validity + all `len+1` offsets + the data
    /// length) are packed into **one pre-sized buffer** and written in a single bulk `write_all`,
    /// then the (potentially large) data payload is written directly. Writing each offset on its
    /// own would trigger one copy-on-write reallocation of the sink *per offset* — O(n) allocs;
    /// this keeps it to two writes total.
    pub fn write_to<W: IOCursor>(&self, sink: &mut W) -> Result<(), IoError> {
        let has_validity = self.has_nulls();
        let validity_bytes: &[u8] = if has_validity {
            self.validity.as_ref().unwrap().as_bytes()
        } else {
            &[]
        };
        let prefix_len = 8 + 1 + validity_bytes.len() + self.offsets.len() * OFFSET_WIDTH + 8;
        let mut prefix = Vec::with_capacity(prefix_len);
        prefix.extend_from_slice(&(self.len as u64).to_le_bytes());
        prefix.push(u8::from(has_validity));
        prefix.extend_from_slice(validity_bytes);
        for &offset in &self.offsets {
            prefix.extend_from_slice(&offset.to_le_bytes());
        }
        prefix.extend_from_slice(&(self.data.len() as u64).to_le_bytes());
        sink.write_all(&prefix)?;
        sink.write_all(&self.data)
    }

    /// Reads a column written by [`write_to`](ByteSerie::write_to). Validates the decoded data
    /// for the kind, and refuses a corrupt (overflowing) length.
    pub fn read_from<R: IOCursor>(source: &mut R) -> Result<Self, IoError> {
        let len = read_u64(source)? as usize;
        let mut flags = [0u8; 1];
        source.read_exact(&mut flags)?;

        let validity = if flags[0] != 0 {
            let bits = source.read_exact_vec(len.div_ceil(8))?;
            Some(Bitmap::from_bytes(&bits, len))
        } else {
            None
        };

        // `len + 1` offsets, guarding the count (and its byte length) against corruption. Read the
        // raw offset bytes through the bounded `read_exact_vec` (a 64 KiB working buffer that grows
        // only as bytes actually arrive) rather than pre-allocating `len + 1` i32s straight from the
        // untrusted `len`: a merely-huge `len` (e.g. `1 << 40`) fits `usize` yet would otherwise
        // request terabytes up front and abort the process before the (short) source is even read.
        let offset_count = len
            .checked_add(1)
            .filter(|n| n.checked_mul(OFFSET_WIDTH).is_some())
            .ok_or(IoError::CorruptLength {
                len: len as u64,
                width: OFFSET_WIDTH,
            })?;
        let offset_bytes = source.read_exact_vec(offset_count * OFFSET_WIDTH)?;
        let offsets: Vec<i32> = offset_bytes
            .chunks_exact(OFFSET_WIDTH)
            .map(|chunk| i32::from_le_bytes(chunk.try_into().unwrap()))
            .collect();

        let data_len = read_u64(source)? as usize;
        let data = source.read_exact_vec(data_len)?;

        // Validate the decoded offsets so every value slice is in bounds (a corrupt/hostile
        // frame must never make a later `get_bytes` index out of the data buffer): they must
        // start at 0, be non-negative and non-decreasing, and end at `data.len()`.
        if offsets[0] != 0 || offsets[offset_count - 1] as usize != data.len() {
            return Err(IoError::CorruptOffsets {
                offset: offsets[offset_count - 1] as i64,
                data_len,
            });
        }
        let mut previous = 0i32;
        for &offset in &offsets {
            if offset < previous {
                return Err(IoError::CorruptOffsets {
                    offset: offset as i64,
                    data_len,
                });
            }
            previous = offset;
        }

        // Validate each value for the kind. The whole `data` blob decoding is *not* sufficient
        // (a multi-byte code point could straddle a value boundary), so check per slice — now
        // safe, since the offsets are bounds-checked above.
        for window in offsets.windows(2) {
            E::validate(&data[window[0] as usize..window[1] as usize])?;
        }

        Ok(Self {
            offsets,
            data,
            validity,
            len,
            field: ByteField::<E>::new("", false),
            _element: PhantomData,
        })
    }

    /// This column's canonical bytes — the same `[len][flags][validity?][offsets][data_len][data]`
    /// frame [`write_to`](ByteSerie::write_to) produces, returned as an owned `Vec`. The exact
    /// inverse of [`deserialize_bytes`](ByteSerie::deserialize_bytes), and the codec the Python /
    /// Node bindings expose (`serialize_bytes` / `serializeBytes`).
    ///
    /// ```
    /// use yggdryl_core::io::var::Utf8Serie;
    ///
    /// let col = Utf8Serie::from_strs(&[Some("a"), None, Some("cd")]);
    /// assert_eq!(Utf8Serie::deserialize_bytes(&col.serialize_bytes()).unwrap(), col);
    /// ```
    pub fn serialize_bytes(&self) -> Vec<u8> {
        let mut sink = Bytes::new();
        self.write_to(&mut sink)
            .expect("writing to an in-memory buffer is infallible");
        sink.as_slice().to_vec()
    }

    /// Reconstructs a column from the bytes produced by
    /// [`serialize_bytes`](ByteSerie::serialize_bytes), validating each value for the kind and
    /// erroring on a truncated or corrupt frame.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        Self::read_from(&mut Bytes::from_slice(bytes))
    }
}

/// Arrow array interop (feature `arrow`): a var column ↔ a
/// [`GenericByteArray`](arrow_array::GenericByteArray) — a `Utf8Serie` ↔ `StringArray`, a
/// `BinarySerie` ↔ `BinaryArray`. The offsets share the same `i32` Arrow layout; the data buffer
/// is copied (a var column stores its own `Vec<u8>`, so this is not zero-copy — Arrow's data
/// buffer is opaque to the [`IOCursor`] codec the column is otherwise built on).
#[cfg(feature = "arrow")]
impl<E: VarElement> ByteSerie<E> {
    /// This column as an Arrow [`GenericByteArray`](arrow_array::GenericByteArray) of the kind's
    /// [`Arrow`](VarElement::Arrow) type — offsets and validity map straight across; the data is copied.
    pub fn to_arrow_array(&self) -> arrow_array::GenericByteArray<E::Arrow> {
        use arrow_buffer::{Buffer, OffsetBuffer, ScalarBuffer};
        let offsets = OffsetBuffer::new(ScalarBuffer::from(self.offsets.clone()));
        let values = Buffer::from(self.data.as_slice());
        let nulls = self.validity.as_ref().map(|bitmap| {
            let buffer = Buffer::from(bitmap.as_bytes());
            arrow_buffer::NullBuffer::new(arrow_buffer::BooleanBuffer::new(buffer, 0, self.len))
        });
        arrow_array::GenericByteArray::<E::Arrow>::new(offsets, values, nulls)
    }

    /// Builds a column from an Arrow [`GenericByteArray`](arrow_array::GenericByteArray), validating
    /// each value for the kind. Reads the array's **logical** window (so a *sliced* array converts
    /// correctly), and a `Utf8` array's guaranteed-valid bytes pass validation unchanged.
    pub fn from_arrow_array(
        array: &arrow_array::GenericByteArray<E::Arrow>,
    ) -> Result<Self, IoError> {
        use arrow_array::Array;
        let len = array.len();
        let offsets = array.value_offsets(); // `len + 1` logical offsets into `values()`
        let data = array.values().as_slice();
        let mut serie = Self::with_capacity(len);
        for index in 0..len {
            if array.is_null(index) {
                serie.push_bytes(None)?;
            } else {
                let (start, end) = (offsets[index] as usize, offsets[index + 1] as usize);
                serie.push_bytes(Some(&data[start..end]))?;
            }
        }
        Ok(serie)
    }
}

/// Reads a little-endian `u64`.
fn read_u64<R: IOCursor>(source: &mut R) -> Result<u64, IoError> {
    let mut bytes = [0u8; 8];
    source.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

impl<E: VarElement> SerieType for ByteSerie<E> {
    type Elem = Box<[u8]>;

    fn len(&self) -> usize {
        self.len
    }

    fn null_count(&self) -> usize {
        self.null_count()
    }

    fn get(&self, index: usize) -> Option<Box<[u8]>> {
        self.get_bytes(index).map(Into::into)
    }
}

impl<E: VarElement> VarSerie for ByteSerie<E> {
    fn value_bytes(&self, index: usize) -> Option<&[u8]> {
        self.get_bytes(index)
    }
}

// Value identity: two columns are equal iff their lengths, offsets, data, and null positions
// match. A fully-present column compares equal whether or not its validity mask is *materialized*
// (an absent mask and a `Some(all-present)` one, left behind after a `set` clears the last null,
// denote the same value) — keeping identity in lock-step with the byte codec, whose
// [`write_to`](ByteSerie::write_to) drops an all-present mask. So
// `deserialize_bytes(serialize_bytes(x)) == x` holds for every column. (A manual impl, not a
// derive, so this normalization holds — the derive would compare the raw `Option<Bitmap>`.)
impl<E: VarElement> PartialEq for ByteSerie<E> {
    fn eq(&self, other: &Self) -> bool {
        if self.len != other.len || self.offsets != other.offsets || self.data != other.data {
            return false;
        }
        match (self.has_nulls(), other.has_nulls()) {
            (false, false) => true, // both fully present (mask or not)
            (true, true) => self.validity == other.validity, // same null positions
            _ => false,             // one has nulls, the other doesn't
        }
    }
}

impl<E: VarElement> Eq for ByteSerie<E> {}

// The UTF-8 column ergonomics (`push_str` / `from_strs` / `get_str` / `to_strs`) live with the
// `Utf8` marker in the `string` sub-module.

impl<E: VarElement> Default for ByteSerie<E> {
    fn default() -> Self {
        Self {
            offsets: vec![0],
            data: Vec::new(),
            validity: None,
            len: 0,
            field: ByteField::<E>::new("", false),
            _element: PhantomData,
        }
    }
}

impl<E: VarElement> core::fmt::Debug for ByteSerie<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ByteSerie")
            .field("type", &E::NAME)
            .field("len", &self.len)
            .field("null_count", &self.null_count())
            .field("data_len", &self.data.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::super::Utf8Serie;

    #[test]
    fn equality_ignores_a_materialized_all_present_mask() {
        // Clearing the last null with `set` leaves a materialized all-present validity mask; the
        // column must still equal (and round-trip byte-equal to) the same values with no mask.
        let mut cleared = Utf8Serie::from_strs(&[Some("a"), None, Some("cd")]);
        cleared.set_str(1, Some("b")).unwrap();
        assert_eq!(cleared.null_count(), 0);

        let dense = Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("cd")]);
        assert_eq!(cleared, dense);
        assert_eq!(
            Utf8Serie::deserialize_bytes(&cleared.serialize_bytes()).unwrap(),
            cleared
        );

        // A null and a present-but-empty value at the same slot must still differ.
        let a = Utf8Serie::from_strs(&[Some(""), Some("x")]);
        let b = Utf8Serie::from_strs(&[None, Some("x")]);
        assert_ne!(a, b);
    }

    #[test]
    fn from_scalars_round_trips_a_column_through_its_own_scalars() {
        let col = Utf8Serie::from_strs(&[Some("a"), None, Some("cd"), Some("")]);
        let scalars: Vec<_> = (0..col.len()).map(|i| col.get_scalar(i)).collect();
        assert_eq!(Utf8Serie::from_scalars(&scalars).unwrap(), col);

        // The empty slice yields the empty column.
        assert_eq!(Utf8Serie::from_scalars(&[]).unwrap(), Utf8Serie::new());
    }
}
