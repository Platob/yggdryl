//! [`DecimalSerie<B>`] — a nullable decimal **column**: a validity bitmap over a contiguous buffer
//! of raw coefficients, all sharing the column's `(precision, scale)`. Zero-copy to/from Arrow's
//! `Decimal{32,64,128,256}Array` (feature `arrow`) — the coefficient bytes *are* the array's
//! values buffer.

use core::marker::PhantomData;

use arrow_buffer::Buffer as ArrowBuffer;

use super::{
    Decimal, DecimalBacking, DecimalCoeff, DecimalError, DecimalField, DecimalScalar, DecimalType,
};
use crate::io::bitmap::Bitmap;
use crate::io::{IOCursor, IoError, SerieType};

/// The largest coefficient is 32 bytes (`d256`); a stack scratch of this size encodes one
/// coefficient with no allocation while building a column's raw bytes.
const MAX_WIDTH: usize = 32;

/// A nullable column of decimals of width `B`, all with precision `precision` and scale `scale`.
/// The raw coefficients live in one contiguous [`ArrowBuffer`]; a null keeps a zero placeholder so
/// the coefficients stay contiguous (Arrow-style), and the validity mask is materialized only when
/// a null appears.
///
/// ```
/// use yggdryl_core::io::SerieType;
/// use yggdryl_core::io::fixed::{D128, D128Serie};
///
/// let mut col = D128Serie::new(20, 2);
/// col.push(Some(D128::new(12345, 2).unwrap())).unwrap(); // 123.45
/// col.push(None).unwrap();
/// col.push(Some(D128::new(600, 2).unwrap())).unwrap();   //   6.00
/// assert_eq!(col.len(), 3);
/// assert_eq!(col.null_count(), 1);
/// assert_eq!(col.get(0).unwrap().to_string(), "123.45");
/// assert_eq!(col.get(1), None);
/// ```
pub struct DecimalSerie<B: DecimalBacking> {
    validity: Option<Bitmap>,
    values: ArrowBuffer,
    len: usize,
    precision: u8,
    scale: i8,
    _backing: PhantomData<B>,
}

impl<B: DecimalBacking> DecimalSerie<B> {
    /// An empty column of precision `precision`, scale `scale` (clamped to the valid range).
    pub fn new(precision: u8, scale: i8) -> Self {
        let dt = DecimalType::<B>::new(precision, scale);
        Self {
            validity: None,
            values: ArrowBuffer::from_vec(Vec::<u8>::new()),
            len: 0,
            precision: dt.precision(),
            scale: dt.scale(),
            _backing: PhantomData,
        }
    }

    /// An empty column that can grow to `capacity` elements before its first reallocation.
    pub fn with_capacity(precision: u8, scale: i8, capacity: usize) -> Self {
        let dt = DecimalType::<B>::new(precision, scale);
        Self {
            validity: None,
            values: ArrowBuffer::from_vec(Vec::<u8>::with_capacity(capacity * B::WIDTH)),
            len: 0,
            precision: dt.precision(),
            scale: dt.scale(),
            _backing: PhantomData,
        }
    }

    /// The precision.
    pub fn precision(&self) -> u8 {
        self.precision
    }

    /// The scale.
    pub fn scale(&self) -> i8 {
        self.scale
    }

    /// Appends one value (`None` is a null). Re-expresses a present value at the column's scale —
    /// a guided [`InexactRescale`](DecimalError::InexactRescale) if it does not fit exactly — and
    /// checks it against the column's precision ([`PrecisionExceeded`](DecimalError::PrecisionExceeded)).
    /// For building from a known set, prefer [`from_values`](DecimalSerie::from_values) /
    /// [`from_options`](DecimalSerie::from_options), which build the coefficients in one pass
    /// instead of re-sealing the immutable buffer per element.
    pub fn push(&mut self, value: Option<Decimal<B>>) -> Result<(), DecimalError> {
        match value {
            Some(value) => {
                let coeff = Self::fit(value, self.precision, self.scale)?;
                self.push_bytes(coeff);
                if let Some(validity) = &mut self.validity {
                    validity.push(true);
                }
            }
            None => {
                self.push_bytes(B::Coeff::ZERO); // zero placeholder keeps coefficients contiguous
                self.validity
                    .get_or_insert_with(|| Bitmap::all_present(self.len))
                    .push(false);
            }
        }
        self.len += 1;
        Ok(())
    }

    /// Re-expresses `value` at `(precision, scale)`, returning its raw coefficient or a guided
    /// [`DecimalError`].
    fn fit(value: Decimal<B>, precision: u8, scale: i8) -> Result<B::Coeff, DecimalError> {
        let rescaled = value.rescale(scale)?;
        if rescaled.precision() > precision as u32 {
            return Err(DecimalError::PrecisionExceeded {
                ty: B::NAME,
                precision: rescaled.precision(),
                max: precision,
            });
        }
        Ok(rescaled.raw_coeff())
    }

    /// Appends one coefficient's little-endian bytes (copy-on-write into the owned allocation).
    fn push_bytes(&mut self, coeff: B::Coeff) {
        let mut scratch = [0u8; MAX_WIDTH];
        coeff.write_le(&mut scratch);
        let current = core::mem::take(&mut self.values);
        let mut vec = match current.into_vec::<u8>() {
            Ok(owned) => owned,
            Err(shared) => shared.as_slice().to_vec(),
        };
        vec.extend_from_slice(&scratch[..B::WIDTH]);
        self.values = ArrowBuffer::from_vec(vec);
    }

    /// The raw coefficient at `index`, or `None` if null or out of range.
    pub fn get_coeff(&self, index: usize) -> Option<B::Coeff> {
        if index >= self.len {
            return None;
        }
        if let Some(validity) = &self.validity {
            if !validity.get(index) {
                return None;
            }
        }
        let start = index * B::WIDTH;
        Some(B::Coeff::read_le(&self.values.as_slice()[start..]))
    }

    /// The value at `index` as a [`Decimal`], or `None` if null or out of range.
    pub fn get(&self, index: usize) -> Option<Decimal<B>> {
        self.get_coeff(index)
            .map(|coeff| Decimal::from_coeff(coeff, self.scale))
    }

    /// Element `index` as a [`DecimalScalar`] carrying the column's `(precision, scale)` — null if
    /// the element is null or out of range.
    pub fn get_scalar(&self, index: usize) -> DecimalScalar<B> {
        DecimalScalar::from_parts(self.get_coeff(index), self.precision, self.scale)
    }

    // ---- in-place set: single element + bulk (from a Serie / scalars / native values) --------

    /// Overwrites element `index` in place — `Some` re-expresses the value at the column's scale
    /// and precision (a guided [`InexactRescale`](DecimalError::InexactRescale) /
    /// [`PrecisionExceeded`](DecimalError::PrecisionExceeded) if it does not fit), `None` a null.
    /// Errors [`IndexOutOfBounds`](DecimalError::IndexOutOfBounds) past the end.
    pub fn set(&mut self, index: usize, value: Option<Decimal<B>>) -> Result<(), DecimalError> {
        if index >= self.len {
            return Err(DecimalError::IndexOutOfBounds {
                index,
                len: self.len,
            });
        }
        let coeff = match value {
            Some(value) => Self::fit(value, self.precision, self.scale)?,
            None => B::Coeff::ZERO,
        };
        self.write_coeff_at(index, coeff);
        if value.is_some() {
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

    /// Overwrites coefficient bytes at `index` (copy-on-write into the owned allocation).
    fn write_coeff_at(&mut self, index: usize, coeff: B::Coeff) {
        let mut scratch = [0u8; MAX_WIDTH];
        coeff.write_le(&mut scratch);
        let start = index * B::WIDTH;
        let current = core::mem::take(&mut self.values);
        let mut vec = match current.into_vec::<u8>() {
            Ok(owned) => owned,
            Err(shared) => shared.as_slice().to_vec(),
        };
        vec[start..start + B::WIDTH].copy_from_slice(&scratch[..B::WIDTH]);
        self.values = ArrowBuffer::from_vec(vec);
    }

    /// Overwrites element `index` from a [`DecimalScalar`] (its value re-expressed at this column's
    /// scale/precision, or a null).
    pub fn set_scalar(
        &mut self,
        index: usize,
        scalar: &DecimalScalar<B>,
    ) -> Result<(), DecimalError> {
        self.set(index, scalar.value())
    }

    /// Bounds-checks a bulk range `[start, start + count)` against the column length.
    fn check_range(&self, start: usize, count: usize) -> Result<(), DecimalError> {
        match start.checked_add(count) {
            Some(end) if end <= self.len => Ok(()),
            _ => Err(DecimalError::IndexOutOfBounds {
                index: start.max(self.len),
                len: self.len,
            }),
        }
    }

    /// Writes `count` optional values into `[start, start + count)` in **one pass**: fits every
    /// value first (so a bad value leaves the column unchanged), then commits the coefficients with
    /// a **single** copy-on-write of the values buffer, not one re-seal per element.
    fn set_options_range(
        &mut self,
        start: usize,
        count: usize,
        mut next: impl FnMut(usize) -> Result<Option<Decimal<B>>, DecimalError>,
    ) -> Result<(), DecimalError> {
        // Fit (validate) every value up front; on error the column is untouched.
        let mut coeffs: Vec<Option<B::Coeff>> = Vec::with_capacity(count);
        for offset in 0..count {
            coeffs.push(match next(offset)? {
                Some(value) => Some(Self::fit(value, self.precision, self.scale)?),
                None => None,
            });
        }
        // One COW of the coefficient buffer for the whole range.
        let current = core::mem::take(&mut self.values);
        let mut vec = match current.into_vec::<u8>() {
            Ok(owned) => owned,
            Err(shared) => shared.as_slice().to_vec(),
        };
        let mut scratch = [0u8; MAX_WIDTH];
        for (offset, coeff) in coeffs.iter().enumerate() {
            coeff.unwrap_or(B::Coeff::ZERO).write_le(&mut scratch);
            let byte_start = (start + offset) * B::WIDTH;
            vec[byte_start..byte_start + B::WIDTH].copy_from_slice(&scratch[..B::WIDTH]);
            match coeff {
                Some(_) => {
                    if let Some(validity) = &mut self.validity {
                        validity.set(start + offset, true);
                    }
                }
                None => {
                    self.validity
                        .get_or_insert_with(|| Bitmap::all_present(self.len))
                        .set(start + offset, false);
                }
            }
        }
        self.values = ArrowBuffer::from_vec(vec);
        Ok(())
    }

    /// Bulk-overwrites `[start, start + source.len())` from another column (nulls included).
    pub fn set_range(
        &mut self,
        start: usize,
        source: &DecimalSerie<B>,
    ) -> Result<(), DecimalError> {
        self.check_range(start, source.len())?;
        self.set_options_range(start, source.len(), |offset| Ok(source.get(offset)))
    }

    /// Bulk-overwrites `[start, start + scalars.len())` from a slice of [`DecimalScalar`]s.
    pub fn set_scalars(
        &mut self,
        start: usize,
        scalars: &[DecimalScalar<B>],
    ) -> Result<(), DecimalError> {
        self.check_range(start, scalars.len())?;
        self.set_options_range(start, scalars.len(), |offset| Ok(scalars[offset].value()))
    }

    /// Bulk-overwrites `[start, start + values.len())` from present [`Decimal`] values.
    pub fn set_values(&mut self, start: usize, values: &[Decimal<B>]) -> Result<(), DecimalError> {
        self.check_range(start, values.len())?;
        self.set_options_range(start, values.len(), |offset| Ok(Some(values[offset])))
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

    /// The typed descriptor.
    pub fn data_type(&self) -> DecimalType<B> {
        DecimalType::new(self.precision, self.scale)
    }

    /// A column from present values (no nulls), each re-expressed at `(precision, scale)`. Builds
    /// the coefficient bytes in **one pass** (no validity mask).
    pub fn from_values(
        precision: u8,
        scale: i8,
        values: &[Decimal<B>],
    ) -> Result<Self, DecimalError> {
        let dt = DecimalType::<B>::new(precision, scale);
        let (precision, scale) = (dt.precision(), dt.scale());
        let mut bytes = Vec::with_capacity(values.len() * B::WIDTH);
        let mut scratch = [0u8; MAX_WIDTH];
        for &value in values {
            Self::fit(value, precision, scale)?.write_le(&mut scratch);
            bytes.extend_from_slice(&scratch[..B::WIDTH]);
        }
        Ok(Self {
            validity: None,
            values: ArrowBuffer::from_vec(bytes),
            len: values.len(),
            precision,
            scale,
            _backing: PhantomData,
        })
    }

    /// A column from optional values, materializing a validity mask only if a null appears. Builds
    /// the coefficient bytes and the mask in **one pass**, then wraps the byte `Vec` with no copy.
    pub fn from_options(
        precision: u8,
        scale: i8,
        values: &[Option<Decimal<B>>],
    ) -> Result<Self, DecimalError> {
        let dt = DecimalType::<B>::new(precision, scale);
        let (precision, scale) = (dt.precision(), dt.scale());
        let mut bytes = Vec::with_capacity(values.len() * B::WIDTH);
        let mut scratch = [0u8; MAX_WIDTH];
        let mut validity: Option<Bitmap> = None;
        for (index, value) in values.iter().enumerate() {
            match value {
                Some(value) => {
                    Self::fit(*value, precision, scale)?.write_le(&mut scratch);
                    if let Some(bitmap) = &mut validity {
                        bitmap.push(true);
                    }
                }
                None => {
                    B::Coeff::ZERO.write_le(&mut scratch); // placeholder keeps coefficients contiguous
                    validity
                        .get_or_insert_with(|| Bitmap::all_present(index))
                        .push(false);
                }
            }
            bytes.extend_from_slice(&scratch[..B::WIDTH]);
        }
        Ok(Self {
            validity,
            values: ArrowBuffer::from_vec(bytes),
            len: values.len(),
            precision,
            scale,
            _backing: PhantomData,
        })
    }

    /// A [`DecimalField`] naming this column, nullability inferred from whether it holds nulls.
    pub fn to_field(&self, name: &str) -> DecimalField<B> {
        DecimalField::new(name, self.precision, self.scale, self.has_nulls())
    }

    /// Writes the column: `[len: u64][precision: u8][scale: i8][flags: u8][validity?][values]`.
    pub fn write_to<W: IOCursor>(&self, sink: &mut W) -> Result<(), IoError> {
        let has_validity = self.has_nulls();
        sink.write_all(&(self.len as u64).to_le_bytes())?;
        sink.write_all(&[self.precision, self.scale as u8, u8::from(has_validity)])?;
        if has_validity {
            sink.write_all(self.validity.as_ref().unwrap().as_bytes())?;
        }
        sink.write_all(self.values.as_slice())
    }

    /// Reads a column written by [`write_to`](DecimalSerie::write_to).
    pub fn read_from<R: IOCursor>(source: &mut R) -> Result<Self, IoError> {
        let mut header = [0u8; 8 + 3];
        source.read_exact(&mut header)?;
        let len = u64::from_le_bytes(header[..8].try_into().unwrap()) as usize;
        let precision = header[8];
        let scale = header[9] as i8;
        let has_validity = header[10] != 0;

        let validity = if has_validity {
            let mut bits = vec![0u8; len.div_ceil(8)];
            source.read_exact(&mut bits)?;
            Some(Bitmap::from_bytes(&bits, len))
        } else {
            None
        };

        let byte_len = len.checked_mul(B::WIDTH).ok_or(IoError::CorruptLength {
            len: len as u64,
            width: B::WIDTH,
        })?;
        let mut values = vec![0u8; byte_len];
        source.read_exact(&mut values)?;
        Ok(Self {
            validity,
            values: ArrowBuffer::from_vec(values),
            len,
            precision,
            scale,
            _backing: PhantomData,
        })
    }
}

impl<B: DecimalBacking> SerieType for DecimalSerie<B> {
    type Elem = Decimal<B>;

    fn len(&self) -> usize {
        self.len
    }

    fn null_count(&self) -> usize {
        self.null_count()
    }

    fn get(&self, index: usize) -> Option<Decimal<B>> {
        self.get(index)
    }
}

// Structural identity: same descriptor, length, validity, and present coefficients. Comparing only
// *present* coefficients means the unspecified bytes under null slots (Arrow leaves them undefined)
// never affect equality.
impl<B: DecimalBacking> PartialEq for DecimalSerie<B> {
    fn eq(&self, other: &Self) -> bool {
        if self.precision != other.precision
            || self.scale != other.scale
            || self.len != other.len
            || self.validity != other.validity
        {
            return false;
        }
        (0..self.len).all(|i| self.get_coeff(i) == other.get_coeff(i))
    }
}
impl<B: DecimalBacking> Eq for DecimalSerie<B> {}

impl<B: DecimalBacking> Clone for DecimalSerie<B> {
    fn clone(&self) -> Self {
        Self {
            validity: self.validity.clone(),
            values: self.values.clone(), // Arc bump, not a payload copy
            len: self.len,
            precision: self.precision,
            scale: self.scale,
            _backing: PhantomData,
        }
    }
}

impl<B: DecimalBacking> core::fmt::Debug for DecimalSerie<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DecimalSerie")
            .field("type", &B::NAME)
            .field("precision", &self.precision)
            .field("scale", &self.scale)
            .field("len", &self.len)
            .field("null_count", &self.null_count())
            .finish()
    }
}

/// Zero-copy interop with Arrow's decimal `PrimitiveArray` (feature `arrow`): the coefficient
/// bytes share the `Arc` allocation, and the validity mask is byte-identical to Arrow's null
/// buffer, so conversion is a refcount bump (bar a one-off realignment of a byte-level slice).
#[cfg(feature = "arrow")]
impl<B: DecimalBacking> DecimalSerie<B>
where
    B::Coeff: arrow_buffer::ArrowNativeType,
{
    /// The coefficient bytes as an **element-aligned** Arrow value buffer — zero-copy (an `Arc`
    /// bump) when already aligned, else realigned with one copy.
    fn arrow_values(&self) -> ArrowBuffer {
        if self
            .values
            .as_ptr()
            .align_offset(core::mem::align_of::<B::Coeff>())
            == 0
        {
            self.values.clone()
        } else {
            ArrowBuffer::from(self.values.as_slice())
        }
    }

    /// This column as an Arrow decimal `PrimitiveArray` — the coefficients are **zero-copy**; the
    /// validity mask is wrapped as a `NullBuffer`, and the array carries the column's precision/scale.
    pub fn to_arrow_array(&self) -> arrow_array::PrimitiveArray<B::Arrow> {
        let values = arrow_buffer::ScalarBuffer::<B::Coeff>::new(self.arrow_values(), 0, self.len);
        let nulls = self.validity.as_ref().map(|bitmap| {
            let buffer = ArrowBuffer::from(bitmap.as_bytes());
            arrow_buffer::NullBuffer::new(arrow_buffer::BooleanBuffer::new(buffer, 0, self.len))
        });
        arrow_array::PrimitiveArray::<B::Arrow>::new(values, nulls)
            .with_precision_and_scale(self.precision, self.scale)
            .expect("DecimalType clamps precision/scale into Arrow's valid range")
    }

    /// Builds a column from an Arrow decimal `PrimitiveArray` — the coefficients are **zero-copy**
    /// (a shared `Arc`); precision/scale and the validity mask are read back from the array. The
    /// bytes under null slots are carried through as-is (Arrow leaves them undefined; they are
    /// invisible to [`get`](DecimalSerie::get) and to equality).
    pub fn from_arrow_array(array: &arrow_array::PrimitiveArray<B::Arrow>) -> Self {
        use arrow_array::Array;
        let len = array.len();
        let values = array.values().inner().clone();
        let validity = array.nulls().map(|_| {
            let mut bits = vec![0u8; len.div_ceil(8)];
            for index in 0..len {
                if array.is_valid(index) {
                    bits[index / 8] |= 1 << (index % 8);
                }
            }
            Bitmap::from_bytes(&bits, len)
        });
        Self {
            validity,
            values,
            len,
            precision: array.precision(),
            scale: array.scale(),
            _backing: PhantomData,
        }
    }
}
