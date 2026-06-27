//! [`TimeRangeSerie`] — a **lazy** time-of-day range: a start [`Time`], a step
//! [`Duration`] and a length, stored as nanosecond `Time64`. Each step **wraps within
//! the day** (like [`Time::add`](yggdryl_core::Time::add)); computes on demand and
//! materialises into a real `time` column when asked.

use std::any::Any;
use std::sync::Arc;

use arrow_array::{ArrayRef, Time64NanosecondArray};
use yggdryl_core::{DateTime, Duration, Temporal, Time, TimeUnit};
use yggdryl_schema::{DataType, Field};

use crate::scalar::Scalar;
use crate::serie::{Serie, SerieRef, TypedSerie};
use crate::temporal::TemporalSerie;

/// A lazy time-of-day range: `time(i) = start + i * step`, wrapping within the day, for
/// `len` rows (nanosecond `Time64`).
#[derive(Debug, Clone)]
pub struct TimeRangeSerie {
    field: Field,
    start: Time,
    step: Duration,
    len: usize,
}

impl TimeRangeSerie {
    /// A time range named `name` of `len` times, from `start`, stepping `step` each row
    /// (wrapping within the day).
    pub fn new(name: impl Into<String>, start: Time, step: Duration, len: usize) -> TimeRangeSerie {
        TimeRangeSerie {
            field: Field::new(
                name,
                DataType::Time {
                    unit: TimeUnit::Nanosecond,
                },
                false,
            ),
            start,
            step,
            len,
        }
    }

    /// The step between consecutive times.
    pub fn step(&self) -> &Duration {
        &self.step
    }

    /// The [`Time`] at `index` (no bounds check), wrapping within the day.
    fn at(&self, index: usize) -> Time {
        self.start.add(&self.step.mul(index as i64))
    }

    /// The [`Time`] at `index`, or `None` when out of bounds.
    pub fn time(&self, index: usize) -> Option<Time> {
        (index < self.len).then(|| self.at(index))
    }
}

impl Serie for TimeRangeSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn array(&self) -> ArrayRef {
        Arc::new(Time64NanosecondArray::from_iter_values(
            (0..self.len).map(|i| self.at(i).nanos_of_day() as i64),
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
            Scalar::Int(self.at(index).nanos_of_day() as i128)
        }
    }

    /// A sub-range — still lazy (no materialisation).
    fn slice(&self, offset: usize, length: usize) -> SerieRef {
        Arc::new(TimeRangeSerie {
            field: Field::new(
                self.field.name(),
                DataType::Time {
                    unit: TimeUnit::Nanosecond,
                },
                false,
            ),
            start: self.at(offset),
            step: self.step,
            len: length,
        })
    }
}

impl TypedSerie<Time> for TimeRangeSerie {
    fn get(&self, index: usize) -> Option<Time> {
        self.time(index)
    }

    fn iter(&self) -> Box<dyn Iterator<Item = Option<Time>> + '_> {
        Box::new((0..self.len).map(move |i| self.time(i)))
    }
}

impl TemporalSerie for TimeRangeSerie {
    fn datetime_at(&self, index: usize) -> Option<DateTime> {
        self.time(index).map(|t| t.to_datetime())
    }
}
