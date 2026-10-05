//! Native Python view of [`CoreCandle`], [`CoreCandleOptions`] and the
//! [`CoreCandleIterator`] walk: one OHLC per book cross code and bucket over
//! a sorted stream of books.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Mutex, PoisonError};

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::sync::MutexExt;
use pyo3::types::{PyBool, PyDict, PyInt};

use yggdryl::graph::{
    BookEvent as CoreBookEvent, Candle as CoreCandle, CandleIterator as CoreCandleIterator,
    CandleOptions as CoreCandleOptions, Ohlc,
};

use super::book::PyBookEvent;
use super::decimal_scalar;
use super::market_data::market_data_of;
use crate::field::PyField;
use crate::scalar::{PyScalar, from_py, is_mapping, struct_from_entries};
use crate::timezone::{PyTimezone, core_timezone_from_value};
use crate::{Failed, Pulled, python_failure, python_hash, value_error};

/// One OHLC of one book over one bucket: what the books of one cross code
/// whose instants fell in `[start, end)` read at their best bid, their best
/// ask, their midpoint and their spread, the quantities resting at the touch
/// when the bucket closed, and how many books it folded. Immutable; built by
/// `CandleIterator`, `candles` or `from_scalar`.
#[pyclass(
    name = "Candle",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyCandle {
    pub(crate) inner: CoreCandle,
}

impl PyCandle {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: CoreCandle) -> Self {
        Self { inner }
    }
}

/// One reading as the `dict` Python reads: its `open`, `high`, `low` and
/// `close`, each a decimal `Scalar`, in that order; `None` for no reading.
fn reading_dict(py: Python<'_>, reading: Option<Ohlc>) -> PyResult<Option<Bound<'_, PyDict>>> {
    let Some(reading) = reading else {
        return Ok(None);
    };
    let cells = PyDict::new(py);
    for (name, value) in [
        ("open", reading.open),
        ("high", reading.high),
        ("low", reading.low),
        ("close", reading.close),
    ] {
        cells.set_item(name, decimal_scalar(value))?;
    }
    Ok(Some(cells))
}

#[pymethods]
impl PyCandle {
    /// The book's stored cross code, such as `3:0:ACME`.
    #[getter]
    fn crosscode(&self) -> &str {
        self.inner.crosscode.as_str()
    }

    /// The book's ticker, where the bucket's first book stated one.
    #[getter]
    fn ticker(&self) -> Option<&str> {
        self.inner.ticker.as_deref()
    }

    /// The bucket's start: nanoseconds since the Unix epoch, UTC.
    #[getter]
    fn start(&self) -> i64 {
        self.inner.start
    }

    /// The bucket's end, exclusive: nanoseconds since the Unix epoch, UTC.
    #[getter]
    fn end(&self) -> i64 {
        self.inner.end
    }

    /// The best bid over the bucket - `open`, `high`, `low` and `close`,
    /// each a decimal `Scalar` - or `None` where no book stated one.
    #[getter]
    fn bid<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        reading_dict(py, self.inner.bid)
    }

    /// The best ask over the bucket, read as `bid` is.
    #[getter]
    fn ask<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        reading_dict(py, self.inner.ask)
    }

    /// The midpoint of the best bid and offer over the bucket, read as `bid`
    /// is; `None` where no book stated both sides.
    #[getter]
    fn mid<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        reading_dict(py, self.inner.mid)
    }

    /// The spread over the bucket, read as `bid` is; `None` where no book
    /// stated both sides.
    #[getter]
    fn spread<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        reading_dict(py, self.inner.spread)
    }

    /// The quantity resting at the best bid when the bucket closed, as a
    /// decimal; `None` where the last book had none.
    #[getter]
    fn bidqty(&self) -> Option<PyScalar> {
        self.inner.bidqty.map(decimal_scalar)
    }

    /// The quantity resting at the best ask when the bucket closed, as a
    /// decimal; `None` where the last book had none.
    #[getter]
    fn askqty(&self) -> Option<PyScalar> {
        self.inner.askqty.map(decimal_scalar)
    }

    /// How many books folded into the bucket.
    #[getter]
    fn books(&self) -> u64 {
        self.inner.books
    }

    /// The required struct `candle` every candle row is laid out under:
    /// `crosscode`, `ticker`, `start`, `end`, the four cells of each reading
    /// (`bidopen` .. `spreadclose`), `bidqty`, `askqty` and `books`.
    #[staticmethod]
    fn field() -> PyResult<PyField> {
        CoreCandle::field()
            .map(PyField::from_inner)
            .map_err(value_error)
    }

    /// The candle as the named struct `Scalar` of its twenty-three cells, an
    /// absent ticker, reading or quantity a null.
    #[allow(clippy::wrong_self_convention)] // Python `into_*` methods do not consume wrappers.
    fn into_scalar(&self) -> PyScalar {
        PyScalar::from_inner(self.inner.into_scalar())
    }

    /// A candle read back from that named struct - a `Scalar`, or a mapping
    /// of the same names, a name left out a null - or from the ordered row
    /// `field()`'s value door answers; the value passes that door once.
    #[staticmethod]
    fn from_scalar(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        // A mapping crosses `from_py` as a map, and a map is what a Python
        // mapping is; the struct a candle is named by is this door's own
        // shape, so a mapping here takes `Scalar.from_struct`'s door instead.
        let scalar = if is_mapping(value)? {
            struct_from_entries(value)?
        } else {
            from_py(value)?
        };
        CoreCandle::from_scalar(&scalar)
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// The candle's cells as native Python values keyed by name: the `dict`
    /// `into_scalar().as_py()` reads.
    fn as_py(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        crate::scalar::as_py(py, &self.inner.into_scalar())
    }

    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> Py<PyAny> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return py.NotImplemented();
        };
        PyBool::new(py, self.inner == other.inner)
            .to_owned()
            .into_any()
            .unbind()
    }

    /// Hashes the core value, which equal candles share.
    fn __hash__(&self) -> isize {
        let mut state = DefaultHasher::new();
        self.inner.hash(&mut state);
        python_hash(state.finish())
    }

    fn __repr__(&self) -> String {
        format!(
            "Candle({:?}, start={}, end={}, books={})",
            self.inner.crosscode.as_str(),
            self.inner.start,
            self.inner.end,
            self.inner.books,
        )
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (PyScalar,))> {
        Ok((
            py.get_type::<Self>().getattr("from_scalar")?.unbind(),
            (self.into_scalar(),),
        ))
    }
}

/// How instants are bucketed: the interval and the zone whose wall clock
/// the buckets align to, so a daily candle opens at local midnight and hourly
/// candles follow a saving-time change. Immutable.
#[pyclass(
    name = "CandleOptions",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyCandleOptions {
    pub(crate) inner: CoreCandleOptions,
}

impl PyCandleOptions {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: CoreCandleOptions) -> Self {
        Self { inner }
    }
}

/// The options `value` spells - a `CandleOptions`, an `int` of nanoseconds
/// or a spelling such as `"1m"` - aligned to `timezone` where one is given.
pub(crate) fn candle_options_of(
    value: &Bound<'_, PyAny>,
    timezone: Option<&Bound<'_, PyAny>>,
) -> PyResult<CoreCandleOptions> {
    let options = if let Ok(options) = value.extract::<PyRef<'_, PyCandleOptions>>() {
        options.inner.clone()
    } else if let Ok(spelling) = value.extract::<&str>() {
        CoreCandleOptions::from_spelling(spelling).map_err(value_error)?
    } else if value.is_instance_of::<PyBool>() {
        return Err(PyTypeError::new_err(
            "interval must be an int of nanoseconds or a spelling such as '1m', not bool",
        ));
    } else if value.is_instance_of::<PyInt>() {
        CoreCandleOptions::new(value.extract::<i64>()?).map_err(value_error)?
    } else {
        return Err(PyTypeError::new_err(format!(
            "expected CandleOptions, an int of nanoseconds or a spelling such as '1m', got {}",
            value.get_type().name()?
        )));
    };
    match timezone.filter(|zone| !zone.is_none()) {
        Some(zone) => Ok(options.with_timezone(core_timezone_from_value(zone)?)),
        None => Ok(options),
    }
}

#[pymethods]
impl PyCandleOptions {
    /// Buckets of `interval` - an `int` of nanoseconds, a spelling such as
    /// `"30s"`, `"1m"`, `"5m"`, `"1h"`, `"1d"` or `"1w"`, or another
    /// `CandleOptions` - aligned to `timezone`'s wall clock, UTC where none
    /// is given; an interval that is not positive is refused.
    #[new]
    #[pyo3(signature = (interval, timezone=None))]
    fn new(interval: &Bound<'_, PyAny>, timezone: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        candle_options_of(interval, timezone).map(Self::from_core)
    }

    /// The bucket width in nanoseconds of the zone's wall clock.
    #[getter]
    fn interval(&self) -> i64 {
        self.inner.interval()
    }

    /// The zone the buckets align to.
    #[getter]
    fn timezone(&self) -> PyTimezone {
        PyTimezone::from_core(*self.inner.timezone())
    }

    /// The interval as its count and the widest unit that divides it
    /// exactly - `"90s"`, `"2m"`, `"1500ms"` - which the constructor reads
    /// back.
    #[getter]
    fn spelling(&self) -> String {
        self.inner.spelling()
    }

    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> Py<PyAny> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return py.NotImplemented();
        };
        PyBool::new(py, self.inner == other.inner)
            .to_owned()
            .into_any()
            .unbind()
    }

    /// Hashes the interval and the zone, which equal options share.
    fn __hash__(&self) -> isize {
        let mut state = DefaultHasher::new();
        self.inner.hash(&mut state);
        python_hash(state.finish())
    }

    fn __repr__(&self) -> String {
        format!(
            "CandleOptions({:?}, timezone={:?})",
            self.inner.spelling(),
            self.inner.timezone().as_str(),
        )
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (i64, String)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (
                self.inner.interval(),
                self.inner.timezone().as_str().to_owned(),
            ),
        )
    }
}

/// The core walk's source: the caller's items as books, a Python failure
/// crossing as one typed sentinel.
type CandleSource = Box<dyn Iterator<Item = yggdryl::Result<CoreBookEvent>> + Send>;

/// The book `item` is - a `BookEvent`, or a `MarketData` holding one - or a
/// `TypeError` naming what it is instead.
fn book_event_of(item: &Bound<'_, PyAny>) -> PyResult<CoreBookEvent> {
    if let Ok(book) = item.extract::<PyRef<'_, PyBookEvent>>() {
        return Ok(book.inner.clone());
    }
    CoreBookEvent::try_from(market_data_of(item)?)
        .map_err(|error| PyTypeError::new_err(error.to_string()))
}

/// Candles from a sorted stream of books, one per cross code and bucket,
/// pulling its books lazily from the caller's iterable. Yields `Candle`.
#[pyclass(name = "CandleIterator", module = "yggdryl._native", frozen)]
pub(crate) struct PyCandleIterator {
    inner: Mutex<CoreCandleIterator<CandleSource>>,
    failed: Failed,
}

impl PyCandleIterator {
    /// Open the walk over `books` under `options`.
    fn over(books: &Bound<'_, PyAny>, options: CoreCandleOptions) -> PyResult<Self> {
        let pulled = Pulled::new(books, book_event_of)?;
        let failed = pulled.failed.clone();
        // The core walk takes a typed error, not a `PyErr`, so a Python
        // failure crosses it as one sentinel `Err` - peeked, never taken, so
        // `failed` still holds the original for `__next__` to raise as
        // itself once the walk surfaces the sentinel in its place. Yielded
        // exactly once: `Chain` stops calling a side once it answers `None`.
        let source: CandleSource = {
            let sentinel_failed = failed.clone();
            let mut yielded = false;
            Box::new(pulled.map(Ok).chain(std::iter::from_fn(move || {
                if yielded {
                    return None;
                }
                yielded = true;
                Python::attach(|py| sentinel_failed.peek(py))
                    .map(|error| Err(python_failure(error)))
            })))
        };
        Ok(Self {
            inner: Mutex::new(CoreCandleIterator::new(source, options)),
            failed,
        })
    }

    /// The next candle, a Python failure raised as itself.
    ///
    /// The lock is taken and the walk pulled detached: the pull runs the
    /// caller's iterable, which may release the GIL, and a thread waiting on
    /// the lock while attached would keep the puller from taking it back -
    /// two threads calling `next()` on one walk would hang the process.
    /// Each item is read attached, through `Pulled`.
    fn pull(&self, py: Python<'_>) -> PyResult<Option<PyCandle>> {
        let next = py.detach(|| {
            self.inner
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .next()
        });
        match next {
            Some(Ok(candle)) => Ok(Some(PyCandle::from_core(candle))),
            // A core refusal (an unsorted stream) or the sentinel standing in
            // for a Python failure both land here; `self.failed` still holds
            // a genuine Python failure - never taken by the source, only
            // peeked - so it is raised as itself rather than as the typed
            // error it crossed the walk wrapped in.
            Some(Err(error)) => Err(self.failed.take().unwrap_or_else(|| value_error(error))),
            None => match self.failed.take() {
                Some(error) => Err(error),
                None => Ok(None),
            },
        }
    }
}

#[pymethods]
impl PyCandleIterator {
    /// Opens a candle walk over `books` - `BookEvent`s, or `MarketData`
    /// holding one, sorted by their instant; a regression is refused -
    /// bucketed by `options`: a `CandleOptions`, or an `int` of nanoseconds
    /// or a spelling such as `"1m"`, aligned to `timezone` where one is given
    /// - the zone `CandleOptions` and `candles` take by the same name. The
    /// candles of a bucket are yielded in cross-code order when the stream
    /// moves past it and at its end; an empty bucket yields none.
    #[new]
    #[pyo3(signature = (books, options, *, timezone = None))]
    fn new(
        books: &Bound<'_, PyAny>,
        options: &Bound<'_, PyAny>,
        timezone: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        Self::over(books, candle_options_of(options, timezone)?)
    }

    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// The options the walk buckets by.
    #[getter]
    fn options(&self, py: Python<'_>) -> PyCandleOptions {
        // Waited on detached: another thread may hold the lock across a pull
        // of the caller's iterable, which needs the GIL back to finish.
        PyCandleOptions::from_core(
            self.inner
                .lock_py_attached(py)
                .unwrap_or_else(PoisonError::into_inner)
                .options()
                .clone(),
        )
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&self, py: Python<'_>) -> PyResult<Option<PyCandle>> {
        self.pull(py)
    }
}

/// The candles `books` fold into, as a list: `CandleIterator(books,
/// CandleOptions(interval, timezone))` drained, `interval` a
/// `CandleOptions`, an `int` of nanoseconds or a spelling such as `"1m"`,
/// and `timezone` the zone the buckets align to - where none is given, the
/// zone a `CandleOptions` states, else UTC.
#[pyfunction]
#[pyo3(signature = (books, interval, timezone=None))]
pub(crate) fn candles(
    books: &Bound<'_, PyAny>,
    interval: &Bound<'_, PyAny>,
    timezone: Option<&Bound<'_, PyAny>>,
) -> PyResult<Vec<PyCandle>> {
    let walk = PyCandleIterator::over(books, candle_options_of(interval, timezone)?)?;
    let mut collected = Vec::new();
    while let Some(candle) = walk.pull(books.py())? {
        collected.push(candle);
    }
    Ok(collected)
}
