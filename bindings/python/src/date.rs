//! The `Date` pyclass — a proleptic-Gregorian calendar date.

use pyo3::basic::CompareOp;
use pyo3::prelude::*;
use yggdryl_core::{Date as CoreDate, Mapping, Temporal, Timezone as CoreTimezone};

use crate::datetime::DateTime;
use crate::duration::Duration;
use crate::pytime::Time;
use crate::time_err;
use crate::timezone::Timezone;

/// A calendar date (no time of day or timezone), stored as days since the epoch.
#[pyclass(name = "Date", module = "yggdryl")]
#[derive(Clone)]
pub struct Date {
    pub(crate) inner: CoreDate,
}

#[pymethods]
impl Date {
    /// Build from ``(year, month, day)``, validating the calendar.
    #[new]
    fn new(year: i32, month: u32, day: u32) -> PyResult<Self> {
        CoreDate::from_ymd(year, month, day)
            .map(|inner| Date { inner })
            .map_err(time_err)
    }

    /// Parse a date flexibly (ISO ``YYYY-MM-DD``, ``YYYY/MM/DD``, compact
    /// ``YYYYMMDD`` or a full datetime), raising on malformed input.
    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        CoreDate::from_str(value)
            .map(|inner| Date { inner })
            .map_err(time_err)
    }

    /// Build from a :class:`DateTime` (its local calendar date) — the `Temporal`
    /// redirect.
    #[staticmethod]
    fn from_datetime(value: &DateTime) -> Self {
        Date {
            inner: CoreDate::from_datetime(&value.inner),
        }
    }

    /// Build from a count of days since the UNIX epoch.
    #[staticmethod]
    fn from_epoch_days(days: i32) -> Self {
        Date {
            inner: CoreDate::from_epoch_days(days),
        }
    }

    /// Build from a dict (``year`` / ``month`` / ``day``).
    #[staticmethod]
    fn from_mapping(fields: Mapping) -> PyResult<Self> {
        CoreDate::from_mapping(&fields)
            .map(|inner| Date { inner })
            .map_err(time_err)
    }

    /// Parse from the UTF-8 bytes of the canonical string.
    #[staticmethod]
    fn from_bytes(data: Vec<u8>) -> PyResult<Self> {
        CoreDate::from_bytes(&data)
            .map(|inner| Date { inner })
            .map_err(time_err)
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

    /// The day of week (0 = Sunday … 6 = Saturday).
    #[getter]
    fn weekday(&self) -> u32 {
        self.inner.weekday()
    }

    /// Days since the UNIX epoch.
    #[getter]
    fn epoch_days(&self) -> i32 {
        self.inner.epoch_days()
    }

    /// A copy `days` days later (or earlier, if negative).
    fn add_days(&self, days: i32) -> Self {
        Date {
            inner: self.inner.add_days(days),
        }
    }

    /// This date advanced by a :class:`Duration`'s whole days.
    fn add(&self, span: &Duration) -> Self {
        Date {
            inner: self.inner.add(&span.inner),
        }
    }

    /// This date moved back by a :class:`Duration`'s whole days.
    fn sub(&self, span: &Duration) -> Self {
        Date {
            inner: self.inner.sub(&span.inner),
        }
    }

    /// The signed whole-day :class:`Duration` from `other` to ``self``.
    fn duration_since(&self, other: &Date) -> Duration {
        Duration {
            inner: self.inner.duration_since(&other.inner),
        }
    }

    /// This date floored to a multiple of `unit` (whole days) since the epoch.
    fn truncate(&self, unit: &Duration) -> Self {
        Date {
            inner: self.inner.truncate(&unit.inner),
        }
    }

    fn __add__(&self, span: &Duration) -> Self {
        self.add(span)
    }

    fn __sub__(&self, span: &Duration) -> Self {
        self.sub(span)
    }

    /// The timezone this date is anchored to, if any.
    #[getter]
    fn timezone(&self) -> Option<Timezone> {
        self.inner
            .timezone()
            .cloned()
            .map(|inner| Timezone { inner })
    }

    /// A copy anchored to the named timezone.
    fn with_timezone(&self, timezone: &str) -> PyResult<Self> {
        let tz = CoreTimezone::from_str(timezone).map_err(time_err)?;
        Ok(Date {
            inner: self.inner.clone().with_timezone(tz),
        })
    }

    /// A copy with no timezone.
    fn without_timezone(&self) -> Self {
        Date {
            inner: self.inner.clone().without_timezone(),
        }
    }

    /// Midnight on this date (in its timezone) as a :class:`DateTime`.
    fn to_datetime(&self) -> DateTime {
        DateTime {
            inner: self.inner.to_datetime(),
        }
    }

    /// Combine with a :class:`Time` into a :class:`DateTime` in the date's zone.
    fn at(&self, time: &Time) -> DateTime {
        DateTime {
            inner: self.inner.at(time.inner),
        }
    }

    /// Render to a dict (``year`` / ``month`` / ``day``).
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
        format!("Date('{}')", self.inner.to_str())
    }

    fn __richcmp__(&self, other: &Self, op: CompareOp) -> bool {
        op.matches(self.inner.cmp(&other.inner))
    }

    fn __hash__(&self) -> i64 {
        self.inner.epoch_days() as i64
    }

    /// Reconstruct losslessly (incl. timezone) through the component mapping.
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<(PyObject, (Mapping,))> {
        let from_mapping = py.get_type_bound::<Self>().getattr("from_mapping")?;
        Ok((from_mapping.into(), (self.inner.to_mapping(),)))
    }
}
