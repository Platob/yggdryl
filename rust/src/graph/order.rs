//! An order as a market event and as an undated market entry.

use super::{MarketElement, MarketElementData, MarketEvent, MarketEventData};

/// An order operation: every fact a [`MarketEvent`] states, with operation
/// kind fixed to an order independently of lifecycle state.
///
/// The transparent wrapper adds no storage to [`MarketEventData`]. Converting
/// it into the holder, an [`OrderEntry`], or another market-operation value
/// moves the holder without cloning its owned facts.
#[repr(transparent)]
#[derive(Clone, Debug, PartialEq)]
pub struct Order {
    event: MarketEventData,
}

super::delegate_market_event!(Order, event, false);

impl<E: MarketEvent + ?Sized> From<&E> for Order {
    fn from(event: &E) -> Self {
        Self {
            event: MarketEventData::from(event),
        }
    }
}

impl From<MarketEventData> for Order {
    fn from(event: MarketEventData) -> Self {
        Self { event }
    }
}

impl From<Order> for MarketEventData {
    fn from(order: Order) -> Self {
        order.event
    }
}

/// An order entry: every fact a [`MarketElement`] states, without an event
/// instant or lifecycle state.
///
/// The transparent wrapper adds no storage to [`MarketElementData`].
#[repr(transparent)]
#[derive(Clone, Debug, PartialEq)]
pub struct OrderEntry {
    element: MarketElementData,
}

super::delegate_market_element!(OrderEntry, element);

impl<E: MarketElement + ?Sized> From<&E> for OrderEntry {
    fn from(element: &E) -> Self {
        Self {
            element: MarketElementData::from(element),
        }
    }
}

impl From<MarketElementData> for OrderEntry {
    fn from(element: MarketElementData) -> Self {
        Self { element }
    }
}

impl From<OrderEntry> for MarketElementData {
    fn from(order: OrderEntry) -> Self {
        order.element
    }
}

impl From<Order> for OrderEntry {
    fn from(order: Order) -> Self {
        Self {
            element: MarketElementData::from(order.event),
        }
    }
}

impl From<OrderEntry> for Order {
    fn from(order: OrderEntry) -> Self {
        Self {
            event: MarketEventData::from(order.element),
        }
    }
}
