//! A quote as a market event and as an undated market entry.

use super::{MarketElement, MarketElementData, MarketEvent, MarketEventData};

/// A quote operation: every fact a [`MarketEvent`] states, with operation
/// kind fixed to a quote independently of lifecycle state.
///
/// The transparent wrapper adds no storage to [`MarketEventData`]. Converting
/// it into the holder, a [`QuoteEntry`], or another market-operation value
/// moves the holder without cloning its owned facts.
#[repr(transparent)]
#[derive(Clone, Debug, PartialEq)]
pub struct Quote {
    event: MarketEventData,
}

super::delegate_market_event!(Quote, event, false);

impl<E: MarketEvent + ?Sized> From<&E> for Quote {
    fn from(event: &E) -> Self {
        Self {
            event: MarketEventData::from(event),
        }
    }
}

impl From<MarketEventData> for Quote {
    fn from(event: MarketEventData) -> Self {
        Self { event }
    }
}

impl From<Quote> for MarketEventData {
    fn from(quote: Quote) -> Self {
        quote.event
    }
}

/// A quote entry: every fact a [`MarketElement`] states, without an event
/// instant or lifecycle state.
///
/// The transparent wrapper adds no storage to [`MarketElementData`].
#[repr(transparent)]
#[derive(Clone, Debug, PartialEq)]
pub struct QuoteEntry {
    element: MarketElementData,
}

super::delegate_market_element!(QuoteEntry, element);

impl<E: MarketElement + ?Sized> From<&E> for QuoteEntry {
    fn from(element: &E) -> Self {
        Self {
            element: MarketElementData::from(element),
        }
    }
}

impl From<MarketElementData> for QuoteEntry {
    fn from(element: MarketElementData) -> Self {
        Self { element }
    }
}

impl From<QuoteEntry> for MarketElementData {
    fn from(quote: QuoteEntry) -> Self {
        quote.element
    }
}

impl From<Quote> for QuoteEntry {
    fn from(quote: Quote) -> Self {
        Self {
            element: MarketElementData::from(quote.event),
        }
    }
}

impl From<QuoteEntry> for Quote {
    fn from(quote: QuoteEntry) -> Self {
        Self {
            event: MarketEventData::from(quote.element),
        }
    }
}
