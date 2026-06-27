//! The `Timezone` pyclass (UTC / fixed offset / named IANA zone with DST rules).

use pyo3::prelude::*;
use pyo3::types::PyType;
use yggdryl_core::Timezone as CoreTimezone;

use crate::{hash_str, time_err};

/// A timezone: ``"UTC"``, a ``"+HH:MM"`` offset, or a named IANA zone (DST-aware).
#[pyclass(name = "Timezone", module = "yggdryl")]
#[derive(Clone)]
pub struct Timezone {
    pub(crate) inner: CoreTimezone,
}

#[pymethods]
impl Timezone {
    /// Parse ``"UTC"`` / ``"Z"``, a ``±HH:MM`` offset, an IANA name or a POSIX TZ string.
    #[new]
    fn new(name: &str) -> PyResult<Self> {
        CoreTimezone::from_str(name)
            .map(|inner| Timezone { inner })
            .map_err(time_err)
    }

    /// Alias for the constructor.
    #[staticmethod]
    fn from_str(name: &str) -> PyResult<Self> {
        Timezone::new(name)
    }

    /// The UTC zone.
    #[staticmethod]
    fn utc() -> Self {
        Timezone {
            inner: CoreTimezone::Utc,
        }
    }

    /// A fixed offset east of UTC, in seconds.
    #[staticmethod]
    fn fixed(offset_seconds: i32) -> Self {
        Timezone {
            inner: if offset_seconds == 0 {
                CoreTimezone::Utc
            } else {
                CoreTimezone::Fixed(offset_seconds)
            },
        }
    }

    /// The offset east of UTC (seconds) at the given UTC epoch second (DST-aware).
    fn offset_seconds(&self, utc_epoch_seconds: i64) -> i32 {
        self.inner.offset_seconds(utc_epoch_seconds)
    }

    /// The canonical name / offset string.
    #[getter]
    fn name(&self) -> String {
        self.inner.name()
    }

    /// Whether this is UTC.
    #[getter]
    fn is_utc(&self) -> bool {
        self.inner.is_utc()
    }

    /// Whether this is a fixed offset.
    #[getter]
    fn is_fixed(&self) -> bool {
        self.inner.is_fixed()
    }

    fn __str__(&self) -> String {
        self.inner.to_str()
    }

    fn __repr__(&self) -> String {
        format!("Timezone('{}')", self.inner.name())
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        hash_str(&self.inner.name())
    }

    fn __reduce__<'py>(&self, py: Python<'py>) -> (Bound<'py, PyType>, (String,)) {
        (py.get_type_bound::<Self>(), (self.inner.name(),))
    }
}
