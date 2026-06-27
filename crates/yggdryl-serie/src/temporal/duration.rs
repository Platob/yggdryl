//! [`DurationSerie`] — the unified elapsed-time column. It backs an Arrow duration
//! array of **any** [`TimeUnit`] and presents its values through the core
//! [`Duration`](yggdryl_core::Duration), respecting the
//! [`Duration`](yggdryl_schema::DataType::Duration) data type. (A duration is a *span*,
//! not an instant, so it is not a [`TemporalSerie`](crate::TemporalSerie).)

use std::any::Any;
use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, DurationMicrosecondArray, DurationMillisecondArray, DurationNanosecondArray,
    DurationSecondArray,
};
use yggdryl_core::{Duration, TimeUnit};
use yggdryl_schema::{DataType, Field};

use crate::error::{SerieError, SerieResult};
use crate::serie::{Serie, TypedSerie};

/// An elapsed-time column over any [`TimeUnit`], exposing values as core
/// [`Duration`](yggdryl_core::Duration). The unit is read from the column's [`Field`].
#[derive(Debug, Clone)]
pub struct DurationSerie {
    field: Field,
    array: ArrayRef,
    unit: TimeUnit,
}

impl DurationSerie {
    /// Wraps a field and a matching duration array (no validation) — the path the
    /// [factory](crate::from_arrow) uses after it has checked the types.
    pub(crate) fn from_parts(field: Field, array: ArrayRef) -> DurationSerie {
        let unit = field
            .data_type()
            .time_unit()
            .unwrap_or(TimeUnit::Nanosecond);
        DurationSerie { field, array, unit }
    }

    /// Builds from a [`Field`] (which must be a `duration`) and an Arrow duration array,
    /// checking the field's type maps to the array's Arrow type.
    pub fn new(field: Field, array: ArrayRef) -> SerieResult<DurationSerie> {
        let expected = field.data_type().to_arrow()?;
        if &expected != array.data_type() {
            return Err(SerieError::TypeMismatch {
                expected: expected.to_string(),
                found: array.data_type().to_string(),
            });
        }
        Ok(DurationSerie::from_parts(field, array))
    }

    /// Builds a nanosecond duration column named `name` from an iterator of optional
    /// [`Duration`]s — the one-line constructor (spans are clamped to the `i64`
    /// nanosecond range).
    pub fn from_values(
        name: impl Into<String>,
        values: impl IntoIterator<Item = Option<Duration>>,
    ) -> DurationSerie {
        let array =
            DurationNanosecondArray::from_iter(values.into_iter().map(|opt| {
                opt.map(|d| d.as_nanos().clamp(i64::MIN as i128, i64::MAX as i128) as i64)
            }));
        let field = Field::new(
            name,
            DataType::Duration {
                unit: TimeUnit::Nanosecond,
            },
            true,
        );
        DurationSerie::from_parts(field, Arc::new(array))
    }

    /// The column's [`TimeUnit`] resolution.
    pub fn unit(&self) -> TimeUnit {
        self.unit
    }

    /// The raw physical value at `index` (a count of `unit`s), or `None` when null /
    /// out of bounds.
    pub fn physical_at(&self, index: usize) -> Option<i64> {
        if !self.is_valid(index) {
            return None;
        }
        let value = match self.unit {
            TimeUnit::Second => self.downcast::<DurationSecondArray>().value(index),
            TimeUnit::Millisecond => self.downcast::<DurationMillisecondArray>().value(index),
            TimeUnit::Microsecond => self.downcast::<DurationMicrosecondArray>().value(index),
            TimeUnit::Nanosecond => self.downcast::<DurationNanosecondArray>().value(index),
        };
        Some(value)
    }

    /// The [`Duration`] at `index`, or `None` when null / out of bounds.
    pub fn duration_at(&self, index: usize) -> Option<Duration> {
        self.physical_at(index)
            .map(|v| Duration::from_unit(v, self.unit))
    }

    fn downcast<A: 'static>(&self) -> &A {
        self.array
            .as_any()
            .downcast_ref::<A>()
            .expect("the duration array matches the unit")
    }
}

impl Serie for DurationSerie {
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

impl TypedSerie<Duration> for DurationSerie {
    fn get(&self, index: usize) -> Option<Duration> {
        self.duration_at(index)
    }

    fn iter(&self) -> Box<dyn Iterator<Item = Option<Duration>> + '_> {
        Box::new((0..self.len()).map(move |i| self.duration_at(i)))
    }
}
