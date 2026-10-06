//! `rust/src/graph/market_data.rs`: one value over every leaf, which leaf it
//! is, the borrows and conversions to each, and the element readings it
//! delegates by variant.

use smol_str::SmolStr;
use yggdryl::graph::{
    BookEvent, BookRef, Element, Event, Execution, ExecutionEvent, Market, MarketData, MarketKind,
    MdUpdateAction, Order, OrderEvent, Quote, QuoteEvent, SnapshotEvent, TradeEvent,
};
use yggdryl::{Decimal, Error, Side, State};

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
        MarketData::from(BookEvent::new(9, "ACME")),
        MarketData::from(snapshot),
        MarketData::from(fix::messages(&fix::ORDERS[..1]).remove(0)),
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
    value.set_price(Some(Decimal::from_int(7)), true);
    value.finalize();
    let leaf = value.as_order_event().unwrap();
    assert_eq!(leaf.get_price(), Some(Decimal::from_int(7)));
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
    let snapshot = one_of_every_leaf()
        .into_iter()
        .find(|value| value.kind() == MarketKind::SnapshotEvent)
        .unwrap();
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

/// The enum is its widest inline leaf, a trade; the book is boxed. The book
/// was the wider of the two when the box was written, so the one value every
/// boundary crosses as did not carry a book's width; it is the narrower now
/// (see the last sentence). Pinned when the enum replaced the generic
/// envelope; a moved number is a
/// design answer, never one to re-pin from a whole run. It moved from 1072
/// when the market facts began to know which identifiers they only derived:
/// one `u64` mask, padded to sixteen bytes' alignment. It moved back to
/// 1072 when an event's `State` became an `i32` member rather than a
/// twenty-four-byte code string, padded to the same alignment. It moved to
/// 1088 when the dated operation facts moved from 1040 to 1056 - the market
/// facts' FX rates, sixteen, less the `marketoperationid` they no longer
/// hold, eight, padded - the trade's executions taking it to 1080, padded
/// to 1088. It moved to 960 when the dated operation facts moved from 1056
/// to 928 - the rates a map in the market facts' padding, and two
/// `IdMap`s fewer - the trade's executions taking it to 952, padded to 960.
/// It moved to 944 when the dated operation facts moved from 928 to 912 -
/// the two boxed lanes they no longer hold - the trade's executions taking
/// it to 936, padded to 944. It moved to 960 when the market facts gained
/// the boxed bid and ask, the dated operation facts sixteen wider. It moved
/// to 1008 when the operation facts gained `accountids`, one `IdMap` of
/// forty-eight. It moved to 992 when the time in force became an enum: an
/// `Option<TimeInForce>` of one byte where a twenty-four-byte string code
/// stood, sixteen fewer after padding. It moved to 912 when the identifiers
/// became `Identifiers`, one 24-byte sorted vector each: the dated operation
/// holds three sets where it held a 56-byte `SecurityIds` with its derived
/// mask and two 56-byte `IdMap`s, 96 fewer, so the trade is 896, and the
/// snapshot - whose market facts hold the one set, 32 fewer after padding -
/// is the widest inline leaf at 912. The book was 992, the wider again,
/// until a side became one shared store and the deltas one list across both
/// sides: a side holds two reference counts where it held a price map and
/// its own deltas, so the book is 912 - the snapshot's width - and the box
/// keeps a book's move a pointer's. The book moved to 880 when its two
/// sides became one optional pair - none on a book stating its deltas
/// alone - each side its two reference counts without the digest it no
/// longer keeps, a side being digested only at a snapshot, and the walk's
/// replaced scopes left the book, every group replacing membership being a
/// snapshot: 40 fewer, 32 after padding.
#[test]
fn the_enum_is_the_size_of_its_widest_inline_leaf() {
    use std::mem::size_of;
    assert_eq!(size_of::<MarketData>(), size_of::<SnapshotEvent>());
    assert!(size_of::<TradeEvent>() < size_of::<MarketData>());
    assert_eq!(size_of::<BookEvent>(), 880);
    assert_eq!(size_of::<MarketData>(), 912);
}

/// An execution follows the order it fills across kinds, through the facts
/// both hold: it follows the same place rule as within one kind, keeps its
/// own kind and digests as an execution; a trade still follows nothing of
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
    // A later instant keeps its own place.
    assert_eq!(leaf.get_seqnum(), 0);
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
}

/// A FIX message is market data as it is: it answers every trait through
/// the message, states the category its dictionary files it under, writes
/// one row that reads back as the leaf of that category, and a book folds
/// the leaves it splits into.
mod fix {
    use std::sync::Arc;

    use yggdryl::graph::{BookIterator, Element, Event, Market, MarketData, MarketKind, Operation};
    use yggdryl::{FixCodec, FixMsg, FixRegistry, MarketDataKind, Side};

    fn registry() -> Arc<FixRegistry> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
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

    pub(super) const ORDERS: [&str; 2] = [
        "8=FIX.4.4|35=D|52=20240102-10:00:00.000|11=C1|55=ACME|54=1|40=2|44=100|38=5|59=0|10=0|",
        "8=FIX.4.4|35=D|52=20240102-10:00:01.000|11=C2|55=ACME|54=2|40=2|44=101|38=3|59=1|10=0|",
    ];

    #[test]
    fn a_message_is_market_data_as_it_is() {
        let message = messages(&ORDERS[..1]).remove(0);
        let held = MarketData::from(message.clone());
        assert_eq!(held.kind(), MarketKind::Fix);
        assert!(held.is_event());
        assert_eq!(held.marketdatakind(), MarketDataKind::Order);
        assert_eq!(held.get_curruuid(), message.get_curruuid());
        assert_eq!(held.get_side(), Side::Buy);
        assert_eq!(held.get_price(), message.get_price());
        assert_eq!(held.as_fix(), Some(&message));
        assert_eq!(FixMsg::try_from(held).unwrap(), message);
    }

    /// Written, a message is the leaves it reports: its one order here.
    #[test]
    fn a_message_row_reads_back_as_the_leaf_of_its_category() {
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
        assert_eq!(leaf.get_currunix(), message.get_currunix());
    }

    #[test]
    fn a_fix_message_held_whole_answers_its_category_through_the_trait() {
        let message = messages(&ORDERS[..1]).remove(0);
        let data = MarketData::from(message);
        assert_eq!(Market::marketdatakind(&data), MarketDataKind::Order);
        assert!(Market::is_sided(&data));
        assert_eq!(data.marketdatakind(), Market::marketdatakind(&data));
    }

    #[test]
    fn a_book_folds_the_leaves_a_message_splits_into() {
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
