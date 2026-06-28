//! [`RangeSerie`] — a **lazy**, **datatype-generic** arithmetic range `[start, start+step,
//! …]`. It holds its `start`, `end` and `step` as rich [`ScalarValue`]s and computes each
//! value on demand through the [`Scalar`](yggdryl_scalar::Scalar) math operations, so the
//! *same* range type spans every datatype whose values add and scale: integers, floats and
//! decimals, and the temporal types (a date / timestamp / time range whose `step` is a
//! [`Duration`](yggdryl_core::Duration)). It stores only those few scalars, materialising
//! into a real column when asked.
//!
//! A `uint64` range doubles as the canonical **row index**: because the values are a known
//! arithmetic progression, the label ↔ position lookups ([`at`](RangeSerie::at) /
//! [`position`](RangeSerie::position) / [`contains`](RangeSerie::contains)) are O(1).
//!
//! **Casting a range preserves its original `start` / `end` / `step`** and only re-types
//! what it *exposes*: [`cast`](RangeSerie::cast) returns a still-lazy range computing in the
//! original type, whose [`value_at`](Serie::value_at) / [`array`](Serie::array) /
//! [`data_type`](Serie::data_type) present the **cast** output — so the original numbers
//! survive while the column reads as the new type.

use std::any::Any;
use std::sync::Arc;

use arrow_array::{
    new_empty_array, Array, ArrayRef, Float32Array, Float64Array, Int16Array, Int32Array,
    Int64Array, Int8Array, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};
use yggdryl_scalar::ScalarValue;
use yggdryl_schema::{DataType, Field};

use crate::error::{SerieError, SerieResult};
use crate::scalar::{scalar_from_value, Scalar};
use crate::serie::{Serie, SerieRef};

/// A lazy, datatype-generic arithmetic range: `value(i) = start + step * i`, for `len`
/// rows; also the default row index (O(1) label ↔ position lookups) when it is `uint64`.
///
/// ```
/// use yggdryl_serie::{RangeSerie, DataType, Serie, Scalar};
///
/// let index = RangeSerie::indices(4);               // lazy [0, 1, 2, 3] (uint64)
/// assert_eq!(index.len(), 4);
/// assert!(index.is_range());
/// assert!(!index.is_materialized());                // computed on demand
/// assert_eq!(index.data_type(), &DataType::int(64, false));
/// assert_eq!(index.at(2), Some(2));                 // label at row 2
/// assert_eq!(index.position(3), Some(3));           // row of label 3
///
/// // casting keeps the original integers, exposes float output
/// let floats = index.cast(&DataType::float(64)).unwrap();
/// assert_eq!(floats.data_type(), &DataType::float(64));
/// assert_eq!(floats.value_at(2), Scalar::Float(2.0));
/// ```
#[derive(Debug, Clone)]
pub struct RangeSerie {
    /// The **output** field — its name, the (possibly cast) output datatype, and
    /// nullability. The original value type is `start.data_type()`.
    field: Field,
    /// The first value, in its original type.
    start: ScalarValue,
    /// The exclusive bound `start + step * len`, in its original type.
    end: ScalarValue,
    /// The step between consecutive values.
    step: ScalarValue,
    /// The number of rows.
    len: usize,
}

impl RangeSerie {
    /// A range named `name` of `len` values `start, start+step, …`, computed in `start`'s
    /// type. `start` and `step` may differ in type when the math is defined (e.g. a date
    /// `start` with a duration `step`).
    pub fn new(
        name: impl Into<String>,
        start: ScalarValue,
        step: ScalarValue,
        len: usize,
    ) -> SerieResult<RangeSerie> {
        let field = Field::new(name, start.data_type(), false);
        let end = compute_raw(&start, &step, len)?;
        Ok(RangeSerie {
            field,
            start,
            end,
            step,
            len,
        })
    }

    /// A `uint64` range named `name` (the common integer range / index backing).
    pub fn uint64(name: impl Into<String>, start: u64, step: u64, len: usize) -> RangeSerie {
        RangeSerie::new(
            name,
            ScalarValue::int(start as i128, 64, false),
            ScalarValue::int(step as i128, 64, false),
            len,
        )
        .expect("uint64 + uint64 arithmetic is always defined")
    }

    /// The canonical row index `0, 1, …, len-1` (`uint64`), named `"index"`.
    pub fn indices(len: usize) -> RangeSerie {
        RangeSerie::uint64("index", 0, 1, len)
    }

    /// The first value (its **original** type, preserved across a [`cast`](RangeSerie::cast)).
    pub fn start(&self) -> &ScalarValue {
        &self.start
    }

    /// The exclusive bound `start + step * len` (its **original** type).
    pub fn end(&self) -> &ScalarValue {
        &self.end
    }

    /// The step between consecutive values (its **original** type).
    pub fn step(&self) -> &ScalarValue {
        &self.step
    }

    /// The number of rows.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the range has no rows.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The original value type (the type `start` / `step` / `end` are in), which a
    /// [`cast`](RangeSerie::cast) preserves even as the exposed [`data_type`](Serie::data_type)
    /// changes.
    pub fn original_type(&self) -> DataType {
        self.start.data_type()
    }

    /// Whether the exposed output type differs from the original — i.e. this is a cast range.
    pub fn is_cast(&self) -> bool {
        *self.field.data_type() != self.original_type()
    }

    /// Whether this is the canonical `0, 1, 2, …` `uint64` index (`start == 0`, `step == 1`,
    /// not cast) — the implicit index a frame carries.
    pub fn is_range(&self) -> bool {
        !self.is_cast()
            && self.start == ScalarValue::int(0, 64, false)
            && self.step == ScalarValue::int(1, 64, false)
    }

    /// The raw value at row `i`, in the **original** type (`start + step * i`).
    fn raw(&self, i: usize) -> SerieResult<ScalarValue> {
        compute_raw(&self.start, &self.step, i)
    }

    /// The value at row `i`, in the **exposed output** type (the original value cast to the
    /// output type; an identity when the range is not cast).
    fn output_at(&self, i: usize) -> SerieResult<ScalarValue> {
        let raw = self.raw(i)?;
        let original = self.original_type();
        // Normalise back to the original type (temporal math can widen the unit) …
        let value = if raw.data_type() == original {
            raw
        } else {
            raw.cast(&original)?
        };
        // … then expose the output type.
        let output = self.field.data_type();
        let value = if value.data_type() == *output {
            value
        } else {
            value.cast(output)?
        };
        // Clamp an integer output to its representable range, so an out-of-range value
        // saturates consistently with the materialised array (rather than wrapping).
        Ok(match value {
            ScalarValue::Int {
                value: v,
                bits,
                signed,
            } => {
                let (lo, hi) = int_bounds(bits, signed);
                ScalarValue::int(v.clamp(lo, hi), bits, signed)
            }
            other => other,
        })
    }

    /// The integer label at row `i`, or `None` when out of bounds or non-integer — the
    /// index accessor (reads the exposed output value).
    pub fn at(&self, i: usize) -> Option<u64> {
        if i >= self.len {
            return None;
        }
        match self.output_at(i).ok()? {
            ScalarValue::Int { value, .. } => u64::try_from(value).ok(),
            _ => None,
        }
    }

    /// The first row whose integer label equals `value`, or `None`. O(1) for an integer
    /// range (inverts `start + i*step`); `None` for a non-integer or cast range.
    pub fn position(&self, value: u64) -> Option<usize> {
        // The inverse is only well-defined over an integer progression in the output type.
        let (start, step) = match (&self.start, &self.step) {
            _ if self.is_cast() => return None,
            (ScalarValue::Int { value: s, .. }, ScalarValue::Int { value: st, .. }) => (*s, *st),
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

    /// Builds the full output array. Fast paths build an integer / float range directly;
    /// the general path computes each output value and concatenates.
    fn build_array(&self) -> SerieResult<ArrayRef> {
        if let Some(array) = self.fast_numeric_array() {
            return Ok(array);
        }
        let output_arrow = self.field.data_type().to_arrow()?;
        if self.len == 0 {
            return Ok(new_empty_array(&output_arrow));
        }
        let arrays = (0..self.len)
            .map(|i| {
                self.output_at(i)
                    .and_then(|v| v.to_array().map_err(SerieError::from))
            })
            .collect::<SerieResult<Vec<_>>>()?;
        let refs: Vec<&dyn Array> = arrays.iter().map(|a| a.as_ref()).collect();
        Ok(arrow_select::concat::concat(&refs)?)
    }

    /// A fast, allocation-light build for a non-cast integer / float range (the common
    /// case, including the index); `None` for cast / temporal / decimal ranges, which take
    /// the general per-value path.
    fn fast_numeric_array(&self) -> Option<ArrayRef> {
        if self.is_cast() {
            return None;
        }
        let len = self.len;
        match (&self.start, &self.step) {
            (
                ScalarValue::Int {
                    value: s,
                    bits,
                    signed,
                },
                ScalarValue::Int { value: st, .. },
            ) => {
                let (s, st) = (*s, *st);
                let (lo, hi) = int_bounds(*bits, *signed);
                let at = |i: usize| {
                    s.saturating_add((i as i128).saturating_mul(st))
                        .clamp(lo, hi)
                };
                Some(match (*bits, *signed) {
                    (8, true) => {
                        Arc::new(Int8Array::from_iter_values((0..len).map(|i| at(i) as i8)))
                    }
                    (16, true) => {
                        Arc::new(Int16Array::from_iter_values((0..len).map(|i| at(i) as i16)))
                    }
                    (32, true) => {
                        Arc::new(Int32Array::from_iter_values((0..len).map(|i| at(i) as i32)))
                    }
                    (64, true) => {
                        Arc::new(Int64Array::from_iter_values((0..len).map(|i| at(i) as i64)))
                    }
                    (8, false) => {
                        Arc::new(UInt8Array::from_iter_values((0..len).map(|i| at(i) as u8)))
                    }
                    (16, false) => Arc::new(UInt16Array::from_iter_values(
                        (0..len).map(|i| at(i) as u16),
                    )),
                    (32, false) => Arc::new(UInt32Array::from_iter_values(
                        (0..len).map(|i| at(i) as u32),
                    )),
                    (64, false) => Arc::new(UInt64Array::from_iter_values(
                        (0..len).map(|i| at(i) as u64),
                    )),
                    _ => return None,
                })
            }
            (ScalarValue::Float { value: s, bits }, ScalarValue::Float { value: st, .. }) => {
                let (s, st) = (s.0, st.0);
                let at = |i: usize| s + st * i as f64;
                Some(match *bits {
                    32 => Arc::new(Float32Array::from_iter_values(
                        (0..len).map(|i| at(i) as f32),
                    )),
                    64 => Arc::new(Float64Array::from_iter_values((0..len).map(at))),
                    _ => return None,
                })
            }
            _ => None,
        }
    }
}

/// The `[min, max]` an integer of `bits` width / signedness can represent (a non-standard
/// width falls back to the full `i128` range).
fn int_bounds(bits: u16, signed: bool) -> (i128, i128) {
    if signed {
        match bits {
            8 => (i8::MIN as i128, i8::MAX as i128),
            16 => (i16::MIN as i128, i16::MAX as i128),
            32 => (i32::MIN as i128, i32::MAX as i128),
            64 => (i64::MIN as i128, i64::MAX as i128),
            _ => (i128::MIN, i128::MAX),
        }
    } else {
        match bits {
            8 => (0, u8::MAX as i128),
            16 => (0, u16::MAX as i128),
            32 => (0, u32::MAX as i128),
            64 => (0, u64::MAX as i128),
            _ => (0, i128::MAX),
        }
    }
}

/// `start + step * i`, in the original type (an `i == 0` short-circuit returns `start`).
fn compute_raw(start: &ScalarValue, step: &ScalarValue, i: usize) -> SerieResult<ScalarValue> {
    if i == 0 {
        return Ok(start.clone());
    }
    let idx = ScalarValue::int(i as i128, 64, false);
    Ok(start.add(&step.mul(&idx)?)?)
}

impl Serie for RangeSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn array(&self) -> ArrayRef {
        self.build_array()
            .expect("a range's computed values build an array")
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
        self.output_at(index)
            .map(|v| scalar_from_value(&v))
            .unwrap_or(Scalar::Null)
    }

    /// A sub-range — still lazy, preserving the output type and any cast. Its first value is
    /// the original value at `offset`.
    fn slice(&self, offset: usize, length: usize) -> SerieRef {
        let start = self.raw(offset).unwrap_or_else(|_| self.start.clone());
        let end = compute_raw(&start, &self.step, length).unwrap_or_else(|_| start.clone());
        Arc::new(RangeSerie {
            field: self.field.clone(),
            start,
            end,
            step: self.step.clone(),
            len: length,
        })
    }

    /// Casting a range **preserves the original `start` / `end` / `step`** and only re-types
    /// what it exposes: the result is a still-lazy range computing in the original type,
    /// whose values / array / data type read as `dtype`. Casting to the current type or to
    /// the [`Any`](DataType::Any) wildcard is skipped.
    fn cast(&self, dtype: &DataType) -> SerieResult<SerieRef> {
        if dtype.is_any() || self.field.data_type() == dtype {
            return Ok(Arc::new(self.clone()));
        }
        // The lazy cast-view: keep the original start/end/step, expose `dtype`. Only when
        // the original type can actually reach `dtype` as a scalar cast.
        if !dtype.is_null() && self.start.cast(dtype).is_ok() {
            return Ok(Arc::new(RangeSerie {
                field: self.field.copy(None, Some(dtype.clone()), None, None),
                start: self.start.clone(),
                end: self.end.clone(),
                step: self.step.clone(),
                len: self.len,
            }));
        }
        // Fall back (e.g. to `null` or a nested target): materialise this range, then run
        // the generic column cast.
        crate::serie::dispatch(self.field.clone(), self.array())?.cast(dtype)
    }
}
