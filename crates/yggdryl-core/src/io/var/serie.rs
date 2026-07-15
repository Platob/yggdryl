//! [`ByteSerie`] — a nullable, variable-length column (`Utf8Serie = ByteSerie<Utf8>`,
//! `BinarySerie = ByteSerie<Binary>`): Arrow-style **offsets + data** with an optional
//! validity bitmap.

use core::marker::PhantomData;

use super::dtype::OFFSET_WIDTH;
use super::{ByteField, ByteScalar, ByteType, VarElement};
use crate::io::bitmap::Bitmap;
use crate::io::{IOCursor, IoError, SerieType};

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
#[derive(Clone, PartialEq, Eq)]
pub struct ByteSerie<E: VarElement> {
    /// `len + 1` offsets into `data` (`offsets[0] == 0`).
    offsets: Vec<i32>,
    /// The concatenated value bytes.
    data: Vec<u8>,
    /// `None` means "no nulls".
    validity: Option<Bitmap>,
    len: usize,
    _element: PhantomData<E>,
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

    /// A column from optional raw byte values, validated for the kind.
    pub fn from_byte_values(values: &[Option<&[u8]>]) -> Result<Self, IoError> {
        let mut serie = Self::with_capacity(values.len());
        for &value in values {
            serie.push_bytes(value)?;
        }
        Ok(serie)
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

    /// A [`ByteField`] naming a column of this serie's type with explicit nullability.
    pub fn field(&self, name: &str, nullable: bool) -> ByteField<E> {
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
            let mut bits = vec![0u8; len.div_ceil(8)];
            source.read_exact(&mut bits)?;
            Some(Bitmap::from_bytes(&bits, len))
        } else {
            None
        };

        // `len + 1` offsets, guarding the count (and its byte length) against corruption.
        let offset_count = len
            .checked_add(1)
            .filter(|n| n.checked_mul(OFFSET_WIDTH).is_some())
            .ok_or(IoError::CorruptLength {
                len: len as u64,
                width: OFFSET_WIDTH,
            })?;
        let mut offsets = Vec::with_capacity(offset_count);
        for _ in 0..offset_count {
            let mut bytes = [0u8; OFFSET_WIDTH];
            source.read_exact(&mut bytes)?;
            offsets.push(i32::from_le_bytes(bytes));
        }

        let data_len = read_u64(source)? as usize;
        let mut data = vec![0u8; data_len];
        source.read_exact(&mut data)?;

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
            _element: PhantomData,
        })
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

// The UTF-8 column ergonomics (`push_str` / `from_strs` / `get_str` / `to_strs`) live with the
// `Utf8` marker in the `string` sub-module.

impl<E: VarElement> Default for ByteSerie<E> {
    fn default() -> Self {
        Self {
            offsets: vec![0],
            data: Vec::new(),
            validity: None,
            len: 0,
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
