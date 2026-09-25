//! Native Python view of [`CoreTradeEvent`].

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;

use yggdryl::graph::{ExecutionEvent as CoreExecutionEvent, TradeEvent as CoreTradeEvent};

use super::market_data::{event_operation_of, market_data_of};
use super::operation::PyExecutionEvent;
use crate::value_error;

/// A composite trade: one market operation event whose executions are the
/// sided fills it is made of. Immutable: every verb answers a new trade.
#[pyclass(
    name = "TradeEvent",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyTradeEvent {
    pub(crate) inner: CoreTradeEvent,
}

impl PyTradeEvent {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: CoreTradeEvent) -> Self {
        Self { inner }
    }
}

graph_methods!(PyTradeEvent, "TradeEvent"; [
    element_getters, event_getters, market_getters, operation_getters, common_verbs, event_verbs
]; {
    /// A trade from its root - any dated operation, whose facts are copied
    /// - and its executions, through the core's own `from_parts`.
    #[staticmethod]
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn from_parts(
        root: &Bound<'_, PyAny>,
        executions: Vec<PyRef<'_, PyExecutionEvent>>,
    ) -> PyResult<Self> {
        let root = market_data_of(root)?;
        let root = event_operation_of(&root).ok_or_else(|| {
            PyTypeError::new_err(format!(
                "expected a dated operation as the trade's root, got {}",
                root.kind().as_str()
            ))
        })?;
        let executions: Vec<CoreExecutionEvent> = executions
            .iter()
            .map(|execution| execution.inner.clone())
            .collect();
        CoreTradeEvent::from_parts(root, executions)
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// The executions the trade is made of, in canonical side, cross code
    /// and identity order.
    #[getter]
    fn executions(&self) -> Vec<PyExecutionEvent> {
        self.inner
            .executions()
            .iter()
            .cloned()
            .map(PyExecutionEvent::from_core)
            .collect()
    }
});
