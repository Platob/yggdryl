//! `rust/src/graph/market_data.rs`: one value over every leaf, which leaf it
//! is, the borrows and conversions to each, and the element readings it
//! delegates by variant.

use smol_str::SmolStr;
use yggdryl::graph::{
    BookEvent, BookRef, BookSide, Element, Event, Execution, ExecutionEvent, Market, MarketData,
    MarketKind, MdUpdateAction, Order, OrderEvent, Quote, QuoteEvent, SnapshotEvent, TradeEvent,
};
use yggdryl::{Decimal18, Error, Side, State};

fn order(unix: i64, code: &str) -> OrderEvent {
    let mut order = OrderEvent::at(unix);
    order.set_crosscode(code.to_owned());
    order.set_ticker(Some(SmolStr::new("ACME")));
    order.set_side(Side::read("Buy").unwrap());
    order.set_price(Some(Decimal18::from_int(100)));
    order.set_quantity(Some(Decimal18::from_int(2)));
    order.finalize();
    order
}

fn execution(unix: i64, code: &str) -> ExecutionEvent {
    let mut execution = ExecutionEvent::from(&order(unix, code));
    execution.set_state(State::read("Filled").unwrap());
    execution.finalize();
    execution
}

fn one_of_every_leaf() -> Vec<MarketData> {
    let root = order(8, "T-8");
    let snapshot = SnapshotEvent::snapshot(&order(10, "W-10"), Some(SmolStr::new("Symbol=ACME")));
    vec![
        MarketData::from(Order::new()),
        MarketData::from(Quote::new()),
        MarketData::from(Execution::new()),
        MarketData::from(BookSide::new(Side::Buy).unwrap()),
        MarketData::from(order(1, "O-1")),
        MarketData::from(QuoteEvent::from(&order(2, "Q-2"))),
        MarketData::from(execution(3, "E-3")),
        MarketData::from(TradeEvent::from_parts(&root, vec![execution(8, "E-8")]).unwrap()),
        MarketData::from(BookEvent::new(9, "ACME")),
        MarketData::from(snapshot),
    ]
}

#[test]
fn each_variant_states_its_kind_and_whether_it_is_dated() {
    let values = one_of_every_leaf();
    let kinds: Vec<MarketKind> = values.iter().map(MarketData::kind).collect();
    assert_eq!(kinds, MarketKind::ALL);
    for value in &values {
        assert_eq!(value.is_event(), value.kind().is_event());
    }
}

#[test]
fn each_borrow_answers_its_own_variant_alone() {
    let values = one_of_every_leaf();
    let borrowed: Vec<[bool; 10]> = values
        .iter()
        .map(|value| {
            [
                value.as_order().is_some(),
                value.as_quote().is_some(),
                value.as_execution().is_some(),
                value.as_book_side().is_some(),
                value.as_order_event().is_some(),
                value.as_quote_event().is_some(),
                value.as_execution_event().is_some(),
                value.as_trade_event().is_some(),
                value.as_book_event().is_some(),
                value.as_snapshot_event().is_some(),
            ]
        })
        .collect();
    for (row, flags) in borrowed.iter().enumerate() {
        for (column, flag) in flags.iter().enumerate() {
            assert_eq!(*flag, row == column, "row {row}, borrow {column}");
        }
    }
}

#[test]
fn a_leaf_converts_back_and_another_kind_is_refused_at_the_kind() {
    let leaf = order(1, "O-1");
    let value = MarketData::from(leaf.clone());
    assert_eq!(OrderEvent::try_from(value.clone()).unwrap(), leaf);
    let error = QuoteEvent::try_from(value.clone()).unwrap_err();
    assert!(
        matches!(&error, Error::InvalidRecord { path, .. } if path == "$.kind"),
        "{error}"
    );
    let message = error.to_string();
    assert!(
        message.contains("expected quote_event, got order_event"),
        "{message}"
    );
    let book = BookEvent::new(9, "ACME");
    assert_eq!(
        BookEvent::try_from(MarketData::from(book.clone())).unwrap(),
        book
    );
    assert!(BookEvent::try_from(value).is_err());
}

#[test]
fn the_readings_are_the_leafs_own() {
    for value in one_of_every_leaf() {
        let (uuid, code, price) = match &value {
            MarketData::OrderEvent(leaf) => (
                leaf.get_curruuid(),
                leaf.get_crosscode().to_owned(),
                leaf.get_price(),
            ),
            MarketData::BookSide(leaf) => (
                leaf.get_curruuid(),
                leaf.get_crosscode().to_owned(),
                leaf.get_price(),
            ),
            MarketData::TradeEvent(leaf) => (
                leaf.get_curruuid(),
                leaf.get_crosscode().to_owned(),
                leaf.get_price(),
            ),
            _ => continue,
        };
        assert_eq!(value.get_curruuid(), uuid);
        assert_eq!(value.get_crosscode(), code);
        assert_eq!(value.get_price(), price);
    }
    // A write reaches the leaf.
    let mut value = MarketData::from(order(1, "O-1"));
    value.set_price(Some(Decimal18::from_int(7)));
    value.finalize();
    let leaf = value.as_order_event().unwrap();
    assert_eq!(leaf.get_price(), Some(Decimal18::from_int(7)));
    assert_eq!(value.get_curruuid(), leaf.get_curruuid());
}

#[test]
fn the_book_control_is_an_operation_events_or_a_snapshots() {
    let control = BookRef {
        action: Some(MdUpdateAction::New),
        ..BookRef::default()
    };
    let entry = MarketData::from(order(1, "O-1").with_book(control.clone()));
    assert_eq!(entry.book(), Some(&control));
    assert_eq!(MarketData::from(order(1, "O-1")).book(), None);
    let snapshot = one_of_every_leaf().pop().unwrap();
    assert_eq!(
        snapshot.book().and_then(|book| book.action),
        Some(MdUpdateAction::Snapshot)
    );
    assert_eq!(MarketData::from(Order::new()).book(), None);
    assert_eq!(MarketData::from(BookEvent::new(1, "ACME")).book(), None);
}

#[test]
fn only_two_dated_values_order_and_only_one_variant_merges() {
    // A millisecond apart: two instants in one millisecond stating the same
    // content are one identity, which follows nothing.
    let first = MarketData::from(order(1_000_000, "O-1"));
    let second = MarketData::from(order(2_000_000, "O-1"));
    assert!(second.is_after(&first));
    assert!(!first.is_after(&second));
    let undated = MarketData::from(Order::new());
    assert!(!undated.is_after(&first) && !first.is_after(&undated));

    let followed = second
        .clone()
        .with_previous(&first)
        .expect("a later order follows");
    let leaf = followed.as_order_event().unwrap();
    assert_eq!(leaf.get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(
        followed,
        MarketData::from(
            order(2_000_000, "O-1")
                .with_previous(first.as_order_event().unwrap())
                .unwrap()
        ),
        "the leaf's own following"
    );

    let quote = MarketData::from(QuoteEvent::from(&order(2_000_000, "O-1")));
    assert!(
        quote.clone().with_previous(&first).is_some(),
        "an operation event follows another kind"
    );
    assert!(
        quote.merge_with(&first).is_none(),
        "a merge never crosses variants"
    );
    assert!(undated.clone().with_previous(&first).is_none());

    let mut restated = order(1_000_000, "O-1");
    restated.set_srcuuids(vec![yggdryl::Uuid::from_v8(9)]);
    let merged = first
        .clone()
        .merge_with(&MarketData::from(restated))
        .expect("another statement of the same order adds its source");
    assert_eq!(merged.get_srcuuids(), [yggdryl::Uuid::from_v8(9)]);
}

/// The enum is its widest inline leaf, a trade: the book is boxed so the
/// one value every boundary crosses as does not carry a book's width.
/// Pinned when the enum replaced the generic envelope; a moved number is a
/// design answer, never one to re-pin from a whole run.
#[test]
fn the_enum_is_the_size_of_its_widest_inline_leaf() {
    use std::mem::size_of;
    assert_eq!(size_of::<MarketData>(), size_of::<TradeEvent>());
    assert!(size_of::<BookEvent>() > size_of::<MarketData>());
    assert_eq!(size_of::<MarketData>(), 1072);
}

/// An execution follows the order it fills across kinds, through the facts
/// both hold: it takes the order's place in the chain, keeps its own kind and
/// digests as an execution; a book side or a trade still follows nothing of
/// another variant.
#[test]
fn an_operation_event_follows_one_of_another_kind_through_their_facts() {
    let first = MarketData::from(order(1_000_000, "O-1"));
    let fill = execution(2_000_000, "O-1");
    let followed = MarketData::from(fill.clone())
        .with_previous(&first)
        .expect("an execution follows the order it fills");
    let leaf = followed.as_execution_event().expect("the kind is kept");
    assert_eq!(leaf.get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(leaf.get_seqnum(), 1);
    assert_eq!(leaf.get_prevpx(), first.get_price());
    assert!(leaf.is_execution());
    assert_ne!(
        leaf.get_curruuid(),
        fill.get_curruuid(),
        "finalized once more"
    );

    let trade = MarketData::from(
        TradeEvent::from_parts(&order(3_000_000, "T-3"), vec![execution(3_000_000, "E-3")])
            .unwrap(),
    );
    assert!(trade.with_previous(&first).is_none());
    assert!(
        MarketData::from(BookSide::new(Side::Buy).unwrap())
            .with_previous(&first)
            .is_none()
    );
}
