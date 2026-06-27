//! [`DateRangeSerie`] — a **lazy** calendar-date range (day resolution, `Date32`
//! storage): a start day, a step in days and a length. Like [`RangeSerie`](super::RangeSerie)
//! it stores only a few numbers and computes each date on demand, materialising into a
//! real `date` column when asked.

use std::any::Any;
use std::sync::Arc;

use arrow_array::{ArrayRef, Date32Array};
use yggdryl_core::Date;
use yggdryl_schema::{DataType, Field};

use crate::scalar::Scalar;
use crate::serie::{Serie, SerieRef, TypedSerie};

/// A lazy day-resolution date range: `day(i) = start_days + i * step_days` (days since
/// the Unix epoch), for `len` rows, stored as `Date32`.
#[derive(Debug, Clone)]
pub struct DateRangeSerie {
    field: Field,
    start_days: i32,
    step_days: i32,
    len: usize,
}

impl DateRangeSerie {
    /// A date range named `name` of `len` days, from `start_days` (days since epoch),
    /// stepping `step_days` each row.
    pub fn new(
        name: impl Into<String>,
        start_days: i32,
        step_days: i32,
        len: usize,
    ) -> DateRangeSerie {
        DateRangeSerie {
            field: Field::new(name, DataType::date(), false),
            start_days,
            step_days,
            len,
        }
    }

    /// A date range starting at a [`Date`], stepping `step_days` each row.
    pub fn from_dates(
        name: impl Into<String>,
        start: Date,
        step_days: i32,
        len: usize,
    ) -> DateRangeSerie {
        DateRangeSerie::new(name, start.epoch_days(), step_days, len)
    }

    /// The first day value (days since the Unix epoch).
    pub fn start_days(&self) -> i32 {
        self.start_days
    }

    /// The step between consecutive days.
    pub fn step_days(&self) -> i32 {
        self.step_days
    }

    /// The day-since-epoch value at `index` (no bounds check). Uses **saturating**
    /// arithmetic, so an out-of-range result is clamped at the `i32` bound rather than
    /// wrapping (release) or panicking (debug).
    fn at(&self, index: usize) -> i32 {
        self.start_days
            .saturating_add((index as i32).saturating_mul(self.step_days))
    }

    /// The [`Date`] at `index`, or `None` when out of bounds.
    pub fn date_at(&self, index: usize) -> Option<Date> {
        (index < self.len).then(|| Date::from_epoch_days(self.at(index)))
    }
}

impl Serie for DateRangeSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn array(&self) -> ArrayRef {
        let (start, step) = (self.start_days, self.step_days);
        Arc::new(Date32Array::from_iter_values(
            (0..self.len).map(|i| start.saturating_add((i as i32).saturating_mul(step))),
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
        Arc::new(DateRangeSerie::new(
            self.field.name(),
            self.at(offset),
            self.step_days,
            length,
        ))
    }
}

impl TypedSerie<i32> for DateRangeSerie {
    fn get(&self, index: usize) -> Option<i32> {
        (index < self.len).then(|| self.at(index))
    }

    fn iter(&self) -> Box<dyn Iterator<Item = Option<i32>> + '_> {
        let (start, step, len) = (self.start_days, self.step_days, self.len);
        Box::new((0..len).map(move |i| Some(start.saturating_add((i as i32).saturating_mul(step)))))
    }
}
