//! Native Python view of [`CoreBookEvent`], [`CoreSnapshotEvent`] and the
//! [`CoreBookIterator`] walk.

use std::sync::{Mutex, PoisonError};

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;

use yggdryl::graph::{
    BookEvent as CoreBookEvent, BookIterator as CoreBookIterator, MarketData as CoreMarketData,
    SnapshotEvent as CoreSnapshotEvent,
};
use yggdryl::{DataType, Scalar, Side as CoreSide};

use super::decimal_scalar;
use super::market_data::{PyMarketData, event_market_of, market_data_of};
use super::operation::{PyBookRef, PyExecutionEvent, PyOrderEvent, PyQuoteEvent};
use crate::expression::filter_from_value;
use crate::scalar::{PyScalar, from_py};
use crate::{Failed, Pulled, python_failure, value_error};

/// The side one argument names - a `Side` member, its code or any spelling
/// the core reads - checked through the `side` datatype's own value contract.
fn side_of(value: &Bound<'_, PyAny>) -> PyResult<CoreSide> {
    match DataType::Side
        .scalar(from_py(value)?)
        .map_err(value_error)?
    {
        Scalar::Side(side) => Ok(side),
        other => Err(PyTypeError::new_err(format!(
            "expected a side, got {}",
            other.kind()
        ))),
    }
}

/// One coherent view of a market at one exact nanosecond instant: the live
/// entries of both sides and each side's price levels on a complete book,
/// the deltas applied since the book before it on every book. Immutable:
/// `with_operations` and every verb answer a new book.
#[pyclass(
    name = "BookEvent",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyBookEvent {
    pub(crate) inner: CoreBookEvent,
}

impl PyBookEvent {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: CoreBookEvent) -> Self {
        Self { inner }
    }
}

graph_methods!(PyBookEvent, "BookEvent"; [
    element_getters, event_getters, market_getters, kind_getters, common_verbs, event_verbs
]; {
    /// An empty book of the ticker `symbol` at `currunix` nanoseconds since
    /// the epoch, keyed by that ticker; an empty `symbol` keys the book
    /// `XX0000000000`, the ISIN that states none, and states no ticker.
    #[new]
    fn new(currunix: i64, symbol: &str) -> Self {
        Self::from_core(CoreBookEvent::new(currunix, symbol))
    }

    /// An empty book keyed `key` at `currunix` nanoseconds since the epoch:
    /// `key` is its crosscode - an instrument's ISIN, a ticker, or
    /// `XX0000000000` - and the book states neither a ticker nor an ISIN.
    /// The empty base a code's first book, stating its deltas alone,
    /// rebuilds over with `with_previous`.
    #[staticmethod]
    fn keyed(currunix: i64, key: &str) -> Self {
        Self::from_core(CoreBookEvent::keyed(currunix, key))
    }

    /// Whether the book holds its sides - every entry alive on it - rather
    /// than only the deltas it applied since the book before it: a book a
    /// caller builds, one a walk emits at a snapshot tick, and one rebuilt
    /// by `with_previous` are complete.
    #[getter]
    fn is_complete(&self) -> bool {
        self.inner.is_complete()
    }

    /// Every entry alive on the book, each once and a `MarketData`: the bid
    /// side's, best price first and every entry stating no price last, then
    /// the ask side's the same way but those resting on the bid too - a
    /// two-sided quote is one entry, listed with the bids. Empty on a book
    /// stating its deltas alone.
    #[getter]
    fn alive(&self) -> Vec<PyMarketData> {
        self.inner
            .alive()
            .cloned()
            .map(PyMarketData::from_core)
            .collect()
    }

    /// The entries alive on the side `side` takes, each a `MarketData`,
    /// best price first and every entry stating no price last - a two-sided
    /// quote on both sides. Empty for a side that is neither a bid nor an
    /// ask, or on a book stating its deltas alone.
    fn alive_on(&self, side: &Bound<'_, PyAny>) -> PyResult<Vec<PyMarketData>> {
        Ok(self
            .inner
            .alive_on(side_of(side)?)
            .cloned()
            .map(PyMarketData::from_core)
            .collect())
    }

    /// Every event of the book's instant since the book before this one,
    /// each a `MarketData`, in the order applied across both sides: the
    /// orders and quotes applied, and the executions recorded, which rest on
    /// no side. What a book stating its deltas alone states, and what
    /// `with_previous` replays over the book before it.
    #[getter]
    fn deltas(&self) -> Vec<PyMarketData> {
        self.inner
            .deltas()
            .cloned()
            .map(PyMarketData::from_core)
            .collect()
    }

    /// The orders resting on the book - every `alive` entry that is an
    /// order - each an `OrderEvent`, in `alive`'s order: the bid side's,
    /// best price first, then the ask side's. Empty on a book stating its
    /// deltas alone.
    #[getter]
    fn ordlive(&self) -> Vec<PyOrderEvent> {
        self.inner
            .ordlive()
            .cloned()
            .map(PyOrderEvent::from_core)
            .collect()
    }

    /// The orders among `deltas`, each an `OrderEvent`, in the order
    /// applied: every order the book's instant placed, changed or ended.
    #[getter]
    fn orddelta(&self) -> Vec<PyOrderEvent> {
        self.inner
            .orddelta()
            .cloned()
            .map(PyOrderEvent::from_core)
            .collect()
    }

    /// The quotes among `deltas`, each a `QuoteEvent`, in the order applied;
    /// a quote resting since an earlier instant is `alive`'s and not here.
    #[getter]
    fn quotes(&self) -> Vec<PyQuoteEvent> {
        self.inner
            .quotes()
            .cloned()
            .map(PyQuoteEvent::from_core)
            .collect()
    }

    /// The executions among `deltas`, each an `ExecutionEvent`, in the order
    /// applied: recorded at the book's instant, resting on no side.
    #[getter]
    fn executions(&self) -> Vec<PyExecutionEvent> {
        self.inner
            .executions()
            .cloned()
            .map(PyExecutionEvent::from_core)
            .collect()
    }

    /// Every other delta - none an order, a quote or an execution - each a
    /// `MarketData`, in the order applied: `orddelta`, `quotes`,
    /// `executions` and these partition `deltas`. Empty today, by
    /// construction: a fold prunes a trade, a batch and a session message,
    /// refuses an undated order, quote or execution and a nested book by
    /// kind, and folds a snapshot control into the sides, never among the
    /// deltas.
    #[getter]
    fn events(&self) -> Vec<PyMarketData> {
        self.inner
            .events()
            .cloned()
            .map(PyMarketData::from_core)
            .collect()
    }

    /// One limit per level of the side `side` takes - a bid side reads the
    /// bid, an ask side the ask - best first and the unpriced limit last:
    /// each the struct `Scalar` of its `price` (`None` on the unpriced
    /// limit), the exact `quantity` resting there, the `uuids` of the
    /// entries resting there and whether the level is `tradable`. Empty for
    /// a side that is neither, and on a book stating its deltas alone.
    fn limits(&self, side: &Bound<'_, PyAny>) -> PyResult<Vec<PyScalar>> {
        Ok(self
            .inner
            .limits(side_of(side)?)
            .map(|limit| PyScalar::from_inner(limit.into_scalar()))
            .collect())
    }

    /// The best tradable price on the side `side` takes, as a decimal: the
    /// first priced level that can trade; `None` where no level can, or for
    /// a side that is neither a bid nor an ask.
    fn best_price(&self, side: &Bound<'_, PyAny>) -> PyResult<Option<PyScalar>> {
        Ok(self.inner.best_price(side_of(side)?).map(decimal_scalar))
    }

    /// The aggregate quantity at `best_price(side)`, as a decimal; `None`
    /// where that is.
    fn best_quantity(&self, side: &Bound<'_, PyAny>) -> PyResult<Option<PyScalar>> {
        Ok(self.inner.best_quantity(side_of(side)?).map(decimal_scalar))
    }

    /// The exact sum of the first `levels` limits' quantities of the side
    /// `side` takes, the unpriced limit counted where reached: zero for an
    /// empty side or no level, `None` past what a decimal holds or for a
    /// side that is neither a bid nor an ask.
    fn depth(&self, side: &Bound<'_, PyAny>, levels: usize) -> PyResult<Option<PyScalar>> {
        Ok(self.inner.depth(side_of(side)?, levels).map(decimal_scalar))
    }

    /// Whether the best tradable bid is strictly above the best tradable
    /// ask.
    #[getter]
    fn is_crossed(&self) -> bool {
        self.inner.is_crossed()
    }

    /// Whether both sides state a best tradable price and the two are
    /// equal.
    #[getter]
    fn is_locked(&self) -> bool {
        self.inner.is_locked()
    }

    /// The best tradable ask less the best tradable bid, negative when the
    /// book is crossed; `None` where a side states no best price.
    #[getter]
    fn spread(&self) -> Option<PyScalar> {
        self.inner.spread().map(decimal_scalar)
    }

    /// `(bid - ask) / (bid + ask)` over the two sides' `depth(levels)`: one
    /// for a bid-only book, minus one for an ask-only one, `None` where the
    /// total is zero - both sides empty, or no level.
    fn imbalance(&self, levels: usize) -> Option<PyScalar> {
        self.inner.imbalance(levels).map(decimal_scalar)
    }

    /// The arithmetic midpoint of a coherent two-sided best bid and offer.
    #[getter]
    fn bbo_midpoint(&self) -> Option<PyScalar> {
        self.inner.bbo_midpoint().map(decimal_scalar)
    }

    /// The two-value median of the best bid and ask aggregate quantities.
    #[getter]
    fn median_quantity(&self) -> Option<PyScalar> {
        self.inner.median_quantity().map(decimal_scalar)
    }

    /// This book with every operation of one atomic group applied: each an
    /// order, quote, execution or trade event, a snapshot control, or a
    /// `MarketData` holding one.
    fn with_operations(&self, operations: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut items = Vec::new();
        for (index, item) in operations.try_iter()?.enumerate() {
            let item = item?;
            items.push(market_data_of(&item).map_err(|error| {
                PyTypeError::new_err(format!("operations[{index}]: {error}"))
            })?);
        }
        let mut book = self.inner.clone();
        book.add_operations(items).map_err(value_error)?;
        Ok(Self::from_core(book))
    }
});

/// The full-snapshot control an empty FIX `W` is: an event stating the
/// scope it replaces and no entry of its own.
#[pyclass(
    name = "SnapshotEvent",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PySnapshotEvent {
    pub(crate) inner: CoreSnapshotEvent,
}

impl PySnapshotEvent {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: CoreSnapshotEvent) -> Self {
        Self { inner }
    }
}

graph_methods!(PySnapshotEvent, "SnapshotEvent"; [
    element_getters, event_getters, market_getters, kind_getters, common_verbs, event_verbs
]; {
    /// The snapshot control over `event` - any dated leaf, whose event and
    /// market facts are copied - replacing `scope`.
    #[staticmethod]
    #[pyo3(signature = (event, scope=None))]
    fn snapshot(event: &Bound<'_, PyAny>, scope: Option<&str>) -> PyResult<Self> {
        let data = market_data_of(event)?;
        let event = event_market_of(&data).ok_or_else(|| {
            PyTypeError::new_err(format!(
                "expected a dated leaf to snapshot, got {}",
                data.kind().as_str()
            ))
        })?;
        Ok(Self::from_core(CoreSnapshotEvent::snapshot(
            event,
            scope.map(Into::into),
        )))
    }

    /// The control's book facts: always a full snapshot, with its scope.
    #[getter]
    fn book(&self) -> PyBookRef {
        PyBookRef::from_core(self.inner.book().clone())
    }
});

/// The core stage's source: the caller's items as `MarketData`, a Python
/// failure crossing as one typed sentinel.
type BookSource = Box<dyn Iterator<Item = yggdryl::Result<CoreMarketData>> + Send>;

/// Books from a sorted stream of operations, one per book key and effective
/// timestamp, pulling its items lazily from the caller's iterable. Yields
/// `BookEvent`.
#[pyclass(name = "BookIterator", module = "yggdryl._native", frozen)]
pub(crate) struct PyBookIterator {
    inner: Mutex<CoreBookIterator<BookSource>>,
    failed: Failed,
}

#[pymethods]
impl PyBookIterator {
    /// Opens a book walk over `items` - any leaf or `MarketData`, sorted by
    /// their own event order; `snapshot_millis == 0` disables grid
    /// snapshots, so a book is emitted whole only at a full refresh.
    ///
    /// The walk folds orders, quotes and snapshot controls, records every
    /// execution among the deltas of its book at its instant, and prunes
    /// every other input where it is pulled. `filter` - a `Filter`, a
    /// `Term`, an `Expression` or the text of a predicate over the
    /// `marketdata` row - narrows it further, bound once here; it never
    /// admits a trade. `None` keeps every recorded input.
    #[new]
    #[pyo3(signature = (items, snapshot_millis=0, filter=None))]
    fn new(
        items: &Bound<'_, PyAny>,
        snapshot_millis: u64,
        filter: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let filter = filter.map(filter_from_value).transpose()?;
        let pulled = Pulled::new(items, market_data_of)?;
        let failed = pulled.failed.clone();
        // The core stage takes a typed error, not a `PyErr`, so a Python
        // failure crosses it as one sentinel `Err` - peeked, never taken,
        // so `failed` still holds the original for `__next__` to raise as
        // itself once the stage surfaces the sentinel in its place. Yielded
        // exactly once: `Chain` stops calling a side once it answers `None`.
        let source: BookSource = {
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
        let mut inner = CoreBookIterator::new(source, snapshot_millis).map_err(value_error)?;
        if let Some(filter) = filter {
            inner = inner.with_filter(filter).map_err(value_error)?;
        }
        Ok(Self {
            inner: Mutex::new(inner),
            failed,
        })
    }

    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&self, py: Python<'_>) -> PyResult<Option<PyBookEvent>> {
        // The GIL is released while the core pulls: a parse worker that
        // warns takes it to reach Python's `logging`, and the worker this
        // pull waits on must not wait on this thread.
        let next = py.detach(|| {
            self.inner
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .next()
        });
        match next {
            Some(Ok(book)) => Ok(Some(PyBookEvent::from_core(book))),
            // A core refusal (a bad item) or the sentinel standing in for a
            // Python failure both land here; `self.failed` still holds a
            // genuine Python failure - never taken by the source, only
            // peeked - so it is raised as itself rather than as the typed
            // error it crossed the stage wrapped in.
            Some(Err(error)) => Err(self.failed.take().unwrap_or_else(|| value_error(error))),
            None => match self.failed.take() {
                Some(error) => Err(error),
                None => Ok(None),
            },
        }
    }
}
