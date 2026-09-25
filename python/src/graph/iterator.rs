//! Native Python view of the [`CoreEventIterator`] walk over
//! [`CoreMarketData`].

use pyo3::prelude::*;

use yggdryl::graph::{EventIterator as CoreEventIterator, MarketData as CoreMarketData};

use super::market_data::{PyMarketData, market_data_of};
use crate::{Failed, Pulled};

/// A walk that chains each operation event to the live element it follows
/// and yields it enriched, pulling its items lazily from the caller's
/// iterable: any leaf or `MarketData`, the dated operations and trades
/// walking and every other variant yielded unchanged, in place. Yields
/// `MarketData`.
#[pyclass(name = "EventIterator", module = "yggdryl._native")]
pub(crate) struct PyEventIterator {
    inner: CoreEventIterator<CoreMarketData, Pulled<CoreMarketData>>,
    failed: Failed,
}

#[pymethods]
impl PyEventIterator {
    /// Opens a walk over `items`. `sorted` states that they arrive in their
    /// own order already; where they do not, the walk collects and sorts
    /// them first. `snapshot_ns`, given, is the grid step in nanoseconds the
    /// walk also yields living-identity snapshots at.
    #[new]
    #[pyo3(signature = (items, sorted=true, snapshot_ns=None))]
    fn new(items: &Bound<'_, PyAny>, sorted: bool, snapshot_ns: Option<i64>) -> PyResult<Self> {
        let pulled = Pulled::new(items, market_data_of)?;
        let failed = pulled.failed.clone();
        let mut inner = CoreEventIterator::new(pulled, sorted);
        if let Some(snapshot_ns) = snapshot_ns {
            inner = inner.with_snapshot_ns(snapshot_ns);
        }
        // `sorted=False` collects and sorts the whole source right here, so
        // a failure partway through it is already known: raising it now,
        // rather than only once the sorted prefix is exhausted, keeps a
        // caller from reading a truncated walk as a complete one.
        if !sorted && let Some(error) = failed.take() {
            return Err(error);
        }
        Ok(Self { inner, failed })
    }

    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// The grid step in nanoseconds the walk reads snapshots at, or `None`.
    #[getter]
    fn snapshot_ns(&self) -> Option<i64> {
        self.inner.snapshot_ns()
    }

    /// The elements still alive after what the walk has read so far, one
    /// per identity, in no order.
    fn alive(&self) -> Vec<PyMarketData> {
        self.inner
            .alive()
            .cloned()
            .map(PyMarketData::from_core)
            .collect()
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> PyResult<Option<PyMarketData>> {
        match self.inner.next() {
            Some(data) => Ok(Some(PyMarketData::from_core(data))),
            None => match self.failed.take() {
                Some(error) => Err(error),
                None => Ok(None),
            },
        }
    }
}
