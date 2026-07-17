//! [`TemporalSerie<B>`] — a nullable temporal **column**: a validity bitmap over a contiguous
//! buffer of raw physical counts, all sharing the column's `(unit, tz)`. Interop with Arrow's
//! `Date*` / `Time*` / `Timestamp` / `Duration` arrays (and `FixedSizeBinary(12)` for the wide
//! `ts96`) under feature `arrow` — zero-copy on the native-width path.

use core::marker::PhantomData;

use arrow_buffer::Buffer as ArrowBuffer;

use super::time::{unit_from_tag, unit_tag};
use super::{
    Temporal, TemporalBacking, TemporalError, TemporalField, TemporalNative, TemporalScalar,
    TemporalType, TimeUnit, Tz,
};
use crate::io::any_serie::filter_len_mismatch;
use crate::io::bitmap::{extend_validity, Bitmap};
use crate::io::field_carrier::field_accessors;
use crate::io::{AnyField, Bytes, IOCursor, IoError, SerieType};

/// The largest physical count is 12 bytes (`ts96`); a stack scratch of this size encodes one count
/// with no allocation while building a column's raw bytes.
const MAX_WIDTH: usize = 12;

/// A nullable column of temporal values of concept+width `B`, all with resolution `unit` and zone
/// `tz`. The raw counts live in one contiguous [`ArrowBuffer`]; a null keeps a zero placeholder so
/// the counts stay contiguous (Arrow-style), and the validity mask is materialized only when a null
/// appears.
///
/// ```
/// use yggdryl_core::io::fixed::Ts64Serie;
/// use yggdryl_core::io::fixed::temporal::{Ts64, TimeUnit, Tz};
///
/// let a = Ts64::from_epoch(1_000, TimeUnit::Second, Tz::UTC).unwrap();
/// let b = Ts64::from_epoch(2_000, TimeUnit::Second, Tz::UTC).unwrap();
/// let mut col = Ts64Serie::from_options(TimeUnit::Second, Tz::UTC, &[Some(a), None, Some(b)]).unwrap();
/// assert_eq!(col.len(), 3);
/// assert_eq!(col.null_count(), 1);
/// assert_eq!(col.get(0).unwrap().epoch_value(), 1_000);
/// assert_eq!(col.get(1), None);
/// assert_eq!(Ts64Serie::deserialize_bytes(&col.serialize_bytes()).unwrap(), col);
/// ```
pub struct TemporalSerie<B: TemporalBacking> {
    validity: Option<Bitmap>,
    values: ArrowBuffer,
    len: usize,
    /// The column's own [`TemporalField`] descriptor — its name, declared nullability, metadata, and
    /// the `(unit, tz)` dtype params. The `(unit, tz)` join the data in value identity and the byte
    /// codec; the name / nullable / metadata are excluded.
    field: TemporalField<B>,
    _backing: PhantomData<B>,
}

impl<B: TemporalBacking> TemporalSerie<B> {
    /// An empty column at `(unit, tz)` (clamped to what `B` admits).
    pub fn new(unit: TimeUnit, tz: Tz) -> Self {
        Self {
            validity: None,
            values: ArrowBuffer::from_vec(Vec::<u8>::new()),
            len: 0,
            field: TemporalField::new("", unit, tz, false),
            _backing: PhantomData,
        }
    }

    /// An empty column that can grow to `capacity` elements before its first reallocation.
    pub fn with_capacity(unit: TimeUnit, tz: Tz, capacity: usize) -> Self {
        Self {
            validity: None,
            values: ArrowBuffer::from_vec(Vec::<u8>::with_capacity(capacity * B::WIDTH)),
            len: 0,
            field: TemporalField::new("", unit, tz, false),
            _backing: PhantomData,
        }
    }

    field_accessors!();

    /// The resolution (from the held field).
    pub fn unit(&self) -> TimeUnit {
        self.field.unit()
    }

    /// The timezone (from the held field).
    pub fn timezone(&self) -> Tz {
        self.field.timezone()
    }

    /// The erased [`AnyField`] this column contributes — its **held field** (name + metadata +
    /// unit/tz) with **effective** nullability `self.nullable() || self.has_nulls()` folded in — a
    /// lenient, Arrow-standard over-approximation.
    pub fn field(&self) -> AnyField {
        let mut field = self.field.clone();
        field.set_nullable(self.nullable() || self.has_nulls());
        AnyField::leaf(field.erase())
    }

    /// Like [`field`](TemporalSerie::field) but **consumes** the column.
    pub fn into_field(mut self) -> AnyField {
        let nullable = self.nullable() || self.has_nulls();
        self.field.set_nullable(nullable);
        AnyField::leaf(self.field.erase())
    }

    /// Re-expresses `value`'s physical count into the column's `unit` and validates it fits this
    /// width's range, returning the fitted count or a guided [`TemporalError`].
    ///
    /// DESIGN: a value stored at a different resolution is converted into the column's unit
    /// (truncating a finer→coarser step, like the value types' own `to_unit`); a calendar unit or
    /// an out-of-range count errors rather than silently truncating the bytes.
    fn fit(value: B::Native, unit: TimeUnit, tz: Tz) -> Result<i128, TemporalError> {
        let count = TimeUnit::convert(value.to_count(), value.time_unit(), unit).ok_or(
            TemporalError::Overflow {
                ty: B::NAME,
                op: "unit",
            },
        )?;
        // Rebuild through the value type so the count is range-checked for this width/unit.
        Ok(B::Native::from_count(count, unit, tz)?.to_count())
    }

    /// Appends one value (`None` is a null). Re-expresses a present value at the column's unit (see
    /// [`fit`](TemporalSerie::fit)). For building from a known set, prefer
    /// [`from_values`](TemporalSerie::from_values) / [`from_options`](TemporalSerie::from_options),
    /// which build the counts in one pass instead of re-sealing the immutable buffer per element.
    pub fn push(&mut self, value: Option<B::Native>) -> Result<(), TemporalError> {
        match value {
            Some(value) => {
                let count = Self::fit(value, self.unit(), self.timezone())?;
                self.push_bytes(count);
                if let Some(validity) = &mut self.validity {
                    validity.push(true);
                }
            }
            None => {
                self.push_bytes(0); // zero placeholder keeps counts contiguous
                self.validity
                    .get_or_insert_with(|| Bitmap::all_present(self.len))
                    .push(false);
            }
        }
        self.len += 1;
        Ok(())
    }

    /// Appends one count's little-endian bytes (copy-on-write into the owned allocation).
    fn push_bytes(&mut self, count: i128) {
        let mut scratch = [0u8; MAX_WIDTH];
        write_count_le(count, B::WIDTH, &mut scratch);
        let current = core::mem::take(&mut self.values);
        let mut vec = match current.into_vec::<u8>() {
            Ok(owned) => owned,
            Err(shared) => shared.as_slice().to_vec(),
        };
        vec.extend_from_slice(&scratch[..B::WIDTH]);
        self.values = ArrowBuffer::from_vec(vec);
    }

    /// The raw physical count at `index`, or `None` if null or out of range.
    pub fn get_count(&self, index: usize) -> Option<i128> {
        if index >= self.len {
            return None;
        }
        if let Some(validity) = &self.validity {
            if !validity.get(index) {
                return None;
            }
        }
        let start = index * B::WIDTH;
        Some(read_count_le(&self.values.as_slice()[start..], B::WIDTH))
    }

    /// The value at `index`, or `None` if null or out of range.
    pub fn get(&self, index: usize) -> Option<B::Native> {
        self.get_count(index)
            .and_then(|count| B::Native::from_count(count, self.unit(), self.timezone()).ok())
    }

    /// Element `index` as a [`TemporalScalar`] carrying the column's `(unit, tz)` — null if the
    /// element is null or out of range.
    pub fn get_scalar(&self, index: usize) -> TemporalScalar<B> {
        TemporalScalar::from_parts(self.get_count(index), self.unit(), self.timezone())
    }

    /// Overwrites element `index` in place — `Some` re-expresses the value at the column's unit (a
    /// guided [`TemporalError`] if it does not fit), `None` a null. Errors
    /// [`OutOfRange`](TemporalError::OutOfRange) past the end.
    pub fn set(&mut self, index: usize, value: Option<B::Native>) -> Result<(), TemporalError> {
        if index >= self.len {
            return Err(TemporalError::OutOfRange { ty: B::NAME });
        }
        let count = match value {
            Some(value) => Self::fit(value, self.unit(), self.timezone())?,
            None => 0,
        };
        self.write_count_at(index, count);
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

    /// Overwrites count bytes at `index` (copy-on-write into the owned allocation).
    fn write_count_at(&mut self, index: usize, count: i128) {
        let mut scratch = [0u8; MAX_WIDTH];
        write_count_le(count, B::WIDTH, &mut scratch);
        let start = index * B::WIDTH;
        let current = core::mem::take(&mut self.values);
        let mut vec = match current.into_vec::<u8>() {
            Ok(owned) => owned,
            Err(shared) => shared.as_slice().to_vec(),
        };
        vec[start..start + B::WIDTH].copy_from_slice(&scratch[..B::WIDTH]);
        self.values = ArrowBuffer::from_vec(vec);
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

    /// The raw little-endian count bytes (`len * B::WIDTH`) — the flat-buffer view for erasing one
    /// count to a cell value.
    pub fn count_bytes(&self) -> &[u8] {
        self.values.as_slice()
    }

    /// The typed descriptor.
    pub fn data_type(&self) -> TemporalType<B> {
        TemporalType::new(self.unit(), self.timezone())
    }

    /// A [`TemporalField`] naming this column, nullability inferred from whether it holds nulls.
    pub fn to_field(&self, name: &str) -> TemporalField<B> {
        TemporalField::new(name, self.unit(), self.timezone(), self.has_nulls())
    }

    /// A column from present values (no nulls), each re-expressed at the column's unit. Builds the
    /// count bytes in **one pass** (no validity mask).
    pub fn from_values(
        unit: TimeUnit,
        tz: Tz,
        values: &[B::Native],
    ) -> Result<Self, TemporalError> {
        let dt = TemporalType::<B>::new(unit, tz);
        let (unit, tz) = (dt.unit(), dt.timezone());
        let mut bytes = Vec::with_capacity(values.len() * B::WIDTH);
        let mut scratch = [0u8; MAX_WIDTH];
        for &value in values {
            write_count_le(Self::fit(value, unit, tz)?, B::WIDTH, &mut scratch);
            bytes.extend_from_slice(&scratch[..B::WIDTH]);
        }
        Ok(Self {
            validity: None,
            values: ArrowBuffer::from_vec(bytes),
            len: values.len(),
            field: TemporalField::new("", unit, tz, false),
            _backing: PhantomData,
        })
    }

    /// A column from optional values, materializing a validity mask only if a null appears. Builds
    /// the count bytes and the mask in **one pass**, then wraps the byte `Vec` with no copy.
    pub fn from_options(
        unit: TimeUnit,
        tz: Tz,
        values: &[Option<B::Native>],
    ) -> Result<Self, TemporalError> {
        let dt = TemporalType::<B>::new(unit, tz);
        let (unit, tz) = (dt.unit(), dt.timezone());
        let mut bytes = Vec::with_capacity(values.len() * B::WIDTH);
        let mut scratch = [0u8; MAX_WIDTH];
        let mut validity: Option<Bitmap> = None;
        for (index, value) in values.iter().enumerate() {
            match value {
                Some(value) => {
                    write_count_le(Self::fit(*value, unit, tz)?, B::WIDTH, &mut scratch);
                    if let Some(bitmap) = &mut validity {
                        bitmap.push(true);
                    }
                }
                None => {
                    write_count_le(0, B::WIDTH, &mut scratch); // placeholder keeps counts contiguous
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
            field: TemporalField::new("", unit, tz, false),
            _backing: PhantomData,
        })
    }

    /// This column **viewed as a single [`TemporalScalar`]** (carrying the column's `(unit, tz)`),
    /// if it holds exactly one element — the inverse of
    /// [`TemporalScalar::to_serie`](TemporalScalar::to_serie).
    pub fn as_scalar(&self) -> Option<TemporalScalar<B>> {
        (self.len == 1).then(|| self.get_scalar(0))
    }

    /// A length-1 column broadcasting `scalar` at its own `(unit, tz)` — the singular of
    /// [`from_scalars`](TemporalSerie::from_scalars); the inverse of
    /// [`as_scalar`](TemporalSerie::as_scalar).
    pub fn from_scalar(scalar: TemporalScalar<B>) -> Result<Self, TemporalError> {
        Self::from_options(scalar.unit(), scalar.timezone(), &[scalar.value()])
    }

    /// A column at `(unit, tz)` from a slice of [`TemporalScalar`]s — each present scalar's value is
    /// re-expressed at the column's unit (see [`fit`](TemporalSerie::fit); a guided
    /// [`TemporalError`] if it does not fit), a null scalar a null. The bulk factory mirroring
    /// [`from_options`](TemporalSerie::from_options) over each scalar's value.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::{Ts64Scalar, Ts64Serie};
    /// use yggdryl_core::io::fixed::temporal::{Ts64, TimeUnit, Tz};
    ///
    /// let a = Ts64::from_epoch(1_000, TimeUnit::Second, Tz::UTC).unwrap();
    /// let col = Ts64Serie::from_scalars(
    ///     TimeUnit::Second,
    ///     Tz::UTC,
    ///     &[Ts64Scalar::of(a), Ts64Scalar::null(TimeUnit::Second, Tz::UTC)],
    /// )
    /// .unwrap();
    /// assert_eq!(col.get(0).unwrap().epoch_value(), 1_000);
    /// assert_eq!(col.get(1), None);
    /// ```
    pub fn from_scalars(
        unit: TimeUnit,
        tz: Tz,
        scalars: &[TemporalScalar<B>],
    ) -> Result<Self, TemporalError> {
        Self::from_options(
            unit,
            tz,
            &scalars
                .iter()
                .map(TemporalScalar::value)
                .collect::<Vec<_>>(),
        )
    }

    // ---- grow: append single + bulk (the mutator vocabulary) ----------------------------

    /// Appends `count` optional values in **one pass**: re-expresses (fits) every value at this
    /// column's unit first — so a bad value leaves the column unchanged — then commits the counts
    /// with a **single** copy-on-write append of the values buffer, not one re-seal per element
    /// (which would be O(n²) over a bulk grow). Shared by every `extend_*`.
    fn extend_with(
        &mut self,
        count: usize,
        mut next: impl FnMut(usize) -> Result<Option<B::Native>, TemporalError>,
    ) -> Result<(), TemporalError> {
        if count == 0 {
            return Ok(());
        }
        // Fit (validate) every value up front; on error the column is untouched.
        let mut counts: Vec<Option<i128>> = Vec::with_capacity(count);
        for offset in 0..count {
            counts.push(match next(offset)? {
                Some(value) => Some(Self::fit(value, self.unit(), self.timezone())?),
                None => None,
            });
        }
        self.append_counts(&counts);
        Ok(())
    }

    /// Commits pre-fitted `counts` (each present-or-null) with a **single** copy-on-write append of
    /// the counts buffer, growing the validity mask in lock-step. The one place the immutable buffer
    /// is re-sealed for a grow.
    fn append_counts(&mut self, counts: &[Option<i128>]) {
        let base = self.len;
        let current = core::mem::take(&mut self.values);
        let mut vec = match current.into_vec::<u8>() {
            Ok(owned) => owned,
            Err(shared) => shared.as_slice().to_vec(),
        };
        vec.reserve(counts.len() * B::WIDTH);
        let mut scratch = [0u8; MAX_WIDTH];
        for (offset, count) in counts.iter().enumerate() {
            write_count_le(count.unwrap_or(0), B::WIDTH, &mut scratch); // zero placeholder under a null
            vec.extend_from_slice(&scratch[..B::WIDTH]);
            match count {
                Some(_) => {
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
        }
        self.values = ArrowBuffer::from_vec(vec);
        self.len += counts.len();
    }

    /// Appends one present value from its raw little-endian count bytes at the column's unit (no
    /// re-expression) — the erased single-row append path
    /// ([`AnySerie::append_scalar`](crate::io::AnySerie::append_scalar)). Assumes `bytes.len() ==
    /// B::WIDTH` and the bytes are already a count at this column's `(unit, tz)`.
    pub(crate) fn append_count_bytes(&mut self, bytes: &[u8]) {
        self.push_bytes(read_count_le(bytes, B::WIDTH));
        if let Some(validity) = &mut self.validity {
            validity.push(true);
        }
        self.len += 1;
    }

    /// Overwrites the present value at `index` from its raw little-endian count bytes at the column's
    /// unit (no re-expression), **preserving the length** — the erased length-preserving cell set path
    /// ([`AnySerie::set_cell`](crate::io::AnySerie::set_cell)). Assumes `bytes.len() == B::WIDTH` and
    /// the bytes are already a count at this column's `(unit, tz)`. Errors
    /// [`IndexOutOfBounds`](IoError::IndexOutOfBounds) past the end.
    pub(crate) fn set_count_bytes(&mut self, index: usize, bytes: &[u8]) -> Result<(), IoError> {
        if index >= self.len {
            return Err(IoError::IndexOutOfBounds {
                index,
                len: self.len,
            });
        }
        self.write_count_at(index, read_count_le(bytes, B::WIDTH));
        if let Some(validity) = &mut self.validity {
            validity.set(index, true);
        }
        Ok(())
    }

    /// Appends a slice of **present** values (no nulls), each re-expressed at this column's unit —
    /// the bulk grow twin of [`from_values`](TemporalSerie::from_values). One copy-on-write; a guided
    /// [`TemporalError`] if a value does not fit (the column is left unchanged).
    pub fn extend_values(&mut self, values: &[B::Native]) -> Result<(), TemporalError> {
        self.extend_with(values.len(), |offset| Ok(Some(values[offset])))
    }

    /// Appends a slice of **optional** values — the bulk grow twin of
    /// [`from_options`](TemporalSerie::from_options). A null lazily materializes the validity mask.
    pub fn extend_options(&mut self, values: &[Option<B::Native>]) -> Result<(), TemporalError> {
        self.extend_with(values.len(), |offset| Ok(values[offset]))
    }

    /// Appends a slice of [`TemporalScalar`]s (each its value re-expressed at this column's unit, or
    /// a null) — the bulk grow twin of [`from_scalars`](TemporalSerie::from_scalars).
    pub fn extend_scalars(&mut self, scalars: &[TemporalScalar<B>]) -> Result<(), TemporalError> {
        self.extend_with(scalars.len(), |offset| Ok(scalars[offset].value()))
    }

    /// Appends **another whole column** of the same concept+width to this one — the two columns
    /// concatenate. When the source shares this column's `(unit, tz)` the raw count bytes are
    /// appended with a **single** copy-on-write (a memcpy, no per-element re-expression); otherwise
    /// each source value is re-expressed at this column's unit (a guided [`TemporalError`] if it does
    /// not fit). Null positions carry over in the same pass.
    pub fn concat(&mut self, source: &TemporalSerie<B>) -> Result<(), TemporalError> {
        if source.len == 0 {
            return Ok(());
        }
        if source.unit() == self.unit() && source.timezone() == self.timezone() {
            // Fast path: identical descriptor — memcpy the raw count bytes in one COW.
            let base = self.len;
            let current = core::mem::take(&mut self.values);
            let mut vec = match current.into_vec::<u8>() {
                Ok(owned) => owned,
                Err(shared) => shared.as_slice().to_vec(),
            };
            vec.extend_from_slice(source.count_bytes());
            self.values = ArrowBuffer::from_vec(vec);
            extend_validity(&mut self.validity, base, source.len, |offset| {
                source.validity.as_ref().is_none_or(|mask| mask.get(offset))
            });
            self.len += source.len;
            Ok(())
        } else {
            // Re-express path: fit each source value at this column's (unit, tz).
            self.extend_with(source.len, |offset| Ok(source.get(offset)))
        }
    }

    // ---- reshape: filter (keep selected rows) + fill_null (replace nulls) -----------------

    /// A **new** column keeping only the elements where `mask[i]` is `true` — the bitmap-optimized
    /// row filter, over the raw physical count bytes (the `(unit, tz)` is preserved). Errors
    /// ([`Unsupported`](IoError::Unsupported)) if `mask.len() != self.len()`.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Ts64Serie;
    /// use yggdryl_core::io::fixed::temporal::{Ts64, TimeUnit, Tz};
    ///
    /// let a = Ts64::from_epoch(1_000, TimeUnit::Second, Tz::UTC).unwrap();
    /// let b = Ts64::from_epoch(2_000, TimeUnit::Second, Tz::UTC).unwrap();
    /// let col = Ts64Serie::from_options(TimeUnit::Second, Tz::UTC, &[Some(a), None, Some(b)]).unwrap();
    /// let kept = col.filter(&[true, false, true]).unwrap();
    /// assert_eq!(kept.len(), 2);
    /// assert_eq!(kept.get(1), Some(b));
    /// ```
    pub fn filter(&self, mask: &[bool]) -> Result<TemporalSerie<B>, IoError> {
        if mask.len() != self.len {
            return Err(filter_len_mismatch(mask.len(), self.len));
        }
        let kept = mask.iter().filter(|&&keep| keep).count();
        let src = self.values.as_slice();
        let mut bytes = Vec::with_capacity(kept * B::WIDTH);
        let mut validity: Option<Bitmap> = None;
        let mut out_len = 0;
        for (index, &keep) in mask.iter().enumerate() {
            if !keep {
                continue;
            }
            bytes.extend_from_slice(&src[index * B::WIDTH..(index + 1) * B::WIDTH]);
            if self
                .validity
                .as_ref()
                .is_none_or(|bitmap| bitmap.get(index))
            {
                if let Some(bitmap) = &mut validity {
                    bitmap.push(true);
                }
            } else {
                validity
                    .get_or_insert_with(|| Bitmap::all_present(out_len))
                    .push(false);
            }
            out_len += 1;
        }
        Ok(Self {
            validity,
            values: ArrowBuffer::from_vec(bytes),
            len: kept,
            field: self.field.clone(),
            _backing: PhantomData,
        })
    }

    /// A **new** column with every null count replaced by the raw little-endian bytes `fill` (already
    /// a count at this column's `(unit, tz)`, exactly `B::WIDTH` long) — one pass, the erased
    /// [`fill_null`](crate::io::AnySerie::fill_null) path. If the column has no nulls it is cloned;
    /// otherwise the counts are copied, each null slot overwritten with `fill`, and the validity mask
    /// **dropped** (fully present).
    pub(crate) fn fill_null_count_bytes(&self, fill: &[u8]) -> TemporalSerie<B> {
        if !self.has_nulls() {
            return self.clone();
        }
        let mut bytes = self.count_bytes().to_vec();
        if let Some(validity) = &self.validity {
            for index in 0..self.len {
                if !validity.get(index) {
                    bytes[index * B::WIDTH..(index + 1) * B::WIDTH].copy_from_slice(fill);
                }
            }
        }
        Self {
            validity: None,
            values: ArrowBuffer::from_vec(bytes),
            len: self.len,
            field: self.field.clone(),
            _backing: PhantomData,
        }
    }

    /// A **new** column at `(unit, tz)` from already-computed raw physical `counts` (each present or
    /// a null) — the rebuild path of the vectorized [`dyn AnySerie::add`](crate::io::AnySerie) family
    /// on a temporal column: the erased op runs the arithmetic through this column's **backing
    /// integer** and hands the wrapped result counts back here. It writes each count's low
    /// `B::WIDTH` little-endian bytes **directly** (no re-fit / range-check — the backing wrap is
    /// already applied, so a result outside the value type's normal range still round-trips its
    /// bytes), materializing the validity mask only if a null appears. One pass.
    pub(crate) fn from_result_counts(unit: TimeUnit, tz: Tz, counts: &[Option<i128>]) -> Self {
        let mut bytes = Vec::with_capacity(counts.len() * B::WIDTH);
        let mut scratch = [0u8; MAX_WIDTH];
        let mut validity: Option<Bitmap> = None;
        for (index, count) in counts.iter().enumerate() {
            match count {
                Some(count) => {
                    write_count_le(*count, B::WIDTH, &mut scratch);
                    if let Some(bitmap) = &mut validity {
                        bitmap.push(true);
                    }
                }
                None => {
                    write_count_le(0, B::WIDTH, &mut scratch); // zero placeholder under the null
                    validity
                        .get_or_insert_with(|| Bitmap::all_present(index))
                        .push(false);
                }
            }
            bytes.extend_from_slice(&scratch[..B::WIDTH]);
        }
        Self {
            validity,
            values: ArrowBuffer::from_vec(bytes),
            len: counts.len(),
            field: TemporalField::new("", unit, tz, false),
            _backing: PhantomData,
        }
    }

    /// Writes the column: `[len: u64][unit tag: u8][tz name][flags: u8][validity?][values]`.
    pub fn write_to<W: IOCursor>(&self, sink: &mut W) -> Result<(), IoError> {
        let has_validity = self.has_nulls();
        sink.write_all(&(self.len as u64).to_le_bytes())?;
        sink.write_all(&[unit_tag(self.unit())])?;
        let name = self.timezone().name();
        sink.write_all(&(name.len() as u16).to_le_bytes())?;
        sink.write_all(name.as_bytes())?;
        sink.write_all(&[u8::from(has_validity)])?;
        if has_validity {
            sink.write_all(self.validity.as_ref().unwrap().as_bytes())?;
        }
        sink.write_all(self.values.as_slice())
    }

    /// This column's canonical bytes — the [`write_to`](TemporalSerie::write_to) frame as an owned
    /// `Vec`, the exact inverse of [`deserialize_bytes`](TemporalSerie::deserialize_bytes) and the
    /// codec the Python / Node bindings expose (`serialize_bytes` / `serializeBytes`).
    pub fn serialize_bytes(&self) -> Vec<u8> {
        let mut sink = Bytes::new();
        self.write_to(&mut sink)
            .expect("writing to an in-memory buffer is infallible");
        sink.as_slice().to_vec()
    }

    /// Reconstructs a column from the bytes produced by
    /// [`serialize_bytes`](TemporalSerie::serialize_bytes), erroring on a truncated or corrupt frame.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        Self::read_from(&mut Bytes::from_slice(bytes))
    }

    /// Reads a column written by [`write_to`](TemporalSerie::write_to).
    pub fn read_from<R: IOCursor>(source: &mut R) -> Result<Self, IoError> {
        let err = || IoError::Unsupported {
            what: format!("corrupt {} column frame", B::NAME),
        };
        let mut header = [0u8; 8 + 1];
        source.read_exact(&mut header)?;
        let len = u64::from_le_bytes(header[..8].try_into().unwrap()) as usize;
        let unit = unit_from_tag(header[8]).ok_or_else(err)?;

        let mut name_len = [0u8; 2];
        source.read_exact(&mut name_len)?;
        let name = source.read_exact_vec(u16::from_le_bytes(name_len) as usize)?;
        let tz = core::str::from_utf8(&name)
            .ok()
            .and_then(Tz::parse)
            .ok_or_else(err)?;

        let mut flags = [0u8; 1];
        source.read_exact(&mut flags)?;
        let has_validity = flags[0] != 0;
        let validity = if has_validity {
            let bits = source.read_exact_vec(len.div_ceil(8))?;
            Some(Bitmap::from_bytes(&bits, len))
        } else {
            None
        };

        let byte_len = len.checked_mul(B::WIDTH).ok_or(IoError::CorruptLength {
            len: len as u64,
            width: B::WIDTH,
        })?;
        let values = source.read_exact_vec(byte_len)?;
        Ok(Self {
            validity,
            values: ArrowBuffer::from_vec(values),
            len,
            field: TemporalField::new("", unit, tz, false),
            _backing: PhantomData,
        })
    }
}

/// Writes `count`'s low `width` little-endian (two's-complement) bytes into `out`.
fn write_count_le(count: i128, width: usize, out: &mut [u8]) {
    out[..width].copy_from_slice(&count.to_le_bytes()[..width]);
}

/// Sign-extends `width` little-endian bytes into an `i128` count.
fn read_count_le(bytes: &[u8], width: usize) -> i128 {
    let mut buf = [0u8; 16];
    buf[..width].copy_from_slice(&bytes[..width]);
    if width < 16 && buf[width - 1] & 0x80 != 0 {
        for byte in &mut buf[width..] {
            *byte = 0xff;
        }
    }
    i128::from_le_bytes(buf)
}

impl<B: TemporalBacking> SerieType for TemporalSerie<B> {
    type Elem = B::Native;

    fn len(&self) -> usize {
        self.len
    }

    fn null_count(&self) -> usize {
        self.null_count()
    }

    fn get(&self, index: usize) -> Option<B::Native> {
        self.get(index)
    }
}

// Structural identity: same `(unit, tz)`, length, and — at every index — the same present-or-null
// physical count. Because [`get_count`](TemporalSerie::get_count) returns `None` for a null slot,
// this compare covers null *positions* directly (independent of whether the mask is materialized)
// and never reads the placeholder bytes under a null — kept in lock-step with the byte codec, which
// zeroes those placeholders so equal columns serialize equal. It deliberately does **not** do
// cross-unit instant equality (a `ts64[s]` and a `ts64[ms]` column never compare equal).
impl<B: TemporalBacking> PartialEq for TemporalSerie<B> {
    fn eq(&self, other: &Self) -> bool {
        // Identity is over the **dtype params** (unit/tz, read from the held field) + the data —
        // never the field's name / nullable / metadata (schema intent).
        if self.unit() != other.unit()
            || self.timezone() != other.timezone()
            || self.len != other.len
        {
            return false;
        }
        (0..self.len).all(|i| self.get_count(i) == other.get_count(i))
    }
}
impl<B: TemporalBacking> Eq for TemporalSerie<B> {}
impl<B: TemporalBacking> core::hash::Hash for TemporalSerie<B> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.unit().hash(state);
        self.timezone().hash(state);
        self.len.hash(state);
        for index in 0..self.len {
            self.get_count(index).hash(state);
        }
    }
}

impl<B: TemporalBacking> Clone for TemporalSerie<B> {
    fn clone(&self) -> Self {
        Self {
            validity: self.validity.clone(),
            values: self.values.clone(), // Arc bump, not a payload copy
            len: self.len,
            field: self.field.clone(),
            _backing: PhantomData,
        }
    }
}

impl<B: TemporalBacking> core::fmt::Debug for TemporalSerie<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TemporalSerie")
            .field("type", &B::NAME)
            .field("unit", &self.unit())
            .field("tz", &self.timezone())
            .field("len", &self.len)
            .field("null_count", &self.null_count())
            .finish()
    }
}

/// Interop with the Arrow temporal arrays (feature `arrow`).
///
/// The **native-width** path (`date32`/`date64`/`time32`/`time64`/`ts64`/`duration64` — physical
/// `i32`/`i64`) shares the counts `Arc` (an `Arc` bump, bar a one-off realignment). The narrow
/// `ts32`/`duration32` **widen** their `i32` counts into a fresh `i64` `Timestamp`/`Duration`
/// array (Arrow has no 32-bit form; the narrow logical type is recovered on import via the field's
/// logical-type tag). The wide `ts96` has no native Arrow temporal type, so it maps to a
/// `FixedSizeBinary(12)` over the raw 12-byte counts (its unit/tz ride the field metadata).
#[cfg(feature = "arrow")]
impl<B: TemporalBacking> TemporalSerie<B> {
    /// This column as an Arrow [`ArrayRef`](arrow_array::ArrayRef) — zero-copy on the native path.
    /// Errors [`Unsupported`](IoError::Unsupported) with a guided message for a `unit` Arrow cannot
    /// represent (`Minute`…`Year`).
    pub fn to_arrow_array(&self) -> Result<arrow_array::ArrayRef, IoError> {
        use crate::io::DataTypeId;
        match B::TYPE_ID {
            DataTypeId::Ts32 => self.widen_to_arrow(DataTypeId::Ts64),
            DataTypeId::Duration32 => self.widen_to_arrow(DataTypeId::Duration64),
            DataTypeId::Ts96 => Ok(self.fsb_to_arrow()),
            _ => self.native_to_arrow(),
        }
    }

    /// The counts `Arc`, element-aligned — zero-copy when already aligned, else realigned once.
    fn aligned_values(&self) -> ArrowBuffer {
        if self.values.as_ptr().align_offset(B::WIDTH) == 0 {
            self.values.clone()
        } else {
            ArrowBuffer::from(self.values.as_slice())
        }
    }

    /// The validity mask as an Arrow null bitmap buffer.
    fn null_buffer(&self) -> Option<ArrowBuffer> {
        self.validity
            .as_ref()
            .map(|bitmap| ArrowBuffer::from(bitmap.as_bytes()))
    }

    /// The guided error for a resolution Arrow's temporal types cannot express.
    fn calendar_unit_error(&self) -> IoError {
        IoError::Unsupported {
            what: format!(
                "Arrow has no {} temporal type; convert to second/millisecond/microsecond/nanosecond first",
                self.unit().name()
            ),
        }
    }

    /// The zero-copy native path — the counts *are* the Arrow array's values buffer.
    fn native_to_arrow(&self) -> Result<arrow_array::ArrayRef, IoError> {
        let data_type = B::TYPE_ID
            .to_arrow_temporal(self.unit(), self.timezone())
            .ok_or_else(|| self.calendar_unit_error())?;
        let data = arrow_data::ArrayData::try_new(
            data_type,
            self.len,
            self.null_buffer(),
            0,
            vec![self.aligned_values()],
            vec![],
        )
        .expect("a temporal column's Arc buffer is valid for its Arrow type");
        Ok(arrow_array::make_array(data))
    }

    /// The widen path — sign-extend the `i32` counts into a fresh `i64` buffer and emit `target_id`'s
    /// `Timestamp` / `Duration` array.
    fn widen_to_arrow(
        &self,
        target_id: crate::io::DataTypeId,
    ) -> Result<arrow_array::ArrayRef, IoError> {
        let data_type = target_id
            .to_arrow_temporal(self.unit(), self.timezone())
            .ok_or_else(|| self.calendar_unit_error())?;
        let src = self.values.as_slice();
        let mut wide = Vec::<i64>::with_capacity(self.len);
        for index in 0..self.len {
            let start = index * 4;
            let mut bytes = [0u8; 4];
            bytes.copy_from_slice(&src[start..start + 4]);
            wide.push(i32::from_le_bytes(bytes) as i64);
        }
        let data = arrow_data::ArrayData::try_new(
            data_type,
            self.len,
            self.null_buffer(),
            0,
            vec![ArrowBuffer::from_vec(wide)],
            vec![],
        )
        .expect("a widened temporal buffer is valid for its Arrow type");
        Ok(arrow_array::make_array(data))
    }

    /// The `ts96` path — the raw 12-byte counts as a `FixedSizeBinary(12)` array.
    ///
    /// DESIGN: Arrow has no 96-bit temporal type, so this is **lossy** at the type level — the
    /// unit/tz are dropped from the array and ride the field metadata (recovered on import via the
    /// `unit` / `timezone` keys). The 12-byte counts still share their `Arc` (byte data needs no
    /// element alignment).
    fn fsb_to_arrow(&self) -> arrow_array::ArrayRef {
        let data = arrow_data::ArrayData::try_new(
            arrow_schema::DataType::FixedSizeBinary(B::WIDTH as i32),
            self.len,
            self.null_buffer(),
            0,
            vec![self.values.clone()],
            vec![],
        )
        .expect("a ts96 FixedSizeBinary(12) buffer is valid");
        arrow_array::make_array(data)
    }

    /// Builds a column from an Arrow temporal array + its [`Field`](arrow_schema::Field) — the
    /// `(unit, tz)` come from the Arrow type (or the field's `unit`/`timezone` metadata for the
    /// `ts96` `FixedSizeBinary` form), and the physical counts are read back offset-aware with the
    /// bytes under null slots zeroed (so equal columns serialize equal). Narrows an `i64` count to
    /// `i32` for `ts32`/`duration32`, erroring [`OutOfRange`](TemporalError::OutOfRange) via
    /// [`Unsupported`](IoError::Unsupported) if a foreign value exceeds `i32`.
    pub fn from_arrow_array(
        array: &dyn arrow_array::Array,
        field: &arrow_schema::Field,
    ) -> Result<Self, IoError> {
        use crate::io::DataTypeId;

        let len = array.len();
        let (unit, tz) = DataTypeId::arrow_temporal_params(array.data_type())
            .unwrap_or_else(|| temporal_params_from_metadata::<B>(field));
        let dt = TemporalType::<B>::new(unit, tz);
        let (unit, tz) = (dt.unit(), dt.timezone());

        let data = array.to_data();
        let arrow_width = arrow_element_width(array.data_type());
        if arrow_width == 0 || data.buffers().is_empty() {
            return Err(IoError::Unsupported {
                what: format!(
                    "cannot import Arrow type {:?} as a {} column",
                    array.data_type(),
                    B::NAME
                ),
            });
        }
        let src = data.buffers()[0].as_slice();
        let base = data.offset() * arrow_width;

        // The validity mask is byte-identical to Arrow's null bitmap in either path.
        let validity = array.nulls().map(|_| {
            let mut bits = vec![0u8; len.div_ceil(8)];
            for index in 0..len {
                if array.is_valid(index) {
                    bits[index / 8] |= 1 << (index % 8);
                }
            }
            Bitmap::from_bytes(&bits, len)
        });

        // Reads and range-checks the present count at `index`, returning the guided error a
        // foreign value that does not fit `B` produces (shared by both paths).
        let validate = |index: usize| -> Result<(), IoError> {
            let start = base + index * arrow_width;
            let count = read_count_le(&src[start..start + arrow_width], arrow_width);
            B::Native::from_count(count, unit, tz).map_err(|error| IoError::Unsupported {
                what: format!(
                    "Arrow value at index {index} does not fit a {} column: {error}",
                    B::NAME
                ),
            })?;
            Ok(())
        };

        // FAST PATH (zero-copy, mirrors `DecimalSerie::from_arrow_array`): a **native-width** array
        // (not the widened `ts32` / `duration32`, whose `arrow_width` 8 ≠ `B::WIDTH` 4) with no
        // slice offset, an exactly-sized buffer, and canonical (zeroed) bytes under its nulls —
        // which every yggdryl-produced array is. The source bytes *are* already the column's counts,
        // so the values buffer is shared as an `Arc` bump (no payload copy). Validation is **not**
        // dropped: each present count is still range-checked (a non-fitting foreign value is the
        // same guided error), and on this path a value that validates round-trips byte-for-byte, so
        // sharing stays byte-canonical (equal columns serialize equal).
        let has_garbage = array.nulls().is_some()
            && (0..len).any(|index| {
                array.is_null(index)
                    && src[base + index * arrow_width..base + (index + 1) * arrow_width]
                        .iter()
                        .any(|&byte| byte != 0)
            });
        if arrow_width == B::WIDTH
            && data.offset() == 0
            && src.len() == len * B::WIDTH
            && !has_garbage
        {
            for index in 0..len {
                if !array.is_null(index) {
                    validate(index)?;
                }
            }
            return Ok(Self {
                validity,
                values: data.buffers()[0].clone(), // dense, canonical -> share the Arc
                len,
                field: TemporalField::new("", unit, tz, false),
                _backing: PhantomData,
            });
        }

        // SLOW PATH: the widened `ts32` / `duration32` (narrow an `i64` count to `i32`), any sliced
        // / offset array, or an array carrying garbage under a null — copy the logical window,
        // reconstruct each present cell (which range-checks + narrows), and zero the null slots.
        let mut values = vec![0u8; len * B::WIDTH];
        for index in 0..len {
            if array.is_null(index) {
                continue; // leave the canonical zero placeholder
            }
            let start = base + index * arrow_width;
            let count = read_count_le(&src[start..start + arrow_width], arrow_width);
            // Rebuild through the value type: this range-checks (and narrows) the count for `B`.
            let native =
                B::Native::from_count(count, unit, tz).map_err(|error| IoError::Unsupported {
                    what: format!(
                        "Arrow value at index {index} does not fit a {} column: {error}",
                        B::NAME
                    ),
                })?;
            native.write_le(&mut values[index * B::WIDTH..]);
        }

        Ok(Self {
            validity,
            values: ArrowBuffer::from_vec(values),
            len,
            field: TemporalField::new("", unit, tz, false),
            _backing: PhantomData,
        })
    }
}

/// The `(unit, tz)` a temporal field records under the reserved metadata keys — the recovery path
/// for the `ts96` `FixedSizeBinary` form, whose Arrow type carries neither.
#[cfg(feature = "arrow")]
fn temporal_params_from_metadata<B: TemporalBacking>(
    field: &arrow_schema::Field,
) -> (TimeUnit, Tz) {
    use crate::io::DataTypeId;
    let unit = field
        .metadata()
        .get(DataTypeId::TIME_UNIT_METADATA_KEY)
        .and_then(|value| TimeUnit::parse(value))
        .unwrap_or(B::DEFAULT_UNIT);
    let tz = field
        .metadata()
        .get(DataTypeId::TIMEZONE_METADATA_KEY)
        .and_then(|value| Tz::parse(value))
        .unwrap_or(Tz::NAIVE);
    (unit, tz)
}

/// The physical byte width of one element of an Arrow temporal data type (`0` if not temporal).
#[cfg(feature = "arrow")]
fn arrow_element_width(data_type: &arrow_schema::DataType) -> usize {
    use arrow_schema::DataType as A;
    match data_type {
        A::Date32 | A::Time32(_) => 4,
        A::Date64 | A::Time64(_) | A::Timestamp(_, _) | A::Duration(_) => 8,
        A::FixedSizeBinary(n) => (*n).max(0) as usize,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::super::Ts64;
    use super::*;
    use crate::io::fixed::Ts64Serie;

    #[cfg(feature = "arrow")]
    use super::super::{Duration32, Ts32, Ts96};
    #[cfg(feature = "arrow")]
    use crate::io::fixed::{Duration32Serie, Ts32Serie, Ts96Serie};

    #[test]
    fn equality_ignores_a_materialized_all_present_mask() {
        let a = Ts64::from_epoch(10, TimeUnit::Second, Tz::UTC).unwrap();
        let b = Ts64::from_epoch(20, TimeUnit::Second, Tz::UTC).unwrap();
        let mut cleared =
            Ts64Serie::from_options(TimeUnit::Second, Tz::UTC, &[Some(a), None]).unwrap();
        cleared.set(1, Some(b)).unwrap();
        assert_eq!(cleared.null_count(), 0);

        let dense = Ts64Serie::from_values(TimeUnit::Second, Tz::UTC, &[a, b]).unwrap();
        assert_eq!(cleared, dense);
        assert_eq!(
            Ts64Serie::deserialize_bytes(&cleared.serialize_bytes()).unwrap(),
            cleared
        );

        let with_null =
            Ts64Serie::from_options(TimeUnit::Second, Tz::UTC, &[Some(a), None]).unwrap();
        assert_ne!(with_null, dense);
    }

    #[test]
    fn from_scalars_round_trips_a_column_through_its_own_scalars() {
        let a = Ts64::from_epoch(1_000, TimeUnit::Second, Tz::UTC).unwrap();
        let b = Ts64::from_epoch(2_000, TimeUnit::Second, Tz::UTC).unwrap();
        let col =
            Ts64Serie::from_options(TimeUnit::Second, Tz::UTC, &[Some(a), None, Some(b)]).unwrap();
        let scalars: Vec<_> = (0..col.len()).map(|i| col.get_scalar(i)).collect();
        assert_eq!(
            Ts64Serie::from_scalars(TimeUnit::Second, Tz::UTC, &scalars).unwrap(),
            col
        );

        // The empty slice yields the empty column at the given (unit, tz).
        assert_eq!(
            Ts64Serie::from_scalars(TimeUnit::Second, Tz::UTC, &[]).unwrap(),
            Ts64Serie::new(TimeUnit::Second, Tz::UTC)
        );
    }

    #[test]
    fn no_cross_unit_equality() {
        let secs = Ts64Serie::from_values(
            TimeUnit::Second,
            Tz::UTC,
            &[Ts64::from_epoch(1, TimeUnit::Second, Tz::UTC).unwrap()],
        )
        .unwrap();
        let millis = Ts64Serie::from_values(
            TimeUnit::Millisecond,
            Tz::UTC,
            &[Ts64::from_epoch(1_000, TimeUnit::Millisecond, Tz::UTC).unwrap()],
        )
        .unwrap();
        // Same instant, different unit -> distinct columns.
        assert_ne!(secs, millis);
    }

    #[cfg(feature = "arrow")]
    #[test]
    fn native_round_trip_through_arrow() {
        let a = Ts64::from_epoch(1_700_000_000, TimeUnit::Second, Tz::UTC).unwrap();
        let col = Ts64Serie::from_options(TimeUnit::Second, Tz::UTC, &[Some(a), None]).unwrap();
        let array = col.to_arrow_array().unwrap();
        let field = col.to_field("t").to_arrow();
        let back = Ts64Serie::from_arrow_array(array.as_ref(), &field).unwrap();
        assert_eq!(back, col);
    }

    #[cfg(feature = "arrow")]
    #[test]
    fn ts32_widens_and_narrows_back() {
        let a = Ts32::from_epoch(1_000, TimeUnit::Second, Tz::UTC).unwrap();
        let col = Ts32Serie::from_options(TimeUnit::Second, Tz::UTC, &[Some(a), None]).unwrap();
        let array = col.to_arrow_array().unwrap();
        // Widened to a real i64 Timestamp.
        assert!(matches!(
            array.data_type(),
            arrow_schema::DataType::Timestamp(_, _)
        ));
        let field = col.to_field("t").to_arrow();
        let back = Ts32Serie::from_arrow_array(array.as_ref(), &field).unwrap();
        assert_eq!(back, col);
    }

    #[cfg(feature = "arrow")]
    #[test]
    fn duration32_widens_and_narrows_back() {
        let a = Duration32::milliseconds(1_500);
        let col = Duration32Serie::from_values(TimeUnit::Millisecond, Tz::NAIVE, &[a]).unwrap();
        let array = col.to_arrow_array().unwrap();
        assert!(matches!(
            array.data_type(),
            arrow_schema::DataType::Duration(_)
        ));
        let field = col.to_field("d").to_arrow();
        let back = Duration32Serie::from_arrow_array(array.as_ref(), &field).unwrap();
        assert_eq!(back, col);
    }

    #[cfg(feature = "arrow")]
    #[test]
    fn ts96_round_trips_through_fixed_size_binary() {
        let a = Ts96::from_epoch(1_700_000_000_000_000_000, TimeUnit::Nanosecond, Tz::UTC).unwrap();
        let col = Ts96Serie::from_options(TimeUnit::Nanosecond, Tz::UTC, &[Some(a), None]).unwrap();
        let array = col.to_arrow_array().unwrap();
        assert!(matches!(
            array.data_type(),
            arrow_schema::DataType::FixedSizeBinary(12)
        ));
        let field = col.to_field("t").to_arrow();
        let back = Ts96Serie::from_arrow_array(array.as_ref(), &field).unwrap();
        assert_eq!(back, col);
    }

    #[cfg(feature = "arrow")]
    #[test]
    fn calendar_unit_errs_on_arrow_export() {
        let a = Ts64::from_epoch(5, TimeUnit::Minute, Tz::UTC).unwrap();
        let col = Ts64Serie::from_values(TimeUnit::Minute, Tz::UTC, &[a]).unwrap();
        let err = col.to_arrow_array().unwrap_err();
        assert!(matches!(err, IoError::Unsupported { .. }));
    }
}
