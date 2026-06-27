//! [`TimeSerie`] — the unified time-of-day column. It backs an Arrow time array of
//! **any** [`TimeUnit`] (`Time32` second/millisecond, `Time64` microsecond/nanosecond)
//! and presents its values through the core [`Time`](yggdryl_core::Time). It replaces
//! the per-unit time aliases, respecting the [`Time`](yggdryl_schema::DataType::Time)
//! data type.

use std::any::Any;
use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, Time32MillisecondArray, Time32SecondArray, Time64MicrosecondArray,
    Time64NanosecondArray,
};
use yggdryl_core::{DateTime, Temporal, Time, TimeUnit};
use yggdryl_schema::{DataType, Field};

use crate::error::{SerieError, SerieResult};
use crate::serie::{Serie, TypedSerie};
use crate::temporal::TemporalSerie;

/// A time-of-day column over any [`TimeUnit`], exposing values as core
/// [`Time`](yggdryl_core::Time). The unit is read from the column's [`Field`].
#[derive(Debug, Clone)]
pub struct TimeSerie {
    field: Field,
    array: ArrayRef,
    unit: TimeUnit,
}

impl TimeSerie {
    /// Wraps a field and a matching time array (no validation) — the path the
    /// [factory](crate::from_arrow) uses after it has checked the types.
    pub(crate) fn from_parts(field: Field, array: ArrayRef) -> TimeSerie {
        let unit = field
            .data_type()
            .time_unit()
            .unwrap_or(TimeUnit::Nanosecond);
        TimeSerie { field, array, unit }
    }

    /// Builds from a [`Field`] (which must be a `time`) and an Arrow time array,
    /// checking the field's type maps to the array's Arrow type.
    pub fn new(field: Field, array: ArrayRef) -> SerieResult<TimeSerie> {
        let expected = field.data_type().to_arrow()?;
        if &expected != array.data_type() {
            return Err(SerieError::TypeMismatch {
                expected: expected.to_string(),
                found: array.data_type().to_string(),
            });
        }
        Ok(TimeSerie::from_parts(field, array))
    }

    /// Builds a nanosecond time-of-day column named `name` from an iterator of optional
    /// [`Time`]s — the one-line constructor.
    pub fn from_values(
        name: impl Into<String>,
        values: impl IntoIterator<Item = Option<Time>>,
    ) -> TimeSerie {
        let array = Time64NanosecondArray::from_iter(
            values
                .into_iter()
                .map(|opt| opt.map(|t| t.nanos_of_day() as i64)),
        );
        let field = Field::new(
            name,
            DataType::Time {
                unit: TimeUnit::Nanosecond,
            },
            true,
        );
        TimeSerie::from_parts(field, Arc::new(array))
    }

    /// The column's [`TimeUnit`] resolution.
    pub fn unit(&self) -> TimeUnit {
        self.unit
    }

    /// The raw physical value at `index` (a count of `unit`s since midnight), or `None`
    /// when null / out of bounds.
    pub fn physical_at(&self, index: usize) -> Option<i64> {
        if !self.is_valid(index) {
            return None;
        }
        let value = match self.unit {
            TimeUnit::Second => self.downcast::<Time32SecondArray>().value(index) as i64,
            TimeUnit::Millisecond => self.downcast::<Time32MillisecondArray>().value(index) as i64,
            TimeUnit::Microsecond => self.downcast::<Time64MicrosecondArray>().value(index),
            TimeUnit::Nanosecond => self.downcast::<Time64NanosecondArray>().value(index),
        };
        Some(value)
    }

    /// The [`Time`] at `index`, or `None` when null / out of bounds (or the physical
    /// value is not a valid time of day).
    pub fn time_at(&self, index: usize) -> Option<Time> {
        let physical = self.physical_at(index)?;
        let nanos = (physical as i128) * (self.unit.nanos() as i128);
        Time::from_nanos_of_day(u64::try_from(nanos).ok()?).ok()
    }

    fn downcast<A: 'static>(&self) -> &A {
        self.array
            .as_any()
            .downcast_ref::<A>()
            .expect("the time array matches the unit")
    }
}

impl Serie for TimeSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn array(&self) -> ArrayRef {
        self.array.clone()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn len(&self) -> usize {
        self.array.len()
    }

    fn null_count(&self) -> usize {
        self.array.null_count()
    }

    fn is_null(&self, index: usize) -> bool {
        index >= self.array.len() || self.array.is_null(index)
    }
}

impl TypedSerie<Time> for TimeSerie {
    fn get(&self, index: usize) -> Option<Time> {
        self.time_at(index)
    }

    fn iter(&self) -> Box<dyn Iterator<Item = Option<Time>> + '_> {
        Box::new((0..self.len()).map(move |i| self.time_at(i)))
    }
}

impl TemporalSerie for TimeSerie {
    fn datetime_at(&self, index: usize) -> Option<DateTime> {
        self.time_at(index).map(|t| t.to_datetime())
    }
}
