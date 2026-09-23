//! `rust/src/graph/quote.rs`: a quote is a transparent, non-execution market
//! event and conversions between concrete market values move through one holder.

use std::mem::size_of;

use yggdryl::graph::{
    Event, MarketElement, MarketElementData, MarketEntryValue, MarketEventData,
    MarketOperationValue, Order, OrderEntry, Quote, QuoteEntry,
};
use yggdryl::{Decimal18, State};

#[test]
fn quote_wrappers_are_transparent_and_never_classify_as_executions() {
    assert_eq!(size_of::<Quote>(), size_of::<MarketEventData>());
    assert_eq!(size_of::<QuoteEntry>(), size_of::<MarketElementData>());

    let mut event = MarketEventData::at(9);
    event.set_state(State::from_spelling("Trade").expect("a shipped state"));
    let quote = Quote::from(event);
    assert!(!quote.is_execution());

    let event = MarketEventData::from(quote);
    assert_eq!(event.get_currunix(), 9);
    assert!(
        event.is_execution(),
        "the holder keeps its lifecycle reading"
    );
}

#[test]
fn generic_market_value_conversion_changes_only_the_wrapper_kind() {
    let mut event = MarketEventData::at(11);
    event.set_price(Decimal18::from_int(42));
    event.set_symbolticker(Some("ABC".to_owned()));
    let expected = event.clone();

    let quote: Quote = Order::from(event).into_operation();
    assert_eq!(MarketEventData::from(quote), expected);

    let mut element = MarketElementData::default();
    element.set_quantity(Decimal18::from_int(7));
    element.set_tif(Some("Day".to_owned()));
    let expected = element.clone();
    let quote: QuoteEntry = OrderEntry::from(element).into_entry();
    assert_eq!(MarketElementData::from(quote), expected);
}

#[test]
fn quote_and_entry_drop_or_add_only_event_facts() {
    let mut event = MarketEventData::at(17);
    event.set_price(Decimal18::from_int(5));
    event.set_symbolticker(Some("XYZ".to_owned()));
    let quote = Quote::from(event);
    let entry = QuoteEntry::from(quote);
    assert_eq!(entry.get_price(), Decimal18::from_int(5));
    assert_eq!(entry.get_symbolticker(), Some("XYZ"));

    let quote = Quote::from(entry);
    assert_eq!(quote.get_currunix(), 0);
    assert_eq!(quote.get_price(), Decimal18::from_int(5));
}
