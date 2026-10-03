//! Every column whose rows are one fixed-width value in a values buffer.
//!
//! Arrow lays all of them out the same way - a `values` buffer of the
//! native width, and a validity bitmap where the column admits absence - so
//! they are one generic type, [`PrimitiveSerie<T>`], and each leaf is a name
//! for one instantiation of it: [`Int32Serie`] is `PrimitiveSerie<Int32Type>`
//! and lends `&[i32]`, [`Decimal128Serie`] lends `&[i128]`,
//! [`DateTimeNanosecondSerie`] lends `&[i64]`. Nothing is boxed and nothing
//! is copied to read one.
//!
//! A typed writer exists only where the native domain is the datatype's
//! whole domain - [`NativeLeaf`] - and it validates what remains, which is
//! nullability. A decimal, a `Date64`, a `Time32` and a `Time64` leaf have
//! none: precision, whole-day and time-of-day ranges are narrower than the
//! storage, so a [`Scalar`] through the field's contract is their one
//! writer.

use std::cmp::Ordering;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use arrow_array::builder::PrimitiveBuilder;
use arrow_array::types::{
    Date32Type, Date64Type, Decimal32Type, Decimal64Type, Decimal128Type, Decimal256Type,
    DurationMicrosecondType, DurationMillisecondType, DurationNanosecondType, DurationSecondType,
    Float16Type, Float32Type, Float64Type, Int8Type, Int16Type, Int32Type, Int64Type,
    IntervalDayTimeType, IntervalMonthDayNanoType, IntervalYearMonthType, Time32MillisecondType,
    Time32SecondType, Time64MicrosecondType, Time64NanosecondType, TimestampMicrosecondType,
    TimestampMillisecondType, TimestampNanosecondType, TimestampSecondType, UInt8Type, UInt16Type,
    UInt32Type, UInt64Type,
};
use arrow_array::{Array, ArrayRef, ArrowNativeTypeOp, ArrowPrimitiveType, PrimitiveArray};
use arrow_buffer::{
    IntervalDayTime, IntervalMonthDayNano, NullBuffer, ScalarBuffer, bit_util, i256,
};
use arrow_schema::DataType as ArrowDataType;

use super::{Serie, require_range, require_row, require_window};
use crate::serie::value::Reading;
use crate::spill::Backing;
use crate::value::SerieValue;
use crate::{Field, Result, Scalar, SortOptions};

/// The invariant a canonical row carries into a write: it lays out as the
/// field's own array, because the field's contract already rewrote it.
const LAID_OUT: &str = "a canonical row lays out as its field's array: the contract rewrote it";

/// Which leaf of the [`Serie`] root one primitive width widens to.
///
/// Arrow's type parameter is what makes a values buffer typed, and this is
/// the one place that parameter is tied to the crate's own leaf, so
/// [`PrimitiveSerie`] stays one implementation over every width.
pub trait PrimitiveLeaf: ArrowPrimitiveType + Sized + Send + Sync + 'static {
    /// The leaf's name, as its debug rendering spells it.
    const NAME: &'static str;

    /// Widen a column of this width to the serie root.
    fn into_serie(column: PrimitiveSerie<Self>) -> Serie;

    /// Narrow a serie root to a column of this width.
    fn from_serie(serie: &Serie) -> Option<&PrimitiveSerie<Self>>;
}

/// A primitive leaf whose native domain is its datatype's whole domain, so a
/// native value needs no contract beyond nullability and a typed writer can
/// take one.
pub trait NativeLeaf: PrimitiveLeaf {}

/// How two natives of one width order as the values they hold: Arrow's own
/// order over the natives, except that every NaN of a float is the one
/// value a [`Scalar`] reads it as - equal to every other NaN and above every
/// number - so the typed rung sorts as the values do.
pub(crate) trait NativeOrder: Copy {
    /// `self` against `other`, ascending.
    fn order(self, other: Self) -> Ordering;
}

/// The natives whose every bit pattern is one value: Arrow's order is theirs.
macro_rules! native_order {
    ($($native:ty),+ $(,)?) => {$(
        impl NativeOrder for $native {
            fn order(self, other: Self) -> Ordering {
                self.compare(other)
            }
        }
    )+};
}

native_order!(
    i8,
    i16,
    i32,
    i64,
    i128,
    i256,
    u8,
    u16,
    u32,
    u64,
    IntervalDayTime,
    IntervalMonthDayNano,
);

/// The floats: Arrow's total order over the bits, every NaN one value.
macro_rules! float_order {
    ($($native:ty),+ $(,)?) => {$(
        impl NativeOrder for $native {
            fn order(self, other: Self) -> Ordering {
                match (self.is_nan(), other.is_nan()) {
                    (true, true) => Ordering::Equal,
                    (true, false) => Ordering::Greater,
                    (false, true) => Ordering::Less,
                    (false, false) => self.compare(other),
                }
            }
        }
    )+};
}

float_order!(half::f16, f32, f64);

/// One column of fixed-width rows: the field that types them, and the Arrow
/// buffers that hold them.
///
/// The values buffer and the validity bitmap are Arrow's own, held apart
/// and shared rather than copied, so a clone costs pointer bumps,
/// [`values`](Self::values) lends the native slice straight out of the
/// buffer, and a row read builds no array.
pub struct PrimitiveSerie<T: PrimitiveLeaf> {
    field: Arc<Field>,
    values: ScalarBuffer<T::Native>,
    nulls: Option<NullBuffer>,
    /// The storage type the landing matched, kept beside `reading` for the
    /// same reason: resolved once where the column landed. A rebuild is
    /// `PrimitiveArray::new(values.clone(), nulls.clone()).with_data_type(dtype.clone())`:
    /// pointer bumps, no allocation.
    dtype: ArrowDataType,
    /// How a slot reads as the field's value, resolved from the field once
    /// where the column landed.
    reading: Reading<T::Native>,
    /// Where `values` and `nulls` live; a write puts them back on the heap.
    backing: Backing,
}

impl<T: PrimitiveLeaf> PrimitiveSerie<T> {
    /// Pair a field with the buffers that hold its rows and the reading its
    /// datatype resolved to, the array taken apart into them.
    pub(crate) fn new(
        field: Arc<Field>,
        values: PrimitiveArray<T>,
        reading: Reading<T::Native>,
    ) -> Self {
        let (dtype, values, nulls) = values.into_parts();
        Self {
            field,
            values,
            nulls,
            dtype,
            reading,
            backing: Backing::Heap,
        }
    }

    /// Borrow the values buffer, without copying it.
    ///
    /// Every row has a slot, a null row included; what a null row's slot
    /// holds is not a value, and [`Self::nulls`] is what says which slots
    /// those are.
    pub fn values(&self) -> &[T::Native] {
        &self.values
    }

    /// Read row `index` off the values buffer: `None` when the row is
    /// absent or past the end.
    ///
    /// One bounds check and one buffer read; no value is built.
    pub fn value(&self, index: usize) -> Option<T::Native> {
        (index < self.values.len() && self.is_valid(index)).then(|| self.values[index])
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub fn nulls(&self) -> Option<&NullBuffer> {
        self.nulls.as_ref()
    }

    /// The Arrow array these buffers are, rebuilt around them: pointer
    /// bumps, no allocation and no copy.
    pub fn array(&self) -> PrimitiveArray<T> {
        PrimitiveArray::new(self.values.clone(), self.nulls.clone())
            .with_data_type(self.dtype.clone())
    }

    /// Where the buffers live.
    pub(crate) const fn backing(&self) -> &Backing {
        &self.backing
    }

    /// State where the buffers live: a spill names the mapping it laid them
    /// in, a write the heap.
    pub(crate) fn set_backing(&mut self, backing: Backing) {
        self.backing = backing;
    }

    /// Whether row `index`, inside the column, holds a value: read off the
    /// validity bitmap.
    fn is_valid(&self, index: usize) -> bool {
        self.nulls
            .as_ref()
            .is_none_or(|nulls| nulls.is_valid(index))
    }

    /// Take this column's rows as a builder to write into.
    ///
    /// The buffers move out of the column into one temporary array, so it
    /// is their one holder: Arrow hands them back as a builder when nothing
    /// else holds them and their pointer was never advanced, which is what
    /// makes an append or a slot write in place; a foreign, shared, sliced
    /// or mapped buffer is copied once, and every later edit is in place.
    /// The builder carries the field's Arrow datatype, because
    /// `into_builder` erases it to the width's own and a `decimal(10,2)`
    /// would come back `Decimal128(38,10)`.
    fn take_builder(&mut self) -> PrimitiveBuilder<T> {
        let dtype = self.dtype.clone();
        let taken = PrimitiveArray::<T>::new(std::mem::take(&mut self.values), self.nulls.take());
        match taken.into_builder() {
            Ok(builder) => builder.with_data_type(dtype),
            // Arrow hands a buffer it would not release back under the
            // width's own datatype, so the copy is restated under the
            // column's before it is appended.
            Err(shared) => {
                let mut builder = PrimitiveBuilder::<T>::with_capacity(shared.len() + 1)
                    .with_data_type(dtype.clone());
                builder.append_array(&shared.with_data_type(dtype));
                builder
            }
        }
    }

    /// Hold the buffers a write finished as this column's rows, on the heap
    /// whatever the buffers they replace lay in.
    fn put(&mut self, written: PrimitiveArray<T>) {
        let (_, values, nulls) = written.into_parts();
        self.values = values;
        self.nulls = nulls;
        self.set_backing(Backing::Heap);
    }

    /// Replace rows `range` by `replacement`, which lays out as this column.
    ///
    /// An append writes into the buffer the column holds; one present slot
    /// over one present slot is one buffer write; anything else is one
    /// rebuild through the builder - prefix, replacement, suffix, one pass.
    fn write_array(&mut self, range: Range<usize>, replacement: PrimitiveArray<T>) {
        let len = self.values.len();
        if range.start == len {
            let mut builder = self.take_builder();
            builder.append_array(&replacement);
            self.put(builder.finish());
            return;
        }
        if range.len() == 1
            && replacement.len() == 1
            && replacement.is_valid(0)
            && self.is_valid(range.start)
        {
            let mut builder = self.take_builder();
            builder.values_slice_mut()[range.start] = replacement.value(0);
            self.put(builder.finish());
            return;
        }
        let held = self.array();
        let prefix = held.slice(0, range.start);
        let suffix = held.slice(range.end, len - range.end);
        let mut builder =
            PrimitiveBuilder::<T>::with_capacity(prefix.len() + replacement.len() + suffix.len())
                .with_data_type(self.dtype.clone());
        builder.append_array(&prefix);
        builder.append_array(&replacement);
        builder.append_array(&suffix);
        self.put(builder.finish());
    }

    /// Lay canonical `rows` out as this column's array, once.
    ///
    /// The crate's one scalar-array boundary, so a column never grows a
    /// second writer; the rows went through the field's contract, so the
    /// layout cannot refuse them.
    fn laid_out(&self, rows: &[Scalar]) -> PrimitiveArray<T> {
        let borrowed: Vec<&Scalar> = rows.iter().collect();
        let array = crate::serie::value::array_of_rows(&self.field, &borrowed).expect(LAID_OUT);
        array
            .as_any()
            .downcast_ref::<PrimitiveArray<T>>()
            .cloned()
            .expect(LAID_OUT)
    }

    /// Refuse what a write could not do: nothing, for a fixed width.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        let _ = (range, rows);
        Ok(())
    }

    /// Write canonical `rows` over a checked `range`.
    pub(crate) fn write(&mut self, range: Range<usize>, rows: Vec<Scalar>) {
        let replacement = self.laid_out(&rows);
        self.write_array(range, replacement);
    }

    /// Append `other`'s buffers, whose field agrees with this one's; a
    /// values buffer reaches any total.
    pub(crate) fn append(&mut self, other: &Self) -> bool {
        let mut builder = self.take_builder();
        builder.append_array(&other.array());
        self.put(builder.finish());
        true
    }

    /// The row positions of `range` in sorted order, stable, under
    /// `options`: a typed sort over the values buffer in the values' own
    /// order, the absent rows gathered to the end `options` names.
    ///
    /// One allocation, the answer; no value is built.
    pub(crate) fn sort_indices(&self, range: Range<usize>, options: SortOptions) -> Vec<u32>
    where
        T::Native: NativeOrder,
    {
        let values: &[T::Native] = &self.values;
        let present = |index: usize| self.is_valid(index);
        let absent = SerieValue::null_count(self);
        let mut order: Vec<u32> = Vec::with_capacity(range.len());
        if options.is_nulls_first() {
            order.extend(
                range
                    .clone()
                    .filter(|index| !present(*index))
                    .map(|index| index as u32),
            );
        }
        let first_present = order.len();
        order.extend(
            range
                .clone()
                .filter(|index| present(*index))
                .map(|index| index as u32),
        );
        // Stable: equal rows keep their order, so the sort is one every leaf
        // answers alike; its scratch is the one allocation beside the answer.
        order[first_present..].sort_by(|left, right| {
            let step = values[*left as usize].order(values[*right as usize]);
            if options.is_descending() {
                step.reverse()
            } else {
                step
            }
        });
        if !options.is_nulls_first() && absent > 0 {
            order.extend(
                range
                    .filter(|index| !present(*index))
                    .map(|index| index as u32),
            );
        }
        order
    }

    /// Sort rows `range` in place under `options`: the native slice sorted
    /// where it stands in the values' own order, the absent rows gathered to
    /// the end `options` names, the validity bits of the range rewritten as
    /// one run.
    ///
    /// No row is built and nothing is copied when the column holds its
    /// buffers alone: the validity is read off the builder's own bits, so no
    /// second holder makes Arrow copy them. A shared, foreign or mapped
    /// buffer is copied once by the builder, as every write copies it.
    pub(crate) fn sort_in_place(&mut self, range: Range<usize>, options: SortOptions)
    where
        T::Native: NativeOrder,
    {
        let mut builder = self.take_builder();
        let (values, validity) = builder.slices_mut();
        let window = &mut values[range.clone()];
        let len = window.len();
        let present = match validity.as_deref() {
            Some(bits) => {
                // Compact the present values to the front, in order, so the
                // sort runs over them alone.
                let mut write = 0;
                for read in 0..len {
                    if bit_util::get_bit(bits, range.start + read) {
                        window[write] = window[read];
                        write += 1;
                    }
                }
                write
            }
            None => len,
        };
        // Unstable, because two natives that order equal are one value -
        // two NaN payloads included, which every reading answers as one NaN
        // - and the unstable sort allocates no scratch.
        let sorted = &mut window[..present];
        if options.is_descending() {
            sorted.sort_unstable_by(|left, right| right.order(*left));
        } else {
            sorted.sort_unstable_by(|left, right| left.order(*right));
        }
        let absent = len - present;
        if absent > 0 {
            if options.is_nulls_first() {
                window.rotate_right(absent);
            }
            if let Some(bits) = validity {
                let (first_absent, last_absent) = if options.is_nulls_first() {
                    (range.start, range.start + absent)
                } else {
                    (range.start + present, range.end)
                };
                for index in range.clone() {
                    if (first_absent..last_absent).contains(&index) {
                        bit_util::unset_bit(bits, index);
                    } else {
                        bit_util::set_bit(bits, index);
                    }
                }
            }
        }
        self.put(builder.finish());
    }

    /// Reverse rows `range` in place: the native slice reversed where it
    /// stands, the validity bits of the range reversed with it.
    pub(crate) fn reverse_in_place(&mut self, range: Range<usize>) {
        let mut builder = self.take_builder();
        let (values, validity) = builder.slices_mut();
        values[range.clone()].reverse();
        if let Some(bits) = validity {
            let (mut low, mut high) = (range.start, range.end);
            while low + 1 < high {
                high -= 1;
                let (left, right) = (bit_util::get_bit(bits, low), bit_util::get_bit(bits, high));
                if left != right {
                    if right {
                        bit_util::set_bit(bits, low);
                        bit_util::unset_bit(bits, high);
                    } else {
                        bit_util::unset_bit(bits, low);
                        bit_util::set_bit(bits, high);
                    }
                }
                low += 1;
            }
        }
        self.put(builder.finish());
    }
}

impl<T: NativeLeaf> PrimitiveSerie<T> {
    /// Refuse an absent native row under a field that admits none, naming
    /// the column.
    fn require_present(&self, values: &[Option<T::Native>]) -> Result<()> {
        if self.field.is_nullable() {
            return Ok(());
        }
        match values.iter().position(Option::is_none) {
            None => Ok(()),
            Some(at) => Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(self.field.name()),
                reason: smol_str::format_smolstr!(
                    "{} is not null, got an absent value at {at} of the {} written",
                    self.field.name(),
                    values.len()
                ),
            }),
        }
    }

    /// Append one native row, without building a value for it.
    ///
    /// # Errors
    ///
    /// Returns an error naming the column when the row is absent and the
    /// field admits no absent row.
    pub fn push_value(&mut self, value: Option<T::Native>) -> Result<()> {
        let len = self.values.len();
        self.splice_values(len..len, vec![value])
    }

    /// Overwrite row `index` with one native value.
    ///
    /// A present row overwritten by a present value is one buffer write; a
    /// row that gains or loses its absence rebuilds the validity bitmap.
    ///
    /// # Errors
    ///
    /// Returns an error naming the column when `index` is past the end, or
    /// when the row is absent and the field admits no absent row.
    pub fn set_value(&mut self, index: usize, value: Option<T::Native>) -> Result<()> {
        require_row(self.field.name(), index, self.values.len())?;
        self.splice_values(index..index + 1, vec![value])
    }

    /// Replace rows `range` by native `values`.
    ///
    /// # Errors
    ///
    /// Returns an error naming the column when `range` is reversed or
    /// reaches past the end, or when a value is absent and the field admits
    /// no absent row.
    pub fn splice_values(
        &mut self,
        range: Range<usize>,
        values: Vec<Option<T::Native>>,
    ) -> Result<()> {
        require_range(self.field.name(), &range, self.values.len())?;
        self.require_present(&values)?;
        // A width shared with a narrower datatype - a `duration32` count in
        // Arrow's one duration width - holds only what the field's own
        // reading accepts.
        if !self.field.dtype().layout_is_contract() {
            for value in values.iter().flatten() {
                (self.reading)(self.field.dtype(), *value)?;
            }
        }
        let mut replacement =
            PrimitiveBuilder::<T>::with_capacity(values.len()).with_data_type(self.dtype.clone());
        for value in values {
            replacement.append_option(value);
        }
        self.write_array(range, replacement.finish());
        Ok(())
    }

    /// Append native `values`.
    ///
    /// # Errors
    ///
    /// [`Self::splice_values`] carries the rule.
    pub fn extend_values(&mut self, values: Vec<Option<T::Native>>) -> Result<()> {
        let len = self.values.len();
        self.splice_values(len..len, values)
    }
}

impl<T: PrimitiveLeaf> SerieValue for PrimitiveSerie<T> {
    fn field(&self) -> &Field {
        &self.field
    }

    fn field_ref(&self) -> &Arc<Field> {
        &self.field
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    fn null_count(&self) -> usize {
        self.nulls.as_ref().map_or(0, NullBuffer::null_count)
    }

    fn is_null(&self, index: usize) -> Result<bool> {
        require_row(self.field.name(), index, self.values.len())?;
        Ok(!self.is_valid(index))
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        require_row(self.field.name(), index, self.values.len())?;
        if !self.is_valid(index) {
            return Ok(Scalar::Null);
        }
        Ok((self.reading)(self.field.dtype(), self.values[index])?)
    }

    fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        require_window(self.field.name(), offset, length, self.values.len())?;
        Ok(Self {
            field: Arc::clone(&self.field),
            values: self.values.slice(offset, length),
            nulls: self.nulls.as_ref().map(|nulls| nulls.slice(offset, length)),
            dtype: self.dtype.clone(),
            reading: self.reading,
            backing: self.backing,
        })
    }

    fn splice(&mut self, range: Range<usize>, rows: Vec<Scalar>) -> Result<()> {
        require_range(self.field.name(), &range, self.values.len())?;
        let canonical = rows
            .into_iter()
            .map(|row| self.field.scalar(row))
            .collect::<Result<Vec<Scalar>>>()?;
        self.check(&range, &canonical)?;
        self.write(range, canonical);
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        Arc::new(self.array())
    }

    fn memory_size(&self) -> usize {
        // The typed array is pointer bumps, so nothing is boxed to count it.
        crate::arrow::sliced_size(&self.array())
    }

    fn resident_size(&self) -> usize {
        if self.backing().is_mapped() {
            0
        } else {
            self.memory_size()
        }
    }

    fn into_serie(self) -> Serie {
        T::into_serie(self)
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        T::from_serie(value)
    }
}

impl<T: PrimitiveLeaf> Clone for PrimitiveSerie<T> {
    fn clone(&self) -> Self {
        Self {
            field: Arc::clone(&self.field),
            values: self.values.clone(),
            nulls: self.nulls.clone(),
            dtype: self.dtype.clone(),
            reading: self.reading,
            backing: self.backing,
        }
    }
}

impl<T: PrimitiveLeaf> fmt::Debug for PrimitiveSerie<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_column(self, T::NAME, formatter)
    }
}

serie_leaf!(PrimitiveSerie<T: PrimitiveLeaf>);

/// Build the column `field` types out of a primitive array, or answer `None`
/// for a layout that is not one.
///
/// The door proved the layout against the field's projection, so the
/// downcast cannot miss for the width the Arrow datatype names.
pub(crate) fn column_of(
    field: Arc<Field>,
    array: ArrayRef,
    parent: Option<&NullBuffer>,
    proof: &super::arrow::Proof,
    _budget: &mut crate::budget::MaterializationBudget,
    _resolved: Option<&super::arrow::Resolved>,
) -> crate::arrow::Result<Option<Serie>> {
    use arrow_schema::{IntervalUnit, TimeUnit as ArrowTimeUnit};

    use crate::serie::value::{
        duration_reading, read_date32, read_date64, read_datetime, read_day_time, read_decimal32,
        read_decimal64, read_decimal128, read_decimal256, read_enum, read_month_day_nano,
        read_native, read_time32, read_time64, read_year_month,
    };

    let _ = (parent, proof);
    // The width is the array's; the reading is the field's, resolved here
    // once so a cell read is one buffer read and one constructor.
    macro_rules! primitive {
        ($arrow:ty, $reading:expr) => {
            Ok(Some(
                PrimitiveSerie::<$arrow>::new(
                    Arc::clone(&field),
                    super::arrow::held::<PrimitiveArray<$arrow>>(&array)?,
                    $reading,
                )
                .into_serie(),
            ))
        };
    }
    match array.data_type() {
        ArrowDataType::Int8 => primitive!(Int8Type, read_native),
        ArrowDataType::Int16 => primitive!(Int16Type, read_native),
        ArrowDataType::Int32 => primitive!(Int32Type, read_native),
        ArrowDataType::Int64 => primitive!(Int64Type, read_native),
        // An enum member is the code of its leaf, read back as the member.
        ArrowDataType::UInt8 if field.dtype().is_enum() => primitive!(UInt8Type, read_enum),
        ArrowDataType::UInt8 => primitive!(UInt8Type, read_native),
        ArrowDataType::UInt16 if field.dtype().is_enum() => primitive!(UInt16Type, read_enum),
        ArrowDataType::UInt16 => primitive!(UInt16Type, read_native),
        ArrowDataType::UInt32 => primitive!(UInt32Type, read_native),
        ArrowDataType::UInt64 => primitive!(UInt64Type, read_native),
        ArrowDataType::Float16 => primitive!(Float16Type, read_native),
        ArrowDataType::Float32 => primitive!(Float32Type, read_native),
        ArrowDataType::Float64 => primitive!(Float64Type, read_native),
        ArrowDataType::Decimal32(..) => primitive!(Decimal32Type, read_decimal32),
        ArrowDataType::Decimal64(..) => primitive!(Decimal64Type, read_decimal64),
        ArrowDataType::Decimal128(..) => primitive!(Decimal128Type, read_decimal128),
        ArrowDataType::Decimal256(..) => primitive!(Decimal256Type, read_decimal256),
        ArrowDataType::Date32 => primitive!(Date32Type, read_date32),
        ArrowDataType::Date64 => primitive!(Date64Type, read_date64),
        ArrowDataType::Time32(ArrowTimeUnit::Second) => primitive!(Time32SecondType, read_time32),
        ArrowDataType::Time32(_) => primitive!(Time32MillisecondType, read_time32),
        ArrowDataType::Time64(ArrowTimeUnit::Microsecond) => {
            primitive!(Time64MicrosecondType, read_time64)
        }
        ArrowDataType::Time64(_) => primitive!(Time64NanosecondType, read_time64),
        ArrowDataType::Timestamp(ArrowTimeUnit::Second, _) => {
            primitive!(TimestampSecondType, read_datetime)
        }
        ArrowDataType::Timestamp(ArrowTimeUnit::Millisecond, _) => {
            primitive!(TimestampMillisecondType, read_datetime)
        }
        ArrowDataType::Timestamp(ArrowTimeUnit::Microsecond, _) => {
            primitive!(TimestampMicrosecondType, read_datetime)
        }
        ArrowDataType::Timestamp(ArrowTimeUnit::Nanosecond, _) => {
            primitive!(TimestampNanosecondType, read_datetime)
        }
        ArrowDataType::Duration(ArrowTimeUnit::Second) => {
            primitive!(DurationSecondType, duration_reading(field.dtype())?)
        }
        ArrowDataType::Duration(ArrowTimeUnit::Millisecond) => {
            primitive!(DurationMillisecondType, duration_reading(field.dtype())?)
        }
        ArrowDataType::Duration(ArrowTimeUnit::Microsecond) => {
            primitive!(DurationMicrosecondType, duration_reading(field.dtype())?)
        }
        ArrowDataType::Duration(ArrowTimeUnit::Nanosecond) => {
            primitive!(DurationNanosecondType, duration_reading(field.dtype())?)
        }
        ArrowDataType::Interval(IntervalUnit::YearMonth) => {
            primitive!(IntervalYearMonthType, read_year_month)
        }
        ArrowDataType::Interval(IntervalUnit::DayTime) => {
            primitive!(IntervalDayTimeType, read_day_time)
        }
        ArrowDataType::Interval(IntervalUnit::MonthDayNano) => {
            primitive!(IntervalMonthDayNanoType, read_month_day_nano)
        }
        _ => Ok(None),
    }
}

/// Name one primitive width as a leaf of the root, and tie Arrow's type
/// parameter to the root variant it widens to.
macro_rules! primitive_leaf {
    ($(#[$meta:meta])* $name:ident, $arrow:ty) => {
        $(#[$meta])*
        pub type $name = PrimitiveSerie<$arrow>;

        impl PrimitiveLeaf for $arrow {
            const NAME: &'static str = stringify!($name);

            fn into_serie(column: PrimitiveSerie<Self>) -> Serie {
                super::Leaf::root(column)
            }

            fn from_serie(serie: &Serie) -> Option<&PrimitiveSerie<Self>> {
                super::Leaf::narrow_laid(serie)
            }
        }
    };
    // A width whose native domain is the datatype's whole domain.
    ($(#[$meta:meta])* $name:ident, $arrow:ty, native) => {
        primitive_leaf!($(#[$meta])* $name, $arrow);

        impl NativeLeaf for $arrow {}
    };
}

primitive_leaf!(
    /// A column of signed 8-bit integers.
    Int8Serie, Int8Type, native
);
primitive_leaf!(
    /// A column of signed 16-bit integers.
    Int16Serie, Int16Type, native
);
primitive_leaf!(
    /// A column of signed 32-bit integers.
    Int32Serie, Int32Type, native
);
primitive_leaf!(
    /// A column of signed 64-bit integers.
    Int64Serie, Int64Type, native
);
primitive_leaf!(
    /// A column of unsigned 8-bit integers.
    UInt8Serie, UInt8Type, native
);
primitive_leaf!(
    /// A column of unsigned 16-bit integers.
    UInt16Serie, UInt16Type, native
);
primitive_leaf!(
    /// A column of unsigned 32-bit integers.
    UInt32Serie, UInt32Type, native
);
primitive_leaf!(
    /// A column of unsigned 64-bit integers.
    UInt64Serie, UInt64Type, native
);

primitive_leaf!(
    /// A column of IEEE binary16 floats.
    Float16Serie, Float16Type, native
);
primitive_leaf!(
    /// A column of IEEE binary32 floats.
    Float32Serie, Float32Type, native
);
primitive_leaf!(
    /// A column of IEEE binary64 floats.
    Float64Serie, Float64Type, native
);

primitive_leaf!(
    /// A column of 32-bit coefficient-and-scale decimals.
    Decimal32Serie, Decimal32Type
);
primitive_leaf!(
    /// A column of 64-bit coefficient-and-scale decimals.
    Decimal64Serie, Decimal64Type
);
primitive_leaf!(
    /// A column of 128-bit coefficient-and-scale decimals.
    Decimal128Serie, Decimal128Type
);
primitive_leaf!(
    /// A column of 256-bit coefficient-and-scale decimals.
    Decimal256Serie, Decimal256Type
);

primitive_leaf!(
    /// A column of 32-bit day-count dates.
    Date32Serie, Date32Type, native
);
primitive_leaf!(
    /// A column of 64-bit millisecond-count dates.
    Date64Serie, Date64Type
);
primitive_leaf!(
    /// A column of second-count times of day.
    Time32SecondSerie, Time32SecondType
);
primitive_leaf!(
    /// A column of millisecond-count times of day.
    Time32MillisecondSerie, Time32MillisecondType
);
primitive_leaf!(
    /// A column of microsecond-count times of day.
    Time64MicrosecondSerie, Time64MicrosecondType
);
primitive_leaf!(
    /// A column of nanosecond-count times of day.
    Time64NanosecondSerie, Time64NanosecondType
);
primitive_leaf!(
    /// A column of second-count datetimes.
    DateTimeSecondSerie, TimestampSecondType, native
);
primitive_leaf!(
    /// A column of millisecond-count datetimes.
    DateTimeMillisecondSerie, TimestampMillisecondType, native
);
primitive_leaf!(
    /// A column of microsecond-count datetimes.
    DateTimeMicrosecondSerie, TimestampMicrosecondType, native
);
primitive_leaf!(
    /// A column of nanosecond-count datetimes.
    DateTimeNanosecondSerie, TimestampNanosecondType, native
);
primitive_leaf!(
    /// A column of second-count durations.
    DurationSecondSerie, DurationSecondType, native
);
primitive_leaf!(
    /// A column of millisecond-count durations.
    DurationMillisecondSerie, DurationMillisecondType, native
);
primitive_leaf!(
    /// A column of microsecond-count durations.
    DurationMicrosecondSerie, DurationMicrosecondType, native
);
primitive_leaf!(
    /// A column of nanosecond-count durations.
    DurationNanosecondSerie, DurationNanosecondType, native
);
primitive_leaf!(
    /// A column of year-month calendar intervals.
    IntervalYearMonthSerie, IntervalYearMonthType, native
);
primitive_leaf!(
    /// A column of day-time calendar intervals.
    IntervalDayTimeSerie, IntervalDayTimeType, native
);
primitive_leaf!(
    /// A column of month-day-nanosecond calendar intervals.
    IntervalMonthDayNanoSerie, IntervalMonthDayNanoType, native
);
