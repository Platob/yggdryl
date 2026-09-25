//! Native Node.js view of [`CoreTradeEvent`].

use napi::bindgen_prelude::{Result, Unknown};
use napi_derive::napi;
use yggdryl::graph::{
    Event, ExecutionEvent as CoreExecutionEvent, MarketData as CoreMarketData, Operation,
    TradeEvent as CoreTradeEvent,
};

use super::market_data_from;
use super::operation::JsExecutionEvent;
use crate::napi_error;

/// A dated operation, whichever leaf holds it: what a trade's root reads.
pub(crate) trait EventOperation: Event + Operation {}
impl<T: Event + Operation + ?Sized> EventOperation for T {}

/// The dated operation `data` holds - an order, a quote, an execution or a
/// trade event - or `None` for any other variant.
fn event_operation_of(data: &CoreMarketData) -> Option<&dyn EventOperation> {
    match data {
        CoreMarketData::OrderEvent(leaf) => Some(leaf),
        CoreMarketData::QuoteEvent(leaf) => Some(leaf),
        CoreMarketData::ExecutionEvent(leaf) => Some(leaf),
        CoreMarketData::TradeEvent(leaf) => Some(leaf),
        _ => None,
    }
}

/// A composite trade: one market operation event whose executions are the
/// sided fills it is made of. Immutable: every verb answers a new trade.
#[napi(js_name = "TradeEvent")]
#[derive(Clone)]
pub struct JsTradeEvent {
    pub(crate) inner: CoreTradeEvent,
}

impl JsTradeEvent {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: CoreTradeEvent) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsTradeEvent {
    /// A trade from its root - any dated operation, whose facts are copied -
    /// and its executions, through the core's own `from_parts`.
    #[napi(
        factory,
        ts_args_type = "root: MarketData | OrderEvent | QuoteEvent | ExecutionEvent | TradeEvent, executions: Array<ExecutionEvent>"
    )]
    pub fn from_parts(root: Unknown<'_>, executions: Vec<&JsExecutionEvent>) -> Result<Self> {
        let root = market_data_from(root)?;
        let root = event_operation_of(&root).ok_or_else(|| {
            napi_error(format!(
                "expected a dated operation as the trade's root, got {}",
                root.kind().as_str()
            ))
        })?;
        let executions: Vec<CoreExecutionEvent> = executions
            .into_iter()
            .map(|execution| execution.inner.clone())
            .collect();
        CoreTradeEvent::from_parts(root, executions)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The executions the trade is made of, in canonical side, cross code
    /// and identity order.
    #[napi(getter)]
    pub fn executions(&self) -> Vec<JsExecutionEvent> {
        self.inner
            .executions()
            .iter()
            .cloned()
            .map(JsExecutionEvent::from_core)
            .collect()
    }
}

element_getters!(JsTradeEvent);
event_getters!(JsTradeEvent);
market_getters!(JsTradeEvent);
operation_getters!(JsTradeEvent);
common_verbs!(JsTradeEvent);
event_verbs!(JsTradeEvent, "TradeEvent");
