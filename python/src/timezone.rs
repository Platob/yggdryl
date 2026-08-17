//! `Timezone`, exposed to Python beside the zone objects it interoperates with.
//!
//! Python has three ways of naming a zone - a `zoneinfo.ZoneInfo`, a
//! `datetime.timezone` fixed offset, and a bare string - and a binding that
//! accepted only one of them would push the conversion onto every caller. All
//! three are accepted here and canonicalized by the core, so a schema written
//! from `ZoneInfo("Asia/Calcutta")` and one written from `"Asia/Kolkata"`
//! compare equal, which they do not in Python itself.

use pyo3::prelude::*;
use pyo3::types::{PyTuple, PyType};

use yggdryl::Timezone;

use crate::value_error;

/// Read a core zone out of anything Python uses to name one.
///
/// Accepts a `Timezone`, a string, anything exposing an IANA `key` (which is
/// what `zoneinfo.ZoneInfo` does), and any `datetime.tzinfo` whose
/// `utcoffset(None)` is a fixed offset.
pub(crate) fn core_timezone_from_value(value: &Bound<'_, PyAny>) -> PyResult<Timezone> {
    if let Ok(zone) = value.extract::<PyRef<'_, PyTimezone>>() {
        return Ok(zone.inner.clone());
    }
    if let Ok(name) = value.extract::<String>() {
        return Timezone::from_str(&name).map_err(value_error);
    }
    // `zoneinfo.ZoneInfo` carries the IANA name on `key`, which is exactly the
    // spelling the core registry indexes.
    if let Ok(key) = value.getattr("key")
        && let Ok(name) = key.extract::<String>()
    {
        return Timezone::from_str(&name).map_err(value_error);
    }
    // Any other tzinfo is usable only if its offset does not depend on the
    // instant, because a zone that varies cannot be named by one offset.
    if let Ok(offset) = value.call_method1("utcoffset", (py_none(value.py()),))
        && !offset.is_none()
        && let Ok(seconds) = offset.call_method0("total_seconds")
        && let Ok(seconds) = seconds.extract::<f64>()
    {
        #[allow(clippy::cast_possible_truncation)]
        return Timezone::from_offset(seconds as i32).map_err(value_error);
    }
    Err(pyo3::exceptions::PyTypeError::new_err(
        "expected a Timezone, an IANA name, a zoneinfo.ZoneInfo, or a fixed-offset tzinfo",
    ))
}

/// Borrow Python's `None` for a call that needs it positionally.
fn py_none(py: Python<'_>) -> Py<PyAny> {
    py.None()
}

/// A canonical IANA time zone, with the offset rules this build knows.
#[pyclass(name = "Timezone", module = "yggdryl._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyTimezone {
    inner: Timezone,
}

impl PyTimezone {
    pub(crate) fn from_core(inner: Timezone) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyTimezone {
    /// Parse and canonicalize a zone name, alias, or fixed offset.
    #[new]
    fn new(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(core_timezone_from_value(value)?))
    }

    /// Parse a zone from anything that names one.
    #[classmethod]
    fn from_value(_cls: &Bound<'_, PyType>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        Self::new(value)
    }

    /// Build a zone from a fixed offset east of UTC, in seconds.
    #[classmethod]
    fn from_offset(_cls: &Bound<'_, PyType>, seconds: i32) -> PyResult<Self> {
        Timezone::from_offset(seconds)
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// Every zone this build knows the rules for, sorted by name.
    #[classmethod]
    fn registered<'py>(
        _cls: &Bound<'py, PyType>,
        py: Python<'py>,
    ) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, Timezone::registered().map(Self::from_core))
    }

    /// Every alias and the canonical name it resolves to.
    #[classmethod]
    fn aliases<'py>(_cls: &Bound<'py, PyType>, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, Timezone::aliases())
    }

    /// Coordinated Universal Time.
    #[classattr]
    #[allow(non_snake_case)]
    fn UTC() -> Self {
        Self::from_core(Timezone::UTC)
    }

    /// The canonical name.
    #[getter]
    fn key(&self) -> &str {
        // Named `key` to match `zoneinfo.ZoneInfo`, so the two are duck-type
        // compatible wherever a caller only reads the name.
        self.inner.as_str()
    }

    /// Whether this zone is UTC itself.
    fn is_utc(&self) -> bool {
        self.inner.is_utc()
    }

    /// Whether this build knows the offset rules for this zone.
    fn is_known(&self) -> bool {
        self.inner.is_known()
    }

    /// Whether the name is a fixed offset rather than a place.
    fn is_fixed(&self) -> bool {
        self.inner.is_fixed()
    }

    /// Whether this zone ever observes daylight saving.
    fn observes_saving(&self) -> bool {
        self.inner.observes_saving()
    }

    /// The offset east of UTC, in seconds, at an instant.
    ///
    /// `epoch` is seconds since the Unix epoch. Returns `None` for a zone this
    /// build has no rules for, rather than guessing.
    fn offset_at(&self, epoch: i64) -> Option<i32> {
        self.inner.offset_at(epoch)
    }

    /// The standard offset east of UTC, ignoring daylight saving.
    #[getter]
    fn standard_offset(&self) -> Option<i32> {
        self.inner.standard_offset()
    }

    /// Whether daylight saving is in force at an instant.
    fn is_saving_at(&self, epoch: i64) -> Option<bool> {
        self.inner.is_saving_at(epoch)
    }

    /// The abbreviation in use at an instant, such as `EST` or `CEST`.
    fn abbreviation_at(&self, epoch: i64) -> Option<&'static str> {
        self.inner.abbreviation_at(epoch)
    }

    /// The local reading, in epoch seconds, of a UTC instant.
    fn to_local(&self, epoch: i64) -> PyResult<i64> {
        self.inner.to_local(epoch).map_err(value_error)
    }

    /// The UTC instant a local reading names.
    fn to_utc(&self, local: i64) -> PyResult<i64> {
        self.inner.to_utc(local).map_err(value_error)
    }

    /// The offset as a `datetime.timedelta`, as `tzinfo.utcoffset` returns.
    ///
    /// This is the duck-typing half: a `Timezone` can stand in for a `tzinfo`
    /// wherever the caller only needs the offset at a known instant.
    #[pyo3(signature = (epoch = 0))]
    fn utcoffset<'py>(&self, py: Python<'py>, epoch: i64) -> PyResult<Option<Bound<'py, PyAny>>> {
        let Some(offset) = self.inner.offset_at(epoch) else {
            return Ok(None);
        };
        let datetime = py.import("datetime")?;
        let seconds = datetime.call_method1("timedelta", (0, offset))?;
        Ok(Some(seconds))
    }

    /// A deterministic cross-language hash of the canonical name.
    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __str__(&self) -> &str {
        self.inner.as_str()
    }

    fn __repr__(&self) -> String {
        format!("Timezone({:?})", self.inner.as_str())
    }

    fn __hash__(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __richcmp__(
        &self,
        other: &Bound<'_, PyAny>,
        operation: pyo3::basic::CompareOp,
    ) -> PyResult<Py<PyAny>> {
        // Comparing against a plain name or a ZoneInfo is the point: the core
        // canonicalizes both, so two spellings of one zone compare equal.
        let py = other.py();
        let Ok(other) = core_timezone_from_value(other) else {
            return Ok(py.NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other), operation)
            .into_pyobject(py)?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (String,)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (self.inner.as_str().to_owned(),),
        )
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}
