//! Native Python view of [`CoreMarketData`], the one value over every leaf,
//! its lifted Arrow doors and the pickle stream every leaf shares.

use std::iter::FusedIterator;
use std::sync::{Mutex, PoisonError};

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyTuple;

use yggdryl::FieldPath;
use yggdryl::graph::{
    Event, Market, MarketData as CoreMarketData, MarketKind, MarketView as CoreMarketView,
    Operation,
};
use yggdryl::holder::Buffer;
use yggdryl::ipc::{self, IpcOptions};

use super::book::{PyBookEvent, PyBookSide, PySnapshotEvent};
use super::operation::{
    PyBookRef, PyExecution, PyExecutionEvent, PyOrder, PyOrderEvent, PyQuote, PyQuoteEvent,
};
use super::trade::PyTradeEvent;
use crate::expression::PyPlan;
use crate::field::PyField;
use crate::iomedia::{batch_reader_from_value, batch_reader_to_pyarrow};
use crate::text::line::core_path_from_value;
use crate::{Failed, Pulled, python_failure, value_error};

/// A dated operation, whichever leaf holds it: what a trade's root reads.
pub(crate) trait EventOperation: Event + Operation {}
impl<T: Event + Operation + ?Sized> EventOperation for T {}

/// A dated market element, whichever leaf holds it: what a snapshot
/// control copies.
pub(crate) trait EventMarket: Event + Market {}
impl<T: Event + Market + ?Sized> EventMarket for T {}

/// The dated operation `data` holds - an order, a quote, an execution or a
/// trade event - or `None` for any other variant.
pub(crate) fn event_operation_of(data: &CoreMarketData) -> Option<&dyn EventOperation> {
    match data {
        CoreMarketData::OrderEvent(leaf) => Some(leaf),
        CoreMarketData::QuoteEvent(leaf) => Some(leaf),
        CoreMarketData::ExecutionEvent(leaf) => Some(leaf),
        CoreMarketData::TradeEvent(leaf) => Some(leaf),
        _ => None,
    }
}

/// The dated market element `data` holds - any of the six dated leaves -
/// or `None` for an undated one.
pub(crate) fn event_market_of(data: &CoreMarketData) -> Option<&dyn EventMarket> {
    match data {
        CoreMarketData::OrderEvent(leaf) => Some(leaf),
        CoreMarketData::QuoteEvent(leaf) => Some(leaf),
        CoreMarketData::ExecutionEvent(leaf) => Some(leaf),
        CoreMarketData::TradeEvent(leaf) => Some(leaf),
        CoreMarketData::BookEvent(leaf) => Some(leaf.as_ref()),
        CoreMarketData::SnapshotEvent(leaf) => Some(leaf),
        _ => None,
    }
}

/// The `MarketData` `item` is - a `MarketData` itself or any leaf, wrapped
/// through the core's own `From` - or a `TypeError` naming what it is.
pub(crate) fn market_data_of(item: &Bound<'_, PyAny>) -> PyResult<CoreMarketData> {
    macro_rules! wrapped {
        ($($class:ty),*) => {
            $(
                if let Ok(leaf) = item.extract::<PyRef<'_, $class>>() {
                    return Ok(CoreMarketData::from(leaf.inner.clone()));
                }
            )*
        };
    }
    wrapped!(
        PyMarketData,
        PyOrder,
        PyQuote,
        PyExecution,
        PyBookSide,
        PyOrderEvent,
        PyQuoteEvent,
        PyExecutionEvent,
        PyTradeEvent,
        PyBookEvent,
        PySnapshotEvent
    );
    Err(PyTypeError::new_err(format!(
        "expected MarketData or a market leaf, got {}",
        item.get_type().name()?
    )))
}

/// The lifts a view appends, each a `FieldPath` or its text, resolved once;
/// `None` is no lifts, as Node reads `null`.
fn lifts_of(lifts: Option<Vec<Bound<'_, PyAny>>>) -> PyResult<Vec<FieldPath>> {
    lifts
        .unwrap_or_default()
        .into_iter()
        .map(|lift| core_path_from_value(&lift))
        .collect()
}

/// The leaf object `data` holds, as the class of its variant.
pub(crate) fn leaf_object(py: Python<'_>, data: CoreMarketData) -> PyResult<Py<PyAny>> {
    Ok(match data {
        CoreMarketData::Order(leaf) => Py::new(py, PyOrder::from_core(leaf))?.into_any(),
        CoreMarketData::Quote(leaf) => Py::new(py, PyQuote::from_core(leaf))?.into_any(),
        CoreMarketData::Execution(leaf) => Py::new(py, PyExecution::from_core(leaf))?.into_any(),
        CoreMarketData::BookSide(leaf) => Py::new(py, PyBookSide::from_core(leaf))?.into_any(),
        CoreMarketData::OrderEvent(leaf) => Py::new(py, PyOrderEvent::from_core(leaf))?.into_any(),
        CoreMarketData::QuoteEvent(leaf) => Py::new(py, PyQuoteEvent::from_core(leaf))?.into_any(),
        CoreMarketData::ExecutionEvent(leaf) => {
            Py::new(py, PyExecutionEvent::from_core(leaf))?.into_any()
        }
        CoreMarketData::TradeEvent(leaf) => Py::new(py, PyTradeEvent::from_core(leaf))?.into_any(),
        CoreMarketData::BookEvent(leaf) => Py::new(py, PyBookEvent::from_core(*leaf))?.into_any(),
        CoreMarketData::SnapshotEvent(leaf) => {
            Py::new(py, PySnapshotEvent::from_core(leaf))?.into_any()
        }
    })
}

/// The IPC stream of `data`'s one `MarketData::arrow_reader` row: what
/// pickle carries for every leaf.
pub(crate) fn into_ipc(data: CoreMarketData) -> PyResult<Vec<u8>> {
    let reader = CoreMarketData::arrow_reader([data], None, None).map_err(value_error)?;
    let mut buffer = Buffer::new();
    ipc::overwrite_arrow_reader(&mut buffer, reader, &IpcOptions::new()).map_err(value_error)?;
    Ok(buffer.into_bytes())
}

/// The one value an [`into_ipc`] stream holds, read back through
/// `MarketData::from_arrow_reader`.
pub(crate) fn from_ipc(stream: &[u8]) -> PyResult<CoreMarketData> {
    let buffer = Buffer::from_bytes(stream.to_vec());
    let reader = ipc::read_batch_reader(&buffer, None, &IpcOptions::new()).map_err(value_error)?;
    let mut rows = CoreMarketData::from_arrow_reader(reader).map_err(value_error)?;
    let data = rows
        .next()
        .ok_or_else(|| PyValueError::new_err("the pickled stream holds no MarketData row"))?
        .map_err(value_error)?;
    if rows.next().is_some() {
        return Err(PyValueError::new_err(
            "the pickled stream holds more than one MarketData row",
        ));
    }
    Ok(data)
}

/// One value over every market leaf - an order, a quote or an execution,
/// undated or dated, a book side, a trade, a book or a snapshot control -
/// answering the element and market facts its leaf answers. Immutable:
/// every verb answers a new value.
#[pyclass(
    name = "MarketData",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyMarketData {
    pub(crate) inner: CoreMarketData,
}

impl PyMarketData {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: CoreMarketData) -> Self {
        Self { inner }
    }
}

/// The `MarketData` segment of its `#[pymethods]` block: the leaf kinds,
/// and one `as_<leaf>` borrow per variant - the leaf object where the value
/// is that leaf, else `None`.
macro_rules! market_data_leaves {
    ($class:ident, $name:literal; [$($rest:ident),*]; { $($body:tt)* }) => {
        market_data_leaves!(@as $class, $name; [$($rest),*]; { $($body)* };
            as_order => PyOrder,
            as_quote => PyQuote,
            as_execution => PyExecution,
            as_book_side => PyBookSide,
            as_order_event => PyOrderEvent,
            as_quote_event => PyQuoteEvent,
            as_execution_event => PyExecutionEvent,
            as_trade_event => PyTradeEvent,
            as_book_event => PyBookEvent,
            as_snapshot_event => PySnapshotEvent
        );
    };
    (@as $class:ident, $name:literal; [$($rest:ident),*]; { $($body:tt)* };
        $($method:ident => $leaf:ident),*) => {
        graph_methods!($class, $name; [$($rest),*]; { $($body)*
            /// Every leaf kind a value may be, in declaration order.
            #[classattr]
            fn kinds(py: Python<'_>) -> PyResult<Py<PyTuple>> {
                Ok(PyTuple::new(py, MarketKind::ALL.map(MarketKind::as_str))?.unbind())
            }

            $(
                #[doc = concat!("The leaf `", stringify!($method), "` names, where this value is one; else `None`.")]
                fn $method(&self) -> Option<$leaf> {
                    self.inner.$method().map(|leaf| $leaf::from_core(leaf.clone()))
                }
            )*
        });
    };
}

graph_methods!(PyMarketData, "MarketData"; [
    element_getters, market_getters, common_verbs, market_data_leaves
]; {
    /// Wrap any market leaf, through the core's own `From`.
    #[new]
    fn new(leaf: &Bound<'_, PyAny>) -> PyResult<Self> {
        market_data_of(leaf).map(Self::from_core)
    }

    /// Which leaf this is, as its `MarketData.kinds` spelling.
    #[getter]
    fn kind(&self) -> &'static str {
        self.inner.kind().as_str()
    }

    /// Whether the leaf is one of the six dated ones.
    #[getter]
    fn is_event(&self) -> bool {
        self.inner.is_event()
    }

    /// The book control of an operation event or a snapshot control, else
    /// `None`.
    #[getter]
    fn book(&self) -> Option<PyBookRef> {
        self.inner.book().cloned().map(PyBookRef::from_core)
    }

    /// The leaf this value holds, as its own class.
    #[allow(clippy::wrong_self_convention)] // The core's `TryFrom`, over `&self`: frozen, never moved.
    fn into_leaf(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        leaf_object(py, self.inner.clone())
    }

    /// The lifted `marketdata` row field every leaf is written under.
    #[staticmethod]
    fn field() -> PyResult<PyField> {
        CoreMarketData::field()
            .map(PyField::from_inner)
            .map_err(value_error)
    }

    /// Streams `items` - any leaf or `MarketData`, pulled lazily - into
    /// bounded `marketdata` record batches, as a `pyarrow.RecordBatchReader`.
    #[staticmethod]
    #[pyo3(signature = (items, batch_row_size=None, batch_byte_size=None))]
    fn arrow_reader<'py>(
        py: Python<'py>,
        items: &Bound<'py, PyAny>,
        batch_row_size: Option<usize>,
        batch_byte_size: Option<u64>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let pulled = Pulled::new(items, market_data_of)?;
        let failed = pulled.failed.clone();
        let items = pulled.map(Ok).chain(std::iter::from_fn(move || {
            failed.take().map(|error| Err(python_failure(error)))
        }));
        let reader = CoreMarketData::arrow_reader(items, batch_row_size, batch_byte_size)
            .map_err(value_error)?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// Reads `MarketData` from record batches - any subset of the lifted
    /// columns, in any order, foreign columns ignored: a lazy iterator,
    /// fused after an error, the error raised at the failing item.
    #[staticmethod]
    fn from_arrow_reader(reader: &Bound<'_, PyAny>) -> PyResult<PyMarketDataRowIterator> {
        let reader = batch_reader_from_value(reader)?;
        let rows = CoreMarketData::from_arrow_reader(reader).map_err(value_error)?;
        Ok(PyMarketDataRowIterator::over(rows))
    }

    /// The plan one named view is over a `marketdata` stream - `view` one
    /// of `enums.MARKET_VIEWS`, read ignoring ASCII case - with each of
    /// `lifts`, a `FieldPath` or its text such as
    /// `"securityids['ISIN'] as isin"`, appended after the view's own
    /// columns; `None` is no lifts. `crosscode` is the chain the `lifecycle`
    /// view follows: that view needs one and every other view refuses one.
    #[staticmethod]
    #[pyo3(
        signature = (view, lifts = None, *, crosscode = None),
        text_signature = "(view, lifts=(), *, crosscode=None)"
    )]
    fn plan(
        view: &str,
        lifts: Option<Vec<Bound<'_, PyAny>>>,
        crosscode: Option<&str>,
    ) -> PyResult<PyPlan> {
        let view = CoreMarketView::read(view, crosscode).map_err(value_error)?;
        let lifts = lifts_of(lifts)?;
        CoreMarketData::plan(&view, &lifts)
            .map(PyPlan::from_core)
            .map_err(value_error)
    }

    /// One named view over `source` - a `pyarrow.RecordBatchReader`, a table,
    /// a batch or any Arrow C stream exporter of `marketdata` rows - as a
    /// `pyarrow.RecordBatchReader`: exactly `MarketData.plan(view, lifts,
    /// crosscode=crosscode)` applied to it, bound once against its schema.
    /// A lift naming a column the rows do not hold is refused there.
    #[staticmethod]
    #[pyo3(
        signature = (view, source, lifts = None, *, crosscode = None),
        text_signature = "(view, source, lifts=(), *, crosscode=None)"
    )]
    fn apply_view<'py>(
        py: Python<'py>,
        view: &str,
        source: &Bound<'py, PyAny>,
        lifts: Option<Vec<Bound<'py, PyAny>>>,
        crosscode: Option<&str>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let view = CoreMarketView::read(view, crosscode).map_err(value_error)?;
        let lifts = lifts_of(lifts)?;
        let reader = batch_reader_from_value(source)?;
        let reader = CoreMarketData::apply_view(&view, &lifts, reader).map_err(value_error)?;
        batch_reader_to_pyarrow(py, reader)
    }

    fn __repr__(&self) -> String {
        format!(
            "MarketData({}, kind={:?}, crosscode={:?})",
            yggdryl::graph::Element::get_curruuid(&self.inner),
            self.inner.kind().as_str(),
            yggdryl::graph::Element::get_crosscode(&self.inner),
        )
    }
});

/// A lazy stream of `MarketData` a core stage answers: the rows
/// `MarketData.from_arrow_reader` decodes, or the operations
/// `FixCodec.market_operations` sorted out of a capture.
///
/// Behind a lock, as [`crate::fix::PyFixMessages`] is: a cursor a single
/// caller advances, never contended, and the lock is what makes the boxed
/// trait object's `Send` enough for a `pyclass`.
#[pyclass(name = "MarketDataRowIterator", module = "yggdryl._native")]
pub(crate) struct PyMarketDataRowIterator {
    inner: Mutex<Box<dyn FusedIterator<Item = yggdryl::Result<CoreMarketData>> + Send>>,
    /// Where the Python source behind `inner` failed, when there is one.
    failed: Option<Failed>,
}

impl PyMarketDataRowIterator {
    /// A stream over a core iterator that pulls nothing from Python.
    fn over<I>(inner: I) -> Self
    where
        I: FusedIterator<Item = yggdryl::Result<CoreMarketData>> + Send + 'static,
    {
        Self {
            inner: Mutex::new(Box::new(inner)),
            failed: None,
        }
    }

    /// A stream over a core stage fed by a Python iterable: the source's
    /// own failure, which ended the pull, is raised once the stage is
    /// exhausted.
    pub(crate) fn pulling<I>(inner: I, failed: Failed) -> Self
    where
        I: FusedIterator<Item = yggdryl::Result<CoreMarketData>> + Send + 'static,
    {
        Self {
            inner: Mutex::new(Box::new(inner)),
            failed: Some(failed),
        }
    }
}

#[pymethods]
impl PyMarketDataRowIterator {
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&self) -> PyResult<Option<PyMarketData>> {
        let next = self
            .inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next();
        match next {
            Some(held) => held
                .map(|data| Some(PyMarketData::from_core(data)))
                .map_err(value_error),
            None => match self.failed.as_ref().and_then(Failed::take) {
                Some(error) => Err(error),
                None => Ok(None),
            },
        }
    }
}
