//! [`DatetimeSerie`] — the unified timestamp column. It backs an Arrow timestamp array
//! of **any** [`TimeUnit`] (second / millisecond / microsecond / nanosecond) and an
//! optional [`Timezone`], presenting its values through the core
//! [`DateTime`](yggdryl_core::DateTime). It replaces the per-unit timestamp aliases.

use std::any::Any;

use arrow_array::{
    Array, ArrayRef, TimestampMicrosecondArray, TimestampMillisecondArray,
    TimestampNanosecondArray, TimestampSecondArray,
};
use yggdryl_core::{DateTime, TimeUnit, Timezone};
use yggdryl_schema::Field;

use crate::error::{SerieError, SerieResult};
use crate::serie::{Serie, TypedSerie};
use crate::temporal::TemporalSerie;

/// A timestamp column over any [`TimeUnit`], exposing values as core
/// [`DateTime`](yggdryl_core::DateTime). The unit and optional [`Timezone`] are read
/// from the column's [`Field`].
#[derive(Debug, Clone)]
pub struct DatetimeSerie {
    field: Field,
    array: ArrayRef,
    unit: TimeUnit,
    timezone: Option<Timezone>,
}

impl DatetimeSerie {
    /// Wraps a field and a matching timestamp array (no validation) — the path the
    /// [factory](crate::from_arrow) uses after it has checked the types. The unit and
    /// timezone are taken from the field.
    pub(crate) fn from_parts(field: Field, array: ArrayRef) -> DatetimeSerie {
        let unit = field
            .data_type()
            .time_unit()
            .unwrap_or(TimeUnit::Nanosecond);
        let timezone = field.data_type().timezone().cloned();
        DatetimeSerie {
            field,
            array,
            unit,
            timezone,
        }
    }

    /// Builds from a [`Field`] (which must be a `timestamp`) and an Arrow timestamp
    /// array, checking the field's type maps to the array's Arrow type.
    pub fn new(field: Field, array: ArrayRef) -> SerieResult<DatetimeSerie> {
        let expected = field.data_type().to_arrow()?;
        if &expected != array.data_type() {
            return Err(SerieError::TypeMismatch {
                expected: expected.to_string(),
                found: array.data_type().to_string(),
            });
        }
        Ok(DatetimeSerie::from_parts(field, array))
    }

    /// The column's [`TimeUnit`] resolution.
    pub fn unit(&self) -> TimeUnit {
        self.unit
    }

    /// The column's display [`Timezone`], if zoned.
    pub fn timezone(&self) -> Option<&Timezone> {
        self.timezone.as_ref()
    }

    /// The raw physical value at `index` (a count of `unit`s since the epoch), or
    /// `None` when null / out of bounds.
    pub fn physical_at(&self, index: usize) -> Option<i64> {
        if !self.is_valid(index) {
            return None;
        }
        let value = match self.unit {
            TimeUnit::Second => self.downcast::<TimestampSecondArray>().value(index),
            TimeUnit::Millisecond => self.downcast::<TimestampMillisecondArray>().value(index),
            TimeUnit::Microsecond => self.downcast::<TimestampMicrosecondArray>().value(index),
            TimeUnit::Nanosecond => self.downcast::<TimestampNanosecondArray>().value(index),
        };
        Some(value)
    }

    /// Downcasts the backing array to a concrete timestamp array (panics only on an
    /// internal type mismatch, which the constructors prevent).
    fn downcast<A: 'static>(&self) -> &A {
        self.array
            .as_any()
            .downcast_ref::<A>()
            .expect("the timestamp array matches the unit")
    }
}

impl Serie for DatetimeSerie {
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

impl TypedSerie<DateTime> for DatetimeSerie {
    fn get(&self, index: usize) -> Option<DateTime> {
        self.datetime_at(index)
    }

    fn iter(&self) -> Box<dyn Iterator<Item = Option<DateTime>> + '_> {
        Box::new((0..self.len()).map(move |i| self.datetime_at(i)))
    }
}

impl TemporalSerie for DatetimeSerie {
    fn datetime_at(&self, index: usize) -> Option<DateTime> {
        let physical = self.physical_at(index)?;
        let nanos = (physical as i128) * (self.unit.nanos() as i128);
        Some(DateTime::from_epoch_nanos(nanos, self.timezone.clone()))
    }
}
