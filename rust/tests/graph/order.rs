//! `rust/src/graph/order.rs`: an order is a transparent, non-execution
//! market-event wrapper and its entry is the same market value without time.

use std::mem::size_of;

use yggdryl::graph::{
    Element, Event, MarketElement, MarketElementData, MarketEventData, Order, OrderEntry,
};
use yggdryl::{Currency, Decimal18, Side, State, Uuid};

fn full_order() -> MarketEventData {
    let mut event = MarketEventData::at(1_700_000_000_000_000_000);
    event.set_crosscode("O-100".to_owned());
    event.set_srcuuids(vec![Uuid::from_v8(7)]);
    event.set_state(State::from_spelling("Filled").expect("a shipped state"));
    event.set_price(Decimal18::from_int(82));
    event.set_currency(Currency::new("USD").expect("a currency"));
    event.set_quantity(Decimal18::from_int(10));
    event.set_unit("lot".to_owned());
    event.set_side(Side::read("Buy").expect("a side"));
    event.set_lastpx(Some(Decimal18::from_int(81)));
    event.set_lastqty(Some(Decimal18::from_int(2)));
    event.set_tif(Some("GoodTillCancel".to_owned()));
    event.set_tradable(Some(true));
    event.set_symbolticker(Some("BRN".to_owned()));
    event.set_avgpx(Some(Decimal18::from_int(80)));
    event.set_cumqty(Some(Decimal18::from_int(4)));
    event.set_leavesqty(Some(Decimal18::from_int(6)));
    event.set_prevpx(Some(Decimal18::from_int(79)));
    event.set_prevqty(Some(Decimal18::from_int(12)));
    event.finalize();
    event
}

#[test]
fn order_wrappers_are_transparent_and_move_the_canonical_holders() {
    assert_eq!(size_of::<Order>(), size_of::<MarketEventData>());
    assert_eq!(size_of::<OrderEntry>(), size_of::<MarketElementData>());

    let source = full_order();
    let order = Order::from(&source);
    assert!(
        !order.is_execution(),
        "kind wins over a filled lifecycle state"
    );
    assert_eq!(MarketEventData::from(order.clone()), source);

    let entry = OrderEntry::from(order);
    let expected = MarketElementData::from(&source);
    assert_eq!(MarketElementData::from(entry.clone()), expected);
    let redated = MarketEventData::from(Order::from(entry));
    assert_eq!(MarketElementData::from(redated), expected);
}

#[test]
fn borrowed_conversion_copies_every_market_fact() {
    let source = full_order();
    let order = Order::from(&source);
    assert_eq!(order.get_lastpx(), source.get_lastpx());
    assert_eq!(order.get_lastqty(), source.get_lastqty());
    assert_eq!(order.get_tif(), source.get_tif());
    assert_eq!(order.get_tradable(), source.get_tradable());
    assert_eq!(order.get_symbolticker(), source.get_symbolticker());
    assert_eq!(order.get_avgpx(), source.get_avgpx());
    assert_eq!(order.get_cumqty(), source.get_cumqty());
    assert_eq!(order.get_leavesqty(), source.get_leavesqty());
    assert_eq!(order.get_prevpx(), source.get_prevpx());
    assert_eq!(order.get_prevqty(), source.get_prevqty());
}

#[test]
fn order_lifecycle_delegates_to_the_market_event_holder() {
    let mut first = MarketEventData::at(1_000_000);
    first.set_crosscode("O-100".to_owned());
    first.set_price(Decimal18::from_int(80));
    first.finalize();
    let first = Order::from(first);

    let mut second = MarketEventData::at(2_000_000);
    second.set_crosscode("O-100".to_owned());
    second.set_price(Decimal18::from_int(81));
    second.finalize();
    let second = Order::from(second)
        .with_previous(&first)
        .expect("the later order follows");

    assert_eq!(second.get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(second.get_prevpx(), Some(Decimal18::from_int(80)));
    assert_eq!(second.get_seqnum(), 1);
}
