//! `rust/market/src/graph/market_data.rs`: one value over every leaf, which leaf it
//! is, the borrows and conversions to each, and the element readings it
//! delegates by variant.

use smol_str::SmolStr;
use yggdryl::graph::{Element, Event};
use yggdryl::{Decimal, State};
use yggdryl_market::Side;
use yggdryl_market::graph::{
    BookEvent, BookRef, Execution, ExecutionEvent, Market, MarketData, MarketKind, MdUpdateAction,
    Order, OrderEvent, Quote, QuoteEvent, SnapshotEvent, TradeEvent,
};

fn order(unix: i64, code: &str) -> OrderEvent {
    let mut order = OrderEvent::at(unix);
    order.set_crosscode(code.to_owned());
    order.set_ticker(Some(SmolStr::new("ACME")), true);
    order.set_side(Side::read("Buy").unwrap(), true);
    order.set_price(Some(Decimal::from_int(100)), true);
    order.set_quantity(Some(Decimal::from_int(2)), true);
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
        MarketData::from(order(1, "O-1")),
        MarketData::from(QuoteEvent::from(&order(2, "Q-2"))),
        MarketData::from(execution(3, "E-3")),
        MarketData::from(TradeEvent::from_parts(&root, vec![execution(8, "E-8")]).unwrap()),
        MarketData::from(BookEvent::keyed(9, "ACME")),
        MarketData::from(snapshot),
        MarketData::from(fix::messages(&fix::ORDERS[..1]).remove(0)),
    ]
}

#[test]
fn each_variant_states_its_kind_and_whether_it_is_dated() {
    crate::install::installed();
    let values = one_of_every_leaf();
    let kinds: Vec<MarketKind> = values.iter().map(MarketData::kind).collect();
    assert_eq!(kinds, MarketKind::ALL);
    for value in &values {
        assert_eq!(value.is_event(), value.kind().is_event());
    }
}

#[test]
fn each_borrow_answers_its_own_variant_alone() {
    crate::install::installed();
    let values = one_of_every_leaf();
    let borrowed: Vec<[bool; 9]> = values
        .iter()
        .map(|value| {
            [
                value.as_order().is_some(),
                value.as_quote().is_some(),
                value.as_execution().is_some(),
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
fn the_readings_are_the_leafs_own() {
    crate::install::installed();
    for value in one_of_every_leaf() {
        let (uuid, code, price) = match &value {
            MarketData::OrderEvent(leaf) => (
                leaf.get_uuid(),
                leaf.get_crosscode().to_owned(),
                leaf.get_price(),
            ),
            MarketData::TradeEvent(leaf) => (
                leaf.get_uuid(),
                leaf.get_crosscode().to_owned(),
                leaf.get_price(),
            ),
            _ => continue,
        };
        assert_eq!(value.get_uuid(), uuid);
        assert_eq!(value.get_crosscode(), code);
        assert_eq!(value.get_price(), price);
    }
    // A write reaches the leaf.
    let mut value = MarketData::from(order(1, "O-1"));
    value.set_price(Some(Decimal::from_int(7)), true);
    value.finalize();
    let leaf = value.as_order_event().unwrap();
    assert_eq!(leaf.get_price(), Some(Decimal::from_int(7)));
    assert_eq!(value.get_uuid(), leaf.get_uuid());
}

#[test]
fn the_book_control_is_an_operation_events_or_a_snapshots() {
    crate::install::installed();
    let control = BookRef {
        action: Some(MdUpdateAction::New),
        ..BookRef::default()
    };
    let entry = MarketData::from(order(1, "O-1").with_book(control.clone()));
    assert_eq!(entry.book(), Some(&control));
    assert_eq!(MarketData::from(order(1, "O-1")).book(), None);
    let snapshot = one_of_every_leaf()
        .into_iter()
        .find(|value| value.kind() == MarketKind::SnapshotEvent)
        .unwrap();
    assert_eq!(
        snapshot.book().and_then(|book| book.action),
        Some(MdUpdateAction::Snapshot)
    );
    assert_eq!(MarketData::from(Order::new()).book(), None);
    assert_eq!(MarketData::from(BookEvent::keyed(1, "ACME")).book(), None);
}

/// A FIX message is market data as it is: it answers every trait through
/// the message, states the category its dictionary files it under, writes
/// one row that reads back as the leaf of that category, and a book folds
/// the leaves it splits into.
mod fix {
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event};
    use yggdryl_fix::{FixCodec, FixMsg, FixRegistry};
    use yggdryl_market::graph::{BookIterator, Market, MarketData, MarketKind, Operation};
    use yggdryl_market::{MarketDataKind, Side};

    fn registry() -> Arc<FixRegistry> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/fix");
        let folder = yggdryl::local::LocalFolder::new(root).expect("the local seed path");
        Arc::new(FixRegistry::from_handle(&folder).expect("the committed dictionary loads"))
    }

    pub(super) fn messages(lines: &[&str]) -> Vec<FixMsg> {
        let codec = FixCodec::new(registry());
        lines
            .iter()
            .flat_map(|line| codec.parse_line(line.as_bytes()).expect("a readable line"))
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("every frame parses")
    }

    /// Two orders stating their instrument's real ISIN, so each holds the
    /// code a book is keyed by.
    pub(super) const ORDERS: [&str; 2] = [
        "8=FIX.4.4|35=D|52=20240102-10:00:00.000|11=C1|55=ACME|48=US0378331005|22=4|54=1|40=2|44=100|38=5|59=0|10=0|",
        "8=FIX.4.4|35=D|52=20240102-10:00:01.000|11=C2|55=ACME|48=US0378331005|22=4|54=2|40=2|44=101|38=3|59=1|10=0|",
    ];

    #[test]
    fn a_message_is_market_data_as_it_is() {
        crate::install::installed();
        let message = messages(&ORDERS[..1]).remove(0);
        let held = MarketData::from(message.clone());
        assert_eq!(held.kind(), MarketKind::Fix);
        assert!(held.is_event());
        assert_eq!(held.marketdatakind(), MarketDataKind::Order);
        assert_eq!(held.get_uuid(), message.get_uuid());
        assert_eq!(held.get_side(), Side::Buy);
        assert_eq!(held.get_price(), message.get_price());
        assert_eq!(held.as_message::<FixMsg>(), Some(&message));
        assert_eq!(FixMsg::try_from(held).unwrap(), message);
    }

    /// Written, a message is the leaves it reports: its one order here.
    #[test]
    fn a_message_row_reads_back_as_the_leaf_of_its_category() {
        crate::install::installed();
        let message = messages(&ORDERS[..1]).remove(0);
        let held = vec![MarketData::from(message.clone())];
        let reader = MarketData::arrow_reader(held, None, None).expect("one row");
        let read: Vec<MarketData> = MarketData::from_arrow_reader(reader)
            .expect("the row's schema")
            .collect::<yggdryl::Result<_>>()
            .expect("the row reads");
        assert_eq!(read.len(), 1);
        let leaf = read[0].as_order_event().expect("an order event");
        assert_eq!(leaf.get_price(), message.get_price());
        assert_eq!(leaf.get_side(), message.get_side());
        assert_eq!(leaf.get_ordqty(), message.get_ordqty());
        assert_eq!(leaf.get_timeinforce(), message.get_timeinforce());
        assert_eq!(leaf.get_transunix(), message.get_transunix());
    }

    /// The trait object owes the enum holding it a copy, an equality, a hash
    /// and the way back to its own type; each is the message's own.
    #[test]
    fn a_held_message_clones_compares_hashes_and_downcasts_as_the_message_it_is() {
        crate::install::installed();
        use yggdryl_market::graph::market_data::MarketMessage;

        let mut parsed = messages(&ORDERS);
        let second = parsed.remove(1);
        let first = parsed.remove(0);
        let held = MarketData::from(first.clone());
        let other = MarketData::from(second.clone());

        // A copy of the enum is a copy of the message, equal to it.
        let copy = held.clone();
        assert_eq!(copy, held);
        assert_ne!(held, other);
        assert_eq!(copy.as_message::<FixMsg>(), Some(&first));
        assert_eq!(other.as_message::<FixMsg>(), Some(&second));

        let (MarketData::Fix(boxed), MarketData::Fix(boxed_other)) = (held, other) else {
            panic!("a message held whole");
        };
        let twin = boxed.clone_box();
        assert!(boxed.dyn_eq(&*twin));
        assert!(twin.dyn_eq(&*boxed));
        assert!(!boxed.dyn_eq(&*boxed_other));
        assert!(!boxed_other.dyn_eq(&*boxed));

        // The hash is the message's own, so a copy hashes as the original and
        // two different messages do not.
        assert_eq!(MarketMessage::stable_hash(&*boxed), first.stable_hash());
        assert_eq!(boxed.stable_hash(), twin.stable_hash());
        assert_ne!(boxed.stable_hash(), boxed_other.stable_hash());

        // The way back: borrowed, or owned, to the type it is and to no other.
        assert_eq!(boxed.as_any().downcast_ref::<FixMsg>(), Some(&first));
        assert!(boxed.as_any().downcast_ref::<String>().is_none());
        let owned = boxed
            .into_any()
            .downcast::<FixMsg>()
            .expect("the message itself");
        assert_eq!(*owned, first);
        assert!(boxed_other.into_any().downcast::<String>().is_err());
    }

    /// Only a held message lends a message: every leaf the enum holds lends
    /// none, and the one it holds whole lends the type it is.
    #[test]
    fn only_a_held_message_lends_a_message() {
        crate::install::installed();
        for value in super::one_of_every_leaf() {
            assert_eq!(
                value.as_message::<FixMsg>().is_some(),
                value.kind() == MarketKind::Fix,
                "{:?}",
                value.kind()
            );
        }
    }

    #[test]
    fn a_fix_message_held_whole_answers_its_category_through_the_trait() {
        crate::install::installed();
        let message = messages(&ORDERS[..1]).remove(0);
        let data = MarketData::from(message);
        assert_eq!(Market::marketdatakind(&data), MarketDataKind::Order);
        assert!(Market::is_sided(&data));
        assert_eq!(data.marketdatakind(), Market::marketdatakind(&data));
    }

    #[test]
    fn a_book_folds_the_leaves_a_message_splits_into() {
        crate::install::installed();
        let parsed = messages(&ORDERS);
        let whole: Vec<MarketData> = parsed.iter().cloned().map(MarketData::from).collect();
        let split: Vec<MarketData> = parsed
            .into_iter()
            .flat_map(|message| message.into_market_data().expect("market data"))
            .collect();
        let books = |items: Vec<MarketData>| {
            BookIterator::new(items.into_iter(), 0)
                .expect("a fold")
                .collect::<yggdryl::Result<Vec<_>>>()
                .expect("books")
        };
        let from_whole = books(whole);
        assert!(!from_whole.is_empty());
        assert_eq!(from_whole, books(split));
    }
}
