//! Every column whose rows are one fixed-width value in a values buffer.
//!
//! Arrow lays all of them out the same way - a `values` buffer of the
//! native width, and a validity bitmap where the column admits absence - so
//! they are one generic type, [`PrimitiveSerie<T>`], and each leaf is a name
//! for one instantiation of it: [`Int32Serie`] is `PrimitiveSerie<Int32Type>`
//! and lends `&[i32]`, [`Decimal128Serie`] lends `&[i128]`,
//! [`DateTime64Serie`] lends `&[i64]`. Nothing is boxed and nothing is copied
//! to read one.
//!
//! [`BooleanSerie`] and [`NullSerie`] sit beside them: a boolean is a bitmap
//! rather than a values buffer, and a null column is a length and nothing
//! else, so neither is a `PrimitiveSerie` even though both are fixed width.

use std::fmt;
use std::sync::Arc;

use arrow_array::builder::{BooleanBuilder, PrimitiveBuilder};
use arrow_array::types::{
    Date32Type, Date64Type, Decimal32Type, Decimal64Type, Decimal128Type, Decimal256Type,
    DurationMicrosecondType, DurationMillisecondType, DurationNanosecondType, DurationSecondType,
    Float16Type, Float32Type, Float64Type, Int8Type, Int16Type, Int32Type, Int64Type,
    IntervalDayTimeType, IntervalMonthDayNanoType, IntervalYearMonthType, Time32MillisecondType,
    Time32SecondType, Time64MicrosecondType, Time64NanosecondType, TimestampMicrosecondType,
    TimestampMillisecondType, TimestampNanosecondType, TimestampSecondType, UInt8Type, UInt16Type,
    UInt32Type, UInt64Type,
};
use arrow_array::{Array, ArrayRef, ArrowPrimitiveType, BooleanArray, NullArray, PrimitiveArray};
use arrow_buffer::{BooleanBuffer, NullBuffer};

use super::{Serie, one_row};
use crate::value::SerieValue;
use crate::{Field, Result, Scalar};

/// Which leaf of the [`Serie`] root one primitive width widens to.
///
/// Arrow's type parameter is what makes a values buffer typed, and this is
/// the one place that parameter is tied to the crate's own leaf, so
/// [`PrimitiveSerie`] stays one implementation over every width.
pub trait PrimitiveLeaf: ArrowPrimitiveType + Sized + Send + Sync + 'static {
    /// Widen a column of this width to the serie root.
    fn into_serie(column: PrimitiveSerie<Self>) -> Serie;

    /// Narrow a serie root to a column of this width.
    fn from_serie(serie: &Serie) -> Option<&PrimitiveSerie<Self>>;
}

/// One column of fixed-width rows: the field that types them, and the Arrow
/// buffers that hold them.
///
/// The values buffer and the validity bitmap are Arrow's own, shared rather
/// than copied, so a clone costs two pointer bumps and
/// [`values`](Self::values) lends the native slice straight out of the
/// buffer.
pub struct PrimitiveSerie<T: PrimitiveLeaf> {
    field: Arc<Field>,
    values: PrimitiveArray<T>,
}

impl<T: PrimitiveLeaf> PrimitiveSerie<T> {
    /// Pair a field with the buffers that hold its rows.
    pub(crate) const fn new(field: Arc<Field>, values: PrimitiveArray<T>) -> Self {
        Self { field, values }
    }

    /// Borrow the values buffer, without copying it.
    ///
    /// Every row has a slot, a null row included; what a null row's slot
    /// holds is not a value, and [`Self::nulls`] is what says which slots
    /// those are.
    pub fn values(&self) -> &[T::Native] {
        self.values.values()
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub fn nulls(&self) -> Option<&NullBuffer> {
        self.values.nulls()
    }

    /// Borrow the Arrow array these buffers are.
    pub const fn array(&self) -> &PrimitiveArray<T> {
        &self.values
    }

    /// Read row `index` off the values buffer.
    ///
    /// One bounds check and one buffer read; no value is built.
    pub fn value(&self, index: usize) -> Option<T::Native> {
        (index < self.values.len() && !self.values.is_null(index)).then(|| self.values.value(index))
    }

    /// Append one native row, without building a value for it.
    ///
    /// The values buffer is written in place where this column holds it
    /// alone, and copied once where it does not.
    pub fn push_value(&mut self, value: Option<T::Native>) {
        self.edit(|builder| builder.append_option(value));
    }

    /// Overwrite row `index` with one native value.
    ///
    /// A present row overwritten by a present value is one buffer write; a
    /// row that gains or loses its absence rebuilds the validity bitmap.
    ///
    /// # Errors
    ///
    /// Returns an error when `index` is past the end.
    pub fn set_value(&mut self, index: usize, value: Option<T::Native>) -> Result<()> {
        super::require_row(self.field.name(), index, self.values.len())?;
        let fast = value.is_some() && !self.values.is_null(index);
        self.edit(|builder| {
            if fast {
                if let Some(value) = value {
                    builder.values_slice_mut()[index] = value;
                }
            } else {
                let mut rebuilt =
                    PrimitiveBuilder::<T>::with_capacity(builder.values_slice().len())
                        .with_data_type(T::DATA_TYPE);
                let held = builder.finish();
                for row in 0..held.len() {
                    if row == index {
                        rebuilt.append_option(value);
                    } else if held.is_null(row) {
                        rebuilt.append_null();
                    } else {
                        rebuilt.append_value(held.value(row));
                    }
                }
                *builder = rebuilt;
            }
        });
        Ok(())
    }

    /// Run one edit against this column's own builder.
    ///
    /// Arrow hands the buffers back as a builder when nothing else holds
    /// them, which is what makes an append or a slot write in place; when
    /// something does, the rows are copied once into a fresh one.
    fn edit(&mut self, change: impl FnOnce(&mut PrimitiveBuilder<T>)) {
        let dtype = self.values.data_type().clone();
        let empty = PrimitiveArray::<T>::new_null(0).with_data_type(dtype.clone());
        let taken = std::mem::replace(&mut self.values, empty);
        let mut builder = match taken.into_builder() {
            Ok(builder) => builder,
            Err(shared) => {
                let mut builder = PrimitiveBuilder::<T>::with_capacity(shared.len() + 1);
                for row in 0..shared.len() {
                    if shared.is_null(row) {
                        builder.append_null();
                    } else {
                        builder.append_value(shared.value(row));
                    }
                }
                builder
            }
        };
        change(&mut builder);
        self.values = builder.finish().with_data_type(dtype);
    }
}

impl<T: PrimitiveLeaf> SerieValue for PrimitiveSerie<T> {
    fn field(&self) -> &Field {
        &self.field
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    fn null_count(&self) -> usize {
        self.values.null_count()
    }

    fn is_null(&self, index: usize) -> bool {
        index >= self.values.len() || self.values.is_null(index)
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        super::scalar_at(&self.field, &self.values, index)
    }

    fn set(&mut self, index: usize, value: Scalar) -> Result<()> {
        super::require_row(self.field.name(), index, self.values.len())?;
        let row = one_row::<PrimitiveArray<T>>(&self.field, value)?;
        self.set_value(
            index,
            if row.is_null(0) {
                None
            } else {
                Some(row.value(0))
            },
        )
    }

    fn push(&mut self, value: Scalar) -> Result<()> {
        let row = one_row::<PrimitiveArray<T>>(&self.field, value)?;
        self.push_value(if row.is_null(0) {
            None
        } else {
            Some(row.value(0))
        });
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        Arc::new(self.values.clone())
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
        }
    }
}

serie_leaf!(PrimitiveSerie, T: PrimitiveLeaf);

/// Name one primitive width as a leaf of the root, and tie Arrow's type
/// parameter to the family variant it widens through.
macro_rules! primitive_leaf {
    ($(#[$meta:meta])* $name:ident, $arrow:ty, $family:ident, $held:ident, $variant:ident) => {
        $(#[$meta])*
        pub type $name = PrimitiveSerie<$arrow>;

        impl PrimitiveLeaf for $arrow {
            fn into_serie(column: PrimitiveSerie<Self>) -> Serie {
                Serie::$family(::std::sync::Arc::new(super::$held::$variant(column)))
            }

            fn from_serie(serie: &Serie) -> Option<&PrimitiveSerie<Self>> {
                match serie {
                    Serie::$family(family) => match family.as_ref() {
                        super::$held::$variant(column) => Some(column),
                        _ => None,
                    },
                    _ => None,
                }
            }
        }
    };
}

primitive_leaf!(
    /// A column of signed 8-bit integers.
    Int8Serie,
    Int8Type,
    Integer,
    IntegerSerie,
    Int8
);
primitive_leaf!(
    /// A column of signed 16-bit integers.
    Int16Serie,
    Int16Type,
    Integer,
    IntegerSerie,
    Int16
);
primitive_leaf!(
    /// A column of signed 32-bit integers.
    Int32Serie,
    Int32Type,
    Integer,
    IntegerSerie,
    Int32
);
primitive_leaf!(
    /// A column of signed 64-bit integers.
    Int64Serie,
    Int64Type,
    Integer,
    IntegerSerie,
    Int64
);
primitive_leaf!(
    /// A column of unsigned 8-bit integers.
    UInt8Serie,
    UInt8Type,
    Integer,
    IntegerSerie,
    UInt8
);
primitive_leaf!(
    /// A column of unsigned 16-bit integers.
    UInt16Serie,
    UInt16Type,
    Integer,
    IntegerSerie,
    UInt16
);
primitive_leaf!(
    /// A column of unsigned 32-bit integers.
    UInt32Serie,
    UInt32Type,
    Integer,
    IntegerSerie,
    UInt32
);
primitive_leaf!(
    /// A column of unsigned 64-bit integers.
    UInt64Serie,
    UInt64Type,
    Integer,
    IntegerSerie,
    UInt64
);

primitive_leaf!(
    /// A column of IEEE binary16 floats.
    Float16Serie,
    Float16Type,
    Floating,
    FloatingSerie,
    Float16
);
primitive_leaf!(
    /// A column of IEEE binary32 floats.
    Float32Serie,
    Float32Type,
    Floating,
    FloatingSerie,
    Float32
);
primitive_leaf!(
    /// A column of IEEE binary64 floats.
    Float64Serie,
    Float64Type,
    Floating,
    FloatingSerie,
    Float64
);

primitive_leaf!(
    /// A column of 32-bit coefficient-and-scale decimals.
    Decimal32Serie,
    Decimal32Type,
    Decimal,
    DecimalSerie,
    Decimal32
);
primitive_leaf!(
    /// A column of 64-bit coefficient-and-scale decimals.
    Decimal64Serie,
    Decimal64Type,
    Decimal,
    DecimalSerie,
    Decimal64
);
primitive_leaf!(
    /// A column of 128-bit coefficient-and-scale decimals.
    Decimal128Serie,
    Decimal128Type,
    Decimal,
    DecimalSerie,
    Decimal128
);
primitive_leaf!(
    /// A column of 256-bit coefficient-and-scale decimals.
    Decimal256Serie,
    Decimal256Type,
    Decimal,
    DecimalSerie,
    Decimal256
);

primitive_leaf!(
    /// A column of 32-bit day-count dates.
    Date32Serie,
    Date32Type,
    Temporal,
    TemporalSerie,
    Date32
);
primitive_leaf!(
    /// A column of 64-bit millisecond-count dates.
    Date64Serie,
    Date64Type,
    Temporal,
    TemporalSerie,
    Date64
);
primitive_leaf!(
    /// A column of second-count times of day.
    Time32SecondSerie,
    Time32SecondType,
    Temporal,
    TemporalSerie,
    Time32Second
);
primitive_leaf!(
    /// A column of millisecond-count times of day.
    Time32MillisecondSerie,
    Time32MillisecondType,
    Temporal,
    TemporalSerie,
    Time32Millisecond
);
primitive_leaf!(
    /// A column of microsecond-count times of day.
    Time64MicrosecondSerie,
    Time64MicrosecondType,
    Temporal,
    TemporalSerie,
    Time64Microsecond
);
primitive_leaf!(
    /// A column of nanosecond-count times of day.
    Time64NanosecondSerie,
    Time64NanosecondType,
    Temporal,
    TemporalSerie,
    Time64Nanosecond
);
primitive_leaf!(
    /// A column of second-count datetimes.
    DateTimeSecondSerie,
    TimestampSecondType,
    Temporal,
    TemporalSerie,
    DateTimeSecond
);
primitive_leaf!(
    /// A column of millisecond-count datetimes.
    DateTimeMillisecondSerie,
    TimestampMillisecondType,
    Temporal,
    TemporalSerie,
    DateTimeMillisecond
);
primitive_leaf!(
    /// A column of microsecond-count datetimes.
    DateTimeMicrosecondSerie,
    TimestampMicrosecondType,
    Temporal,
    TemporalSerie,
    DateTimeMicrosecond
);
primitive_leaf!(
    /// A column of nanosecond-count datetimes.
    DateTimeNanosecondSerie,
    TimestampNanosecondType,
    Temporal,
    TemporalSerie,
    DateTimeNanosecond
);
primitive_leaf!(
    /// A column of second-count durations.
    DurationSecondSerie,
    DurationSecondType,
    Temporal,
    TemporalSerie,
    DurationSecond
);
primitive_leaf!(
    /// A column of millisecond-count durations.
    DurationMillisecondSerie,
    DurationMillisecondType,
    Temporal,
    TemporalSerie,
    DurationMillisecond
);
primitive_leaf!(
    /// A column of microsecond-count durations.
    DurationMicrosecondSerie,
    DurationMicrosecondType,
    Temporal,
    TemporalSerie,
    DurationMicrosecond
);
primitive_leaf!(
    /// A column of nanosecond-count durations.
    DurationNanosecondSerie,
    DurationNanosecondType,
    Temporal,
    TemporalSerie,
    DurationNanosecond
);
primitive_leaf!(
    /// A column of year-month calendar intervals.
    IntervalYearMonthSerie,
    IntervalYearMonthType,
    Temporal,
    TemporalSerie,
    IntervalYearMonth
);
primitive_leaf!(
    /// A column of day-time calendar intervals.
    IntervalDayTimeSerie,
    IntervalDayTimeType,
    Temporal,
    TemporalSerie,
    IntervalDayTime
);
primitive_leaf!(
    /// A column of month-day-nanosecond calendar intervals.
    IntervalMonthDayNanoSerie,
    IntervalMonthDayNanoType,
    Temporal,
    TemporalSerie,
    IntervalMonthDayNano
);

// ------------------------------------------------------------------------
// The two fixed-width layouts that are not a values buffer.
// ------------------------------------------------------------------------

/// One column of booleans: a bitmap of values beside a bitmap of validity.
#[derive(Clone)]
pub struct BooleanSerie {
    field: Arc<Field>,
    values: BooleanArray,
}

impl BooleanSerie {
    /// Pair a field with the bitmaps that hold its rows.
    pub(crate) const fn new(field: Arc<Field>, values: BooleanArray) -> Self {
        Self { field, values }
    }

    /// Borrow the values bitmap, without copying it.
    pub fn values(&self) -> &BooleanBuffer {
        self.values.values()
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub fn nulls(&self) -> Option<&NullBuffer> {
        self.values.nulls()
    }

    /// Borrow the Arrow array these bitmaps are.
    pub const fn array(&self) -> &BooleanArray {
        &self.values
    }

    /// Read row `index` off the values bitmap.
    pub fn value(&self, index: usize) -> Option<bool> {
        (index < self.values.len() && !self.values.is_null(index)).then(|| self.values.value(index))
    }

    /// Append one native row, without building a value for it.
    ///
    /// Arrow hands no builder back for a bitmap, so the bits are rewritten
    /// once - which is what a bit-packed layout costs, and why the value is
    /// still never boxed.
    pub fn push_value(&mut self, value: Option<bool>) {
        let mut builder = BooleanBuilder::with_capacity(self.values.len() + 1);
        for row in 0..self.values.len() {
            if self.values.is_null(row) {
                builder.append_null();
            } else {
                builder.append_value(self.values.value(row));
            }
        }
        builder.append_option(value);
        self.values = builder.finish();
    }

    /// Overwrite row `index` with one native value.
    ///
    /// The bits are rewritten once, for the reason
    /// [`push_value`](Self::push_value) states.
    ///
    /// # Errors
    ///
    /// Returns an error when `index` is past the end.
    pub fn set_value(&mut self, index: usize, value: Option<bool>) -> Result<()> {
        super::require_row(self.field.name(), index, self.values.len())?;
        let mut builder = BooleanBuilder::with_capacity(self.values.len());
        for row in 0..self.values.len() {
            if row == index {
                builder.append_option(value);
            } else if self.values.is_null(row) {
                builder.append_null();
            } else {
                builder.append_value(self.values.value(row));
            }
        }
        self.values = builder.finish();
        Ok(())
    }
}

impl SerieValue for BooleanSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    fn null_count(&self) -> usize {
        self.values.null_count()
    }

    fn is_null(&self, index: usize) -> bool {
        index >= self.values.len() || self.values.is_null(index)
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        super::scalar_at(&self.field, &self.values, index)
    }

    fn set(&mut self, index: usize, value: Scalar) -> Result<()> {
        super::require_row(self.field.name(), index, self.values.len())?;
        let row = one_row::<BooleanArray>(&self.field, value)?;
        self.set_value(
            index,
            if row.is_null(0) {
                None
            } else {
                Some(row.value(0))
            },
        )
    }

    fn push(&mut self, value: Scalar) -> Result<()> {
        let row = one_row::<BooleanArray>(&self.field, value)?;
        self.push_value(if row.is_null(0) {
            None
        } else {
            Some(row.value(0))
        });
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        Arc::new(self.values.clone())
    }

    fn into_serie(self) -> Serie {
        Serie::Boolean(Arc::new(self))
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        match value {
            Serie::Boolean(column) => Some(column.as_ref()),
            _ => None,
        }
    }
}

serie_leaf!(BooleanSerie);

/// One column of nulls: a length, and no buffer at all.
#[derive(Clone)]
pub struct NullSerie {
    field: Arc<Field>,
    rows: usize,
}

impl NullSerie {
    /// Pair a field with the count of rows it holds.
    pub(crate) const fn new(field: Arc<Field>, rows: usize) -> Self {
        Self { field, rows }
    }

    /// Append one row.
    pub const fn push_value(&mut self) {
        self.rows += 1;
    }
}

impl SerieValue for NullSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn len(&self) -> usize {
        self.rows
    }

    fn null_count(&self) -> usize {
        self.rows
    }

    fn is_null(&self, _index: usize) -> bool {
        true
    }

    fn scalar(&self, _index: usize) -> Result<Scalar> {
        Ok(Scalar::Null)
    }

    fn set(&mut self, index: usize, value: Scalar) -> Result<()> {
        super::require_row(self.field.name(), index, self.rows)?;
        self.field.scalar(value).map(drop)
    }

    fn push(&mut self, value: Scalar) -> Result<()> {
        self.field.scalar(value)?;
        self.rows += 1;
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        Arc::new(NullArray::new(self.rows))
    }

    fn into_serie(self) -> Serie {
        Serie::Null(Arc::new(self))
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        match value {
            Serie::Null(column) => Some(column.as_ref()),
            _ => None,
        }
    }
}

serie_leaf!(NullSerie);

impl<T: PrimitiveLeaf> fmt::Debug for PrimitiveSerie<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_leaf(self, "PrimitiveSerie", formatter)
    }
}

impl fmt::Debug for BooleanSerie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_leaf(self, "BooleanSerie", formatter)
    }
}

impl fmt::Debug for NullSerie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_leaf(self, "NullSerie", formatter)
    }
}
