//! The `Duration` pyclass — a signed span of time (nanoseconds).

use pyo3::basic::CompareOp;
use pyo3::prelude::*;
use pyo3::types::PyType;
use yggdryl_core::{Duration as CoreDuration, Mapping};

use crate::{time_err, time_unit_from};

/// A signed span of time with nanosecond resolution.
#[pyclass(name = "Duration", module = "yggdryl")]
#[derive(Clone)]
pub struct Duration {
    pub(crate) inner: CoreDuration,
}

#[pymethods]
impl Duration {
    /// Build from a count of nanoseconds.
    #[new]
    #[pyo3(signature = (nanos = 0))]
    fn new(nanos: i128) -> Self {
        Duration {
            inner: CoreDuration::from_nanos(nanos),
        }
    }

    /// Parse a compact span (``"1h30m"`` / ``"1s500ms"`` / ``"-2d"``) or a number
    /// of seconds.
    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        CoreDuration::from_str(value)
            .map(|inner| Duration { inner })
            .map_err(time_err)
    }

    /// Parse flexibly (compact / ISO-8601 / seconds); with ``raise_error=False``
    /// return ``None`` instead of raising.
    #[staticmethod]
    #[pyo3(signature = (value, raise_error = true))]
    fn parse(value: &str, raise_error: bool) -> PyResult<Option<Self>> {
        match CoreDuration::from_str(value) {
            Ok(inner) => Ok(Some(Duration { inner })),
            Err(e) if raise_error => Err(time_err(e)),
            Err(_) => Ok(None),
        }
    }

    /// A span of `seconds` seconds.
    #[staticmethod]
    fn from_secs(seconds: i64) -> Self {
        Duration {
            inner: CoreDuration::from_secs(seconds),
        }
    }

    /// A span of `millis` milliseconds.
    #[staticmethod]
    fn from_millis(millis: i64) -> Self {
        Duration {
            inner: CoreDuration::from_millis(millis),
        }
    }

    /// A span of `nanos` nanoseconds.
    #[staticmethod]
    fn from_nanos(nanos: i128) -> Self {
        Duration {
            inner: CoreDuration::from_nanos(nanos),
        }
    }

    /// A span of `value` of the given unit (``"s"`` / ``"ms"`` / ``"us"`` / ``"ns"``).
    #[staticmethod]
    fn from_unit(value: i64, unit: &str) -> PyResult<Self> {
        Ok(Duration {
            inner: CoreDuration::from_unit(value, time_unit_from(unit)?),
        })
    }

    /// Build from a dict (``nanoseconds``).
    #[staticmethod]
    fn from_mapping(fields: Mapping) -> PyResult<Self> {
        CoreDuration::from_mapping(&fields)
            .map(|inner| Duration { inner })
            .map_err(time_err)
    }

    /// The whole seconds (truncated toward zero).
    fn as_seconds(&self) -> i64 {
        self.inner.as_seconds()
    }

    /// The total milliseconds.
    fn as_millis(&self) -> i128 {
        self.inner.as_millis()
    }

    /// The total nanoseconds.
    fn as_nanos(&self) -> i128 {
        self.inner.as_nanos()
    }

    /// The span as fractional seconds.
    fn as_seconds_f64(&self) -> f64 {
        self.inner.as_seconds_f64()
    }

    #[getter]
    fn is_zero(&self) -> bool {
        self.inner.is_zero()
    }

    #[getter]
    fn is_negative(&self) -> bool {
        self.inner.is_negative()
    }

    /// The absolute (non-negative) span.
    fn abs(&self) -> Self {
        Duration {
            inner: self.inner.abs(),
        }
    }

    /// The negated span.
    fn negate(&self) -> Self {
        Duration {
            inner: self.inner.negate(),
        }
    }

    fn __add__(&self, other: &Self) -> Self {
        Duration {
            inner: self.inner.add(&other.inner),
        }
    }

    fn __sub__(&self, other: &Self) -> Self {
        Duration {
            inner: self.inner.sub(&other.inner),
        }
    }

    /// Render to a dict (``nanoseconds``).
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
        format!("Duration('{}')", self.inner.to_str())
    }

    fn __richcmp__(&self, other: &Self, op: CompareOp) -> bool {
        op.matches(self.inner.cmp(&other.inner))
    }

    fn __hash__(&self) -> i64 {
        self.inner.as_nanos() as i64
    }

    fn __reduce__<'py>(&self, py: Python<'py>) -> (Bound<'py, PyType>, (i128,)) {
        (py.get_type_bound::<Self>(), (self.inner.as_nanos(),))
    }

    /// A span of `micros` microseconds.
    #[staticmethod]
    fn from_micros(micros: i64) -> Self {
        Duration {
            inner: CoreDuration::from_micros(micros),
        }
    }
}

impl Duration {
    /// Build a wrapper from a core [`CoreDuration`] (used by other modules).
    #[allow(dead_code)]
    pub(crate) fn wrap(inner: CoreDuration) -> Duration {
        Duration { inner }
    }
}
