//! [`DateTimeRangeSerie`] — a **lazy** timestamp range: a start instant, a step
//! [`Duration`] and a length, stored as nanosecond `Timestamp` (timezone-naive UTC
//! instants). Computes each instant on demand and materialises into a
//! [`DatetimeSerie`](crate::DatetimeSerie) when asked.

use std::any::Any;
use std::sync::Arc;

use arrow_array::{ArrayRef, TimestampNanosecondArray};
use yggdryl_core::{DateTime, Duration, TimeUnit};
use yggdryl_schema::{DataType, Field};

use crate::scalar::Scalar;
use crate::serie::{Serie, SerieRef, TypedSerie};
use crate::temporal::TemporalSerie;

/// A lazy nanosecond timestamp range: `instant(i) = start + i * step`, for `len` rows.
/// The instants are timezone-naive (UTC); use [`DatetimeSerie`](crate::DatetimeSerie)
/// for a zoned timestamp column.
#[derive(Debug, Clone)]
pub struct DateTimeRangeSerie {
    field: Field,
    start_nanos: i128,
    step_nanos: i128,
    len: usize,
}

impl DateTimeRangeSerie {
    /// A timestamp range named `name` of `len` instants, from `start`, stepping `step`
    /// each row.
    pub fn new(
        name: impl Into<String>,
        start: &DateTime,
        step: &Duration,
        len: usize,
    ) -> DateTimeRangeSerie {
        DateTimeRangeSerie {
            field: Field::new(name, DataType::timestamp(TimeUnit::Nanosecond, None), false),
            start_nanos: start.epoch_nanos(),
            step_nanos: step.as_nanos(),
            len,
        }
    }

    /// The step between consecutive instants.
    pub fn step(&self) -> Duration {
        Duration::from_nanos(self.step_nanos)
    }

    /// The instant at `index` in nanoseconds since the epoch (no bounds check).
    /// Saturating **and clamped to the `i64` range** — the physical storage of a
    /// nanosecond `Timestamp` — so the lazy value, `value_at` and the materialised
    /// array always agree (no wrapping i128→i64 truncation).
    fn at_nanos(&self, index: usize) -> i128 {
        self.start_nanos
            .saturating_add((index as i128).saturating_mul(self.step_nanos))
            .clamp(i64::MIN as i128, i64::MAX as i128)
    }

    /// The [`DateTime`] at `index`, or `None` when out of bounds.
    pub fn datetime(&self, index: usize) -> Option<DateTime> {
        (index < self.len).then(|| DateTime::from_epoch_nanos(self.at_nanos(index), None))
    }
}

impl Serie for DateTimeRangeSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn array(&self) -> ArrayRef {
        Arc::new(TimestampNanosecondArray::from_iter_values(
            (0..self.len).map(|i| self.at_nanos(i) as i64),
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
            Scalar::Int(self.at_nanos(index))
        }
    }

    /// A sub-range — still lazy (no materialisation).
    fn slice(&self, offset: usize, length: usize) -> SerieRef {
        Arc::new(DateTimeRangeSerie {
            field: Field::new(
                self.field.name(),
                DataType::timestamp(TimeUnit::Nanosecond, None),
                false,
            ),
            start_nanos: self.at_nanos(offset),
            step_nanos: self.step_nanos,
            len: length,
        })
    }
}

impl TypedSerie<DateTime> for DateTimeRangeSerie {
    fn get(&self, index: usize) -> Option<DateTime> {
        self.datetime(index)
    }

    fn iter(&self) -> Box<dyn Iterator<Item = Option<DateTime>> + '_> {
        Box::new((0..self.len).map(move |i| self.datetime(i)))
    }
}

impl TemporalSerie for DateTimeRangeSerie {
    fn datetime_at(&self, index: usize) -> Option<DateTime> {
        self.datetime(index)
    }
}
