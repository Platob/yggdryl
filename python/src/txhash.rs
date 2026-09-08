//! An instant coupled with an xxHash digest, and the columns that carry them.
//!
//! [`PyTxHash`] is the value: a unix count, UTC, at a clock resolution, and
//! the digest behind it. [`PyTxHasher`] is one configuration - resolution,
//! algorithm, seed, secret - applied to bytes, values, rows, and batches.
//! Every `unix` argument reads the same way: an `int` is the count already,
//! and a `datetime`, a `date`, a `str`, or a native `Scalar` is read through
//! the core's one instant intake, so a caller hands over whatever names the
//! instant and the resolution is the only thing settled here.

use arrow_array::RecordBatch as ArrowRecordBatch;
use arrow_pyarrow::{FromPyArrow, ToPyArrow};
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyDate, PyDateTime, PyTuple, PyType};

use yggdryl::txhash::{self, TxHash, TxHasher};
use yggdryl::xxhash::{Xxh3, Xxh32, Xxh64, Xxh128};
use yggdryl::{Digester, Scalar, TimeUnit, Timezone};

use crate::types::datatype::{PyDataType, arrow_array_from_pyarrow, arrow_array_to_pyarrow};
use crate::types::field::core_field_from_value;
use crate::types::scalar::{PyScalar, date_epoch_days, datetime_utc_microseconds};
use crate::value_error;
use crate::xxhash::{
    PyDigest, PyDigester, PyXxh3, PyXxh32, PyXxh64, PyXxh128, algorithm_from_str, feed_content,
};

/// Register this module's classes and functions on the native module.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyTxHash>()?;
    module.add_class::<PyTxHasher>()?;
    module.add_function(pyo3::wrap_pyfunction!(txh32, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(txh64, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(txh3, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(txh128, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(txhash_digest, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(txhash_row_txhashes, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(txhash_column_txhashes, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(txhash_compose, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(txhash_decompose, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(txhash_unix_array, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(txhash_unix_now, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(txhash_unix_of, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(txhash_restate_unix, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(txhash_width, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(txhash_dtype, module)?)?;
    Ok(())
}

/// Parse a clock resolution, keeping the core's message.
pub(crate) fn unit_from_str(value: &str) -> PyResult<TimeUnit> {
    TimeUnit::from_str(value).map_err(value_error)
}

/// Read any Python spelling of an instant as a unix count of `unit`.
///
/// An `int` is the count already. A `bool` is refused before it can pass as
/// one, because `True` is no instant. Everything else - a `datetime`, a
/// `date`, timestamp text, a native `Scalar` - crosses into a `Scalar` and
/// takes the core's one intake, so the accepted spellings are its.
pub(crate) fn unix_from_py(value: &Bound<'_, PyAny>, unit: TimeUnit) -> PyResult<i64> {
    if value.is_instance_of::<PyBool>() {
        return Err(PyTypeError::new_err("a bool names no instant"));
    }
    if let Ok(count) = value.extract::<i64>() {
        return txhash::unix_from_scalar(&Scalar::from(count), unit).map_err(value_error);
    }
    // A datetime and a date cross through the conversion every Scalar intake
    // uses, minus the zone: a unix count has none, so an aware value is read
    // under UTC rather than resolving the zone its `tzinfo` names.
    if value.is_instance_of::<PyDateTime>() {
        let (count, aware) = datetime_utc_microseconds(value)?;
        let zone = if aware {
            Timezone::UTC
        } else {
            Timezone::NAIVE
        };
        let instant =
            Scalar::from_datetime(count, TimeUnit::Microsecond, zone).map_err(value_error)?;
        return txhash::unix_from_scalar(&instant, unit).map_err(value_error);
    }
    if value.is_instance_of::<PyDate>() {
        let day = Scalar::from_date(date_epoch_days(value)?, TimeUnit::Day, Timezone::NAIVE)
            .map_err(value_error)?;
        return txhash::unix_from_scalar(&day, unit).map_err(value_error);
    }
    let scalar = match value.extract::<PyRef<'_, PyScalar>>() {
        Ok(scalar) => scalar.inner.clone(),
        Err(_) => crate::types::scalar::from_py(value)?,
    };
    txhash::unix_from_scalar(&scalar, unit).map_err(value_error)
}

/// One instant coupled with one digest.
///
/// `bytes(value)` is the canonical layout - the instant big-endian, then the
/// digest - and `str(value)` the `<unix>@<unit>:<algorithm>:<hex>` spelling
/// the constructor reads back. Values order by unit, instant, then digest,
/// which is the order their bytes sort in from the epoch on.
#[pyclass(
    name = "TxHash",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyTxHash {
    inner: TxHash,
}

impl PyTxHash {
    pub(crate) const fn from_core(inner: TxHash) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyTxHash {
    /// Parse the canonical `<unix>@<unit>:<algorithm>:<hex>` spelling.
    #[new]
    fn new(value: &str) -> PyResult<Self> {
        TxHash::from_str(value)
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// Rebuild a value from its canonical bytes.
    #[classmethod]
    fn from_bytes(
        _cls: &Bound<'_, PyType>,
        unit: &str,
        algorithm: &str,
        data: &[u8],
    ) -> PyResult<Self> {
        TxHash::from_bytes(unit_from_str(unit)?, algorithm_from_str(algorithm)?, data)
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// Couple an instant with a digest already computed.
    ///
    /// `unix` and `unit` are the keywords every binding spells, so the two
    /// names stay beside each other here.
    #[classmethod]
    #[pyo3(signature = (unix, digest, unit = "us"))]
    #[allow(clippy::similar_names)]
    fn from_parts(
        _cls: &Bound<'_, PyType>,
        unix: &Bound<'_, PyAny>,
        digest: &PyDigest,
        unit: &str,
    ) -> PyResult<Self> {
        let unit = unit_from_str(unit)?;
        let unix = unix_from_py(unix, unit)?;
        TxHash::new_in(unix, unit, digest.inner())
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// The instant as a unix count of `unit`.
    #[getter]
    fn unix(&self) -> i64 {
        self.inner.unix()
    }

    /// The clock resolution the instant is counted in.
    #[getter]
    fn unit(&self) -> &'static str {
        self.inner.unit().as_str()
    }

    /// The digest half, carrying its algorithm.
    #[getter]
    fn digest(&self) -> PyDigest {
        PyDigest::from_core(self.inner.digest())
    }

    /// The canonical algorithm token of the digest half.
    #[getter]
    fn algorithm(&self) -> &'static str {
        self.inner.algorithm().as_str()
    }

    /// The width of the canonical bytes.
    #[getter]
    fn width(&self) -> usize {
        self.inner.width()
    }

    /// The datatype a column of values like this one is stored under.
    #[getter]
    fn dtype(&self) -> PyDataType {
        PyDataType::from_inner(self.inner.dtype())
    }

    /// Restate the instant at another clock resolution, keeping the digest.
    fn with_unit(&self, unit: &str) -> PyResult<Self> {
        self.inner
            .with_unit(unit_from_str(unit)?)
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// The instant as a UTC datetime `Scalar` at this value's resolution.
    #[allow(clippy::wrong_self_convention)] // Binding `into_*` methods do not consume wrappers.
    fn into_datetime(&self) -> PyScalar {
        PyScalar {
            inner: self.inner.into_datetime(),
        }
    }

    /// The canonical bytes as a fixed-width byte `Scalar`.
    #[allow(clippy::wrong_self_convention)] // Binding `into_*` methods do not consume wrappers.
    fn into_scalar(&self) -> PyScalar {
        PyScalar {
            inner: self.inner.into_scalar(),
        }
    }

    /// A deterministic cross-language hash of this value.
    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.into_bytes())
    }

    fn __len__(&self) -> usize {
        self.inner.width()
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("TxHash({:?})", self.inner.to_string())
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(
        &self,
        other: &Bound<'_, PyAny>,
        operation: pyo3::basic::CompareOp,
    ) -> PyResult<Py<PyAny>> {
        let py = other.py();
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(py.NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(py)?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (String,)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (self.inner.to_string(),),
        )
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// One resolution and one digest configuration, applied to many values.
///
/// The one-shot functions answer at microseconds with the default seed; this
/// is the form for another resolution, a seed, an XXH3 secret carried in
/// through a configured state, or an algorithm read from configuration.
#[pyclass(name = "TxHasher", module = "yggdryl._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyTxHasher {
    inner: TxHasher,
}

#[pymethods]
impl PyTxHasher {
    /// Start a hasher for one algorithm at one resolution, optionally seeded.
    #[new]
    #[pyo3(signature = (algorithm = "xxh3-64", *, unit = "us", seed = None))]
    fn new(algorithm: &str, unit: &str, seed: Option<u64>) -> PyResult<Self> {
        let hasher = TxHasher::new_in(unit_from_str(unit)?, algorithm_from_str(algorithm)?)
            .map_err(value_error)?;
        Ok(Self {
            inner: match seed {
                Some(seed) => hasher.with_seed(seed),
                None => hasher,
            },
        })
    }

    /// Start from a configured state, keeping its algorithm, seed, and secret.
    ///
    /// This is how a custom XXH3 secret reaches a hasher: build `Xxh3` or
    /// `Xxh128` with it and hand it over. Bytes already fed to the state are
    /// discarded.
    #[classmethod]
    #[pyo3(signature = (state, *, unit = "us"))]
    fn from_state(
        _cls: &Bound<'_, PyType>,
        state: &Bound<'_, PyAny>,
        unit: &str,
    ) -> PyResult<Self> {
        let digester = digester_from_state(state)?;
        TxHasher::from_digester(unit_from_str(unit)?, digester)
            .map(|inner| Self { inner })
            .map_err(value_error)
    }

    /// The clock resolution every answer counts its instant in.
    #[getter]
    fn unit(&self) -> &'static str {
        self.inner.unit().as_str()
    }

    /// The canonical algorithm token every answer's digest half computes.
    #[getter]
    fn algorithm(&self) -> &'static str {
        self.inner.algorithm().as_str()
    }

    /// The width of every answer's canonical bytes.
    #[getter]
    fn width(&self) -> usize {
        self.inner.width()
    }

    /// The datatype a column of answers is stored under.
    #[getter]
    fn dtype(&self) -> PyDataType {
        PyDataType::from_inner(self.inner.dtype())
    }

    /// Couple an instant with the digest of raw bytes, a string, or a buffer.
    fn digest(&self, data: &Bound<'_, PyAny>, unix: &Bound<'_, PyAny>) -> PyResult<PyTxHash> {
        let unix = unix_from_py(unix, self.inner.unit())?;
        let mut state = self.inner.digester().clone();
        feed_content(data, &mut |bytes| state.write_bytes(bytes))?;
        TxHash::new_in(unix, self.inner.unit(), state.as_digest())
            .map(PyTxHash::from_core)
            .map_err(value_error)
    }

    /// Couple an instant with a value's canonical feed.
    fn digest_scalar(&self, value: &PyScalar, unix: &Bound<'_, PyAny>) -> PyResult<PyTxHash> {
        let unix = unix_from_py(unix, self.inner.unit())?;
        Ok(PyTxHash::from_core(
            self.inner.digest_scalar(&value.inner, unix),
        ))
    }

    /// Read any instant as a unix count of this hasher's resolution.
    fn unix_of(&self, time: &Bound<'_, PyAny>) -> PyResult<i64> {
        unix_from_py(time, self.inner.unit())
    }

    /// Fill default digest holders in one `PyArrow` `RecordBatch`.
    ///
    /// The configured state is the prototype every holder reads its seed and
    /// secret from; a holder coupling an instant keeps its own `digest:unit`.
    #[pyo3(signature = (root, batch, *, force = false))]
    fn apply_arrow_batch<'py>(
        &self,
        py: Python<'py>,
        root: &Bound<'py, PyAny>,
        batch: &Bound<'py, PyAny>,
        force: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let root = core_field_from_value(root)?;
        let batch = ArrowRecordBatch::from_pyarrow_bound(batch)?;
        self.inner
            .apply_arrow_batch(&root, batch, force)
            .map_err(value_error)?
            .to_pyarrow(py)
    }

    /// Couple every row's seeded digest with the instant beside it.
    fn row_txhashes<'py>(
        &self,
        py: Python<'py>,
        batch: &Bound<'py, PyAny>,
        times: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let batch = ArrowRecordBatch::from_pyarrow_bound(batch)?;
        let times = arrow_array_from_pyarrow(times)?;
        let coupled = self
            .inner
            .row_txhashes(&batch, times.as_ref())
            .map_err(value_error)?;
        arrow_array_to_pyarrow(py, &coupled, None)
    }

    /// Couple every cell's seeded digest with the instant beside it.
    fn column_txhashes<'py>(
        &self,
        py: Python<'py>,
        times: &Bound<'py, PyAny>,
        array: &Bound<'py, PyAny>,
        field: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let times = arrow_array_from_pyarrow(times)?;
        let values = arrow_array_from_pyarrow(array)?;
        let field = core_field_from_value(field)?;
        let coupled = self
            .inner
            .column_txhashes(times.as_ref(), values, &field)
            .map_err(value_error)?;
        arrow_array_to_pyarrow(py, &coupled, None)
    }

    fn __repr__(&self) -> String {
        format!(
            "TxHasher({:?}, unit={:?})",
            self.inner.algorithm().as_str(),
            self.inner.unit().as_str()
        )
    }

    // A configuration is mutable through nothing, but its equality is not
    // stated either, so it stays unhashable like the states it wraps.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// Read the state a hasher is started from: one of the four named states,
/// or the runtime dispatcher.
fn digester_from_state(state: &Bound<'_, PyAny>) -> PyResult<Digester> {
    if let Ok(state) = state.extract::<PyRef<'_, PyDigester>>() {
        return Ok(state.digester());
    }
    if let Ok(state) = state.extract::<PyRef<'_, PyXxh32>>() {
        return Ok(state.digester());
    }
    if let Ok(state) = state.extract::<PyRef<'_, PyXxh64>>() {
        return Ok(state.digester());
    }
    if let Ok(state) = state.extract::<PyRef<'_, PyXxh3>>() {
        return Ok(state.digester());
    }
    if let Ok(state) = state.extract::<PyRef<'_, PyXxh128>>() {
        return Ok(state.digester());
    }
    Err(PyTypeError::new_err(
        "expected an Xxh32, Xxh64, Xxh3, Xxh128, or Digester state",
    ))
}

/// Couple a microsecond instant with a one-shot digest of `data`.
fn couple(unix: &Bound<'_, PyAny>, digest: yggdryl::Digest) -> PyResult<PyTxHash> {
    let unix = unix_from_py(unix, txhash::DEFAULT_UNIT)?;
    Ok(PyTxHash::from_core(TxHash::new(unix, digest)))
}

/// Couple a microsecond instant with XXH32 of a complete value.
#[pyfunction]
#[pyo3(name = "txh32", signature = (data, unix, seed = 0))]
pub(crate) fn txh32(
    data: &Bound<'_, PyAny>,
    unix: &Bound<'_, PyAny>,
    seed: u32,
) -> PyResult<PyTxHash> {
    let mut state = Xxh32::with_seed(seed);
    feed_content(data, &mut |bytes| state.write_bytes(bytes))?;
    couple(unix, state.as_digest())
}

/// Couple a microsecond instant with XXH64 of a complete value.
#[pyfunction]
#[pyo3(name = "txh64", signature = (data, unix, seed = 0))]
pub(crate) fn txh64(
    data: &Bound<'_, PyAny>,
    unix: &Bound<'_, PyAny>,
    seed: u64,
) -> PyResult<PyTxHash> {
    let mut state = Xxh64::with_seed(seed);
    feed_content(data, &mut |bytes| state.write_bytes(bytes))?;
    couple(unix, state.as_digest())
}

/// Couple a microsecond instant with XXH3-64 of a complete value.
#[pyfunction]
#[pyo3(name = "txh3", signature = (data, unix, seed = 0, secret = None))]
pub(crate) fn txh3(
    data: &Bound<'_, PyAny>,
    unix: &Bound<'_, PyAny>,
    seed: u64,
    secret: Option<&[u8]>,
) -> PyResult<PyTxHash> {
    let mut state = match secret {
        Some(secret) => Xxh3::from_seed_and_secret(seed, secret).map_err(value_error)?,
        None => Xxh3::with_seed(seed),
    };
    feed_content(data, &mut |bytes| state.write_bytes(bytes))?;
    couple(unix, state.as_digest())
}

/// Couple a microsecond instant with XXH3-128 of a complete value.
#[pyfunction]
#[pyo3(name = "txh128", signature = (data, unix, seed = 0, secret = None))]
pub(crate) fn txh128(
    data: &Bound<'_, PyAny>,
    unix: &Bound<'_, PyAny>,
    seed: u64,
    secret: Option<&[u8]>,
) -> PyResult<PyTxHash> {
    let mut state = match secret {
        Some(secret) => Xxh128::from_seed_and_secret(seed, secret).map_err(value_error)?,
        None => Xxh128::with_seed(seed),
    };
    feed_content(data, &mut |bytes| state.write_bytes(bytes))?;
    couple(unix, state.as_digest())
}

/// Couple a microsecond instant with a complete value's digest.
#[pyfunction]
#[pyo3(name = "txhash_digest")]
pub(crate) fn txhash_digest(
    data: &Bound<'_, PyAny>,
    unix: &Bound<'_, PyAny>,
    algorithm: &str,
) -> PyResult<PyTxHash> {
    let mut state = algorithm_from_str(algorithm)?.digester();
    feed_content(data, &mut |bytes| state.write_bytes(bytes))?;
    couple(unix, state.as_digest())
}

/// Couple every row's digest with the instant beside it.
///
/// The digest half is `xxhash.row_digests` of the batch; the instant column
/// is any timestamp, date, or integer array of the batch's length, read at
/// `unit`. The answer is a `fixed_size_binary` of the coupled width, null
/// where the instant is.
#[pyfunction]
#[pyo3(name = "txhash_row_txhashes", signature = (batch, times, unit = "us", algorithm = "xxh3-64"))]
pub(crate) fn txhash_row_txhashes<'py>(
    py: Python<'py>,
    batch: &Bound<'py, PyAny>,
    times: &Bound<'py, PyAny>,
    unit: &str,
    algorithm: &str,
) -> PyResult<Bound<'py, PyAny>> {
    let batch = ArrowRecordBatch::from_pyarrow_bound(batch)?;
    let times = arrow_array_from_pyarrow(times)?;
    let coupled = txhash::arrow::row_txhashes(
        &batch,
        times.as_ref(),
        unit_from_str(unit)?,
        algorithm_from_str(algorithm)?,
    )
    .map_err(value_error)?;
    arrow_array_to_pyarrow(py, &coupled, None)
}

/// Couple every cell's digest with the instant beside it.
#[pyfunction]
#[pyo3(name = "txhash_column_txhashes", signature = (times, array, field, unit = "us", algorithm = "xxh3-64"))]
pub(crate) fn txhash_column_txhashes<'py>(
    py: Python<'py>,
    times: &Bound<'py, PyAny>,
    array: &Bound<'py, PyAny>,
    field: &Bound<'py, PyAny>,
    unit: &str,
    algorithm: &str,
) -> PyResult<Bound<'py, PyAny>> {
    let times = arrow_array_from_pyarrow(times)?;
    let values = arrow_array_from_pyarrow(array)?;
    let field = core_field_from_value(field)?;
    let coupled = txhash::arrow::column_txhashes(
        times.as_ref(),
        values,
        &field,
        unit_from_str(unit)?,
        algorithm_from_str(algorithm)?,
    )
    .map_err(value_error)?;
    arrow_array_to_pyarrow(py, &coupled, None)
}

/// Couple an instant column with a digest column already computed.
#[pyfunction]
#[pyo3(name = "txhash_compose", signature = (times, digests, unit = "us", algorithm = "xxh3-64"))]
pub(crate) fn txhash_compose<'py>(
    py: Python<'py>,
    times: &Bound<'py, PyAny>,
    digests: &Bound<'py, PyAny>,
    unit: &str,
    algorithm: &str,
) -> PyResult<Bound<'py, PyAny>> {
    let times = arrow_array_from_pyarrow(times)?;
    let digests = arrow_array_from_pyarrow(digests)?;
    let coupled = txhash::arrow::compose(
        times.as_ref(),
        digests.as_ref(),
        unit_from_str(unit)?,
        algorithm_from_str(algorithm)?,
    )
    .map_err(value_error)?;
    arrow_array_to_pyarrow(py, &coupled, None)
}

/// Split a coupled column into its UTC timestamp column and its digest column.
#[pyfunction]
#[pyo3(name = "txhash_decompose", signature = (array, unit = "us", algorithm = "xxh3-64"))]
pub(crate) fn txhash_decompose<'py>(
    py: Python<'py>,
    array: &Bound<'py, PyAny>,
    unit: &str,
    algorithm: &str,
) -> PyResult<Bound<'py, PyTuple>> {
    let array = arrow_array_from_pyarrow(array)?;
    let (times, digests) = txhash::arrow::decompose(
        array.as_ref(),
        unit_from_str(unit)?,
        algorithm_from_str(algorithm)?,
    )
    .map_err(value_error)?;
    PyTuple::new(
        py,
        [
            arrow_array_to_pyarrow(py, &times, None)?,
            arrow_array_to_pyarrow(py, &digests, None)?,
        ],
    )
}

/// Read a timestamp, date, or integer column as unix counts of `unit`.
#[pyfunction]
#[pyo3(name = "txhash_unix_array", signature = (array, unit = "us"))]
pub(crate) fn txhash_unix_array<'py>(
    py: Python<'py>,
    array: &Bound<'py, PyAny>,
    unit: &str,
) -> PyResult<Bound<'py, PyAny>> {
    let array = arrow_array_from_pyarrow(array)?;
    let counts =
        txhash::arrow::unix_array(array.as_ref(), unit_from_str(unit)?).map_err(value_error)?;
    arrow_array_to_pyarrow(
        py,
        &(std::sync::Arc::new(counts) as arrow_array::ArrayRef),
        None,
    )
}

/// Read the system clock as a unix count of `unit`.
#[pyfunction]
#[pyo3(name = "txhash_unix_now", signature = (unit = "us"))]
pub(crate) fn txhash_unix_now(unit: &str) -> PyResult<i64> {
    txhash::unix_now(unit_from_str(unit)?).map_err(value_error)
}

/// Read any instant as a unix count of `unit`.
#[pyfunction]
#[pyo3(name = "txhash_unix_of", signature = (value, unit = "us"))]
pub(crate) fn txhash_unix_of(value: &Bound<'_, PyAny>, unit: &str) -> PyResult<i64> {
    unix_from_py(value, unit_from_str(unit)?)
}

/// Restate a count of one resolution as a count of another.
#[pyfunction]
#[pyo3(name = "txhash_restate_unix", signature = (count, from_unit, into_unit))]
pub(crate) fn txhash_restate_unix(count: i64, from_unit: &str, into_unit: &str) -> PyResult<i64> {
    txhash::restate_unix(count, unit_from_str(from_unit)?, unit_from_str(into_unit)?)
        .map_err(value_error)
}

/// The width of a value coupling an instant with an algorithm's digest.
#[pyfunction]
#[pyo3(name = "txhash_width")]
pub(crate) fn txhash_width(algorithm: &str) -> PyResult<usize> {
    Ok(txhash::width(algorithm_from_str(algorithm)?))
}

/// The datatype a column of coupled values is stored under.
#[pyfunction]
#[pyo3(name = "txhash_dtype")]
pub(crate) fn txhash_dtype(algorithm: &str) -> PyResult<PyDataType> {
    Ok(PyDataType::from_inner(txhash::dtype(algorithm_from_str(
        algorithm,
    )?)))
}
