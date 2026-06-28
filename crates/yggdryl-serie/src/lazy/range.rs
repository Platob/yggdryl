//! [`RangeSerie<A>`] — a **lazy**, **type-parameterised** arithmetic range `[start,
//! start+step, …]`. It is parameterised by an Arrow primitive type `A` (like
//! [`PrimitiveSerie<A>`](crate::PrimitiveSerie)) and stores its `start` / `step` as the
//! native physical values (`u64`, `i64`, a timestamp's `i64`, …), computing each value with
//! **native arithmetic** and building a `PrimitiveArray<A>` directly — so an integer or
//! timestamp range is as cheap as a typed array read, with no per-value boxing.
//!
//! A `uint64` range ([`UInt64RangeSerie`]) doubles as the canonical **row index**: because
//! the values are a known arithmetic progression, the label ↔ position lookups
//! ([`at`](RangeSerie::at) / [`position`](RangeSerie::position) /
//! [`contains`](RangeSerie::contains)) are O(1).
//!
//! **Casting a range preserves its original `start` / `step`** and only re-types what it
//! *exposes*: [`cast`](Serie::cast) keeps the native progression and re-types the output, so
//! [`value_at`](Serie::value_at) / [`array`](Serie::array) / [`data_type`](Serie::data_type)
//! read as the cast type while the original numbers survive and the column stays lazy.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use arrow_array::types::UInt64Type;
use arrow_array::{ArrayRef, ArrowPrimitiveType, PrimitiveArray};
use yggdryl_schema::{DataType, Field};

use crate::error::SerieResult;
use crate::scalar::{scalar_at_ref, Scalar};
use crate::serie::{dispatch, Serie, SerieRef};

/// The native physical value of a [`RangeSerie`] — the small set of operations the range
/// needs from `A::Native`: build it from a row index, add and (wrapping-)multiply, and map
/// it to a type-erased [`Scalar`]. Implemented for every integer and float native, which
/// covers the integer / float / date / time / timestamp / duration ranges.
///
/// It is `pub` only so [`RangeSerie<A>`]'s `where A::Native: RangeNative` bound can be
/// named; it is not meant to be implemented downstream.
pub trait RangeNative: Copy {
    /// The native value of a row index `i` (`i` widened to the native type).
    fn from_index(i: usize) -> Self;
    /// `self + other` (wrapping — a range never panics on overflow).
    fn range_add(self, other: Self) -> Self;
    /// `self * other` (wrapping).
    fn range_mul(self, other: Self) -> Self;
    /// The value as a type-erased [`Scalar`] (the same widening as a column read).
    fn to_scalar(self) -> Scalar;
}

/// Implements [`RangeNative`] for an integer native (widening into [`Scalar::Int`]).
macro_rules! range_native_int {
    ($($ty:ty),+) => {$(
        impl RangeNative for $ty {
            fn from_index(i: usize) -> Self { i as $ty }
            fn range_add(self, other: Self) -> Self { self.wrapping_add(other) }
            fn range_mul(self, other: Self) -> Self { self.wrapping_mul(other) }
            fn to_scalar(self) -> Scalar { Scalar::Int(self as i128) }
        }
    )+};
}
range_native_int!(i8, i16, i32, i64, u8, u16, u32, u64);

/// Implements [`RangeNative`] for a float native (widening into [`Scalar::Float`]).
macro_rules! range_native_float {
    ($($ty:ty),+) => {$(
        impl RangeNative for $ty {
            fn from_index(i: usize) -> Self { i as $ty }
            fn range_add(self, other: Self) -> Self { self + other }
            fn range_mul(self, other: Self) -> Self { self * other }
            fn to_scalar(self) -> Scalar { Scalar::Float(self as f64) }
        }
    )+};
}
range_native_float!(f32, f64);

/// A lazy, type-parameterised arithmetic range over the Arrow primitive type `A`:
/// `value(i) = start + step * i`, for `len` rows. Aliased for the common widths (see
/// [`UInt64RangeSerie`]); a `uint64` one is the canonical row index.
///
/// ```
/// use yggdryl_serie::{UInt64RangeSerie, DataType, Serie, Scalar};
///
/// let index = UInt64RangeSerie::indices(4);         // lazy [0, 1, 2, 3] (uint64)
/// assert_eq!(index.len(), 4);
/// assert!(index.is_range());
/// assert!(!index.is_materialized());                // computed on demand
/// assert_eq!(index.data_type(), &DataType::int(64, false));
/// assert_eq!(index.at(2), Some(2));                 // label at row 2
/// assert_eq!(index.position(3), Some(3));           // row of label 3
///
/// // casting keeps the original uint64 progression, exposes float output
/// let floats = index.cast(&DataType::float(64)).unwrap();
/// assert_eq!(floats.data_type(), &DataType::float(64));
/// assert_eq!(floats.value_at(2), Scalar::Float(2.0));
/// ```
pub struct RangeSerie<A: ArrowPrimitiveType>
where
    A::Native: RangeNative,
{
    /// The **output** field — its name, the (possibly cast) output datatype, and
    /// nullability. The original value type is `A`'s.
    field: Field,
    /// The first value (native physical).
    start: A::Native,
    /// The step between consecutive values (native physical).
    step: A::Native,
    /// The number of rows.
    len: usize,
    /// Whether the output type differs from `A`'s (a cast range) — cached so the hot
    /// `value_at` / `array` paths skip recomputing the original type.
    casted: bool,
}

impl<A: ArrowPrimitiveType> RangeSerie<A>
where
    A::Native: RangeNative,
{
    /// A range named `name` of `len` values `start, start+step, …` (native `A` physicals).
    pub fn new(name: impl Into<String>, start: A::Native, step: A::Native, len: usize) -> Self {
        let dtype = DataType::from_arrow(&A::DATA_TYPE);
        RangeSerie {
            field: Field::new(name, dtype, false),
            start,
            step,
            len,
            casted: false,
        }
    }

    /// The first value (native physical, the **original** progression, preserved across a
    /// [`cast`](Serie::cast)).
    pub fn start(&self) -> A::Native {
        self.start
    }

    /// The step between consecutive values (native physical).
    pub fn step(&self) -> A::Native {
        self.step
    }

    /// The number of rows.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the range has no rows.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The original value type (`A`'s), preserved even as the exposed
    /// [`data_type`](Serie::data_type) changes across a cast.
    pub fn original_type(&self) -> DataType {
        DataType::from_arrow(&A::DATA_TYPE)
    }

    /// Whether the exposed output type differs from the original — i.e. this is a cast range.
    pub fn is_cast(&self) -> bool {
        self.casted
    }

    /// Whether this is the canonical `0, 1, 2, …` index (`start == 0`, `step == 1`, not
    /// cast) — the implicit index a frame carries.
    pub fn is_range(&self) -> bool {
        !self.is_cast()
            && self.start.to_scalar() == Scalar::Int(0)
            && self.step.to_scalar() == Scalar::Int(1)
    }

    /// The native value at row `i` (`start + step * i`, wrapping on overflow).
    fn native_at(&self, i: usize) -> A::Native {
        self.start
            .range_add(self.step.range_mul(RangeNative::from_index(i)))
    }

    /// The raw (original-typed) Arrow array of all `len` values.
    fn raw_array(&self) -> ArrayRef {
        Arc::new(PrimitiveArray::<A>::from_iter_values(
            (0..self.len).map(|i| self.native_at(i)),
        ))
    }

    /// The integer label at row `i`, or `None` when out of bounds or non-integer — the index
    /// accessor (reads the original native progression).
    pub fn at(&self, i: usize) -> Option<u64> {
        if i >= self.len {
            return None;
        }
        match self.native_at(i).to_scalar() {
            Scalar::Int(value) => u64::try_from(value).ok(),
            _ => None,
        }
    }

    /// The first row whose integer label equals `value`, or `None`. O(1): inverts the
    /// progression; `None` for a non-integer or cast range.
    pub fn position(&self, value: u64) -> Option<usize> {
        if self.is_cast() {
            return None;
        }
        let (start, step) = match (self.start.to_scalar(), self.step.to_scalar()) {
            (Scalar::Int(s), Scalar::Int(st)) => (s, st),
            _ => return None,
        };
        let value = value as i128;
        if step == 0 {
            return (value == start && self.len > 0).then_some(0);
        }
        let offset = value - start;
        if offset % step != 0 {
            return None;
        }
        let row = offset / step;
        (row >= 0 && (row as usize) < self.len).then_some(row as usize)
    }

    /// Whether `value` is one of the range's labels.
    pub fn contains(&self, value: u64) -> bool {
        self.position(value).is_some()
    }

    /// The output Arrow type (the exposed, possibly cast type).
    fn output_arrow(&self) -> SerieResult<arrow_schema::DataType> {
        Ok(self.field.data_type().to_arrow()?)
    }
}

/// `uint64` row-index / range constructors (the common case, and the index backing).
impl RangeSerie<UInt64Type> {
    /// A `uint64` range named `name`.
    pub fn uint64(name: impl Into<String>, start: u64, step: u64, len: usize) -> Self {
        RangeSerie::new(name, start, step, len)
    }

    /// The canonical row index `0, 1, …, len-1` (`uint64`), named `"index"`.
    pub fn indices(len: usize) -> Self {
        RangeSerie::uint64("index", 0, 1, len)
    }
}

impl<A: ArrowPrimitiveType> Clone for RangeSerie<A>
where
    A::Native: RangeNative,
{
    fn clone(&self) -> Self {
        RangeSerie {
            field: self.field.clone(),
            start: self.start,
            step: self.step,
            len: self.len,
            casted: self.casted,
        }
    }
}

impl<A: ArrowPrimitiveType> fmt::Debug for RangeSerie<A>
where
    A::Native: RangeNative,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RangeSerie")
            .field("field", &self.field)
            .field("start", &self.start.to_scalar())
            .field("step", &self.step.to_scalar())
            .field("len", &self.len)
            .finish()
    }
}

impl<A: ArrowPrimitiveType> Serie for RangeSerie<A>
where
    A::Native: RangeNative,
{
    fn field(&self) -> &Field {
        &self.field
    }

    fn array(&self) -> ArrayRef {
        let raw = self.raw_array();
        if !self.is_cast() {
            return raw;
        }
        // Expose the cast output by running the Arrow kernel over the raw progression.
        let target = self
            .output_arrow()
            .expect("a range's output type converts to Arrow");
        arrow_cast::cast(raw.as_ref(), &target).expect("a range casts to its output type")
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn len(&self) -> usize {
        self.len
    }

    fn null_count(&self) -> usize {
        0
    }

    fn is_null(&self, index: usize) -> bool {
        index >= self.len
    }

    fn is_materialized(&self) -> bool {
        false
    }

    fn value_at(&self, index: usize) -> Scalar {
        if index >= self.len {
            return Scalar::Null;
        }
        let native = self.native_at(index);
        if !self.is_cast() {
            return native.to_scalar();
        }
        // Cast the single native value to the output type and read it back.
        let raw: ArrayRef = Arc::new(PrimitiveArray::<A>::from_iter_values([native]));
        match self
            .output_arrow()
            .ok()
            .and_then(|target| arrow_cast::cast(raw.as_ref(), &target).ok())
        {
            Some(array) => scalar_at_ref(array.as_ref(), 0),
            None => Scalar::Null,
        }
    }

    /// A sub-range — still lazy, preserving the output type and any cast. Its first value is
    /// the original native value at `offset`.
    fn slice(&self, offset: usize, length: usize) -> SerieRef {
        Arc::new(RangeSerie::<A> {
            field: self.field.clone(),
            start: self.native_at(offset),
            step: self.step,
            len: length,
            casted: self.casted,
        })
    }

    /// Casting a range **preserves the original native `start` / `step`** and only re-types
    /// what it exposes: the result is a still-lazy range whose values / array / data type
    /// read as `dtype`. Casting to the current type or to [`Any`](DataType::Any) is skipped.
    fn cast(&self, dtype: &DataType) -> SerieResult<SerieRef> {
        if dtype.is_any() || self.field.data_type() == dtype {
            return Ok(Arc::new(self.clone()));
        }
        // The lazy cast-view: keep the native progression, expose `dtype` — only when the
        // original type can actually reach `dtype` as an Arrow cast.
        if !dtype.is_null() {
            if let (Ok(from), Ok(to)) = (self.original_type().to_arrow(), dtype.to_arrow()) {
                if arrow_cast::can_cast_types(&from, &to) {
                    return Ok(Arc::new(RangeSerie::<A> {
                        field: self.field.copy(None, Some(dtype.clone()), None, None),
                        start: self.start,
                        step: self.step,
                        len: self.len,
                        // The output now differs from `A` unless we happened to cast to it.
                        casted: *dtype != self.original_type(),
                    }));
                }
            }
        }
        // Fall back (e.g. to `null` or a nested target): materialise, then run the generic
        // column cast.
        dispatch(self.field.clone(), self.array())?.cast(dtype)
    }
}

/// A `uint64` range — the canonical row index (and the common integer range).
pub type UInt64RangeSerie = RangeSerie<UInt64Type>;
