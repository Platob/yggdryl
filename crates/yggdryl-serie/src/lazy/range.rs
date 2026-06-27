//! [`UInt64RangeSerie`] — a **lazy** `uint64` arithmetic range `[start, start+step, …]` of
//! a fixed length. It stores only three numbers, computing each value on demand; it
//! [materialises](crate::Serie::materialize) into a real `uint64` column when asked.
//!
//! It doubles as the canonical **row index**: because the values are a known arithmetic
//! progression, the label ↔ position lookups ([`at`](UInt64RangeSerie::at) /
//! [`position`](UInt64RangeSerie::position) / [`contains`](UInt64RangeSerie::contains)) are
//! O(1). [`indices`](UInt64RangeSerie::indices) builds the implicit `0..len` index a frame
//! carries when no explicit one is set.

use std::any::Any;
use std::sync::Arc;

use arrow_array::{ArrayRef, UInt64Array};
use yggdryl_schema::{DataType, Field};

use crate::scalar::Scalar;
use crate::serie::{Serie, SerieRef, TypedSerie};

/// A lazy `uint64` range column: `value(i) = start + i * step`, for `len` rows; also the
/// default row index (O(1) label ↔ position lookups).
///
/// ```
/// use yggdryl_serie::{UInt64RangeSerie, DataType, Serie};
///
/// let index = UInt64RangeSerie::indices(4);         // lazy [0, 1, 2, 3] (uint64)
/// assert_eq!(index.len(), 4);
/// assert!(index.is_range());
/// assert!(!index.is_materialized());                // computed on demand
/// assert_eq!(index.data_type(), &DataType::int(64, false));
/// assert_eq!(index.at(2), Some(2));                 // label at row 2
/// assert_eq!(index.position(3), Some(3));           // row of label 3
/// assert!(!index.contains(4));
/// ```
#[derive(Debug, Clone)]
pub struct UInt64RangeSerie {
    field: Field,
    start: u64,
    step: u64,
    len: usize,
}

impl UInt64RangeSerie {
    /// A range named `name` of `len` values `start, start+step, …` (`uint64`,
    /// non-nullable).
    pub fn new(name: impl Into<String>, start: u64, step: u64, len: usize) -> UInt64RangeSerie {
        UInt64RangeSerie {
            field: Field::new(name, DataType::int(64, false), false),
            start,
            step,
            len,
        }
    }

    /// The canonical row index `0, 1, …, len-1`, named `"index"`.
    pub fn indices(len: usize) -> UInt64RangeSerie {
        UInt64RangeSerie::new("index", 0, 1, len)
    }

    /// The first value.
    pub fn start(&self) -> u64 {
        self.start
    }

    /// The step between consecutive values.
    pub fn step(&self) -> u64 {
        self.step
    }

    /// Whether this is the canonical `0, 1, 2, …` index (`start == 0`, `step == 1`) — the
    /// implicit index a frame carries; slicing it shifts `start`, dropping the flag.
    pub fn is_range(&self) -> bool {
        self.start == 0 && self.step == 1
    }

    /// The value at `index` (no bounds check). Uses **saturating** arithmetic, so an
    /// out-of-range result is clamped at `u64::MAX` rather than wrapping (release) or
    /// panicking (debug).
    fn compute(&self, index: usize) -> u64 {
        self.start
            .saturating_add((index as u64).saturating_mul(self.step))
    }

    /// The label at row `index`, or `None` when out of bounds — the index accessor.
    pub fn at(&self, index: usize) -> Option<u64> {
        (index < self.len).then(|| self.compute(index))
    }

    /// The first row whose label equals `value`, or `None`. O(1): inverts the arithmetic
    /// progression (`(value - start) / step`), so it works for a sliced range too.
    pub fn position(&self, value: u64) -> Option<usize> {
        if self.step == 0 {
            return (value == self.start && self.len > 0).then_some(0);
        }
        if value < self.start {
            return None;
        }
        let offset = value - self.start;
        if !offset.is_multiple_of(self.step) {
            return None;
        }
        let row = (offset / self.step) as usize;
        (row < self.len).then_some(row)
    }

    /// Whether `value` is one of the range's labels.
    pub fn contains(&self, value: u64) -> bool {
        self.position(value).is_some()
    }
}

impl Serie for UInt64RangeSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn array(&self) -> ArrayRef {
        let (start, step) = (self.start, self.step);
        Arc::new(UInt64Array::from_iter_values(
            (0..self.len).map(|i| start.saturating_add((i as u64).saturating_mul(step))),
        ))
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
            Scalar::Null
        } else {
            Scalar::Int(self.compute(index) as i128)
        }
    }

    /// A sub-range — still lazy (no materialisation). Its labels start at the sliced
    /// `offset`, so it is no longer the canonical `0..len` index.
    fn slice(&self, offset: usize, length: usize) -> SerieRef {
        Arc::new(UInt64RangeSerie::new(
            self.field.name(),
            self.compute(offset),
            self.step,
            length,
        ))
    }
}

impl TypedSerie<u64> for UInt64RangeSerie {
    fn get(&self, index: usize) -> Option<u64> {
        (index < self.len).then(|| self.compute(index))
    }

    fn iter(&self) -> Box<dyn Iterator<Item = Option<u64>> + '_> {
        let (start, step, len) = (self.start, self.step, self.len);
        Box::new((0..len).map(move |i| Some(start.saturating_add((i as u64).saturating_mul(step)))))
    }
}
