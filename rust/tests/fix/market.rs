//! `rust/src/fix/market.rs`: the typed FIX boundary into graph operations.

use std::sync::Arc;

use super::{SoleMessage, committed_registry, fixed_codec};
use yggdryl::graph::book::{ENTRY_ID, ENTRY_REF_ID};
use yggdryl::graph::{
    BookEvent, Element, Event, ExecutionEvent, Market, MarketData, MarketKind, MdUpdateAction,
    Operation,
};
use yggdryl::{
    DataType, Decimal, Error, Field, FixCode, FixMsg, FixRegistry, IdKey, IdSource, IdType,
    Identifier, MarketDataKind, Scalar, Side, State, StructType,
};

fn message(line: &[u8]) -> FixMsg {
    fixed_codec(committed_registry())
        .sole_line(line)
        .expect("one FIX message")
}

/// The decimal text of a stated price or quantity, `None` where none is.
fn text(value: Option<Decimal>) -> Option<String> {
    value.map(|held| held.to_string())
}

/// A dated operation, read through the traits it answers.
trait Stated: Event + Operation {}

impl<T: Event + Operation + ?Sized> Stated for T {}

/// The dated order, quote or execution a value is; a trade or a control is
/// a fixture mistake.
fn operation_of(value: &MarketData) -> &dyn Stated {
    match value {
        MarketData::OrderEvent(operation) => operation,
        MarketData::QuoteEvent(operation) => operation,
        MarketData::ExecutionEvent(operation) => operation,
        other => panic!("expected an operation event, got {}", other.kind().as_str()),
    }
}

/// The event a value is: an operation, a trade or a snapshot control.
fn event_of(value: &MarketData) -> &dyn Event {
    match value {
        MarketData::OrderEvent(event) => event,
        MarketData::QuoteEvent(event) => event,
        MarketData::ExecutionEvent(event) => event,
        MarketData::TradeEvent(event) => event,
        MarketData::SnapshotEvent(event) => event,
        other => panic!("expected an event, got {}", other.kind().as_str()),
    }
}

/// The instant a value happened at.
fn currunix(value: &MarketData) -> i64 {
    event_of(value).get_currunix()
}

/// The book scope a value states.
fn scope_of(value: &MarketData) -> &str {
    value
        .book()
        .and_then(|book| book.scope.as_deref())
        .expect("a scoped entry")
}

/// Whether a value is part of a full-snapshot replacement.
fn is_full_snapshot(value: &MarketData) -> bool {
    value.book().and_then(|book| book.action) == Some(MdUpdateAction::Snapshot)
}

/// The entries alive on one side of `book`, best first: `bid` the bid
/// side, else the ask side.
fn alive(book: &BookEvent, bid: bool) -> Vec<&MarketData> {
    book.alive()
        .filter(|entry| entry.get_side().is_bid() == bid)
        .collect()
}

/// The deltas applied to one side of `book`, in the order they were.
fn deltas(book: &BookEvent, bid: bool) -> Vec<&MarketData> {
    book.deltas()
        .filter(|entry| entry.get_side().is_bid() == bid)
        .collect()
}

/// The books a lifted `marketdata` stream holds, each a `book_event` row.
fn books_of(reader: yggdryl::arrow::BatchReader) -> Vec<BookEvent> {
    MarketData::from_arrow_reader(reader)
        .unwrap()
        .map(|value| BookEvent::try_from(value?))
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap()
}

/// Every message a line parses into: what it states, then what its parse
/// split off it.
fn split(line: &[u8]) -> Vec<FixMsg> {
    fixed_codec(committed_registry())
        .parse_line(line)
        .expect("a line")
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("every message")
}

/// The one leaf a message is.
fn leaf_of(message: FixMsg) -> MarketData {
    let mut leaves = message.into_market_data().expect("one leaf");
    assert_eq!(leaves.len(), 1, "one message, one leaf");
    leaves.remove(0)
}

/// The execution a split-off execution message is.
fn execution_of(message: FixMsg) -> ExecutionEvent {
    ExecutionEvent::try_from(leaf_of(message)).expect("an execution")
}

/// Whether `split` names `source` and what `source` was read from as its
/// sources.
fn names_its_source(split: &FixMsg, source: &FixMsg) -> bool {
    split.get_srcuuids().contains(&source.get_curruuid())
        && source
            .get_srcuuids()
            .iter()
            .all(|held| split.get_srcuuids().contains(held))
}

#[test]
fn direct_market_categories_move_into_their_operation_kind_and_arrow_round_trip() {
    let cases: &[(&[u8], u8, MarketKind)] = &[
        (
            b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|44=100|38=5|10=0|",
            10,
            MarketKind::OrderEvent,
        ),
        (
            b"8=FIX.4.4|35=S|117=Q1|55=AAPL|54=1|132=99|134=7|10=0|",
            14,
            MarketKind::QuoteEvent,
        ),
        // An execution report reporting a fill is its order's report: the
        // fill is the execution its parse splits off.
        (
            b"8=FIX.4.4|35=8|17=E1|37=O1|55=AAPL|31=100|32=2|150=F|10=0|",
            10,
            MarketKind::OrderEvent,
        ),
    ];

    for (line, operation_id, kind) in cases {
        let source = message(line);
        assert_eq!(source.msgcat().code(), *operation_id);
        assert_eq!(
            source.get_by_tag(yggdryl::MARKETDATAKIND_TAG_NAME.0),
            Some(Scalar::MarketDataKind(source.msgcat()))
        );
        let operations = source.into_market_data().expect("a market category");
        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0].kind(), *kind, "{line:?}");
        assert_eq!(operations[0].marketdatakind().code(), *operation_id);
        assert_eq!(
            operations[0].book(),
            None,
            "a direct message is no book entry"
        );
        let expected = operations[0].clone();
        let encoded = MarketData::arrow_reader(operations, Some(1), None).unwrap();
        let actual = MarketData::from_arrow_reader(encoded)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(actual, expected);
    }
}

/// A12: an order's execution report splits once, at the parse, into the
/// order's report - `ORDR`, its own state - and the execution it reports -
/// `EXEC`, `FILLED`, its own identity and chain, naming the report as its
/// source.
#[test]
fn an_order_execution_splits_off_one_filled_execution_message() {
    let messages = split(
        b"8=FIX.4.4|35=8|52=20260921-10:00:00|17=E-1|37=O-9|11=C-9|39=1|150=F|55=AAPL|54=1|38=100|14=40|32=40|31=10.5|10=0|",
    );
    let [report, execution] = messages.as_slice() else {
        panic!("the report and its execution, got {}", messages.len())
    };
    assert_eq!(report.msgcat(), MarketDataKind::Order);
    assert_eq!(*report.get_state(), State::PartiallyFilled);
    assert!(!report.is_execution());
    assert_eq!(execution.msgcat(), MarketDataKind::Execution);
    assert_eq!(*execution.get_state(), State::Filled);
    assert!(execution.is_execution());
    assert!(names_its_source(execution, report));
    assert_ne!(execution.get_curruuid(), report.get_curruuid());
    assert_ne!(execution.get_crossuuid(), report.get_crossuuid());
    // Its chain is its own, keyed by the fill, and stored under its kind
    // and its side.
    assert_eq!(execution.get_crosscode(), "8:1:E-1");
    assert_eq!(report.get_crosscode(), "10:1:O-9");
    assert_eq!(text(execution.get_lastqty()).as_deref(), Some("40"));
    assert_eq!(text(execution.get_lastpx()).as_deref(), Some("10.5"));

    // One message, one leaf: the report an order, the execution an
    // execution reading `FILLED`.
    assert_eq!(leaf_of(report.clone()).kind(), MarketKind::OrderEvent);
    let leaf = execution_of(execution.clone());
    assert_eq!(*leaf.get_state(), State::Filled);

    // Both are rows of their own, read back whole.
    let schema = yggdryl::fix_schema(&committed_registry(), "fix").unwrap();
    for held in [report, execution] {
        let row = held.into_row(&schema).unwrap();
        let back = FixMsg::from_row(committed_registry(), &schema, &row).unwrap();
        assert_eq!(back.msgcat(), held.msgcat());
        assert_eq!(back.get_state(), held.get_state());
        assert_eq!(back.get_curruuid(), held.get_curruuid());
        assert_eq!(back.get_srcuuids(), held.get_srcuuids());
    }

    // A report of no fill splits nothing, and is its order's report all
    // the same: a lifecycle chains within one category.
    let acknowledged = split(b"8=FIX.4.4|35=8|17=A-1|37=O-9|39=0|150=0|55=AAPL|54=1|10=0|");
    assert_eq!(acknowledged.len(), 1);
    assert_eq!(acknowledged[0].msgcat(), MarketDataKind::Order);
}

/// A fill's report and the execution split off it are settled by what the
/// split moved alone - the category, the state, the chain, the sources -
/// never by rebuilding what the fields state: each is exactly what a whole
/// settle of it answers, anomalies included, over every line of the bridge
/// capture that splits an execution off and over the numeric fills here.
#[test]
fn a_fill_and_its_report_are_what_a_whole_settle_answers() {
    let codec = fixed_codec(committed_registry()).with_exclude_msgtypes::<[&str; 0], &str>([]);
    let numeric: [&[u8]; 3] = [
        b"8=FIX.4.4|35=8|52=20260921-10:00:00|17=E-1|37=O-9|11=C-9|39=1|150=F|55=AAPL|54=1|38=100|14=40|32=40|31=10.5|10=0|",
        b"8=FIX.4.4|35=8|17=E-2|37=O-9|39=1|150=F|55=AAPL|54=2|32=5|31=10|10=0|",
        b"8=FIX.4.2|35=8|17=E-3|37=O-9|117=Q-1|39=2|150=2|55=AAPL|54=1|32=5|31=10|453=1|448=BROKER|447=D|452=1|10=0|",
    ];
    let lines = include_bytes!("ulbridge.log")
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .chain(numeric);
    let (mut fills, mut reports) = (0, 0);
    for line in lines {
        let Ok(parsed) = codec.parse_line(line) else {
            continue;
        };
        let messages: Vec<FixMsg> = parsed.filter_map(Result::ok).collect();
        let Some((report, split)) = messages.split_first() else {
            continue;
        };
        if !split
            .iter()
            .any(|held| held.msgcat() == MarketDataKind::Execution)
        {
            continue;
        }
        reports += usize::from(report.msgcat() != MarketDataKind::Trade);
        for held in &messages {
            fills += usize::from(held.msgcat() == MarketDataKind::Execution);
            let mut settled = held.clone();
            settled.finalize();
            assert!(
                settled == *held,
                "a {} message split off {:?} is not what a settle answers",
                held.msgcat().as_str(),
                String::from_utf8_lossy(line)
            );
            assert_eq!(settled.anomalies(), held.anomalies());
            assert_eq!(settled.get_currhashcode(), held.get_currhashcode());
            assert_eq!(settled.get_curruuid(), held.get_curruuid());
        }
    }
    // Every numeric fill and the bridge's own: a fixture that stopped
    // splitting would pass the loop by checking nothing.
    assert!(
        reports > numeric.len(),
        "{reports} reports split a fill off"
    );
    assert!(fills >= reports, "{fills} executions");
}

/// A12: a trade splits off one sided execution message per `NoSides(552)`
/// occurrence, each an `ExecutionReport` of its side and `FILLED`, and the
/// trade itself is no leaf: its fills are those messages, once.
#[test]
fn a_trade_splits_off_one_sided_execution_message_per_side() {
    let messages = split(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|150=F|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|54=1|1427=BUY-EXEC|1009=4|37=BUY-ORDER|11=BUY-CLIENT|54=2|1427=SELL-EXEC|1009=6|37=SELL-ORDER|11=SELL-CLIENT|10=0|",
    );
    let [trade, buy, sell] = messages.as_slice() else {
        panic!("the trade and its two sides, got {}", messages.len())
    };
    assert_eq!(trade.msgcat(), MarketDataKind::Trade);
    assert!(
        trade.market_data().unwrap().is_empty(),
        "a trade is no leaf: its fills are its sides' executions"
    );
    for side in [buy, sell] {
        assert_eq!(side.msgcat(), MarketDataKind::Execution);
        assert_eq!(*side.get_state(), State::Filled);
        assert!(names_its_source(side, trade));
    }
    assert!(buy.get_side().is_bid());
    assert!(sell.get_side().is_ask());
    // A side's last executed quantity is its `SideLastQty(1009)`, beside
    // the trade's last price; a side states no price and no quantity.
    assert_eq!(text(buy.get_lastqty()).as_deref(), Some("4"));
    assert_eq!(text(sell.get_lastqty()).as_deref(), Some("6"));
    assert_eq!(text(buy.get_lastpx()).as_deref(), Some("101.25"));
    assert_eq!((buy.get_price(), buy.get_quantity()), (None, None));
    // The side's identifiers are the execution's.
    assert_eq!(buy.get_identifiers().get(&IdType::ExecId), Some("BUY-EXEC"));
    assert_eq!(
        sell.get_identifiers().get(&IdType::ExecId),
        Some("SELL-EXEC")
    );
    assert_eq!(
        buy.get_identifiers().get(&IdType::OrderId),
        Some("BUY-ORDER")
    );
    assert_eq!(
        sell.get_identifiers().get(&IdType::ClOrdId),
        Some("SELL-CLIENT")
    );
    assert_ne!(buy.get_curruuid(), sell.get_curruuid());
    assert_ne!(buy.get_crossuuid(), sell.get_crossuuid());
    assert!(
        buy.get_crosscode().starts_with("8:1:"),
        "{}",
        buy.get_crosscode()
    );
    assert!(
        sell.get_crosscode().starts_with("8:2:"),
        "{}",
        sell.get_crosscode()
    );

    let leaves: Vec<ExecutionEvent> = [buy, sell]
        .into_iter()
        .map(|side| execution_of(side.clone()))
        .collect();
    for leaf in &leaves {
        assert_eq!(*leaf.get_state(), State::Filled);
    }
    let expected: Vec<MarketData> = leaves.into_iter().map(MarketData::from).collect();
    let encoded = MarketData::arrow_reader(expected.clone(), Some(1), None).unwrap();
    let actual = drained(MarketData::from_arrow_reader(encoded).unwrap()).unwrap();
    assert_eq!(actual, expected);
}

/// An order list is an order batch (`ORDB`): it splits into one order
/// message per `NoOrders(73)` entry, each an order (`ORDR`) chained by the
/// entry's own `ClOrdID(11)` on its side, naming the list among its
/// sources; the list itself is no leaf.
#[test]
fn an_order_list_splits_into_one_sided_order_per_entry() {
    let messages = split(
        b"8=FIX.4.4|35=E|52=20260921-10:00:00|66=L1|394=3|68=2|73=2|11=C1|67=1|55=AAPL|54=1|38=5|40=2|44=100.5|11=C2|67=2|55=MSFT|54=2|38=7|40=2|44=300.25|10=0|",
    );
    let [list, buy, sell] = messages.as_slice() else {
        panic!("the list and its two orders, got {}", messages.len())
    };
    assert_eq!(list.msgcat(), MarketDataKind::OrderBatch);
    assert!(!list.is_sided());
    for order in [buy, sell] {
        assert_eq!(order.msgcat(), MarketDataKind::Order);
        assert!(order.is_sided());
        assert!(names_its_source(order, list));
    }
    assert_eq!((buy.get_side(), sell.get_side()), (Side::Buy, Side::Sell));
    assert_eq!(
        (buy.get_crosscode(), sell.get_crosscode()),
        ("10:1:C1", "10:2:C2")
    );
    assert_eq!(text(buy.get_price()).as_deref(), Some("100.5"));
    assert_eq!(text(sell.get_quantity()).as_deref(), Some("7"));
    assert_eq!(buy.get_ticker(), Some("AAPL"));
    assert_eq!(sell.get_ticker(), Some("MSFT"));
    // Each order is a leaf of its own, as a single order message is.
    let leaves: Vec<MarketData> = [buy, sell]
        .into_iter()
        .map(|order| leaf_of(order.clone()))
        .collect();
    assert!(
        leaves
            .iter()
            .all(|leaf| leaf.kind() == MarketKind::OrderEvent)
    );
}

/// A mass quote is a quote batch (`QUOB`): each `NoQuoteEntries(295)` entry
/// of each `NoQuoteSets(296)` set is a quote (`QUOT`) chained by its set
/// and entry, and a two-sided entry splits again into its `BUY` and `SELL`
/// quotes as any two-sided quote does.
#[test]
fn a_mass_quote_splits_into_sided_quotes_per_entry() {
    let messages = split(
        b"8=FIX.4.4|35=i|52=20260921-10:00:00|117=MQ1|296=1|302=S1|295=1|299=E1|55=AAPL|132=100|133=101|134=5|135=6|10=0|",
    );
    let [batch, quote, bid, offer] = messages.as_slice() else {
        panic!(
            "the batch, its quote and the quote's two sides, got {}",
            messages.len()
        )
    };
    assert_eq!(batch.msgcat(), MarketDataKind::QuoteBatch);
    assert_eq!(quote.msgcat(), MarketDataKind::Quotation);
    assert_eq!(quote.get_crosscode(), "14:0:QuoteSetID=S1|QuoteEntryID=E1");
    assert_eq!(text(quote.get_bidpx()).as_deref(), Some("100"));
    assert_eq!(text(quote.get_askqty()).as_deref(), Some("6"));
    assert_eq!(bid.get_crosscode(), "14:1:QuoteSetID=S1|QuoteEntryID=E1");
    assert_eq!(offer.get_crosscode(), "14:2:QuoteSetID=S1|QuoteEntryID=E1");
    assert_eq!(text(bid.get_price()).as_deref(), Some("100"));
    assert_eq!(text(offer.get_price()).as_deref(), Some("101"));
}

/// An order mass cancel report names each affected order: its entries are
/// orders chained by the order they name - `AffectedOrderID(535)` read as
/// the order's `OrderID(37)` - so each joins that order's chain.
#[test]
fn a_mass_cancel_report_splits_into_the_orders_it_names() {
    let messages = split(
        b"8=FIX.4.4|35=r|52=20260921-10:00:00|37=MC1|1369=R1|530=7|531=7|534=2|1824=C1|535=O1|1824=C2|535=O2|10=0|",
    );
    let [report, first, second] = messages.as_slice() else {
        panic!("the report and its two orders, got {}", messages.len())
    };
    assert_eq!(report.msgcat(), MarketDataKind::OrderBatch);
    assert_eq!(
        (first.msgcat(), second.msgcat()),
        (MarketDataKind::Order, MarketDataKind::Order)
    );
    assert_eq!(
        (first.get_crosscode(), second.get_crosscode()),
        ("10:0:O1", "10:0:O2")
    );
}

/// In a lifecycle, the entry naming a live order joins its chain - the one
/// side alive under the name, whose side and code it takes - and states
/// the cancellation it reports, a terminal state that ends the chain.
#[test]
fn a_mass_cancel_report_entry_ends_the_order_it_names_in_the_lifecycle() {
    let codec = fixed_codec(committed_registry());
    let lines: [&[u8]; 3] = [
        b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|38=5|44=100|10=0|",
        b"8=FIX.4.4|35=8|52=20260921-10:00:01|11=C1|37=O1|150=0|39=0|54=1|55=AAPL|10=0|",
        b"8=FIX.4.4|35=r|52=20260921-10:00:02|37=MC1|1369=R1|530=7|531=7|534=1|1824=C1|535=O1|10=0|",
    ];
    let parsed: Vec<FixMsg> = codec
        .parse_lines(lines)
        .collect::<yggdryl::Result<_>>()
        .expect("the order, its ack, the report and its entry");
    let chained: Vec<FixMsg> = codec
        .lifecycle(parsed)
        .collect::<yggdryl::Result<_>>()
        .expect("the walk");
    let ack = chained
        .iter()
        .find(|held| held.header().msgtype() == "8")
        .expect("the acknowledgement");
    let entry = chained
        .iter()
        .find(|held| held.header().msgtype() == "r" && held.msgcat() == MarketDataKind::Order)
        .expect("the entry");
    assert_eq!(entry.get_prevuuid(), Some(ack.get_curruuid()));
    assert_eq!(entry.get_crosscode(), "10:1:C1");
    assert_eq!(entry.get_crossuuid(), ack.get_crossuuid());
    assert_eq!(entry.get_side(), Side::Buy);
    assert_eq!(*entry.get_state(), State::Canceled);
}

/// A trade side stating no `Side(54)`.
const NO_SIDE: &[u8] = b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|150=F|55=AAPL|32=4|31=101.25|60=20260921-10:00:00|552=1|1427=NO-SIDE|1009=4|37=ORDER-1|11=CLIENT-1|10=0|";

/// A trade side stating a `Side(54)` no side reads, which the parse passes
/// over as an anomaly of the trade.
const UNREADABLE_SIDE: &[u8] = b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|150=F|55=AAPL|32=4|31=101.25|60=20260921-10:00:00|552=1|54=QQ|1427=BAD-SIDE|1009=4|37=ORDER-1|11=CLIENT-1|10=0|";

/// A trade side stating no side, or one no side reads, is still a fill:
/// it splits off an execution of side `UNKN`, chained by its own
/// identifiers; the unreadable side stays beside the trade as the anomaly
/// the parse noted, and stating none is no anomaly.
#[test]
fn a_trade_side_stating_no_side_splits_off_an_unknown_sided_execution() {
    for (line, stable) in [(NO_SIDE, "NO-SIDE"), (UNREADABLE_SIDE, "BAD-SIDE")] {
        let messages = split(line);
        let [trade, execution] = messages.as_slice() else {
            panic!("the trade and its one side, got {}", messages.len())
        };
        assert_eq!(execution.msgcat(), MarketDataKind::Execution);
        assert_eq!(execution.get_side(), Side::Unknown);
        assert_eq!(*execution.get_state(), State::Filled);
        assert!(names_its_source(execution, trade));
        assert_eq!(text(execution.get_lastqty()).as_deref(), Some("4"));
        // An unsided execution's cross code states side `0`.
        assert_eq!(
            execution.get_crosscode(),
            format!("8:0:7:ORDER-1|1427:{}:{stable}", stable.len())
        );
        let leaf = execution_of(execution.clone());
        assert_eq!(leaf.get_side(), Side::Unknown);
        assert_eq!(*leaf.get_state(), State::Filled);
    }

    let stated_none = split(NO_SIDE).remove(0);
    assert!(
        stated_none.anomalies().is_empty(),
        "stating nothing is no anomaly"
    );
    let unreadable = split(UNREADABLE_SIDE).remove(0);
    assert!(
        unreadable
            .anomalies()
            .iter()
            .any(|anomaly| anomaly.reason().contains("QQ")),
        "{:?}",
        unreadable.anomalies()
    );
}

#[test]
fn a_trade_sides_average_price_is_its_own_fact() {
    let messages = split(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|487=0|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=1|54=1|1427=BUY-EXEC|1009=4|1852=100.5|10=0|",
    );
    let [_, buy] = messages.as_slice() else {
        panic!("one execution per stated trade side")
    };
    let buy = execution_of(buy.clone());
    assert_eq!(text(buy.get_avgpx()).as_deref(), Some("100.5"));
    assert_eq!(buy.get_price(), None, "an average is no price");
    assert_eq!(
        text(buy.get_lastpx()).as_deref(),
        Some("101.25"),
        "the last price, never the average"
    );
}

#[test]
fn only_initial_trade_capture_reports_split_without_requiring_exec_type() {
    let messages = split(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|487=0|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|54=1|1427=BUY-EXEC|1009=10|54=2|1427=SELL-EXEC|1009=10|10=0|",
    );
    assert_eq!(messages.len(), 3);
    let executed = messages[0].get_currunix();
    for side in &messages[1..] {
        assert_eq!(side.get_execunix(), Some(executed));
    }

    for line in [
        b"8=FIX.4.4|35=AE|571=CANCEL|487=1|150=F|552=1|54=1|1427=E1|10=0|".as_slice(),
        b"8=FIX.4.4|35=AR|571=ACK|487=0|150=F|552=1|54=1|1427=E1|10=0|".as_slice(),
    ] {
        let messages = split(line);
        assert_eq!(
            messages.len(),
            1,
            "a cancel or an acknowledgement splits nothing"
        );
        assert!(
            messages[0].market_data().unwrap().is_empty(),
            "a cancellation or acknowledgement is not an execution trade"
        );
    }
}

#[test]
fn stable_trade_side_ids_make_group_order_irrelevant_to_identity() {
    let identities = |line: &[u8]| {
        let mut sides: Vec<_> = split(line)[1..]
            .iter()
            .map(|side| (side.get_curruuid(), side.get_crosscode().to_owned()))
            .collect();
        sides.sort();
        sides
    };
    let first = identities(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|487=0|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|54=1|1427=BUY-EXEC|1009=4|54=2|1427=SELL-EXEC|1009=6|10=0|",
    );
    let second = identities(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|487=0|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|54=2|1427=SELL-EXEC|1009=6|54=1|1427=BUY-EXEC|1009=4|10=0|",
    );
    assert_eq!(first.len(), 2);
    assert_eq!(first, second);
}

#[test]
fn anonymous_trade_sides_are_order_independent_and_stable_id_tags_do_not_collide() {
    let codes = |line: &[u8]| {
        let mut sides: Vec<_> = split(line)[1..]
            .iter()
            .map(|side| (side.get_curruuid(), side.get_crosscode().to_owned()))
            .collect();
        sides.sort();
        sides
    };
    let first = codes(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|487=0|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|54=1|1009=4|54=2|1009=6|10=0|",
    );
    let second = codes(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|487=0|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|54=2|1009=6|54=1|1009=4|10=0|",
    );
    assert_eq!(first, second);

    let tagged = codes(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T2|487=0|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|54=1|1427=SAME|1009=4|54=1|1506=SAME|1009=6|10=0|",
    );
    assert_eq!(tagged.len(), 2);
    assert_ne!(
        tagged[0].1, tagged[1].1,
        "the identifier tag distinguishes equal values"
    );
}

/// A12: a capture folds into books holding each execution exactly once -
/// the trade's sided executions and the order's fill - and the trade and
/// the order's report never add one.
#[test]
fn a_capture_folds_each_execution_into_the_books_exactly_once() {
    let codec = fixed_codec(committed_registry()).with_batch_row_size(1);
    let lines: [&[u8]; 3] = [
        b"8=FIX.4.4|35=D|52=20260921-09:59:59|11=C-9|55=AAPL|54=1|44=10.5|38=100|326=17|10=0|",
        b"8=FIX.4.4|35=8|52=20260921-10:00:00|17=E-1|37=O-9|11=C-9|39=1|150=F|55=AAPL|54=1|38=100|14=40|32=40|31=10.5|10=0|",
        b"8=FIX.4.4|35=AE|52=20260921-10:00:01|571=T1|150=F|55=AAPL|32=10|31=101.25|60=20260921-10:00:01|552=2|54=1|1427=BUY-EXEC|1009=4|54=2|1427=SELL-EXEC|1009=6|10=0|",
    ];
    let messages: Vec<FixMsg> = lines
        .iter()
        .flat_map(|line| codec.parse_line(line).unwrap())
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    let execution_messages: Vec<String> = messages
        .iter()
        .filter(|message| message.msgcat() == MarketDataKind::Execution)
        .map(|message| message.get_crosscode().to_owned())
        .collect();
    assert_eq!(execution_messages.len(), 3);
    let books = books_of(
        codec
            .book_arrow_reader(codec.lifecycle(messages), 0)
            .expect("a book stream"),
    );
    let mut folded: Vec<String> = books
        .iter()
        .flat_map(|book| book.executions())
        .map(|execution| execution.get_crosscode().to_owned())
        .collect();
    folded.sort();
    let mut expected = execution_messages;
    expected.sort();
    assert_eq!(folded, expected, "each execution once");
    for book in &books {
        for execution in book.executions() {
            assert_eq!(*execution.get_state(), State::Filled);
        }
    }
}

/// A13: a quote stating a bid and an offer and no side splits into two
/// sided quotes - `BUY` with the bid's facts, `SELL` with the offer's, each
/// keeping both - and reaches the book as two quote events, once each.
#[test]
fn a_two_sided_quote_splits_into_two_sided_quotes_and_two_book_quotes() {
    let codec = fixed_codec(committed_registry()).with_batch_row_size(1);
    let messages: Vec<FixMsg> = codec
        .parse_line(b"8=FIX.4.4|35=S|52=20260921-10:00:00|117=Q1|55=AAPL|15=USD|132=99|134=7|133=101|135=8|188=98.5|189=0.5|326=17|10=0|")
        .unwrap()
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    let [quote, bid, ask] = messages.as_slice() else {
        panic!("the quote and its two sides, got {}", messages.len())
    };
    assert_eq!(quote.get_side(), yggdryl::Side::Unknown);
    assert_eq!(quote.get_price(), None);
    assert_eq!(bid.get_side(), yggdryl::Side::Buy);
    assert_eq!(ask.get_side(), yggdryl::Side::Sell);
    for side in [bid, ask] {
        assert_eq!(side.msgcat(), MarketDataKind::Quotation);
        assert!(names_its_source(side, quote));
        // Each keeps the pair its source stated (A20).
        assert_eq!(text(side.get_bidpx()).as_deref(), Some("99"));
        assert_eq!(text(side.get_askqty()).as_deref(), Some("8"));
        assert_eq!(side.get_bidccy().map(yggdryl::Ccy::as_str), Some("USD"));
    }
    assert_eq!(text(bid.get_price()).as_deref(), Some("99"));
    assert_eq!(text(bid.get_quantity()).as_deref(), Some("7"));
    assert_eq!(text(bid.get_spotrate()).as_deref(), Some("98.5"));
    assert_eq!(text(bid.get_forwardpoints()).as_deref(), Some("0.5"));
    assert_eq!(text(ask.get_price()).as_deref(), Some("101"));
    assert_eq!(text(ask.get_quantity()).as_deref(), Some("8"));
    assert_eq!(bid.get_crosscode(), "14:1:Q1");
    assert_eq!(ask.get_crosscode(), "14:2:Q1");

    let books = books_of(codec.book_arrow_reader(messages, 0).unwrap());
    let quotes: Vec<&MarketData> = books
        .last()
        .expect("a book")
        .alive()
        .filter(|entry| entry.kind() == MarketKind::QuoteEvent)
        .collect();
    assert_eq!(quotes.len(), 2, "two quotes, once each");
    let book = books.last().unwrap();
    assert_eq!(text(book.get_bidpx()).as_deref(), Some("99"));
    assert_eq!(text(book.get_askpx()).as_deref(), Some("101"));
}

/// A quote stating only a bid's facts and no side is that bid: its parse
/// splits off one `BUY` quote, which the book holds, and the unsided source
/// stays out of it - a book stream never meets an entry no side can hold.
#[test]
fn a_quote_stating_one_side_and_no_side_splits_into_that_sided_quote() {
    let codec = fixed_codec(committed_registry()).with_batch_row_size(1);
    let messages: Vec<FixMsg> = codec
        .parse_line(b"8=FIX.4.4|35=S|52=20260921-10:00:00|117=Q2|55=AAPL|15=USD|132=99|134=7|10=0|")
        .unwrap()
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    let [quote, bid] = messages.as_slice() else {
        panic!("the quote and its one side, got {}", messages.len())
    };
    assert_eq!(quote.get_side(), yggdryl::Side::Unknown);
    assert_eq!(bid.get_side(), yggdryl::Side::Buy);
    assert!(names_its_source(bid, quote));
    assert_eq!(text(bid.get_price()).as_deref(), Some("99"));
    assert_eq!(text(bid.get_quantity()).as_deref(), Some("7"));
    assert_eq!(bid.get_crosscode(), "14:1:Q2");

    let books = books_of(codec.book_arrow_reader(messages, 0).unwrap());
    let book = books.last().expect("a book");
    assert_eq!(book.alive().count(), 1, "the sided quote alone");
    assert_eq!(text(book.get_bidpx()).as_deref(), Some("99"));
    assert_eq!(book.get_askpx(), None);
}

/// A14: an execution reads `FILLED` whatever its report stated, and its
/// report keeps its own state; a trade's sided executions read `FILLED`.
#[test]
fn an_execution_reads_filled_and_its_report_keeps_its_state() {
    let [report, execution] = <[FixMsg; 2]>::try_from(split(
        b"8=FIX.4.4|35=8|17=E-2|37=O-9|39=1|150=F|55=AAPL|54=2|32=5|31=10|10=0|",
    ))
    .unwrap();
    assert_eq!(*report.get_state(), State::PartiallyFilled);
    assert_eq!(*execution.get_state(), State::Filled);
    let leaf = execution_of(execution);
    assert_eq!(*leaf.get_state(), State::Filled);
    for side in
        &split(b"8=FIX.4.4|35=AE|571=T9|487=0|55=AAPL|32=2|31=10|552=1|54=2|1427=S-1|1009=2|10=0|")
            [1..]
    {
        assert_eq!(*side.get_state(), State::Filled);
    }
}

/// A20/A21: a quote's bid and ask are its `BidPx(132)`/`BidSize(134)` and
/// `OfferPx(133)`/`OfferSize(135)` - typed, so no leaf's metadata repeats
/// them - each in the currency a `BidCurrency` or `AskCurrency` field names,
/// else the message's; nothing fills them from a price.
#[test]
fn a_quote_states_its_bid_and_ask_in_their_currencies() {
    let quote = message(
        b"8=FIX.4.4|35=S|117=Q2|55=EURUSD|54=1|15=EUR|132=1.1|134=5|133=1.2|135=6|BidCurrency=USD|AskCurrency=GBP|10=0|",
    );
    assert_eq!(text(quote.get_bidpx()).as_deref(), Some("1.1"));
    assert_eq!(text(quote.get_bidqty()).as_deref(), Some("5"));
    assert_eq!(text(quote.get_askpx()).as_deref(), Some("1.2"));
    assert_eq!(text(quote.get_askqty()).as_deref(), Some("6"));
    assert_eq!(quote.get_bidccy().map(yggdryl::Ccy::as_str), Some("USD"));
    assert_eq!(quote.get_askccy().map(yggdryl::Ccy::as_str), Some("GBP"));
    let leaf = leaf_of(quote);
    for key in [
        "bidpx",
        "offerpx",
        "bidsize",
        "offersize",
        "bidcurrency",
        "askcurrency",
    ] {
        assert!(
            !keys(&leaf)
                .iter()
                .any(|held| held.eq_ignore_ascii_case(key)),
            "{key}"
        );
    }

    // Stated in the message's currency where no field names another.
    let plain = message(b"8=FIX.4.4|35=S|117=Q3|55=AAPL|54=2|15=USD|133=101|135=8|10=0|");
    assert_eq!(plain.get_askccy().map(yggdryl::Ccy::as_str), Some("USD"));
    assert_eq!((plain.get_bidpx(), plain.get_bidccy()), (None, None));

    // A buyer's own price and quantity are its bid, in its currency.
    let order = message(b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|44=100|38=5|15=USD|10=0|");
    assert_eq!(
        (order.get_bidpx(), order.get_bidqty(), order.get_bidccy()),
        (
            Some(yggdryl::Decimal::from_int(100)),
            Some(yggdryl::Decimal::from_int(5)),
            Some(&yggdryl::Ccy::new("USD").unwrap())
        )
    );
    assert_eq!(order.get_askpx(), None, "a buyer states no ask");
}

#[test]
fn msgtype_edits_resettle_derived_operation_ids_and_leave_stated_ids_alone() {
    let mut derived = message(b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|44=100|38=5|10=0|");
    assert_eq!(derived.msgcat(), MarketDataKind::Order);

    derived.set(35, Scalar::from("S")).unwrap();
    assert_eq!(derived.msgcat(), MarketDataKind::Quotation);
    assert!(matches!(
        derived.market_data().unwrap().as_slice(),
        [MarketData::QuoteEvent(_)]
    ));

    assert_eq!(derived.remove(35).unwrap(), Some(Scalar::from("S")));
    assert_eq!(derived.msgcat(), MarketDataKind::Unknown);
    assert!(derived.market_data().unwrap().is_empty());

    let mut stated = message(b"8=FIX.4.4|35=D|65016=8|11=C1|55=AAPL|54=1|44=100|38=5|10=0|");
    assert_eq!(stated.msgcat(), MarketDataKind::Execution);
    let execution_hash = stated.get_currhashcode();
    let execution_uuid = stated.get_curruuid();
    stated.set(35, Scalar::from("S")).unwrap();
    assert_eq!(
        stated.msgcat(),
        MarketDataKind::Execution,
        "an explicit MsgCat row value owns the category"
    );
    assert_ne!(stated.get_currhashcode(), execution_hash);
    assert_ne!(stated.get_curruuid(), execution_uuid);
    let explicit_hash = stated.get_currhashcode();
    stated
        .set(yggdryl::MARKETDATAKIND_TAG_NAME.0, Scalar::from(14_i32))
        .unwrap();
    assert_eq!(stated.msgcat(), MarketDataKind::Quotation);
    assert_ne!(stated.get_currhashcode(), explicit_hash);
    assert_eq!(
        stated.remove(yggdryl::MARKETDATAKIND_TAG_NAME.0).unwrap(),
        Some(Scalar::MarketDataKind(MarketDataKind::Quotation))
    );
    assert_eq!(stated.msgcat(), MarketDataKind::Quotation);
}

#[test]
fn msgcat_registry_values_are_stable_int32_operation_ids() {
    let registry = committed_registry();
    let field = registry.field(yggdryl::MARKETDATAKIND_TAG_NAME.0).unwrap();
    assert_eq!(field.dtype(), &DataType::MarketDataKind);
    let codes = registry.codeset_of(field).expect("the MsgCat vocabulary");
    for (name, value) in [
        ("UNKN", "0"),
        ("ACCT", "1"),
        ("ALLO", "2"),
        ("BOOK", "3"),
        ("CERT", "4"),
        ("COLL", "5"),
        ("COMM", "6"),
        ("CONF", "7"),
        ("EXEC", "8"),
        ("MKST", "9"),
        ("ORDR", "10"),
        ("PAYM", "11"),
        ("POSN", "12"),
        ("PRTY", "13"),
        ("QUOT", "14"),
        ("REGI", "15"),
        ("RISK", "16"),
        ("SECU", "17"),
        ("SESS", "18"),
        ("SETL", "19"),
        ("STRM", "20"),
        ("TRAD", "21"),
    ] {
        assert_eq!(
            codes.code_by_name(name).map(|code| code.value()),
            Some(value)
        );
    }
}

#[test]
fn msgcat_operation_ids_are_intrinsic_while_custom_msgtypes_choose_a_category() {
    let mut registry = FixRegistry::new();
    let refused = registry
        .set_codeset("msgcatcodeset", &[FixCode::new("ORDR", "99")])
        .unwrap_err();
    assert!(matches!(refused, Error::Conflict { .. }), "{refused}");
    let refused = registry
        .merge_codeset("msgcatcodeset", &[FixCode::new("VENUE", "99")])
        .unwrap_err();
    assert!(matches!(refused, Error::Conflict { .. }), "{refused}");
    let refused = registry
        .merge_codeset("msgcatcodeset", &[FixCode::new("ORDR", "99")])
        .unwrap_err();
    assert!(matches!(refused, Error::Conflict { .. }), "{refused}");
    let refused = registry
        .merge_codeset("msgcatcodeset", &[FixCode::new("ORDR", "010")])
        .unwrap_err();
    assert!(matches!(refused, Error::Conflict { .. }), "{refused}");
    let refused = registry
        .merge_codeset(
            "msgcatcodeset",
            &[FixCode::new("ORDR", "10").with_aliases(["QUOT"])],
        )
        .unwrap_err();
    assert!(matches!(refused, Error::Conflict { .. }), "{refused}");
    // The set is the enum's, as the state set is: a code states its
    // member's name, code and description and nothing more - an alias, even
    // of its own name, is a second spelling the enum does not own.
    let refused = registry
        .merge_codeset(
            "msgcatcodeset",
            &[FixCode::new("ORDR", "10").with_aliases(["ordr"])],
        )
        .unwrap_err();
    assert!(matches!(refused, Error::Conflict { .. }), "{refused}");
    registry
        .merge_codeset(
            "msgcatcodeset",
            &[FixCode::new("ORDR", "10").with_description(MarketDataKind::Order.description())],
        )
        .expect("an exact named subset changes no intrinsic identifier");
    let refused = registry.remove_codeset("msgcatcodeset").unwrap_err();
    assert!(matches!(refused, Error::Conflict { .. }), "{refused}");
    assert_eq!(
        registry
            .codeset("msgcatcodeset")
            .unwrap()
            .code_by_name("ORDR")
            .map(|code| code.value()),
        Some("10")
    );

    let mut venue =
        DataType::from(StructType::from_fields([]).unwrap()).required_field("venuequote");
    venue.as_fix_mut().set_msgtype("ZZ").unwrap();
    venue.as_fix_mut().set_msgcat("QUOT").unwrap();
    registry.insert(venue).unwrap();
    let custom = fixed_codec(Arc::new(registry))
        .sole_line(b"8=FIX.4.4|35=ZZ|52=20260921-10:00:00|10=0|")
        .unwrap();
    assert_eq!(custom.msgcat(), MarketDataKind::Quotation);
}

#[test]
fn codec_streams_fix_messages_through_books_into_arrow_with_coherent_prices() {
    let codec = fixed_codec(committed_registry()).with_batch_row_size(1);
    let snapshot = codec
        .sole_line(b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|326=17|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|")
        .unwrap();
    let update = codec
        .sole_line(b"8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|326=17|268=2|279=1|269=0|278=B1|270=101|271=11|279=0|269=2|278=T1|270=101|271=2|10=0|")
        .unwrap();
    // The message is a book message; each leaf it expands into states its
    // own category below.
    assert_eq!(snapshot.msgcat(), MarketDataKind::Book);
    assert_eq!(update.msgcat(), MarketDataKind::Book);

    let reader = codec
        .book_arrow_reader([snapshot, update], 0)
        .expect("a centralized FIX book reader");
    let books = books_of(reader);

    assert_eq!(books.len(), 2);
    assert_eq!(
        books[0].best_price(yggdryl::Side::Buy).unwrap().to_string(),
        "100"
    );
    assert_eq!(
        books[0]
            .best_price(yggdryl::Side::Sell)
            .unwrap()
            .to_string(),
        "102"
    );
    assert_eq!(text(books[0].get_price()).as_deref(), Some("101"));
    assert_eq!(
        books[1].best_price(yggdryl::Side::Buy).unwrap().to_string(),
        "101"
    );
    assert_eq!(
        books[1]
            .best_price(yggdryl::Side::Sell)
            .unwrap()
            .to_string(),
        "102"
    );
    assert_eq!(text(books[1].get_price()).as_deref(), Some("101.5"));
    // The best tradable levels are the book's bid and ask (A22).
    assert_eq!(text(books[1].get_bidpx()).as_deref(), Some("101"));
    assert_eq!(text(books[1].get_askqty()).as_deref(), Some("12"));
    assert_eq!(books[1].executions().len(), 1);
    // A leaf states its own category: a book entry naming no order is a
    // quote, whatever the message's `BOOK`.
    assert_eq!(
        alive(&books[1], true)[0].marketdatakind(),
        MarketDataKind::Quotation
    );
    assert_eq!(
        MarketData::from(books[1].executions()[0].clone()).marketdatakind(),
        MarketDataKind::Execution
    );
}

#[test]
fn codec_book_admission_skips_noncontributing_records_between_market_events() {
    let codec = fixed_codec(committed_registry()).with_batch_row_size(1);
    let admitted = [
        b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|44=100|38=5|10=0|".as_slice(),
        b"8=FIX.4.4|35=S|52=20260921-10:00:01|117=Q1|55=AAPL|54=2|133=101|135=7|10=0|".as_slice(),
        b"8=FIX.4.4|35=8|52=20260921-10:00:02|17=E1|37=O1|55=AAPL|54=1|31=100|32=2|150=F|10=0|"
            .as_slice(),
        b"8=FIX.4.4|35=W|52=20260921-10:00:03|55=AAPL|268=1|269=0|278=B1|270=99|271=10|10=0|"
            .as_slice(),
        b"8=FIX.4.4|35=X|52=20260921-10:00:04|55=AAPL|268=1|279=1|269=0|278=B1|271=11|10=0|"
            .as_slice(),
    ]
    .map(|line| codec.parse_fix_line(line).unwrap());
    let ignored = [
        b"8=FIX.4.4|35=0|52=20260922-10:00:00|10=0|".as_slice(),
        b"8=FIX.4.4|35=AE|52=20260922-10:00:00|571=T1|487=0|55=AAPL|32=1|31=100|552=1|54=1|1427=E2|1009=1|10=0|".as_slice(),
        b"8=FIX.4.4|35=8|52=20260922-10:00:00|17=ACK|37=O1|150=0|10=0|".as_slice(),
        b"8=FIX.4.4|35=AR|52=20260922-10:00:00|571=ACK|487=0|150=F|10=0|".as_slice(),
        b"8=FIX.4.4|35=AD|52=20260922-10:00:00|568=REQUEST|10=0|".as_slice(),
        b"8=FIX.4.4|35=AQ|52=20260922-10:00:00|568=ACK|10=0|".as_slice(),
        b"8=FIX.4.4|35=V|52=20260922-10:00:00|262=REQUEST|10=0|".as_slice(),
    ]
    .map(|line| codec.parse_fix_line(line).unwrap());
    let mixed = ignored
        .iter()
        .cloned()
        .zip(admitted.iter().cloned())
        .flat_map(|(ignored, admitted)| [ignored, admitted])
        .chain(ignored.iter().cloned())
        .collect::<Vec<_>>();
    let expected = books_of(codec.book_arrow_reader(admitted, 0).unwrap());
    assert_eq!(expected.len(), 5);
    assert_eq!(deltas(&expected[0], true)[0].kind(), MarketKind::OrderEvent);
    assert_eq!(
        deltas(&expected[1], false)[0].kind(),
        MarketKind::QuoteEvent
    );
    assert_eq!(expected[2].executions().len(), 1);
    let actual = books_of(codec.book_arrow_reader(mixed, 0).unwrap());
    assert_eq!(actual, expected);

    let mut empty = codec.book_arrow_reader(ignored.clone(), 0).unwrap();
    assert!(empty.next().is_none());
    for source in ignored {
        assert!(
            yggdryl::FixMsg::into_market_leaf(source.clone()).is_err(),
            "none is exactly one leaf"
        );
        let mut lazy = yggdryl::fix::FixMarketIterator::new([source].into_iter());
        assert!(lazy.next().is_none(), "none answers a leaf");
    }
}

/// A refusal of what one message states, as an intake would yield it.
fn refused_intake() -> Error {
    Error::InvalidRecord {
        path: "$.intake".into(),
        reason: "one message states what no reading takes".into(),
    }
}

/// The source's own failure, as a cut capture yields it.
fn cut_source() -> Error {
    Error::Io(std::io::Error::other("the capture was cut"))
}

#[test]
fn codec_book_admission_passes_over_a_refused_message_and_ends_on_a_source_failure() {
    let codec = fixed_codec(committed_registry()).with_batch_row_size(1);
    let order = |line: &[u8]| Ok(codec.parse_fix_line(line).unwrap());
    let first = b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|44=100|38=5|10=0|";
    let second = b"8=FIX.4.4|35=D|52=20260921-10:00:01|11=C2|55=AAPL|54=2|44=101|38=6|10=0|";

    let refused = [
        Ok(codec.parse_fix_line(b"8=FIX.4.4|35=0|10=0|").unwrap()),
        order(first),
        Err(refused_intake()),
        order(second),
    ];
    let books = books_of(codec.book_arrow_reader(refused, 0).unwrap());
    assert_eq!(books.len(), 2, "the refused message alone is passed over");
    assert_eq!(text(books[1].get_askpx()).as_deref(), Some("101"));

    let cut = [order(first), Err(cut_source()), order(second)];
    let mut reader = codec.book_arrow_reader(cut, 0).unwrap();
    assert_eq!(reader.next().unwrap().unwrap().num_rows(), 1, "the prefix");
    let error = reader.next().unwrap().unwrap_err().to_string();
    assert!(error.contains("the capture was cut"), "{error}");
    assert!(reader.next().is_none());
    assert!(reader.next().is_none());
}

#[test]
fn codec_book_admission_passes_over_what_no_book_reads() {
    let codec = fixed_codec(committed_registry()).with_batch_row_size(1);
    // A trade is no book input - its executions are the messages its parse
    // splits off - so none reaches the book, whatever its sides state.
    for line in [
        b"8=FIX.4.4|35=AE|571=CANCEL|487=1|150=F|552=1|54=1|1427=E1|10=0|".as_slice(),
        b"8=FIX.4.4|35=AE|571=MISSING-SIDE|487=0|552=1|1427=E1|10=0|".as_slice(),
    ] {
        let trade = codec.parse_fix_line(line).unwrap();
        assert!(trade.market_data().unwrap().is_empty());
        let mut reader = codec.book_arrow_reader([trade], 0).unwrap();
        assert!(reader.next().is_none());
    }
    // An entry stating no incremental action is excluded, and the order
    // after it folds.
    let invalid = codec
        .parse_fix_line(b"8=FIX.4.4|35=X|52=20260921-10:00:00|55=AAPL|268=1|279=9|269=0|278=B1|270=100|271=2|10=0|")
        .unwrap();
    assert!(invalid.market_data().unwrap().is_empty());
    let source = [
        codec.parse_fix_line(b"8=FIX.4.4|35=0|10=0|").unwrap(),
        invalid,
        codec
            .parse_fix_line(
                b"8=FIX.4.4|35=D|52=20260921-10:00:01|11=C1|55=AAPL|54=1|44=100|38=5|10=0|",
            )
            .unwrap(),
    ];
    let books = books_of(codec.book_arrow_reader(source, 0).unwrap());
    assert_eq!(books.len(), 1);
    assert_eq!(text(books[0].get_bidpx()).as_deref(), Some("100"));
}

/// Three messages: a bid, a message whose only entry is an opening price -
/// `MDEntryType(269)` `4`, which no book side holds - and an offer.
const AROUND_AN_OPENING_PRICE: [&[u8]; 3] = [
    b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|44=100|38=5|10=0|",
    b"8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=1|279=0|269=4|270=100.5|10=0|",
    b"8=FIX.4.4|35=D|52=20260921-10:00:02|11=C2|55=AAPL|54=2|44=101|38=6|10=0|",
];

/// An entry type no book side holds is excluded, and neither the entries
/// beside it nor the messages around it are: a book reads both sides.
#[test]
fn a_book_reads_past_an_entry_type_it_does_not_hold() {
    let codec = fixed_codec(committed_registry()).with_batch_row_size(1);
    let capture = messages(&codec, &AROUND_AN_OPENING_PRICE);
    assert!(capture[1].market_data().unwrap().is_empty());
    let books = books_of(codec.book_arrow_reader(capture, 0).unwrap());
    let book = books.last().expect("a book");
    assert_eq!(alive(book, true).len(), 1);
    assert_eq!(alive(book, false).len(), 1);
    assert_eq!(text(book.get_bidpx()).as_deref(), Some("100"));
    assert_eq!(text(book.get_askpx()).as_deref(), Some("101"));

    // Within one message, the entries beside it stand.
    let snapshot = message(
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=3|269=0|278=B1|270=99|271=1|269=4|270=100|269=1|278=A1|270=101|271=2|10=0|",
    );
    let entries = snapshot.market_data().unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.get_side())
            .collect::<Vec<_>>(),
        [Side::Buy, Side::Sell]
    );
    let books = books_of(codec.book_arrow_reader([snapshot], 0).unwrap());
    let book = books.last().expect("a book");
    assert_eq!(text(book.get_bidpx()).as_deref(), Some("99"));
    assert_eq!(text(book.get_askpx()).as_deref(), Some("101"));
}

#[test]
fn partial_fix_order_versions_keep_kind_links_and_lanes_through_book_arrow() {
    let codec = fixed_codec(committed_registry()).with_batch_row_size(1);
    let messages = [
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=1|269=0|278=B1|37=O1|270=100|271=10|10=0|".as_slice(),
        b"8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=1|279=1|269=0|278=B1|271=11|10=0|".as_slice(),
        b"8=FIX.4.4|35=X|52=20260921-10:00:02|55=AAPL|268=1|279=5|269=1|278=B1|270=101|10=0|".as_slice(),
    ]
    .map(|line| codec.sole_line(line).unwrap());
    let reader = codec.book_arrow_reader(messages, 0).unwrap();
    let books = books_of(reader);

    assert_eq!(books.len(), 3);
    assert_eq!(alive(&books[0], true).len(), 1);
    assert_eq!(alive(&books[1], true).len(), 1);
    assert_eq!(alive(&books[2], false).len(), 1);
    assert!(alive(&books[0], false).is_empty());
    assert!(alive(&books[1], false).is_empty());
    assert!(alive(&books[2], true).is_empty());
    let versions = [
        alive(&books[0], true)[0],
        alive(&books[1], true)[0],
        alive(&books[2], false)[0],
    ];
    for (index, (book, version)) in books.iter().zip(versions).enumerate() {
        assert_eq!(version.kind(), MarketKind::OrderEvent);
        // An entry naming its order is an order, whatever the message's
        // `BOOK`.
        assert_eq!(version.marketdatakind(), MarketDataKind::Order);
        let operation = operation_of(version);
        assert_eq!(book.get_ticker(), Some("AAPL"));
        assert_eq!(operation.get_ticker(), Some("AAPL"));
        assert_eq!(operation.get_identifiers().get(&ENTRY_ID), Some("B1"));
        assert_eq!(
            operation.get_identifiers().get(&IdType::OrderId),
            Some("O1")
        );
        // Each version stands alone at its own, later instant (one second
        // apart), so following the one before it leaves its place alone.
        assert_eq!(operation.get_seqnum(), 0);
        assert_eq!(
            text(operation.get_price()).as_deref(),
            Some(["100", "100", "101"][index])
        );
        assert_eq!(
            text(operation.get_quantity()).as_deref(),
            Some(["10", "11", "11"][index])
        );
        if index < 2 {
            assert!(operation.get_side().is_bid());
        } else {
            assert!(operation.get_side().is_ask());
        }
    }
    for pair in versions.windows(2) {
        let (before, after) = (operation_of(pair[0]), operation_of(pair[1]));
        assert_eq!(after.get_prevuuid(), Some(before.get_curruuid()));
        assert_eq!(after.get_prevunix(), Some(before.get_currunix()));
        assert_eq!(after.get_prevpx(), before.get_price());
        assert_eq!(after.get_prevqty(), before.get_quantity());
    }
    assert_eq!(operation_of(versions[0]).get_prevuuid(), None);
    // A row states an entry's scope and nothing else of its control: the
    // action, the position and the price and size the entry stated for
    // itself steered the walk and are no row fact.
    for version in versions {
        let control = version.book().expect("a scoped entry");
        assert!(control.scope.is_some(), "{control:?}");
        assert_eq!(
            (
                control.action,
                control.position,
                control.entry_px,
                control.entry_size
            ),
            (None, None, None, None)
        );
    }
}

#[test]
fn fix_delete_without_order_id_keeps_terminal_order_delta_through_book_arrow() {
    let codec = fixed_codec(committed_registry()).with_batch_row_size(1);
    // The delete states its price like any operation a side takes: a
    // level is a price, and a delete stating none is refused rather than
    // given a zero.
    let messages = [
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=1|269=0|278=B1|37=O1|270=100|271=10|10=0|".as_slice(),
        b"8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=1|279=2|269=0|278=B1|10=0|".as_slice(),
    ]
    .map(|line| codec.sole_line(line).unwrap());
    let reader = codec.book_arrow_reader(messages, 0).unwrap();
    let books = books_of(reader);

    assert_eq!(books.len(), 2);
    let previous = operation_of(alive(&books[0], true)[0]);
    assert!(alive(&books[1], true).is_empty());
    assert!(alive(&books[1], false).is_empty());
    let bid_deltas = deltas(&books[1], true);
    let [deleted] = bid_deltas[..] else {
        panic!("one terminal bid delta")
    };
    assert_eq!(deleted.kind(), MarketKind::OrderEvent);
    // The delete action steered the walk; a row read back states none.
    assert_eq!(deleted.book().and_then(|book| book.action), None);
    let deleted = operation_of(deleted);
    assert!(!deleted.get_state().is_live());
    assert!(deleted.get_side().is_bid());
    assert_eq!(deleted.get_ticker(), Some("AAPL"));
    assert_eq!(deleted.get_identifiers().get(&ENTRY_ID), Some("B1"));
    assert_eq!(deleted.get_identifiers().get(&IdType::OrderId), Some("O1"));
    assert_eq!(deleted.get_prevuuid(), Some(previous.get_curruuid()));
    assert_eq!(deleted.get_prevunix(), Some(previous.get_currunix()));
    // The delete arrives a second after the order, a later instant of its
    // own, so following the order leaves its place alone.
    assert_eq!(deleted.get_seqnum(), 0);
    assert_eq!(deleted.get_prevpx(), previous.get_price());
    assert_eq!(deleted.get_prevqty(), previous.get_quantity());
}

#[test]
fn lifecycled_order_versions_inherit_symbol_before_book_partitioning() {
    let codec = fixed_codec(committed_registry()).with_batch_row_size(1);
    let first = codec
        .sole_line(b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|44=100|38=5|10=0|")
        .unwrap();
    let second = codec
        .sole_line(b"8=FIX.4.4|35=D|52=20260921-10:00:01|11=C1|54=1|44=101|38=6|10=0|")
        .unwrap();
    assert_eq!(second.get_ticker(), None);
    let messages = codec
        .lifecycle([first, second])
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].get_ticker(), Some("AAPL"));
    assert_eq!(messages[1].get_prevuuid(), Some(messages[0].get_curruuid()));

    let reader = codec.book_arrow_reader(messages, 0).unwrap();
    let books = books_of(reader);
    assert_eq!(books.len(), 2);
    for (index, book) in books.iter().enumerate() {
        assert_eq!(book.get_ticker(), Some("AAPL"));
        assert_eq!(alive(book, true).len(), 1);
        assert!(alive(book, false).is_empty());
        let operation = alive(book, true)[0];
        assert_eq!(operation.kind(), MarketKind::OrderEvent);
        assert_eq!(operation.get_ticker(), Some("AAPL"));
        assert_eq!(
            text(operation.get_price()).as_deref(),
            Some(["100", "101"][index])
        );
        assert_eq!(
            text(operation.get_quantity()).as_deref(),
            Some(["5", "6"][index])
        );
    }
}

#[test]
fn full_refresh_expands_equal_time_entries_stably_and_types_each_one() {
    let inputs = message(
        b"8=FIX.4.4|35=W|55=AAPL|262=REQ-1|1021=2|1180=MDP|1181=42|268=3|269=0|278=B1|270=100|271=10|290=1|269=1|278=A1|37=O1|270=101|271=12|290=1|269=2|278=T1|270=100.5|271=2|10=0|",
    )
    .market_data()
    .expect("a full refresh");

    assert_eq!(inputs.len(), 3);
    let operations: Vec<&dyn Stated> = inputs.iter().map(operation_of).collect();
    assert_eq!(inputs[0].kind(), MarketKind::QuoteEvent);
    assert_eq!(inputs[1].kind(), MarketKind::OrderEvent);
    assert_eq!(inputs[2].kind(), MarketKind::ExecutionEvent);
    assert!(operations[0].get_side().is_bid());
    assert!(operations[1].get_side().is_ask());
    assert!(operations[0].get_crosscode().ends_with("|MDEntryID=B1"));
    assert!(operations[1].get_crosscode().ends_with("|MDEntryID=A1"));
    assert!(operations[2].get_crosscode().ends_with("|MDEntryID=T1"));
    assert_eq!(text(operations[0].get_price()).as_deref(), Some("100"));
    assert_eq!(text(operations[1].get_quantity()).as_deref(), Some("12"));
    assert_eq!(operations[0].get_ticker(), Some("AAPL"));
    assert_eq!(
        operations[1].get_identifiers().get(&IdType::OrderId),
        Some("O1")
    );
    assert_eq!(
        operations[1].get_identifiers().get(&IdType::MdReqId),
        Some("REQ-1"),
        "the request the message answers names every entry"
    );
    for (index, (input, operation)) in inputs.iter().zip(&operations).enumerate() {
        let book = input.book().expect("a full refresh entry is a book entry");
        assert_eq!(book.action, Some(MdUpdateAction::Snapshot));
        assert!(is_full_snapshot(input));
        assert_eq!(
            book.position,
            (index < 2).then_some(1),
            "the trade states no position"
        );
        assert!(scope_of(input).contains("Symbol=AAPL"));
        assert!(scope_of(input).contains("MDReqID=REQ-1"));
        // A level is live; a trade entry is one fill, complete in itself.
        if index < 2 {
            assert!(operation.get_state().is_live());
        } else {
            assert_eq!(*operation.get_state(), State::Filled);
        }
    }
    assert_eq!(
        inputs[0].book().and_then(|book| book.entry_px),
        Some("100".parse().unwrap())
    );
    assert_eq!(
        inputs[0].book().and_then(|book| book.entry_size),
        Some("10".parse().unwrap())
    );
}

#[test]
fn lifted_request_id_keeps_full_snapshot_partitions_distinct() {
    let initial = message(
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|262=REQ-1|268=1|269=0|278=B1|270=100|271=10|10=0|",
    )
    .into_market_data()
    .unwrap();
    assert!(scope_of(&initial[0]).contains("MDReqID=REQ-1"));
    let mut book = BookEvent::new(currunix(&initial[0]), "AAPL");
    book.add_operations(initial).unwrap();

    let empty_other_request =
        message(b"8=FIX.4.4|35=W|52=20260921-10:00:01|55=AAPL|262=REQ-2|268=0|10=0|")
            .into_market_data()
            .unwrap();
    assert!(scope_of(&empty_other_request[0]).contains("MDReqID=REQ-2"));
    assert!(is_full_snapshot(&empty_other_request[0]));
    book.add_operations(empty_other_request).unwrap();
    assert_eq!(alive(&book, true).len(), 1);
}

#[test]
fn an_operation_names_its_entry_beside_the_message_identifiers() {
    let mut source = message(b"8=FIX.4.4|35=X|55=AAPL|268=1|279=1|269=0|278=B1|271=11|10=0|");
    // A message's identifiers are its facts: a statement fills a
    // type and source it holds none of, whatever the dictionary maps, and
    // the wire stays as the source sent it.
    let id = |src: IdSource, kind: &str, value: &str| {
        Identifier::new(IdKey::new(src, kind.parse().expect("a type")), value)
            .expect("an identifier")
    };
    assert!(
        source
            .insert_identifier(id(IdSource::Base, "foreign", "kept"))
            .unwrap()
    );
    assert!(
        source
            .insert_identifier(id(IdSource::Base, "mdreqid", "REQ-9"))
            .unwrap()
    );
    assert_eq!(source.get_by_tag(262), None, "no identifier is written");

    let input = yggdryl::FixMsg::into_market_leaf(source).unwrap();
    let operation = operation_of(&input);
    let identifiers = operation.get_identifiers();
    assert_eq!(identifiers.get(&ENTRY_ID), Some("B1"));
    assert_eq!(identifiers.get(&ENTRY_REF_ID), None);
    assert_eq!(
        identifiers.get_from(&IdKey::base(IdType::MdReqId)),
        Some("REQ-9")
    );
    assert_eq!(
        identifiers.get_from(&IdKey::base("foreign".parse::<IdType>().unwrap())),
        Some("kept")
    );
    let book = input.book().expect("an entry states its control");
    assert_eq!(book.action, Some(MdUpdateAction::Change));
    assert_eq!(
        book.entry_size.map(|held| held.to_string()),
        Some("11".to_owned())
    );
    assert_eq!(book.entry_px, None, "the entry restated no price");
    // The scope is the wire's: the request the message names on it.
    assert!(!scope_of(&input).contains("REQ-9"), "{}", scope_of(&input));
}

#[test]
fn incremental_actions_keep_the_wire_action_and_terminal_delete_state() {
    let inputs = message(
        b"8=FIX.4.4|35=X|83=7|1181=43|268=3|279=0|269=0|278=B1|55=AAPL|270=100|271=10|290=1|279=1|269=0|278=B1|55=AAPL|270=101|271=11|290=1|279=2|269=0|280=B1|55=AAPL|10=0|",
    )
    .into_market_data()
    .expect("incremental operations");

    assert_eq!(inputs.len(), 3);
    let operations: Vec<&dyn Stated> = inputs.iter().map(operation_of).collect();
    let action = |input: &MarketData| input.book().and_then(|book| book.action);
    assert_eq!(action(&inputs[0]), Some(MdUpdateAction::New));
    assert_eq!(action(&inputs[1]), Some(MdUpdateAction::Change));
    assert_eq!(action(&inputs[2]), Some(MdUpdateAction::Delete));
    assert!(inputs.iter().all(|input| !is_full_snapshot(input)));
    assert!(operations[0].get_state().is_live());
    assert!(operations[1].get_state().is_live());
    assert!(!operations[2].get_state().is_live());
    assert_eq!(operations[2].get_crosscode(), operations[0].get_crosscode());
    assert_eq!(operations[0].get_identifiers().get(&ENTRY_ID), Some("B1"));
    assert_eq!(operations[2].get_identifiers().get(&ENTRY_ID), None);
    assert_eq!(
        operations[2].get_identifiers().get(&ENTRY_REF_ID),
        Some("B1")
    );
    assert_eq!(inputs[1].book().and_then(|book| book.position), Some(1));
}

#[test]
fn fallback_identity_is_scoped_typed_and_stable_across_price_changes() {
    let input = yggdryl::FixMsg::into_market_leaf(message(
        b"8=FIX.4.4|35=X|1301=XNAS|1300=NASDAQ|268=1|279=0|269=1|55=AAPL|1023=2|290=3|270=101.25|271=4|10=0|",
    ))
    .expect("one operation");
    let code = input.get_crosscode();
    assert!(code.contains("Symbol=AAPL"), "{code}");
    assert!(code.contains("MarketID=XNAS"), "{code}");
    assert!(code.contains("MDEntryType=1"), "{code}");
    assert!(code.contains("MDEntryPositionNo=3"), "{code}");
    assert!(code.contains("MDPriceLevel=2"), "{code}");
    assert!(!code.contains("MDEntryPx="), "{code}");
    assert_eq!(input.book().and_then(|book| book.position), Some(3));
}

/// An entry stating no `MDUpdateAction(279)`, one stating no
/// `MDEntryType(269)`, and a message of no market category - news.
const UNSUPPORTED: [&[u8]; 3] = [
    b"8=FIX.4.4|35=X|268=1|269=0|278=B1|10=0|",
    b"8=FIX.4.4|35=X|268=1|279=0|278=B1|10=0|",
    b"8=FIX.4.4|35=B|148=HEADLINE|10=0|",
];

#[test]
fn unsupported_market_shapes_answer_no_leaf() {
    for line in UNSUPPORTED {
        assert!(
            message(line).into_market_data().unwrap().is_empty(),
            "{}",
            String::from_utf8_lossy(line)
        );
    }
}

#[test]
fn singular_conversion_refuses_a_multi_entry_book_message() {
    let error = yggdryl::FixMsg::into_market_leaf(message(
        b"8=FIX.4.4|35=W|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=101|271=11|10=0|",
    ))
    .expect_err("two entries are not one operation");
    assert!(
        matches!(&error, Error::InvalidRecord { path, reason }
            if path.contains("NoMDEntries(268)") && reason.contains("got 2")),
        "{error}"
    );
}

#[test]
fn market_entry_clock_dates_each_operation_and_precisely_dates_an_execution() {
    let operations = message(
        b"8=FIX.4.4|35=X|52=20260921-10:00:00|55=AAPL|268=2|279=0|269=0|278=B1|270=100|271=10|272=20260920|273=09:30:00.123456789|279=0|269=2|278=T1|270=100|271=2|272=20260920|273=09:30:00.223456789|10=0|",
    )
    .into_market_data()
    .expect("entry clocks");

    assert_eq!(operations.len(), 2);
    assert_eq!(currunix(&operations[0]), 1_789_896_600_123_456_789);
    assert_eq!(currunix(&operations[1]), 1_789_896_600_223_456_789);
    assert_eq!(
        operations[1].get_execunix(),
        Some(1_789_896_600_223_456_789)
    );

    let snapshot = yggdryl::FixMsg::into_market_leaf(message(
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=1|269=0|278=B1|270=100|271=10|272=20260920|273=09:30:00.123456789|10=0|",
    ))
    .expect("one full-snapshot entry");
    assert_eq!(currunix(&snapshot), 1_789_984_800_000_000_000);
    assert_eq!(
        event_of(&snapshot).get_creaunix(),
        Some(1_789_896_600_123_456_789)
    );
}

#[test]
fn borrowed_and_owned_book_expansion_share_stable_effective_time_order() {
    let source = message(
        b"8=FIX.4.4|35=X|52=20260921-10:00:00|55=AAPL|268=3|279=0|269=0|278=LATE|270=102|271=1|272=20260920|273=09:30:00.300000000|279=0|269=0|278=EARLY-A|270=100|271=1|272=20260920|273=09:30:00.100000000|279=0|269=1|278=EARLY-B|270=101|271=1|272=20260920|273=09:30:00.100000000|10=0|",
    );
    let expected = ["EARLY-A", "EARLY-B", "LATE"];
    for operations in [
        source.market_data().expect("borrowed expansion"),
        source.clone().into_market_data().expect("owned expansion"),
    ] {
        assert_eq!(
            operations
                .iter()
                .map(|input| operation_of(input)
                    .get_identifiers()
                    .get(&ENTRY_ID)
                    .unwrap())
                .collect::<Vec<_>>(),
            expected
        );
        assert!(
            operations
                .windows(2)
                .all(|pair| { currunix(&pair[0]) <= currunix(&pair[1]) })
        );
    }
}

#[test]
fn incremental_changes_inherit_price_or_size_the_fix_entry_did_not_restate() {
    let snapshot = message(b"8=FIX.4.4|35=W|55=AAPL|268=1|269=0|278=B1|270=100|271=10|10=0|")
        .into_market_data()
        .unwrap();
    let mut book = BookEvent::new(currunix(&snapshot[0]), "AAPL");
    book.add_operations(snapshot).unwrap();

    let size_only = message(b"8=FIX.4.4|35=X|55=AAPL|268=1|279=1|269=0|278=B1|271=11|10=0|")
        .into_market_data()
        .unwrap();
    book.add_operations(size_only).unwrap();
    let live = alive(&book, true)[0];
    assert_eq!(text(live.get_price()).as_deref(), Some("100"));
    assert_eq!(text(live.get_quantity()).as_deref(), Some("11"));

    let price_only = message(b"8=FIX.4.4|35=X|55=AAPL|268=1|279=5|269=0|278=B1|270=101|10=0|")
        .into_market_data()
        .unwrap();
    book.add_operations(price_only).unwrap();
    let live = alive(&book, true)[0];
    assert_eq!(text(live.get_price()).as_deref(), Some("101"));
    assert_eq!(text(live.get_quantity()).as_deref(), Some("11"));
}

#[test]
fn anonymous_incremental_changes_use_stable_position_identity_or_exclude_ambiguity() {
    let snapshot = message(
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=1|269=0|290=1|270=100|271=10|10=0|",
    )
    .into_market_data()
    .unwrap();
    let identity = snapshot[0].get_crosscode().to_owned();
    let mut book = BookEvent::new(currunix(&snapshot[0]), "AAPL");
    book.add_operations(snapshot).unwrap();

    let change = message(
        b"8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=1|279=1|269=0|290=1|271=11|10=0|",
    )
    .into_market_data()
    .unwrap();
    assert_eq!(change[0].get_crosscode(), identity);
    book.add_operations(change).unwrap();
    assert_eq!(alive(&book, true).len(), 1);
    assert_eq!(
        text(alive(&book, true)[0].get_price()).as_deref(),
        Some("100")
    );
    assert_eq!(
        text(alive(&book, true)[0].get_quantity()).as_deref(),
        Some("11")
    );

    let ambiguous = message(ANONYMOUS_UPDATE).into_market_data().unwrap();
    assert!(
        ambiguous.is_empty(),
        "an anonymous update without a stable coordinate names no entry"
    );
}

/// An incremental change naming no entry and no position or level.
const ANONYMOUS_UPDATE: &[u8] = b"8=FIX.4.4|35=X|55=AAPL|268=1|279=1|269=0|271=12|10=0|";

#[test]
fn an_empty_full_refresh_clears_its_scope() {
    let initial = message(
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=1|269=0|278=B1|270=100|271=10|10=0|",
    )
    .into_market_data()
    .unwrap();
    let mut book = BookEvent::new(currunix(&initial[0]), "AAPL");
    book.add_operations(initial).unwrap();
    assert_eq!(alive(&book, true).len(), 1);

    let empty = message(b"8=FIX.4.4|35=W|52=20260921-10:00:01|55=AAPL|268=0|10=0|")
        .into_market_data()
        .unwrap();
    assert!(matches!(empty.as_slice(), [MarketData::SnapshotEvent(_)]));
    assert!(is_full_snapshot(&empty[0]));
    assert_eq!(empty[0].get_ticker(), Some("AAPL"));
    book.add_operations(empty).unwrap();
    assert!(alive(&book, true).is_empty());
    assert!(alive(&book, false).is_empty());
}

/// A FIX event always states its category, but a snapshot control states
/// no operation of its own: it is a
/// [`yggdryl::graph::SnapshotEvent`], which holds no operation fact, its row
/// states none, and an Arrow round trip returns exactly what went in.
#[test]
fn an_empty_full_refresh_states_no_operation_and_round_trips_through_arrow() {
    let expected = message(b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=0|10=0|")
        .into_market_data()
        .unwrap();
    assert!(matches!(
        expected.as_slice(),
        [MarketData::SnapshotEvent(_)]
    ));

    use arrow_array::Array as _;
    let mut encoded = MarketData::arrow_reader(expected.clone(), Some(1), None).unwrap();
    let batch = encoded.next().unwrap().unwrap();
    assert!(encoded.next().is_none());
    assert_eq!(
        batch.column_by_name("timeinforce").unwrap().null_count(),
        1,
        "the row states no operation fact"
    );
    let encoded = yggdryl::arrow::batch_reader(batch.schema(), [batch]);
    let actual = MarketData::from_arrow_reader(encoded)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn book_scope_escapes_external_delimiters_injectively() {
    let input = yggdryl::FixMsg::into_market_leaf(message(
        b"8=FIX.4.4|35=W|55=A=B%X|268=1|269=0|278=B1|270=100|271=1|10=0|",
    ))
    .unwrap();
    assert!(scope_of(&input).contains("Symbol=A%3DB%25X"));
}

fn tagged(name: &str, tag: i32, dtype: DataType) -> Field {
    let mut field = dtype.required_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field
}

/// A one-entry book message of `msgtype` under a minimal registry whose
/// `MDEntryPx(270)` is a 39-digit decimal, the entry stating
/// `MDUpdateAction(279)` `1`, a bid, `MDEntryID(278)` `B1` and a price no
/// graph decimal holds.
fn too_wide_price(msgtype: &str) -> FixMsg {
    let msgtype_field = tagged("MsgType", 35, DataType::utf8());
    let action = tagged("MDUpdateAction", 279, DataType::utf8());
    let entry_type = tagged("MDEntryType", 269, DataType::utf8());
    let entry_id = tagged("MDEntryID", 278, DataType::utf8());
    let price = tagged(
        "MDEntryPx",
        270,
        DataType::decimal256(39, 0).expect("a 39-digit decimal"),
    );
    let item = StructType::from_fields([action, entry_type, entry_id, price])
        .map(DataType::from)
        .unwrap()
        .required_field("MDEntry");
    let mut entries = DataType::serie(item).required_field("MDEntries");
    entries.as_fix_mut().set_counter(268).unwrap();
    let counter = tagged("NoMDEntries", 268, DataType::Int32);
    let registry = Arc::new(
        FixRegistry::from_fields([counter, msgtype_field.clone(), entries.clone()])
            .expect("the minimal book registry"),
    );
    let root = StructType::from_fields([msgtype_field, entries])
        .map(DataType::from)
        .unwrap()
        .required_field("fix");
    let too_wide = Scalar::decimal256(
        "170141183460469231731687303715884105728".parse().unwrap(),
        0,
    );
    let value = Scalar::from_sequence([
        Scalar::from(msgtype),
        Scalar::from_sequence([Scalar::from_sequence([
            Scalar::from("1"),
            Scalar::from("0"),
            Scalar::from("B1"),
            too_wide,
        ])]),
    ]);
    FixMsg::with_registry(registry, root, value).unwrap()
}

/// A price no graph decimal holds never becomes zero: a snapshot entry,
/// which would rest at it, is excluded, and a change, which keeps the live
/// entry's price, states none.
#[test]
fn a_typed_price_outside_decimal18_excludes_a_resting_entry_and_nulls_an_update() {
    assert!(
        too_wide_price("W").into_market_data().unwrap().is_empty(),
        "a snapshot entry cannot rest unpriced"
    );
    let [change] = <[MarketData; 1]>::try_from(too_wide_price("X").into_market_data().unwrap())
        .expect("the change stands");
    assert_eq!(
        change.book().and_then(|book| book.action),
        Some(MdUpdateAction::Change)
    );
    assert_eq!(operation_of(&change).get_price(), None);
    assert_eq!(change.book().and_then(|book| book.entry_px), None);
    assert_eq!(
        operation_of(&change).get_identifiers().get(&ENTRY_ID),
        Some("B1")
    );
}

/// A snapshot whose entries group the row holds as a column - what a row
/// read out of Arrow carries - answers the operations the parse did.
#[test]
fn a_snapshot_holding_its_entries_as_a_column_answers_the_parsed_operations() {
    let registry = committed_registry();
    let parsed = message(
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|262=REQ-1|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=101|271=11|10=0|",
    );
    let expected = parsed.market_data().expect("a full refresh");
    assert_eq!(expected.len(), 2);
    let (root, row) = super::restatable(&registry, &parsed, &[35, 52, 262]);
    let at = root
        .fields()
        .iter()
        .position(|field| field.dtype().as_serie_type().is_some())
        .expect("the entries' column");
    let name = root.fields()[at].name().to_owned();
    let run = FixMsg::with_registry(Arc::clone(&registry), root.clone(), row.clone())
        .expect("the run-backed message");
    let row = super::with_column_at(&row, at, &super::item_of(&root.fields()[at]));
    let column = FixMsg::with_registry(registry, root, row).expect("the column-backed message");
    assert!(super::holds_column(&column, &name));

    assert_eq!(run.market_data().unwrap(), expected);
    assert_eq!(
        column
            .market_data()
            .expect("the entries read from a column"),
        expected
    );
}

#[test]
fn an_fx_execution_is_lifted_prices_as_spot_plus_points_and_reaches_the_books_executions() {
    // The forward's two parts are lifted under their own tags; `LastPx(31)`,
    // stated by nobody, derives as their sum; the market reads both parts.
    let held = message(
        b"8=FIX.4.4|35=8|17=E1|37=O1|55=EURUSD|54=1|32=1000000|150=F|194=1.25|195=0.0025|10=0|",
    );
    assert_eq!(text(held.lifted().lastspotrate()).as_deref(), Some("1.25"));
    assert_eq!(
        text(held.lifted().lastforwardpoints()).as_deref(),
        Some("0.0025")
    );
    assert_eq!(
        text(held.get_lastpx()).as_deref(),
        Some("1.2525"),
        "31 is their sum"
    );
    assert_eq!(text(held.get_spotrate()).as_deref(), Some("1.25"));
    assert_eq!(text(held.get_forwardpoints()).as_deref(), Some("0.0025"));
    assert_eq!(
        held.get_price(),
        None,
        "an execution states no price of its own"
    );
    assert!(
        held.entries()
            .iter()
            .all(|entry| entry.tag() != 194 && entry.tag() != 195),
        "a lifted tag is a holder's, never an entry"
    );
    assert_eq!(held.by_tag(194).unwrap(), super::decimal("1.25"));
    assert_eq!(held.by_tag(195).unwrap(), super::decimal("0.0025"));

    // The execution the report splits off reaches a book with its FX parts.
    let [_, execution] = <[FixMsg; 2]>::try_from(split(
        b"8=FIX.4.4|35=8|17=E1|37=O1|55=EURUSD|54=1|32=1000000|150=F|194=1.25|195=0.0025|10=0|",
    ))
    .expect("the report and its execution");
    let inputs = execution.into_market_data().expect("an execution");
    assert_eq!(inputs.len(), 1);
    let books = yggdryl::graph::BookIterator::new(inputs.into_iter(), 0)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(books.len(), 1);
    let [execution] = books[0].executions() else {
        panic!("one execution on the book")
    };
    assert_eq!(text(execution.get_spotrate()).as_deref(), Some("1.25"));
    assert_eq!(
        text(execution.get_forwardpoints()).as_deref(),
        Some("0.0025")
    );
    assert_eq!(text(execution.get_lastpx()).as_deref(), Some("1.2525"));
    // What it last executed is `lastpx` and `lastqty`, beside the parts they
    // sum from; its own price stays unstated.
    assert_eq!(execution.get_price(), None);
    assert_eq!(text(execution.get_lastqty()).as_deref(), Some("1000000"));
}

#[test]
fn a_quotes_fx_parts_stay_the_messages_own() {
    // A quote's bid and offer FX parts are the message's lifted fields: no
    // leaf fact reads them, and none is completed from the others.
    let held = message(
        b"8=FIX.4.4|35=S|117=Q1|55=EURUSD|996=Ccy|132=1.2|134=1000000|133=1.21|135=1000000|188=1.19|191=0.011|10=0|",
    );
    assert_eq!(text(held.lifted().bidspotrate()).as_deref(), Some("1.19"));
    assert_eq!(held.lifted().bidforwardpoints(), None);
    assert_eq!(held.lifted().offerspotrate(), None);
    assert_eq!(
        text(held.lifted().offerforwardpoints()).as_deref(),
        Some("0.011")
    );
    assert_eq!(held.get_price(), None, "a two-sided quote states no price");
    assert_eq!(
        held.get_spotrate(),
        None,
        "a two-sided quote states no last price of its own"
    );
}

#[test]
fn a_levels_fx_parts_are_its_own_as_stated() {
    let level = |line: &[u8]| {
        let inputs = message(line).into_market_data().expect("one level");
        let level = operation_of(&inputs[0]);
        [
            level.get_price(),
            level.get_spotrate(),
            level.get_forwardpoints(),
        ]
        .map(text)
    };
    let stated = |held: [Option<&str>; 3]| held.map(|held| held.map(str::to_owned));
    // `MDEntryPx(270)`, `MDEntrySpotRate(1026)` and
    // `MDEntryForwardPoints(1027)` are each read as stated: two complete no
    // third, and three are kept whether or not they agree.
    assert_eq!(
        level(b"8=FIX.4.4|35=W|55=EURUSD|268=1|269=0|278=B1|270=1.2|271=100|1026=1.19|1027=0.01|10=0|"),
        stated([Some("1.2"), Some("1.19"), Some("0.01")])
    );
    assert_eq!(
        level(b"8=FIX.4.4|35=W|55=EURUSD|268=1|269=0|278=B1|271=100|1026=1.19|1027=0.01|10=0|"),
        stated([None, Some("1.19"), Some("0.01")])
    );
    assert_eq!(
        level(b"8=FIX.4.4|35=W|55=EURUSD|268=1|269=0|278=B1|270=1.3|271=100|1026=1.19|1027=0.01|10=0|"),
        stated([Some("1.3"), Some("1.19"), Some("0.01")])
    );
}

/// Two ticker-less instruments of one market and classification, each
/// resting an entry under the same `MDEntryID`: the book the iterator keys
/// them to is their `MIC:CFI` one, and the scope each entry stands in is
/// its ISIN where it states one, else that key.
#[test]
fn ticker_less_instruments_sharing_an_entry_id_stay_apart_where_either_states_an_isin() {
    const TOTAL: &[u8] = b"8=FIX.4.4|35=W|52=20260921-10:00:00|48=FR0000120271|22=4|207=XPAR|461=ESVUFR|268=1|269=0|278=E1|270=60|271=10|10=0|";
    const BNP: &[u8] = b"8=FIX.4.4|35=W|52=20260921-10:00:01|48=FR0000131104|22=4|207=XPAR|461=ESVUFR|268=1|269=0|278=E1|270=70|271=20|10=0|";
    const FIRST: &[u8] = b"8=FIX.4.4|35=W|52=20260921-10:00:00|207=XPAR|461=ESVUFR|268=1|269=0|278=E1|270=60|271=10|10=0|";
    const SECOND: &[u8] = b"8=FIX.4.4|35=W|52=20260921-10:00:01|207=XPAR|461=ESVUFR|268=1|269=0|278=E1|270=70|271=20|10=0|";

    let codec = fixed_codec(committed_registry()).with_batch_row_size(1);
    let scopes = |lines: &[&[u8]]| {
        messages(&codec, lines)
            .into_iter()
            .map(|message| {
                let leaves = message.into_market_data().expect("one level");
                let [leaf] = leaves.as_slice() else {
                    panic!("one level per message")
                };
                assert_eq!(leaf.get_ticker(), None);
                assert_eq!(
                    leaf.get_crosscode(),
                    format!("14:1:{}|MDEntryID=E1", scope_of(leaf))
                );
                scope_of(leaf).to_owned()
            })
            .collect::<Vec<_>>()
    };
    let book = |lines: &[&[u8]]| {
        let books = books_of(
            codec
                .book_arrow_reader(messages(&codec, lines), 0)
                .expect("a book reader"),
        );
        let last = books.last().expect("a last book").clone();
        // One categorized book: no ticker, keyed by market and class, its
        // code stored under the book kind and no side.
        assert_eq!(last.get_ticker(), None);
        assert_eq!(last.get_crosscode(), "3:0:XPAR:ESVUFR");
        alive(&last, true)
            .into_iter()
            .map(|level| text(level.get_price()).expect("a priced level"))
            .collect::<Vec<_>>()
    };

    // Both state an ISIN: two scopes, two live entries.
    assert_eq!(
        scopes(&[TOTAL, BNP]),
        ["Symbol=FR0000120271", "Symbol=FR0000131104"]
    );
    let mut live = book(&[TOTAL, BNP]);
    live.sort();
    assert_eq!(live, ["60", "70"]);
    // One states an ISIN: the other stands in the book's key, still apart.
    assert_eq!(
        scopes(&[TOTAL, SECOND]),
        ["Symbol=FR0000120271", "Symbol=XPAR:ESVUFR"]
    );
    let mut live = book(&[TOTAL, SECOND]);
    live.sort();
    assert_eq!(live, ["60", "70"]);
    // Neither does: the one scope they share holds one entry, the later
    // snapshot's - the limit a ticker-less, identifier-less feed has.
    assert_eq!(
        scopes(&[FIRST, SECOND]),
        ["Symbol=XPAR:ESVUFR", "Symbol=XPAR:ESVUFR"]
    );
    assert_eq!(book(&[FIRST, SECOND]), ["70"]);
}

// ---------------------------------------------------------------------------
// The sorted doors: a capture collected, expanded and sorted by the instant a
// book folds each operation at.
// ---------------------------------------------------------------------------

/// Every operation a door answers, or the first failure.
fn drained(
    operations: impl Iterator<Item = yggdryl::Result<MarketData>>,
) -> yggdryl::Result<Vec<MarketData>> {
    operations.collect()
}

/// The instant a book folds one operation at.
fn effective(value: &MarketData) -> i64 {
    let event = event_of(value);
    event.get_snapunix().unwrap_or_else(|| event.get_currunix())
}

/// Messages of one codec, in the order given.
/// Every message `lines` parse into, what each line's parse split off
/// included, in the order the parse yields them.
fn messages(codec: &yggdryl::FixCodec, lines: &[&[u8]]) -> Vec<FixMsg> {
    lines
        .iter()
        .flat_map(|line| codec.parse_line(line).expect("a FIX line"))
        .collect::<yggdryl::Result<_>>()
        .expect("every message")
}

#[test]
fn the_codec_sorts_a_capture_before_projecting_it() {
    let codec = fixed_codec(committed_registry()).with_batch_row_size(1);
    let unsorted = messages(
        &codec,
        &[
            b"8=FIX.4.4|35=D|52=20260921-10:00:02|11=C2|55=AAPL|54=1|44=100|38=5|10=0|",
            b"8=FIX.4.4|35=S|52=20260921-10:00:00|117=Q1|55=AAPL|54=2|133=101|135=7|10=0|",
            b"8=FIX.4.4|35=D|52=20260921-10:00:01|11=C1|55=AAPL|54=1|44=99|38=5|10=0|",
        ],
    );
    let mut sorted = unsorted.clone();
    sorted.sort_by_key(Event::get_currunix);
    let expected = drained(yggdryl::fix::FixMarketIterator::new(sorted.into_iter()))
        .expect("the strict projection over the sorted capture");
    assert_eq!(expected.len(), 3);

    let actual = drained(codec.market_data(unsorted.clone())).expect("the sorted door");
    assert_eq!(
        actual, expected,
        "the door answers the sorted capture's leaves"
    );

    // The lazy projection yields what it is handed, in that order: an
    // operation dated before one already yielded is a warning, which a
    // book then excludes.
    let lazy = drained(yggdryl::fix::FixMarketIterator::new(
        unsorted.clone().into_iter(),
    ))
    .expect("nothing fails");
    assert_eq!(
        lazy.iter().map(currunix).collect::<Vec<_>>(),
        unsorted.iter().map(Event::get_currunix).collect::<Vec<_>>()
    );

    // The sorted operations fold through the stateful book, one per leaf.
    let books = yggdryl::graph::BookIterator::new(codec.market_data(unsorted), 0)
        .expect("a book iterator")
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("the sorted operations fold");
    assert_eq!(books.len(), 3);
}

#[test]
fn sorted_operations_never_regress_when_an_entry_clock_precedes_a_message() {
    let codec = fixed_codec(committed_registry());
    // The update's entry clock, 10:00:00.5, stands before the order sent at
    // 10:00:01 - though the update itself was sent after it.
    let capture = messages(
        &codec,
        &[
            b"8=FIX.4.4|35=D|52=20260921-10:00:01|11=C1|55=AAPL|54=1|44=99|38=5|10=0|",
            b"8=FIX.4.4|35=X|52=20260921-10:00:02|55=AAPL|268=1|279=0|269=0|278=B1|270=100|271=10|272=20260921|273=10:00:00.500|10=0|",
        ],
    );
    assert!(
        capture
            .windows(2)
            .all(|pair| pair[0].get_currunix() <= pair[1].get_currunix()),
        "the messages are sorted"
    );
    // Sorted messages are not sorted operations: the lazy projection yields
    // the entry after the order it regresses behind.
    let lazy = drained(yggdryl::fix::FixMarketIterator::new(
        capture.clone().into_iter(),
    ))
    .expect("the regression is yielded");
    assert_eq!(
        lazy.iter().map(MarketData::kind).collect::<Vec<_>>(),
        [MarketKind::OrderEvent, MarketKind::QuoteEvent]
    );

    let operations = drained(codec.market_data(capture)).expect("the sorted door");
    assert_eq!(
        operations.iter().map(MarketData::kind).collect::<Vec<_>>(),
        [MarketKind::QuoteEvent, MarketKind::OrderEvent],
        "the entry is placed before the order"
    );
    assert_eq!(effective(&operations[0]), 1_789_984_800_500_000_000);
    assert!(
        operations
            .windows(2)
            .all(|pair| effective(&pair[0]) <= effective(&pair[1]))
    );
    let books = yggdryl::graph::BookIterator::new(operations.into_iter().map(Ok), 0)
        .expect("a book iterator")
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("the operations fold");
    assert_eq!(books.len(), 2);
}

/// Two orders, the later one first, around `between`.
fn around(codec: &yggdryl::FixCodec, between: Error) -> [yggdryl::Result<FixMsg>; 3] {
    [
        codec
            .sole_line(b"8=FIX.4.4|35=D|52=20260921-10:00:01|11=C1|55=AAPL|54=1|44=100|38=5|10=0|"),
        Err(between),
        codec
            .sole_line(b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C2|55=AAPL|54=2|44=101|38=6|10=0|"),
    ]
}

#[test]
fn a_refused_intake_is_passed_over_and_a_source_failure_ends_the_capture_after_its_prefix() {
    let codec = fixed_codec(committed_registry());
    let leaves = drained(codec.market_data(around(&codec, refused_intake())))
        .expect("a refusal of one message fails nothing");
    assert_eq!(
        leaves
            .iter()
            .map(|leaf| leaf.get_crosscode())
            .collect::<Vec<_>>(),
        ["10:2:C2", "10:1:C1"],
        "sorted by instant"
    );
    let reader = codec
        .market_arrow_reader(around(&codec, refused_intake()))
        .expect("a reader");
    let rows = drained(MarketData::from_arrow_reader(reader).expect("the rows read back"))
        .expect("every row");
    assert_eq!(rows, leaves);

    // The source's own failure follows what was read before it, and no
    // later message is read.
    let mut operations = codec.market_data(around(&codec, cut_source()));
    assert_eq!(
        operations
            .next()
            .expect("the prefix")
            .unwrap()
            .get_crosscode(),
        "10:1:C1"
    );
    let error = operations
        .next()
        .expect("the failure")
        .expect_err("the source failed")
        .to_string();
    assert!(error.contains("the capture was cut"), "{error}");
    assert!(operations.next().is_none(), "fused");

    let mut reader = codec
        .market_arrow_reader(around(&codec, cut_source()))
        .expect("a reader");
    assert_eq!(reader.next().expect("the prefix").unwrap().num_rows(), 1);
    let error = reader
        .next()
        .expect("the failure")
        .expect_err("the source failed")
        .to_string();
    assert!(error.contains("the capture was cut"), "{error}");
    assert!(reader.next().is_none(), "and nothing follows it");
}

/// A full refresh stating no `NoMDEntries(268)` group at all states an
/// empty book: an empty snapshot of its scope, and the other messages of
/// the capture stand beside it.
#[test]
fn a_full_refresh_stating_no_entries_group_is_an_empty_snapshot() {
    let codec = fixed_codec(committed_registry());
    let capture = messages(
        &codec,
        &[
            b"8=FIX.4.4|35=D|52=20260921-10:00:01|11=C1|55=AAPL|54=1|44=100|38=5|10=0|",
            NO_ENTRIES_GROUP,
            b"8=FIX.4.4|35=S|52=20260921-10:00:00|117=Q1|55=AAPL|54=2|133=101|135=7|10=0|",
        ],
    );
    let leaves = drained(codec.market_data(capture)).expect("nothing fails");
    assert_eq!(
        leaves.iter().map(MarketData::kind).collect::<Vec<_>>(),
        [
            MarketKind::QuoteEvent,
            MarketKind::OrderEvent,
            MarketKind::SnapshotEvent
        ]
    );
    assert!(is_full_snapshot(&leaves[2]));
    assert_eq!(leaves[2].get_ticker(), Some("AAPL"));
}

/// A book message counting `NoMDEntries(268)=0` states its group holding
/// no entry: a full refresh an empty snapshot of its scope, an incremental
/// refresh no change. The fixed row has no column for the group, so its
/// record keeps the count, and the message read back out of it re-emits
/// the count and reads as the market data the parse reads as.
#[test]
fn a_book_message_counting_no_entries_reads_back_out_of_the_fixed_row_as_it_parsed() {
    for (line, leaves) in [
        (
            &b"8=FIX.4.4|35=W|52=20260921-10:00:04|55=AAPL|268=0|10=0|"[..],
            1,
        ),
        (
            &b"8=FIX.4.4|35=X|52=20260921-10:00:04|55=AAPL|268=0|10=0|"[..],
            0,
        ),
    ] {
        let parsed = message(line);
        let wire = String::from_utf8(parsed.into_bytes(b'|')).unwrap();
        assert!(wire.contains("|268=0|"), "{wire}");
        let schema = yggdryl::fix_schema(parsed.registry(), "fix").unwrap();
        let row = parsed.into_row(&schema).unwrap();
        let read_back =
            FixMsg::from_row(Arc::clone(parsed.registry()), &schema, &row).expect("the row reads");
        let rewritten = String::from_utf8(read_back.into_bytes(b'|')).unwrap();
        assert!(rewritten.contains("|268=0|"), "{rewritten}");
        assert_eq!(read_back.get_currhashcode(), parsed.get_currhashcode());
        let market = parsed.into_market_data().unwrap();
        assert_eq!(market.len(), leaves, "{wire}");
        assert_eq!(read_back.into_market_data().unwrap(), market, "{wire}");
    }
}

/// A full refresh stating no `NoMDEntries(268)` group.
const NO_ENTRIES_GROUP: &[u8] = b"8=FIX.4.4|35=W|52=20260921-10:00:02|55=AAPL|10=0|";

/// Five messages in time order: an order, a two-sided quote, an order's
/// execution report, a trade and an empty snapshot - every dated leaf a
/// capture expands into but a trade, which is its sided executions.
const EVERY_KIND: [&[u8]; 5] = [
    b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|44=100|38=5|40=2|10=0|",
    b"8=FIX.4.4|35=S|52=20260921-10:00:01|117=Q1|55=AAPL|132=99|134=7|133=101|135=8|10=0|",
    b"8=FIX.4.4|35=8|52=20260921-10:00:02|17=E1|37=O1|55=AAPL|54=1|31=100|32=2|150=F|10=0|",
    b"8=FIX.4.4|35=AE|52=20260921-10:00:03|571=T1|487=0|55=AAPL|32=1|31=100|552=1|54=1|1427=E2|1009=1|528=A|10=0|",
    b"8=FIX.4.4|35=W|52=20260921-10:00:04|55=AAPL|268=0|10=0|",
];

#[test]
fn market_arrow_reader_rows_state_every_kind() {
    let codec = fixed_codec(committed_registry()).with_batch_row_size(2);
    let capture = messages(&codec, &EVERY_KIND);
    let expected = drained(codec.market_data(capture.clone())).expect("the leaves");
    let reader = codec.market_arrow_reader(capture).expect("a reader");
    let schema = reader.schema();
    assert_eq!(
        schema.fields().len(),
        MarketData::field().expect("the row").field_len()
    );
    let rows = drained(MarketData::from_arrow_reader(reader).expect("the rows read back"))
        .expect("every row");
    assert_eq!(
        rows.iter()
            .map(|row| row.kind().as_str())
            .collect::<Vec<_>>(),
        [
            "order_event",
            "quote_event",
            "quote_event",
            "order_event",
            "execution_event",
            "execution_event",
            "snapshot_event"
        ]
    );
    assert_eq!(rows, expected);
}

#[test]
fn the_arrow_twin_answers_the_market_batches_and_refuses_a_foreign_source() {
    let registry = committed_registry();
    let codec = fixed_codec(Arc::clone(&registry)).with_batch_row_size(2);
    let capture = messages(&codec, &EVERY_KIND);
    let direct = drained(codec.market_data(capture.clone())).expect("the leaves");
    let rows = codec
        .arrow_reader(
            yggdryl::fix_schema(&registry, "fix").expect("the fixed row"),
            capture,
        )
        .expect("the FIX rows");
    let twin = codec
        .market_data_arrow_reader(rows)
        .expect("the twin opens");
    assert_eq!(
        twin.schema().fields().len(),
        MarketData::field().expect("the row").field_len()
    );
    let twin = drained(MarketData::from_arrow_reader(twin).expect("the rows read back"))
        .expect("every row");
    assert_eq!(
        twin, direct,
        "a FIX row answers the leaves its message does"
    );

    // A schema that makes no root is refused before a row is read, as the
    // lifecycle twin refuses it.
    let foreign = || {
        let column = arrow_schema::Field::new("a", arrow_schema::DataType::Int64, true);
        yggdryl::arrow::batch_reader(
            Arc::new(arrow_schema::Schema::new(vec![column.clone(), column])),
            Vec::<arrow_array::RecordBatch>::new(),
        )
    };
    let refused = codec
        .market_data_arrow_reader(foreign())
        .map(drop)
        .expect_err("a foreign source")
        .to_string();
    let lifecycle = codec
        .lifecycle_arrow_reader(foreign())
        .map(drop)
        .expect_err("a foreign source")
        .to_string();
    assert_eq!(refused, lifecycle);
}

// ---------------------------------------------------------------------------
// A leaf's metadata: every field its message states that no typed column
// reads.
// ---------------------------------------------------------------------------

/// A leaf's metadata as owned pairs, in key order.
fn metadata(value: &MarketData) -> Vec<(String, String)> {
    value
        .get_metadata()
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

/// The keys a leaf's metadata holds.
fn keys(value: &MarketData) -> Vec<String> {
    value
        .get_metadata()
        .keys()
        .map(ToString::to_string)
        .collect()
}

/// An order stating two fields no typed column reads - `ExecInst(18)` and
/// `HandlInst(21)` - beside the header, the trailer and the typed facts,
/// `OrdType(40)` and FIX 4.4's `MaxFloor(111)` among them: how it is priced,
/// and the peak an iceberg shows.
const UNMAPPED_ORDER: &[u8] = b"8=FIX.4.4|9=120|35=D|49=BUYER|56=VENUE|34=12|52=20260921-10:00:00|11=C1|1=ACC1|55=AAPL|54=1|44=100.5|38=5|40=2|18=G|21=1|111=3|60=20260921-10:00:00|10=123|";

#[test]
fn a_leaf_carries_every_unmapped_field_and_no_typed_one() {
    let leaves = message(UNMAPPED_ORDER)
        .into_market_data()
        .expect("an order");
    let [leaf] = leaves.as_slice() else {
        panic!("one order")
    };
    // How it is priced and what it shows are facts of the order, and
    // what it keeps back is the quantity past the peak.
    assert_eq!(leaf.get_marketdatatype(), yggdryl::MarketDataType::OrdLimit);
    assert_eq!(leaf.get_displayqty(), Some(yggdryl::Decimal::from_int(3)));
    assert_eq!(leaf.get_hiddenqty(), Some(yggdryl::Decimal::from_int(2)));
    assert_eq!(
        metadata(leaf),
        [("execinst", "G"), ("handlinst", "1"),]
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
    );
    for typed in [
        // The account is the leaf's `ACCOUNT`.
        "account",
        "symbol",
        "side",
        "price",
        "orderqty",
        "clordid",
        "transacttime",
        "sendingtime",
        "bodylength",
        "msgseqnum",
        "checksum",
        "sendercompid",
        "targetcompid",
        "msgtype",
        "beginstring",
    ] {
        assert!(!keys(leaf).iter().any(|key| key == typed), "{typed}");
    }
    // The typed facts are where they belong.
    let order = operation_of(leaf);
    assert_eq!(order.get_ticker(), Some("AAPL"));
    assert_eq!(text(order.get_price()).as_deref(), Some("100.5"));
    assert_eq!(order.get_partyids().get(&IdType::Account), Some("ACC1"));
}

#[test]
fn the_partyids_a_leaf_holds_leave_its_metadata_and_a_second_of_a_role_stays() {
    let leaves = message(
        b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|38=5|453=3|448=TRADER1|447=D|452=11|448=ACC9|447=D|452=24|448=TRADER2|447=D|452=11|10=0|",
    )
    .into_market_data()
    .expect("an order");
    let [leaf] = leaves.as_slice() else {
        panic!("one order")
    };
    // Each party is typed by its role's name from its source's; the second
    // trader of one role is no party the leaf holds, so its occurrence alone
    // stays, one key under the group's name holding the JSON array of what
    // it kept.
    let order = operation_of(leaf);
    assert_eq!(
        order.get_partyids().to_string(),
        "[customeraccount=ACC9, orderoriginationtrader=TRADER1, proprietary:customeraccount=ACC9, proprietary:orderoriginationtrader=TRADER1]"
    );
    assert_eq!(
        metadata(leaf),
        [(
            "parties",
            r#"[{"partyid":"TRADER2","partyidsource":"D","partyrole":"11"}]"#
        )]
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
    );
    assert!(
        order
            .get_identifiers()
            .get(&IdType::CustomerAccount)
            .is_none()
    );
    assert!(!keys(leaf).iter().any(|key| key == "nopartyids"));
}

#[test]
fn a_scalar_ending_with_one_of_its_messages_identifiers_is_lifted_into_the_leafs_identifiers() {
    let long = "R".repeat(70);
    let line = format!(
        "8=FIX.4.4|35=8|52=20260921-10:00:00|17=E1|11=C1|37=O-1|150=F|39=2|55=AAPL|54=1|\
         38=5|31=10|32=5|40=2|1080=R-1|526=S-1|MARKETORDERID=M-1|VENUE.X.PARENTORDERID=V-1|\
         NAMESPACE.OF.MORE.THAN.THIRTY.TWO.BYTES.ORDERID=N-1|1806={long}|10=0|"
    );
    // A fill's report, of the execution report's type.
    let leaves = message(line.as_bytes()).into_market_data().expect("a fill");
    assert!(!leaves.is_empty());
    for leaf in &leaves {
        let report = operation_of(leaf);
        // An execution report declares `orderid`, `clordid`, `reforderid`
        // and `secondaryclordid` among its identifiers: a dictionary field -
        // `SecondaryClOrdID(526)` one of the secondary identifiers the
        // dictionary maps - a losing alias of `OrderID(37)` and a bridge's
        // namespaced key each go under their own type, beside the sources -
        // a key from the source the rest of its name spells, a dot inside
        // kept, however long; a parent states the base it is a parent of
        // under its own source.
        for (src, kind, value) in [
            ("fix", "clordid", "C1"),
            ("fix", "orderid", "O-1"),
            ("base", "reforderid", "R-1"),
            ("fix", "secondaryclordid", "S-1"),
            ("market", "orderid", "M-1"),
            ("venue.x", "parentorderid", "V-1"),
            ("venue.x", "orderid", "V-1"),
            ("namespace.of.more.than.thirty.two.bytes", "orderid", "N-1"),
        ] {
            assert_eq!(
                report.get_identifiers().get_from(&IdKey::new(
                    src.parse::<IdSource>().unwrap(),
                    kind.parse::<IdType>().unwrap()
                )),
                Some(value),
                "{src}:{kind}: {}",
                report.get_identifiers()
            );
        }
        // What no map takes - a value past sixty-four bytes - stays where it
        // was stated.
        let kept = keys(leaf);
        assert!(
            !kept.iter().any(|key| key.ends_with("two.bytes.orderid")),
            "{kept:?}"
        );
        assert!(kept.contains(&"refclordid".to_owned()), "{kept:?}");
        for lifted in [
            "reforderid",
            "secondaryclordid",
            "marketorderid",
            "venue.x.parentorderid",
        ] {
            assert!(!kept.iter().any(|key| key == lifted), "{lifted}: {kept:?}");
        }
        // How an order is priced is a typed fact of its own.
        assert!(!kept.contains(&"ordtype".to_owned()), "{kept:?}");
    }
}

#[test]
fn a_key_ending_with_no_identifier_its_message_declares_stays() {
    // A dictionary field is lifted only where its message type declares it:
    // a new order declares no `listid`, so `ListID(66)` on one is no
    // identifier of the order's, and stays; `RefOrderID(1080)` is one a new
    // order declares. A key no dictionary tags is lifted by the crate's own
    // names whatever the message, a new order included - and leaves the
    // metadata with it.
    let leaves = message(
        b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|38=5|40=2|\
          1080=R-1|66=LIST|VENUE.X.PARENTORDERID=V-1|10=0|",
    )
    .into_market_data()
    .expect("an order");
    let [leaf] = leaves.as_slice() else {
        panic!("one order")
    };
    let order = operation_of(leaf);
    assert_eq!(
        order
            .get_identifiers()
            .get(&"reforderid".parse::<IdType>().unwrap()),
        Some("R-1")
    );
    assert_eq!(
        order
            .get_identifiers()
            .get(&"listid".parse::<IdType>().unwrap()),
        None
    );
    assert!(
        keys(leaf).contains(&"listid".to_owned()),
        "{:?}",
        keys(leaf)
    );
    assert_eq!(
        order.get_identifiers().get_from(&IdKey::new(
            "venue.x".parse::<IdSource>().unwrap(),
            "parentorderid".parse::<IdType>().unwrap()
        )),
        Some("V-1")
    );
    assert!(
        !keys(leaf).iter().any(|key| key.contains("parentorderid")),
        "{:?}",
        keys(leaf)
    );

    // The execution report declares `listid`: the same field is lifted.
    let leaves = message(
        b"8=FIX.4.4|35=8|52=20260921-10:00:00|17=E1|11=C1|37=O-1|150=F|39=2|55=AAPL|54=1|\
          38=5|31=10|32=5|66=LIST|10=0|",
    )
    .into_market_data()
    .expect("a fill");
    let report = operation_of(&leaves[0]);
    assert_eq!(
        report
            .get_identifiers()
            .get(&"listid".parse::<IdType>().unwrap()),
        Some("LIST")
    );
    assert!(!keys(&leaves[0]).contains(&"listid".to_owned()));
}

/// A key naming an identifier is lifted into its set when the set holds its
/// key free or holds the same value there; where the set holds another value
/// under that key, the key stays in the leaf's metadata as it arrived.
#[test]
fn a_key_whose_set_holds_another_value_under_its_key_stays_in_the_leafs_metadata() {
    // Two spellings of one key, `omsdealer:orderid`: the namespaced one is
    // read first, and the one that states another value is the one that stays.
    let leaves = message(
        b"8=FIX.4.4|35=8|52=20260921-10:00:00|17=E1|37=O1|150=F|39=2|54=1|55=AAPL|31=10|32=1|\
          OMSDEALERORDERID=B|OMSDEALER.ORDERID=A|10=0|",
    )
    .into_market_data()
    .expect("a fill");
    let leaf = &leaves[0];
    assert_eq!(
        operation_of(leaf).get_identifiers().get_from(&IdKey::new(
            "omsdealer".parse::<IdSource>().unwrap(),
            IdType::OrderId
        )),
        Some("A")
    );
    assert_eq!(
        metadata(leaf)
            .into_iter()
            .filter(|(key, _)| key.contains("orderid"))
            .collect::<Vec<_>>(),
        [("omsdealerorderid".to_owned(), "B".to_owned())]
    );

    // The same value under both spellings is one statement: both lifted.
    let leaves = message(
        b"8=FIX.4.4|35=8|52=20260921-10:00:00|17=E1|37=O1|150=F|39=2|54=1|55=AAPL|31=10|32=1|\
          OMSDEALERORDERID=A|OMSDEALER.ORDERID=A|10=0|",
    )
    .into_market_data()
    .expect("a fill");
    let leaf = &leaves[0];
    assert_eq!(
        operation_of(leaf).get_identifiers().get_from(&IdKey::new(
            "omsdealer".parse::<IdSource>().unwrap(),
            IdType::OrderId
        )),
        Some("A")
    );
    assert!(
        !keys(leaf).iter().any(|key| key.contains("orderid")),
        "{:?}",
        keys(leaf)
    );
}

#[test]
fn a_book_entrys_parties_are_its_leafs_accounts_leading_the_messages() {
    let leaves = message(
        b"8=FIX.4.4|35=X|52=20260921-10:00:00|55=AAPL|453=1|448=ROOT|452=1|268=2|\
          279=0|269=0|278=B1|270=100|271=10|453=1|448=MM1|452=1|\
          279=0|269=1|278=A1|270=101|271=11|10=0|",
    )
    .into_market_data()
    .expect("a book");
    let [bid, ask] = leaves.as_slice() else {
        panic!("two entries")
    };
    // The bid's own firm leads the message's; the ask states none of its
    // own and holds the message's.
    assert_eq!(
        operation_of(bid).get_partyids().get(&IdType::ExecutingFirm),
        Some("MM1")
    );
    assert_eq!(
        operation_of(ask).get_partyids().get(&IdType::ExecutingFirm),
        Some("ROOT")
    );
    assert!(
        !keys(bid).contains(&"parties".to_owned()),
        "{:?}",
        keys(bid)
    );
    assert!(
        !keys(ask).contains(&"parties".to_owned()),
        "{:?}",
        keys(ask)
    );
}

#[test]
fn an_empty_snapshot_holds_no_account_and_keeps_its_parties() {
    let leaves = message(
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|1=ACC1|453=1|448=ROOT|452=1|268=0|10=0|",
    )
    .into_market_data()
    .expect("a snapshot");
    let [snapshot] = leaves.as_slice() else {
        panic!("one control")
    };
    assert!(matches!(snapshot, MarketData::SnapshotEvent(_)));
    let kept = keys(snapshot);
    for key in ["account", "parties"] {
        assert!(kept.contains(&key.to_owned()), "{key}: {kept:?}");
    }
}

#[test]
fn a_regulatory_trade_identifier_the_leaf_holds_leaves_its_metadata() {
    let leaves = message(
        b"8=FIX.4.4|35=8|52=20260921-10:00:00|17=E1|37=O1|150=F|39=2|54=1|55=AAPL|31=10|32=1|\
          1907=2|1903=UTI-1|1906=0|1903=UTI-2|1906=0|10=0|",
    )
    .into_market_data()
    .expect("a report and its fill");
    for leaf in &leaves {
        let operation = operation_of(leaf);
        assert_eq!(
            operation.get_identifiers().get(&IdType::RegTradeId),
            Some("UTI-1")
        );
        // The second identifier of one type is none the leaf holds, and
        // alone stays.
        let kept = metadata(leaf);
        let held = kept
            .iter()
            .find(|(key, _)| key == "regulatorytradeids")
            .map(|(_, value)| value.as_str());
        assert_eq!(
            held,
            Some(r#"[{"regulatorytradeid":"UTI-2","regulatorytradeidtype":"0"}]"#),
            "{kept:?}"
        );
    }
}

#[test]
fn an_expanded_entry_carries_the_message_level_map_and_its_own_never_a_siblings() {
    let leaves = message(
        b"8=FIX.4.4|35=X|52=20260921-10:00:00|55=AAPL|1180=MDP|268=2|279=0|269=0|278=B1|270=100|271=10|83=7|279=0|269=1|278=A1|270=101|271=11|83=8|10=0|",
    )
    .into_market_data()
    .expect("an incremental update");
    let [bid, ask] = leaves.as_slice() else {
        panic!("one leaf per entry")
    };
    assert_eq!(
        metadata(bid),
        [("applid", "MDP"), ("rptseq", "7")].map(|(key, value)| (key.to_owned(), value.to_owned()))
    );
    assert_eq!(
        metadata(ask),
        [("applid", "MDP"), ("rptseq", "8")].map(|(key, value)| (key.to_owned(), value.to_owned()))
    );
}

#[test]
fn an_entrys_own_member_leads_a_message_field_of_its_name() {
    let leaves = message(
        b"8=FIX.4.4|35=X|52=20260921-10:00:00|55=AAPL|83=1|268=2|279=0|269=0|278=B1|270=100|271=10|83=7|279=0|269=1|278=A1|270=101|271=11|10=0|",
    )
    .into_market_data()
    .expect("an incremental update");
    let [bid, ask] = leaves.as_slice() else {
        panic!("one leaf per entry")
    };
    let rptseq = |leaf: &MarketData| {
        leaf.get_metadata()
            .get("rptseq")
            .map(|held| held.to_string())
    };
    assert_eq!(rptseq(bid).as_deref(), Some("7"), "the entry's own leads");
    assert_eq!(rptseq(ask).as_deref(), Some("1"), "the message's stands");
}

#[test]
fn a_book_roots_field_no_entry_inherits_rides_every_leaf() {
    // The root states the symbol and a book type every entry inherits, and a
    // price level and a position no entry reads off the root.
    let leaves = message(
        b"8=FIX.4.4|35=X|52=20260921-10:00:00|55=AAPL|1021=2|1023=5|290=3|268=2|279=0|269=0|278=B1|270=100|271=10|279=0|269=1|278=A1|270=101|271=11|10=0|",
    )
    .into_market_data()
    .expect("an incremental update");
    let [bid, ask] = leaves.as_slice() else {
        panic!("one leaf per entry")
    };
    for (leaf, entry) in [(bid, "B1"), (ask, "A1")] {
        let code = format!("Symbol=AAPL|MDBookType=2|MDEntryID={entry}");
        assert_eq!(
            metadata(leaf),
            [("mdentrypositionno", "3"), ("mdpricelevel", "5")]
                .map(|(key, value)| (key.to_owned(), value.to_owned())),
            "{entry}"
        );
        // What the entries inherit is read, and none of it is repeated.
        assert!(!keys(leaf).iter().any(|key| key == "mdbooktype"), "{entry}");
        // The entry's code, stored under the side it takes.
        assert_eq!(event_of(leaf).get_crosscode(), leaf.stored_crosscode(&code));
    }
}

#[test]
fn a_trade_side_keeps_its_own_members_bare_and_the_order_independence_pins_hold() {
    let read = |line: &[u8]| {
        let mut sides: Vec<ExecutionEvent> =
            split(line).into_iter().skip(1).map(execution_of).collect();
        sides.sort_by_key(|side| side.get_side().code());
        sides
    };
    let first = read(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|487=0|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|54=1|1427=BUY-EXEC|1009=4|528=A|54=2|1427=SELL-EXEC|1009=6|528=P|10=0|",
    );
    let second = read(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|487=0|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|54=2|1427=SELL-EXEC|1009=6|528=P|54=1|1427=BUY-EXEC|1009=4|528=A|10=0|",
    );
    let identities =
        |sides: &[ExecutionEvent]| sides.iter().map(Element::get_curruuid).collect::<Vec<_>>();
    assert_eq!(
        identities(&first),
        identities(&second),
        "the side order changes nothing"
    );

    let [buy, sell] = first.as_slice() else {
        panic!("one execution per side")
    };
    let owned = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect::<Vec<_>>()
    };
    let of = |held: &yggdryl::graph::Metadata| {
        held.iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect::<Vec<_>>()
    };
    // Each side's execution carries the trade's own - `TradeReportID(571)`,
    // an identifier a trade capture report declares, is lifted into its
    // identifiers, and `TradeReportTransType(487)`, which says the
    // report executed, is no metadata - and its own side's members, bare,
    // never its sibling's. `OrderCapacity(528)` sits in the side's
    // `TradeReportOrderDetail` component, so it lands in that component's
    // JSON object, under the component's bare name.
    assert_eq!(
        of(buy.get_metadata()),
        owned(&[("tradereportorderdetail", r#"{"ordercapacity":"A"}"#)])
    );
    assert_eq!(
        of(sell.get_metadata()),
        owned(&[("tradereportorderdetail", r#"{"ordercapacity":"P"}"#)])
    );
    for side in [buy, sell] {
        assert_eq!(
            side.get_identifiers().get(&IdType::TradeReportId),
            Some("T1")
        );
    }
}

#[test]
fn the_option_turns_the_fill_off_and_the_identity_says_so() {
    let filled = fixed_codec(committed_registry());
    let bare = fixed_codec(committed_registry()).with_market_metadata(false);
    assert!(filled.market_metadata());
    assert!(!bare.market_metadata());
    let read = |codec: &yggdryl::FixCodec| {
        let capture = messages(codec, &[UNMAPPED_ORDER]);
        drained(codec.market_data(capture)).expect("an order")
    };
    let (filled, bare) = (read(&filled), read(&bare));
    assert!(!filled[0].get_metadata().is_empty());
    assert!(
        bare[0].get_metadata().is_empty(),
        "the switch fills nothing"
    );
    assert_ne!(
        filled[0].get_curruuid(),
        bare[0].get_curruuid(),
        "the map is part of what a leaf digests"
    );
    assert_ne!(filled[0].get_currhashcode(), bare[0].get_currhashcode());
    assert_eq!(
        filled[0].get_crosscode(),
        bare[0].get_crosscode(),
        "and never part of its chain"
    );

    // A message stating nothing unmapped is the same leaf either way.
    let plain = |codec: yggdryl::FixCodec| {
        let capture = messages(
            &codec,
            &[b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|44=100|38=5|10=0|"],
        );
        drained(codec.market_data(capture)).expect("an order")
    };
    assert_eq!(
        plain(fixed_codec(committed_registry())),
        plain(fixed_codec(committed_registry()).with_market_metadata(false))
    );
}

#[test]
fn book_arrow_reader_honours_the_switch() {
    let live = |codec: yggdryl::FixCodec| {
        let codec = codec.with_batch_row_size(1);
        let capture = messages(&codec, &[UNMAPPED_ORDER]);
        let books = books_of(codec.book_arrow_reader(capture, 0).expect("a book reader"));
        let [book] = books.as_slice() else {
            panic!("one book")
        };
        alive(book, true)
            .first()
            .copied()
            .expect("the order rests")
            .clone()
    };
    let filled = live(fixed_codec(committed_registry()));
    let bare = live(fixed_codec(committed_registry()).with_market_metadata(false));
    assert_eq!(filled.get_metadata().len(), 2);
    assert!(bare.get_metadata().is_empty());
    // The account is no metadata: both hold it.
    for leaf in [&filled, &bare] {
        assert_eq!(
            operation_of(leaf).get_partyids().get(&IdType::Account),
            Some("ACC1")
        );
    }
    assert_ne!(filled.get_curruuid(), bare.get_curruuid());
}

#[test]
fn a_lifecycle_merge_keeps_the_union_with_the_reference_leading() {
    let codec = fixed_codec(committed_registry());
    // Two observations of one session event - one type, session, context
    // and sequence - each stating a field the other does not, and one they
    // both state differently. The later is the reference.
    let capture = messages(
        &codec,
        &[
            b"8=FIX.4.4|35=D|34=7|52=20260921-10:00:00|65043=SESSION|65042=CONTEXT|11=C1|55=AAPL|54=1|44=100|38=5|21=1|18=G|10=0|",
            b"8=FIX.4.4|35=D|34=7|52=20260921-10:00:01|65043=SESSION|65042=CONTEXT|11=C1|55=AAPL|54=1|44=100|38=5|21=2|111=3|10=0|",
        ],
    );
    let walked = codec
        .lifecycle(capture)
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("the walk");
    assert_eq!(walked.len(), 1, "one event, observed twice");
    let leaves = drained(codec.market_data(walked)).expect("the merged order");
    let [leaf] = leaves.as_slice() else {
        panic!("one order")
    };
    assert_eq!(
        metadata(leaf),
        [("execinst", "G"), ("handlinst", "2")]
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
    );
}

#[test]
fn leaf_metadata_round_trips_through_arrow() {
    let codec = fixed_codec(committed_registry()).with_batch_row_size(1);
    let capture = messages(
        &codec,
        &[
            UNMAPPED_ORDER,
            b"8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|1180=MDP|268=1|279=0|269=0|278=B1|270=100|271=10|83=7|10=0|",
            b"8=FIX.4.4|35=D|52=20260921-10:00:02|11=C2|55=AAPL|54=1|38=5|40=2|18=G|1080=R-1|453=1|448=TRADER1|447=D|452=11|10=0|",
        ],
    );
    let expected = drained(codec.market_data(capture.clone())).expect("the leaves");
    assert!(expected.iter().all(|leaf| !leaf.get_metadata().is_empty()));
    // The third order's party is its account and its `RefOrderID(1080)` an
    // identifier, lifted out of its metadata.
    let third = operation_of(&expected[2]);
    assert_eq!(
        third.get_partyids().get(&IdType::OrderOriginationTrader),
        Some("TRADER1")
    );
    assert_eq!(
        third
            .get_identifiers()
            .get(&"reforderid".parse::<IdType>().unwrap()),
        Some("R-1")
    );
    assert_eq!(
        expected[2].get_marketdatatype(),
        yggdryl::MarketDataType::OrdLimit,
        "how it is priced is typed"
    );
    assert_eq!(
        metadata(&expected[2]),
        [("execinst".to_owned(), "G".to_owned())]
    );
    let actual = drained(
        MarketData::from_arrow_reader(codec.market_arrow_reader(capture).expect("a reader"))
            .expect("the rows read back"),
    )
    .expect("every row");
    assert_eq!(actual.len(), expected.len());
    for (index, (read, stated)) in actual.iter().zip(&expected).enumerate() {
        assert_eq!(read.get_metadata(), stated.get_metadata(), "{index}");
        assert_eq!(read.get_curruuid(), stated.get_curruuid(), "{index}");
        let (read, stated) = (operation_of(read), operation_of(stated));
        assert_eq!(read.get_partyids(), stated.get_partyids(), "{index}");
        assert_eq!(read.get_identifiers(), stated.get_identifiers(), "{index}");
    }
    // The orders state no book control, and read back whole; the entry's
    // walk-time control is no row fact.
    assert_eq!(actual[0], expected[0]);
    assert_eq!(actual[2], expected[2]);
}

/// A book folds one instant's steps of a chain in the chain's order however
/// its source read them back: a table sorting a first step's unstated place
/// after the second's hands the cancel's reject before the cancel, and the
/// book still reads the cancel first.
#[test]
fn a_book_folds_one_instants_steps_of_a_chain_in_the_chains_order() {
    let codec = fixed_codec(committed_registry());
    let lines = [
        "8=FIX.4.2|35=D|49=B|56=S|34=70|52=20260814-21:50:00|11=C-1|55=2454|54=2|38=100|40=2|44=10|60=20260814-21:50:00|10=0|",
        "8=FIX.4.2|35=8|49=S|56=B|34=71|52=20260814-21:50:01|11=C-1|37=O-1|17=X1|150=0|39=0|54=2|55=2454|38=100|44=10|151=100|14=0|6=0|60=20260814-21:50:01|10=0|",
        "8=FIX.4.2|35=F|49=B|56=S|34=72|52=20260814-21:59:46|11=C-2|41=C-1|37=O-1|54=2|55=2454|38=100|60=20260814-21:59:46|10=0|",
        "8=FIX.4.2|35=9|49=S|56=B|34=73|52=20260814-21:59:47|11=C-2|37=O-1|41=C-1|39=8|434=1|60=20260814-21:59:46|10=0|",
    ];
    let messages: Vec<FixMsg> = lines
        .iter()
        .map(|line| codec.parse_fix_line(line.as_bytes()).expect("a message"))
        .collect();
    let walked: Vec<FixMsg> = codec
        .lifecycle(messages)
        .collect::<yggdryl::Result<_>>()
        .expect("the walk");
    let (cancel, reject) = (walked.len() - 2, walked.len() - 1);
    assert_eq!(walked[cancel].get_currunix(), walked[reject].get_currunix());
    assert!(walked[cancel].get_seqnum() < walked[reject].get_seqnum());
    let fold = |messages: Vec<FixMsg>| {
        books_of(codec.book_arrow_reader(messages, 0).expect("a book reader"))
    };
    let in_order = fold(walked.clone());
    let mut reversed = walked;
    reversed.swap(cancel, reject);
    assert_eq!(fold(reversed), in_order);
}

/// What the market doors say as they pass over what they cannot read: each
/// pinned by its deduplicated warning's count, which is process-wide, so a
/// test pins that it grew rather than what it is.
#[cfg(feature = "internals")]
mod internal {
    use yggdryl::fix::FixMarketIterator;
    use yggdryl::internals::warning::count;
    use yggdryl::{FixMsg, MarketDataKind};

    use super::{
        ANONYMOUS_UPDATE, AROUND_AN_OPENING_PRICE, NO_ENTRIES_GROUP, NO_SIDE, UNREADABLE_SIDE,
        UNSUPPORTED, around, committed_registry, drained, fixed_codec, message, messages,
        refused_intake, split, too_wide_price,
    };

    /// The module every market door warns from.
    const SITE: &str = "yggdryl::fix::market";

    /// What is said of a message with no market reading.
    const NO_READING: &str = "FIX message excluded from market data: it is no order, quote, execution or W/X book message";

    /// Runs `body`, asserting it raised the warning `what` about `subject`.
    fn warns<T>(what: &str, subject: &str, body: impl FnOnce() -> T) -> T {
        let before = count(SITE, what, subject);
        let answer = body();
        assert!(
            count(SITE, what, subject) > before,
            "expected the warning {what:?} about {subject:?}"
        );
        answer
    }

    #[test]
    fn an_entry_no_book_side_holds_is_warned_as_it_is_excluded() {
        let codec = fixed_codec(committed_registry());
        let capture = messages(&codec, &AROUND_AN_OPENING_PRICE);
        let leaves = warns(
            "FIX book entry excluded: MDEntryType is not a bid, an offer or a trade",
            "MDEntryType",
            || capture[1].market_data().unwrap(),
        );
        assert!(leaves.is_empty());
    }

    #[test]
    fn an_entry_or_a_message_that_cannot_stand_is_warned_as_it_is_excluded() {
        let [no_action, no_type, no_reading] = UNSUPPORTED;
        let cases: [(&str, &str, &[u8]); 5] = [
            (
                "FIX book entry excluded: it states no MDUpdateAction",
                "MDUpdateAction",
                no_action,
            ),
            (
                "FIX book entry excluded: it states no MDEntryType",
                "MDEntryType",
                no_type,
            ),
            (NO_READING, "B", no_reading),
            (
                "FIX book entry excluded: MDUpdateAction is not an incremental action",
                "MDUpdateAction",
                b"8=FIX.4.4|35=X|55=AAPL|268=1|279=9|269=0|278=B1|270=100|271=2|10=0|",
            ),
            (
                "FIX book entry excluded: an anonymous incremental update names no entry",
                "MDEntryID",
                ANONYMOUS_UPDATE,
            ),
        ];
        for (what, subject, line) in cases {
            let leaves = warns(what, subject, || message(line).into_market_data().unwrap());
            assert!(leaves.is_empty(), "{what}");
        }
        let acknowledged = message(b"8=FIX.4.4|35=8|17=E1|37=O1|150=0|10=0|");
        assert_eq!(
            acknowledged.msgcat(),
            MarketDataKind::Order,
            "its order's report"
        );
        let leaves = warns(
            "FIX message excluded from market data: an execution report of no fill states no fill",
            "8",
            || acknowledged.into_market_data().unwrap(),
        );
        assert!(leaves.is_empty(), "a report of no fill states no fill");
        let leaves = warns(
            "FIX full refresh states no NoMDEntries group; defaulted to an empty snapshot",
            "NoMDEntries",
            || message(NO_ENTRIES_GROUP).into_market_data().unwrap(),
        );
        assert_eq!(leaves.len(), 1, "the empty snapshot");
    }

    #[test]
    fn a_book_message_counting_no_entries_states_its_scope_empty_without_a_warning() {
        // `NoMDEntries(268)=0` states the group, holding no entry: an
        // incremental refresh counting none changes nothing and says
        // nothing, parsed or read back out of the fixed row whose record
        // keeps the count, while one missing its group is excluded aloud. No
        // other test raises this warning, so its count is this test's.
        const MISSING: &str = "FIX incremental refresh excluded: it states no NoMDEntries group";
        let counted = message(b"8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=0|10=0|");
        let schema = yggdryl::fix_schema(counted.registry(), "fix").unwrap();
        let row = counted.into_row(&schema).unwrap();
        let read_back =
            FixMsg::from_row(std::sync::Arc::clone(counted.registry()), &schema, &row).unwrap();
        let before = count(SITE, MISSING, "NoMDEntries");
        for held in [counted, read_back] {
            assert!(held.into_market_data().unwrap().is_empty(), "no change");
        }
        assert_eq!(count(SITE, MISSING, "NoMDEntries"), before);
        let leaves = warns(MISSING, "NoMDEntries", || {
            message(b"8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|10=0|")
                .into_market_data()
                .unwrap()
        });
        assert!(leaves.is_empty(), "excluded");
    }

    #[test]
    fn a_price_no_decimal_holds_is_warned_whether_it_excludes_or_is_null() {
        warns(
            "FIX book entry excluded: its MDEntryPx is no exact decimal, and a new entry cannot rest unpriced",
            "MDEntryPx",
            || too_wide_price("W").into_market_data().unwrap(),
        );
        warns(
            "FIX book entry value is no exact decimal; defaulted to null",
            "MDEntryPx",
            || too_wide_price("X").into_market_data().unwrap(),
        );
    }

    #[test]
    fn a_trade_side_stating_no_side_is_warned_as_it_defaults_to_unknown() {
        warns(
            "FIX trade side states no Side; its execution's side defaulted to UNKN",
            "Side",
            || split(NO_SIDE),
        );
        // The parse passed over the side no side reads, so the split meets
        // a side stating none.
        warns(
            "FIX trade side states no Side; its execution's side defaulted to UNKN",
            "Side",
            || split(UNREADABLE_SIDE),
        );
    }

    /// A trade's and a batch's leaves are the messages their parse splits
    /// off, so answering none of their own is no warning.
    #[test]
    fn a_trade_and_a_batch_answer_no_leaf_in_silence() {
        for (line, msgtype) in [
            (NO_SIDE, "AE"),
            (
                b"8=FIX.4.4|35=E|52=20260921-10:00:00|66=L1|394=3|68=1|73=1|11=C1|67=1|55=AAPL|54=1|38=5|40=2|44=100.5|10=0|".as_slice(),
                "E",
            ),
        ] {
            let source: FixMsg = split(line).remove(0);
            let before = count(SITE, NO_READING, msgtype);
            assert!(source.market_data().unwrap().is_empty());
            assert_eq!(count(SITE, NO_READING, msgtype), before, "{msgtype}");
        }
    }

    #[test]
    fn a_refused_intake_and_a_regression_are_warned_as_the_doors_go_on() {
        let codec = fixed_codec(committed_registry());
        let leaves = warns(
            "FIX message excluded from market data: its intake refused what it states",
            "intake",
            || drained(codec.market_data(around(&codec, refused_intake()))).unwrap(),
        );
        assert_eq!(leaves.len(), 2);
        let regressing = messages(
            &codec,
            &[
                b"8=FIX.4.4|35=D|52=20260921-10:00:01|11=C1|55=AAPL|54=1|44=99|38=5|10=0|",
                b"8=FIX.4.4|35=S|52=20260921-10:00:00|117=Q1|55=AAPL|54=2|133=101|135=7|10=0|",
            ],
        );
        let lazy = warns(
            "FIX market data operation yielded out of order: a book excludes it",
            "QUOT",
            || drained(FixMarketIterator::new(regressing.into_iter())).unwrap(),
        );
        assert_eq!(lazy.len(), 2);
    }
}
