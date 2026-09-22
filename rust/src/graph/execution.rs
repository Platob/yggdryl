//! An execution as a market event and as an undated market entry.

use super::{MarketElement, MarketElementData, MarketEvent, MarketEventData};

/// An execution operation: every fact a [`MarketEvent`] states, with operation
/// kind fixed to an execution independently of lifecycle state.
///
/// The transparent wrapper adds no storage to [`MarketEventData`]. Converting
/// it into the holder, an [`ExecutionEntry`], or another market-operation
/// value moves the holder without cloning its owned facts.
#[repr(transparent)]
#[derive(Clone, Debug, PartialEq)]
pub struct Execution {
    event: MarketEventData,
}

super::delegate_market_event!(Execution, event, true);

impl<E: MarketEvent + ?Sized> From<&E> for Execution {
    fn from(event: &E) -> Self {
        Self {
            event: MarketEventData::from(event),
        }
    }
}

impl From<MarketEventData> for Execution {
    fn from(event: MarketEventData) -> Self {
        Self { event }
    }
}

impl From<Execution> for MarketEventData {
    fn from(execution: Execution) -> Self {
        execution.event
    }
}

/// An execution entry: every fact a [`MarketElement`] states, without an event
/// instant or lifecycle state.
///
/// The transparent wrapper adds no storage to [`MarketElementData`].
#[repr(transparent)]
#[derive(Clone, Debug, PartialEq)]
pub struct ExecutionEntry {
    element: MarketElementData,
}

super::delegate_market_element!(ExecutionEntry, element);

impl<E: MarketElement + ?Sized> From<&E> for ExecutionEntry {
    fn from(element: &E) -> Self {
        Self {
            element: MarketElementData::from(element),
        }
    }
}

impl From<MarketElementData> for ExecutionEntry {
    fn from(element: MarketElementData) -> Self {
        Self { element }
    }
}

impl From<ExecutionEntry> for MarketElementData {
    fn from(execution: ExecutionEntry) -> Self {
        execution.element
    }
}

impl From<Execution> for ExecutionEntry {
    fn from(execution: Execution) -> Self {
        Self {
            element: MarketElementData::from(execution.event),
        }
    }
}

impl From<ExecutionEntry> for Execution {
    fn from(execution: ExecutionEntry) -> Self {
        Self {
            event: MarketEventData::from(execution.element),
        }
    }
}
