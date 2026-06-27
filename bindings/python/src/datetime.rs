//! The `DateTime` pyclass — an absolute instant with an optional display timezone.

use pyo3::basic::CompareOp;
use pyo3::prelude::*;
use yggdryl_core::{DateTime as CoreDateTime, Mapping, Timezone as CoreTimezone};

use crate::date::Date;
use crate::pytime::Time;
use crate::timezone::Timezone;
use crate::{hash_str, time_err};

/// An instant in time (UTC seconds + nanoseconds) with an optional display
/// :class:`Timezone`; DST-aware conversions. A naive instance (no zone) is UTC.
#[pyclass(name = "DateTime", module = "yggdryl")]
#[derive(Clone)]
pub struct DateTime {
    pub(crate) inner: CoreDateTime,
}

/// Resolves an optional zone name to a core [`CoreTimezone`].
fn zone(tz: Option<&str>) -> PyResult<Option<CoreTimezone>> {
    match tz {
        Some(name) => CoreTimezone::from_str(name).map(Some).map_err(time_err),
        None => Ok(None),
    }
}

#[pymethods]
impl DateTime {
    /// Build from civil components in `timezone` (a zone name, or ``None`` = naive/UTC).
    #[new]
    #[pyo3(signature = (year, month, day, hour = 0, minute = 0, second = 0, nano = 0, timezone = None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
        nano: u32,
        timezone: Option<&str>,
    ) -> PyResult<Self> {
        CoreDateTime::from_ymd_hms(
            year,
            month,
            day,
            hour,
            minute,
            second,
            nano,
            zone(timezone)?,
        )
        .map(|inner| DateTime { inner })
        .map_err(time_err)
    }

    /// Parse an ISO-8601 datetime (``Z`` / ``±HH:MM`` offset, or none for naive).
    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        CoreDateTime::from_str(value)
            .map(|inner| DateTime { inner })
            .map_err(time_err)
    }

    /// The current instant in UTC.
    #[staticmethod]
    fn now() -> Self {
        DateTime {
            inner: CoreDateTime::now(),
        }
    }

    /// Build from UTC epoch seconds, with an optional display zone.
    #[staticmethod]
    #[pyo3(signature = (seconds, timezone = None))]
    fn from_epoch_seconds(seconds: i64, timezone: Option<&str>) -> PyResult<Self> {
        Ok(DateTime {
            inner: CoreDateTime::from_epoch_seconds(seconds, zone(timezone)?),
        })
    }

    /// Build from UTC epoch nanoseconds, with an optional display zone.
    #[staticmethod]
    #[pyo3(signature = (nanos, timezone = None))]
    fn from_epoch_nanos(nanos: i128, timezone: Option<&str>) -> PyResult<Self> {
        Ok(DateTime {
            inner: CoreDateTime::from_epoch_nanos(nanos, zone(timezone)?),
        })
    }

    /// Build from a dict (date/time components plus optional ``timezone``).
    #[staticmethod]
    fn from_mapping(fields: Mapping) -> PyResult<Self> {
        CoreDateTime::from_mapping(&fields)
            .map(|inner| DateTime { inner })
            .map_err(time_err)
    }

    /// Parse from the UTF-8 bytes of the canonical string.
    #[staticmethod]
    fn from_bytes(data: Vec<u8>) -> PyResult<Self> {
        CoreDateTime::from_bytes(&data)
            .map(|inner| DateTime { inner })
            .map_err(time_err)
    }

    #[getter]
    fn epoch_seconds(&self) -> i64 {
        self.inner.epoch_seconds()
    }

    #[getter]
    fn epoch_millis(&self) -> i64 {
        self.inner.epoch_millis()
    }

    #[getter]
    fn epoch_nanos(&self) -> i128 {
        self.inner.epoch_nanos()
    }

    #[getter]
    fn year(&self) -> i32 {
        self.inner.year()
    }

    #[getter]
    fn month(&self) -> u32 {
        self.inner.month()
    }

    #[getter]
    fn day(&self) -> u32 {
        self.inner.day()
    }

    #[getter]
    fn hour(&self) -> u32 {
        self.inner.hour()
    }

    #[getter]
    fn minute(&self) -> u32 {
        self.inner.minute()
    }

    #[getter]
    fn second(&self) -> u32 {
        self.inner.second()
    }

    #[getter]
    fn nanosecond(&self) -> u32 {
        self.inner.nanosecond()
    }

    /// The offset east of UTC (seconds) at this instant in its display zone.
    #[getter]
    fn offset_seconds(&self) -> i32 {
        self.inner.offset_seconds()
    }

    /// The display :class:`Timezone`, or ``None`` if naive.
    #[getter]
    fn timezone(&self) -> Option<Timezone> {
        self.inner
            .timezone()
            .cloned()
            .map(|inner| Timezone { inner })
    }

    /// The local :class:`Date`.
    fn date(&self) -> Date {
        Date {
            inner: self.inner.date(),
        }
    }

    /// The local :class:`Time`.
    fn time(&self) -> Time {
        Time {
            inner: self.inner.time(),
        }
    }

    /// The same instant displayed in `timezone`.
    fn to_timezone(&self, timezone: &str) -> PyResult<Self> {
        let tz = CoreTimezone::from_str(timezone).map_err(time_err)?;
        Ok(DateTime {
            inner: self.inner.to_timezone(tz),
        })
    }

    /// The same instant displayed in UTC.
    fn to_utc(&self) -> Self {
        DateTime {
            inner: self.inner.to_utc(),
        }
    }

    /// Render to a component dict.
    fn to_mapping(&self) -> Mapping {
        self.inner.to_mapping()
    }

    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, pyo3::types::PyBytes> {
        pyo3::types::PyBytes::new_bound(py, &self.inner.to_bytes())
    }

    fn __str__(&self) -> String {
        self.inner.to_str()
    }

    fn __repr__(&self) -> String {
        format!("DateTime('{}')", self.inner.to_str())
    }

    fn __richcmp__(&self, other: &Self, op: CompareOp) -> bool {
        op.matches(self.inner.cmp(&other.inner))
    }

    fn __hash__(&self) -> u64 {
        hash_str(&self.inner.to_str())
    }

    /// Reconstruct losslessly from the canonical ISO string.
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<(PyObject, (String,))> {
        let from_str = py.get_type_bound::<Self>().getattr("from_str")?;
        Ok((from_str.into(), (self.inner.to_str(),)))
    }
}
