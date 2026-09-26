//! Native Python view of [`CoreBookEvent`], [`CoreBookSide`],
//! [`CoreSnapshotEvent`], [`CoreSnapshotPartition`] and the
//! [`CoreBookIterator`] walk.

use std::sync::{Mutex, PoisonError};

use pyo3::class::basic::CompareOp;
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;

use yggdryl::graph::{
    BookEvent as CoreBookEvent, BookIterator as CoreBookIterator, BookSide as CoreBookSide,
    MarketData as CoreMarketData, SnapshotEvent as CoreSnapshotEvent,
    SnapshotPartition as CoreSnapshotPartition,
};
use yggdryl::{Scalar, Side as CoreSide};

use super::market_data::{PyMarketData, event_market_of, market_data_of};
use super::operation::{PyBookRef, PyExecutionEvent};
use super::{decimal_scalar, slot_repr};
use crate::scalar::PyScalar;
use crate::{Failed, Pulled, compare, python_failure, value_error};

/// One scope a full snapshot replaces: a symbol - `None` in global mode -
/// and the book scope the entries stated.
#[pyclass(
    name = "SnapshotPartition",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PySnapshotPartition {
    pub(crate) inner: CoreSnapshotPartition,
}

impl PySnapshotPartition {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: CoreSnapshotPartition) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PySnapshotPartition {
    /// A partition for `scope`, and `symbol` where the operations named
    /// one.
    #[new]
    #[pyo3(signature = (scope, symbol=None))]
    fn new(scope: &str, symbol: Option<&str>) -> Self {
        Self::from_core(CoreSnapshotPartition {
            symbol: symbol.map(Into::into),
            scope: scope.into(),
        })
    }

    /// Rebuild a partition pickle carried.
    #[staticmethod]
    fn _from_pickle(scope: &str, symbol: Option<&str>) -> Self {
        Self::new(scope, symbol)
    }

    /// The book scope this partition replaces.
    #[getter]
    fn scope(&self) -> &str {
        &self.inner.scope
    }

    /// The symbol this partition is for; `None` in global mode.
    #[getter]
    fn symbol(&self) -> Option<&str> {
        self.inner.symbol.as_deref()
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(snapshot_partition_hash(&self.inner))
    }

    fn __repr__(&self) -> String {
        format!(
            "SnapshotPartition(scope={:?}, symbol={})",
            self.inner.scope.as_str(),
            slot_repr(self.inner.symbol.as_deref())
        )
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    #[allow(clippy::type_complexity)]
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (String, Option<String>))> {
        Ok((
            py.get_type::<Self>().getattr("_from_pickle")?.unbind(),
            (
                self.inner.scope.to_string(),
                self.inner.symbol.as_ref().map(ToString::to_string),
            ),
        ))
    }
}

/// One side of a book: persistent live orders and quotes, price ordered,
/// beside the deltas applied since the last emitted book. Immutable:
/// `with_operation` and every verb answer a new side.
#[pyclass(
    name = "BookSide",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyBookSide {
    pub(crate) inner: CoreBookSide,
}

impl PyBookSide {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: CoreBookSide) -> Self {
        Self { inner }
    }
}

graph_methods!(PyBookSide, "BookSide"; [
    element_getters, market_getters, common_verbs, element_repr
]; {
    /// An empty bid or ask side; `side` is read through the core `Side`
    /// vocabulary.
    #[new]
    fn new(side: &str) -> PyResult<Self> {
        let side = CoreSide::read(side).map_err(value_error)?;
        CoreBookSide::new(side)
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// The live orders and quotes, best price first, each a `MarketData`.
    #[getter]
    fn live(&self) -> Vec<PyMarketData> {
        self.inner
            .live()
            .cloned()
            .map(PyMarketData::from_core)
            .collect()
    }

    /// The deltas applied since the last emitted book, each a `MarketData`.
    #[getter]
    fn deltas(&self) -> Vec<PyMarketData> {
        self.inner
            .deltas()
            .iter()
            .cloned()
            .map(PyMarketData::from_core)
            .collect()
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// Whether the side holds no live entry.
    #[getter]
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// The first priced level's price, as a decimal; `None` for an empty
    /// side or one holding unpriced entries only.
    #[getter]
    fn best_price(&self) -> Option<PyScalar> {
        self.inner.best_price().map(decimal_scalar)
    }

    /// The exact aggregate quantity at the best price, an entry stating
    /// none adding nothing; `None` where `best_price` is.
    #[getter]
    fn best_quantity(&self) -> Option<PyScalar> {
        self.inner.best_quantity().map(decimal_scalar)
    }

    /// One limit per price this side holds, best first, and one last for
    /// every entry stating no price: each the struct `Scalar` of its
    /// `price` (`None` on the unpriced limit), the exact `quantity` resting
    /// there and the `uuids` of the entries resting there, in live order.
    #[getter]
    fn limits(&self) -> Vec<PyScalar> {
        self.inner
            .limits()
            .map(|limit| PyScalar::from_inner(limit.into_scalar()))
            .collect()
    }

    /// The exact sum of the first `levels` limits' quantities, the unpriced
    /// limit counted where reached: zero for an empty side or no level,
    /// `None` only past what a decimal holds.
    fn depth(&self, levels: usize) -> Option<PyScalar> {
        self.inner.depth(levels).map(decimal_scalar)
    }

    /// This side with one order or quote event - a leaf or a `MarketData` -
    /// atomically applied.
    fn with_operation(&self, operation: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut side = self.inner.clone();
        side.add_operation(market_data_of(operation)?)
            .map_err(value_error)?;
        Ok(Self::from_core(side))
    }
});

/// One coherent view of a market at one exact nanosecond instant: the bid
/// and ask depth, the executions at that instant and the scopes its last
/// snapshot replaced. Immutable: `with_operations` and every verb answer a
/// new book.
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
    element_getters, event_getters, market_getters, common_verbs, event_verbs
]; {
    /// An empty book for `symbol` at `currunix` nanoseconds since the epoch.
    #[new]
    fn new(currunix: i64, symbol: &str) -> Self {
        Self::from_core(CoreBookEvent::new(currunix, symbol))
    }

    /// The bid side.
    #[getter]
    fn bid(&self) -> PyBookSide {
        PyBookSide::from_core(self.inner.bid().clone())
    }

    /// The ask side, shaped as the bid.
    #[getter]
    fn ask(&self) -> PyBookSide {
        PyBookSide::from_core(self.inner.ask().clone())
    }

    /// The executions at this book's instant.
    #[getter]
    fn executions(&self) -> Vec<PyExecutionEvent> {
        self.inner
            .executions()
            .iter()
            .cloned()
            .map(PyExecutionEvent::from_core)
            .collect()
    }

    /// The scopes this book's last full snapshot replaced.
    #[getter]
    fn snapshot_partitions(&self) -> Vec<PySnapshotPartition> {
        self.inner
            .snapshot_partitions()
            .iter()
            .cloned()
            .map(PySnapshotPartition::from_core)
            .collect()
    }

    /// Whether the best bid is strictly above the best ask.
    #[getter]
    fn is_crossed(&self) -> bool {
        self.inner.is_crossed()
    }

    /// Whether both sides state a best price and the two are equal.
    #[getter]
    fn is_locked(&self) -> bool {
        self.inner.is_locked()
    }

    /// The best ask less the best bid, negative when the book is crossed;
    /// `None` where a side states no best price.
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
    element_getters, event_getters, market_getters, common_verbs, event_verbs
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

/// Books from a sorted stream of operations, one per symbol and effective
/// timestamp, or one consolidated `GLOBAL` book, pulling its items lazily
/// from the caller's iterable. Yields `BookEvent`.
#[pyclass(name = "BookIterator", module = "yggdryl._native")]
pub(crate) struct PyBookIterator {
    inner: Mutex<CoreBookIterator<BookSource>>,
    failed: Failed,
}

#[pymethods]
impl PyBookIterator {
    /// Opens a book walk over `items` - any leaf or `MarketData`, sorted by
    /// their own event order; `snapshot_millis == 0` disables grid snapshots
    /// and `global_` emits one consolidated `GLOBAL` book.
    #[new]
    #[pyo3(signature = (items, snapshot_millis=0, global_=false))]
    fn new(items: &Bound<'_, PyAny>, snapshot_millis: u64, global_: bool) -> PyResult<Self> {
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
        let inner = CoreBookIterator::new(source, snapshot_millis, global_).map_err(value_error)?;
        Ok(Self {
            inner: Mutex::new(inner),
            failed,
        })
    }

    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// Whether this walk emits one consolidated `GLOBAL` book.
    #[getter]
    fn global_(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .global()
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&self) -> PyResult<Option<PyBookEvent>> {
        let next = self
            .inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next();
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

/// The stable hash of a snapshot partition: its symbol and scope as one
/// record `Scalar` - no symbol a null - digested by the crate's one
/// `stable_hash`, so equal partitions hash alike in either language.
pub(crate) fn snapshot_partition_hash(partition: &CoreSnapshotPartition) -> u64 {
    Scalar::from_sequence([
        partition.symbol.clone().map_or(Scalar::Null, Scalar::from),
        Scalar::from(partition.scope.clone()),
    ])
    .stable_hash()
}
