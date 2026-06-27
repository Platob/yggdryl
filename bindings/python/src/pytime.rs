//! The `Time` pyclass — a time of day with nanosecond resolution.

use pyo3::basic::CompareOp;
use pyo3::prelude::*;
use pyo3::types::PyType;
use std::collections::BTreeMap;
use yggdryl_core::{Temporal, Time as CoreTime};

use crate::datetime::DateTime;
use crate::duration::Duration;
use crate::time_err;

/// A time of day (no date or timezone), with nanosecond resolution.
#[pyclass(name = "Time", module = "yggdryl")]
#[derive(Clone)]
pub struct Time {
    pub(crate) inner: CoreTime,
}

#[pymethods]
impl Time {
    /// Build from ``hour:minute:second`` plus optional sub-second nanoseconds.
    #[new]
    #[pyo3(signature = (hour, minute, second, nano = 0))]
    fn new(hour: u32, minute: u32, second: u32, nano: u32) -> PyResult<Self> {
        CoreTime::from_hms_nano(hour, minute, second, nano)
            .map(|inner| Time { inner })
            .map_err(time_err)
    }

    /// Parse ``HH:MM[:SS[.fraction]]`` (or compact ``HHMM`` / ``HHMMSS``), raising on
    /// malformed input.
    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        CoreTime::from_str(value)
            .map(|inner| Time { inner })
            .map_err(time_err)
    }

    /// Build from a dict (``hour`` / ``minute`` / ``second`` / ``nanosecond``).
    #[staticmethod]
    fn from_mapping(fields: BTreeMap<String, String>) -> PyResult<Self> {
        CoreTime::from_mapping(&fields)
            .map(|inner| Time { inner })
            .map_err(time_err)
    }

    /// Build from a :class:`DateTime` (its local time-of-day) — the `Temporal` redirect.
    #[staticmethod]
    fn from_datetime(value: &DateTime) -> Self {
        Time {
            inner: CoreTime::from_datetime(&value.inner),
        }
    }

    /// Parse from the UTF-8 bytes of the canonical string.
    #[staticmethod]
    fn from_bytes(data: Vec<u8>) -> PyResult<Self> {
        CoreTime::from_bytes(&data)
            .map(|inner| Time { inner })
            .map_err(time_err)
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

    /// Nanoseconds since midnight.
    #[getter]
    fn nanos_of_day(&self) -> u64 {
        self.inner.nanos_of_day()
    }

    /// This time of day on the UNIX-epoch day as a naive :class:`DateTime`.
    fn to_datetime(&self) -> DateTime {
        DateTime {
            inner: self.inner.to_datetime(),
        }
    }

    /// This time advanced by a :class:`Duration`, wrapping around midnight.
    fn add(&self, span: &Duration) -> Self {
        Time {
            inner: self.inner.add(&span.inner),
        }
    }

    /// This time moved back by a :class:`Duration`, wrapping around midnight.
    fn sub(&self, span: &Duration) -> Self {
        Time {
            inner: self.inner.sub(&span.inner),
        }
    }

    /// The signed within-day :class:`Duration` from `other` to ``self``.
    fn duration_since(&self, other: &Time) -> Duration {
        Duration {
            inner: self.inner.duration_since(&other.inner),
        }
    }

    /// This time-of-day floored to a multiple of `unit` since midnight.
    fn truncate(&self, unit: &Duration) -> Self {
        Time {
            inner: self.inner.truncate(&unit.inner),
        }
    }

    fn __add__(&self, span: &Duration) -> Self {
        self.add(span)
    }

    fn __sub__(&self, span: &Duration) -> Self {
        self.sub(span)
    }

    /// Render to a dict (``hour`` / ``minute`` / ``second`` / ``nanosecond``).
    fn to_mapping(&self) -> BTreeMap<String, String> {
        self.inner.to_mapping()
    }

    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, pyo3::types::PyBytes> {
        pyo3::types::PyBytes::new_bound(py, &self.inner.to_bytes())
    }

    fn __str__(&self) -> String {
        self.inner.to_str()
    }

    fn __repr__(&self) -> String {
        format!("Time('{}')", self.inner.to_str())
    }

    fn __richcmp__(&self, other: &Self, op: CompareOp) -> bool {
        op.matches(self.inner.cmp(&other.inner))
    }

    fn __hash__(&self) -> u64 {
        self.inner.nanos_of_day()
    }

    fn __reduce__<'py>(&self, py: Python<'py>) -> (Bound<'py, PyType>, (u32, u32, u32, u32)) {
        (
            py.get_type_bound::<Self>(),
            (
                self.inner.hour(),
                self.inner.minute(),
                self.inner.second(),
                self.inner.nanosecond(),
            ),
        )
    }
}
