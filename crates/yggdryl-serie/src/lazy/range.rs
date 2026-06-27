//! [`RangeSerie`] — a **lazy** `uint64` arithmetic range `[start, start+step, …]` of a
//! fixed length. It stores only three numbers, computing each value on demand; it
//! [materialises](crate::Serie::materialize) into a real `uint64` column when asked.
//! This is the backing of the default [`IndexSerie`](crate::IndexSerie).

use std::any::Any;
use std::sync::Arc;

use arrow_array::{ArrayRef, UInt64Array};
use yggdryl_schema::{DataType, Field};

use crate::scalar::Scalar;
use crate::serie::{Serie, SerieRef, TypedSerie};

/// A lazy `uint64` range column: `value(i) = start + i * step`, for `len` rows.
#[derive(Debug, Clone)]
pub struct RangeSerie {
    field: Field,
    start: u64,
    step: u64,
    len: usize,
}

impl RangeSerie {
    /// A range named `name` of `len` values `start, start+step, …` (`uint64`,
    /// non-nullable).
    pub fn new(name: impl Into<String>, start: u64, step: u64, len: usize) -> RangeSerie {
        RangeSerie {
            field: Field::new(name, DataType::int(64, false), false),
            start,
            step,
            len,
        }
    }

    /// The canonical row index `0, 1, …, len-1`, named `"index"`.
    pub fn indices(len: usize) -> RangeSerie {
        RangeSerie::new("index", 0, 1, len)
    }

    /// The first value.
    pub fn start(&self) -> u64 {
        self.start
    }

    /// The step between consecutive values.
    pub fn step(&self) -> u64 {
        self.step
    }

    /// The value at `index` (no bounds check). Uses **saturating** arithmetic, so an
    /// out-of-range result is clamped at `u64::MAX` rather than wrapping (release) or
    /// panicking (debug).
    fn at(&self, index: usize) -> u64 {
        self.start
            .saturating_add((index as u64).saturating_mul(self.step))
    }
}

impl Serie for RangeSerie {
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
            Scalar::Int(self.at(index) as i128)
        }
    }

    /// A sub-range — still lazy (no materialisation).
    fn slice(&self, offset: usize, length: usize) -> SerieRef {
        Arc::new(RangeSerie::new(
            self.field.name(),
            self.at(offset),
            self.step,
            length,
        ))
    }
}

impl TypedSerie<u64> for RangeSerie {
    fn get(&self, index: usize) -> Option<u64> {
        (index < self.len).then(|| self.at(index))
    }

    fn iter(&self) -> Box<dyn Iterator<Item = Option<u64>> + '_> {
        let (start, step, len) = (self.start, self.step, self.len);
        Box::new((0..len).map(move |i| Some(start.saturating_add((i as u64).saturating_mul(step)))))
    }
}
