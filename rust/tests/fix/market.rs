//! `rust/src/fix/market.rs`: the typed FIX boundary into graph operations.

use std::sync::Arc;

use super::{SoleMessage, committed_registry, fixed_codec};
use yggdryl::graph::book::{ENTRY_ID, ENTRY_REF_ID};
use yggdryl::graph::{
    BookEvent, Element, Event, Market, MarketData, MarketKind, MdUpdateAction, Operation,
};
use yggdryl::{DataType, Decimal, Error, Field, FixCode, FixMsg, FixRegistry, Scalar, StructType};

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

/// The books a lifted `marketdata` stream holds, each a `book_event` row.
fn books_of(reader: yggdryl::arrow::BatchReader) -> Vec<BookEvent> {
    MarketData::from_arrow_reader(reader)
        .unwrap()
        .map(|value| BookEvent::try_from(value?))
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap()
}

#[test]
fn direct_market_categories_move_into_their_operation_kind_and_arrow_round_trip() {
    let cases: &[(&[u8], i32, MarketKind)] = &[
        (
            b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|44=100|38=5|10=0|",
            10,
            MarketKind::OrderEvent,
        ),
        (
            b"8=FIX.4.4|35=S|117=Q1|55=AAPL|132=99|134=7|133=101|135=8|10=0|",
            14,
            MarketKind::QuoteEvent,
        ),
        (
            b"8=FIX.4.4|35=8|17=E1|37=O1|55=AAPL|31=100|32=2|150=F|10=0|",
            8,
            MarketKind::ExecutionEvent,
        ),
    ];

    for (line, operation_id, kind) in cases {
        let source = message(line);
        assert_eq!(source.get_marketoperationid(), Some(*operation_id));
        assert_eq!(
            source
                .get_by_tag(yggdryl::MSGCAT_TAG_NAME.0)
                .and_then(|value| value.as_i128()),
            Some(i128::from(*operation_id))
        );
        let operations = source.into_market_operations().expect("a market category");
        assert_eq!(operations.len(), 1);
        let operation = operation_of(&operations[0]);
        assert_eq!(operations[0].kind(), *kind, "{line:?}");
        assert_eq!(operation.get_marketoperationid(), Some(*operation_id));
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

#[test]
fn trade_capture_is_one_trade_with_exact_sided_executions_and_arrow_round_trip() {
    let operations = message(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|150=F|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|54=1|1427=BUY-EXEC|1009=4|37=BUY-ORDER|11=BUY-CLIENT|54=2|1427=SELL-EXEC|1009=6|37=SELL-ORDER|11=SELL-CLIENT|10=0|",
    )
    .into_market_operations()
    .expect("an actual trade with two sides");

    let [MarketData::TradeEvent(trade)] = operations.as_slice() else {
        panic!("one composite trade")
    };
    assert_eq!(trade.get_marketoperationid(), Some(21));
    // A trade capture states no price and no quantity of its own: what it
    // last executed at, and how much, are `lastpx` and `lastqty`, and
    // neither stands in for the price or the quantity.
    assert_eq!((trade.get_price(), trade.get_quantity()), (None, None));
    assert_eq!(text(trade.get_lastpx()).as_deref(), Some("101.25"));
    assert_eq!(text(trade.get_lastqty()).as_deref(), Some("10"));

    let [buy, sell] = trade.executions() else {
        panic!("one execution per stated trade side")
    };
    assert!(buy.get_side().is_bid());
    assert!(sell.get_side().is_ask());
    assert_eq!(buy.kind(), MarketKind::Execution);
    // Each side keeps `SideLastQty(1009)` as its last executed quantity
    // only, beside the trade's last executed price: a side states no
    // price and no quantity, and nothing invents one.
    assert_eq!((buy.get_price(), sell.get_price()), (None, None));
    assert_eq!((buy.get_quantity(), sell.get_quantity()), (None, None));
    assert_eq!(text(buy.get_lastpx()).as_deref(), Some("101.25"));
    assert_eq!(text(sell.get_lastpx()).as_deref(), Some("101.25"));
    assert_eq!(text(buy.get_lastqty()).as_deref(), Some("4"));
    assert_eq!(text(sell.get_lastqty()).as_deref(), Some("6"));
    assert_eq!(buy.get_altids().get("SIDEEXECID"), Some("BUY-EXEC"));
    assert_eq!(sell.get_altids().get("SIDEEXECID"), Some("SELL-EXEC"));
    assert_eq!(buy.get_altids().get("ORDERID"), Some("BUY-ORDER"));
    assert_eq!(sell.get_altids().get("ORDERID"), Some("SELL-ORDER"));
    assert_eq!(buy.get_altids().get("CLORDID"), Some("BUY-CLIENT"));
    assert_eq!(sell.get_altids().get("CLORDID"), Some("SELL-CLIENT"));
    assert_eq!(
        trade.get_altids().get("TRADEID"),
        None,
        "TradeReportID(571) is no alternate identifier source today"
    );
    assert_ne!(buy.get_curruuid(), sell.get_curruuid());
    assert_ne!(buy.get_crossuuid(), sell.get_crossuuid());
    assert_ne!(buy.get_crosscode(), sell.get_crosscode());

    let expected = operations[0].clone();
    let encoded = MarketData::arrow_reader(operations, Some(1), None).unwrap();
    let actual = MarketData::from_arrow_reader(encoded)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn trade_capture_without_a_side_refuses_the_exact_occurrence() {
    let error = message(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|150=F|55=AAPL|32=4|31=101.25|60=20260921-10:00:00|552=1|1427=NO-SIDE|1009=4|37=ORDER-1|11=CLIENT-1|10=0|",
    )
    .into_market_operations()
    .expect_err("an execution side is required");

    assert!(
        matches!(&error, Error::InvalidRecord { path, .. }
            if path == "$.NoSides(552)[0].Side(54)"),
        "{error}"
    );
}

#[test]
fn only_initial_trade_capture_reports_decompose_without_requiring_exec_type() {
    let initial = message(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|487=0|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|54=1|1427=BUY-EXEC|1009=10|54=2|1427=SELL-EXEC|1009=10|10=0|",
    );
    let expected_execunix = initial.get_currunix();
    let operations = initial
        .into_market_operations()
        .expect("an initial AE is an actual trade without ExecType");
    let [MarketData::TradeEvent(trade)] = operations.as_slice() else {
        panic!("one composite trade")
    };
    assert_eq!(trade.get_execunix(), Some(expected_execunix));
    assert!(
        trade
            .executions()
            .iter()
            .all(|execution| execution.get_execunix() == Some(expected_execunix))
    );

    for line in [
        b"8=FIX.4.4|35=AE|571=CANCEL|487=1|150=F|552=1|54=1|1427=E1|10=0|".as_slice(),
        b"8=FIX.4.4|35=AR|571=ACK|487=0|150=F|552=1|54=1|1427=E1|10=0|".as_slice(),
    ] {
        let error = message(line)
            .into_market_operations()
            .expect_err("a cancellation or acknowledgement is not an execution trade");
        assert!(error.to_string().contains("MsgType(35)"), "{error}");
    }
}

#[test]
fn stable_trade_side_ids_make_group_order_irrelevant_to_identity() {
    let first = message(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|487=0|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|54=1|1427=BUY-EXEC|1009=4|54=2|1427=SELL-EXEC|1009=6|10=0|",
    )
    .into_market_operations()
    .unwrap();
    let second = message(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|487=0|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|54=2|1427=SELL-EXEC|1009=6|54=1|1427=BUY-EXEC|1009=4|10=0|",
    )
    .into_market_operations()
    .unwrap();

    let ([MarketData::TradeEvent(first)], [MarketData::TradeEvent(second)]) =
        (first.as_slice(), second.as_slice())
    else {
        panic!("two composite trades")
    };
    assert_eq!(first, second);
    assert_eq!(first.get_curruuid(), second.get_curruuid());
}

#[test]
fn anonymous_trade_sides_are_order_independent_and_stable_id_tags_do_not_collide() {
    let first = message(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|487=0|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|54=1|1009=4|54=2|1009=6|10=0|",
    )
    .into_market_operations()
    .unwrap();
    let second = message(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|487=0|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|54=2|1009=6|54=1|1009=4|10=0|",
    )
    .into_market_operations()
    .unwrap();
    let ([MarketData::TradeEvent(first)], [MarketData::TradeEvent(second)]) =
        (first.as_slice(), second.as_slice())
    else {
        panic!("two anonymous composite trades")
    };
    assert_eq!(first, second);
    assert_eq!(first.get_curruuid(), second.get_curruuid());

    let tagged = message(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T2|487=0|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|54=1|1427=SAME|1009=4|54=1|1506=SAME|1009=6|10=0|",
    )
    .into_market_operations()
    .expect("the identifier tag distinguishes equal values");
    let [MarketData::TradeEvent(tagged)] = tagged.as_slice() else {
        panic!("one tagged composite trade")
    };
    assert_ne!(
        tagged.executions()[0].get_crosscode(),
        tagged.executions()[1].get_crosscode()
    );
    assert_eq!(
        tagged
            .executions()
            .iter()
            .filter_map(|execution| execution.get_altids().get("SIDEEXECID"))
            .collect::<Vec<_>>(),
        ["SAME"]
    );
    assert_eq!(
        tagged
            .executions()
            .iter()
            .filter_map(|execution| execution.get_altids().get("SIDETRADEID"))
            .collect::<Vec<_>>(),
        ["SAME"]
    );
}

#[test]
fn lifecycled_trade_capture_streams_its_sided_executions_into_books() {
    let codec = fixed_codec(committed_registry()).with_batch_row_size(1);
    let trade = codec
        .sole_line(
            b"8=FIX.4.4|35=AE|49=SELL|56=BUY|34=7|52=20260921-10:00:00|571=T1|150=F|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|54=1|1427=BUY-EXEC|1009=4|37=BUY-ORDER|11=BUY-CLIENT|54=2|1427=SELL-EXEC|1009=6|37=SELL-ORDER|11=SELL-CLIENT|10=0|",
        )
        .unwrap();
    let reader = codec
        .book_arrow_reader(codec.lifecycle([trade]), 0, false)
        .expect("a lifecycled trade book stream");
    let books = books_of(reader);

    assert_eq!(books.len(), 1);
    assert_eq!(books[0].executions().len(), 2);
    assert!(
        books[0]
            .executions()
            .iter()
            .any(|execution| execution.get_side().is_bid())
    );
    assert!(
        books[0]
            .executions()
            .iter()
            .any(|execution| execution.get_side().is_ask())
    );
    assert!(books[0].bid().is_empty());
    assert!(books[0].ask().is_empty());
}

#[test]
fn msgtype_edits_resettle_derived_operation_ids_and_leave_stated_ids_alone() {
    let mut derived = message(b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|44=100|38=5|10=0|");
    assert_eq!(derived.get_marketoperationid(), Some(10));

    derived.set(35, Scalar::from("S")).unwrap();
    assert_eq!(derived.get_marketoperationid(), Some(14));
    assert!(matches!(
        derived.market_operations().unwrap().as_slice(),
        [MarketData::QuoteEvent(_)]
    ));

    assert_eq!(derived.remove(35).unwrap(), Some(Scalar::from("S")));
    assert_eq!(derived.get_marketoperationid(), Some(0));
    assert!(derived.market_operations().is_err());

    let mut stated = message(b"8=FIX.4.4|35=D|65054=8|11=C1|55=AAPL|54=1|44=100|38=5|10=0|");
    assert_eq!(stated.get_marketoperationid(), Some(8));
    let execution_hash = stated.get_currhashcode();
    let execution_uuid = stated.get_curruuid();
    stated.set(35, Scalar::from("S")).unwrap();
    assert_eq!(
        stated.get_marketoperationid(),
        Some(8),
        "an explicit MsgCat row value owns the generic operation ID"
    );
    assert_ne!(stated.get_currhashcode(), execution_hash);
    assert_ne!(stated.get_curruuid(), execution_uuid);
    let explicit_hash = stated.get_currhashcode();
    stated
        .set(yggdryl::MSGCAT_TAG_NAME.0, Scalar::from(14_i32))
        .unwrap();
    assert_eq!(stated.get_marketoperationid(), Some(14));
    assert_ne!(stated.get_currhashcode(), explicit_hash);
    assert_eq!(
        stated.remove(yggdryl::MSGCAT_TAG_NAME.0).unwrap(),
        Some(Scalar::from(14_i32))
    );
    assert_eq!(stated.get_marketoperationid(), Some(14));
}

#[test]
fn msgcat_registry_values_are_stable_int32_operation_ids() {
    let registry = committed_registry();
    let field = registry.field(yggdryl::MSGCAT_TAG_NAME.0).unwrap();
    assert_eq!(field.dtype(), &DataType::Int32);
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
    registry
        .merge_codeset(
            "msgcatcodeset",
            &[FixCode::new("ORDR", "10").with_aliases(["ordr"])],
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
    assert_eq!(custom.get_marketoperationid(), Some(14));
}

#[test]
fn codec_streams_fix_messages_through_books_into_arrow_with_coherent_prices() {
    let codec = fixed_codec(committed_registry()).with_batch_row_size(1);
    let snapshot = codec
        .sole_line(b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|")
        .unwrap();
    let update = codec
        .sole_line(b"8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=2|279=1|269=0|278=B1|270=101|271=11|279=0|269=2|278=T1|270=101|271=2|10=0|")
        .unwrap();

    let reader = codec
        .book_arrow_reader([snapshot, update], 0, false)
        .expect("a centralized FIX book reader");
    let books = books_of(reader);

    assert_eq!(books.len(), 2);
    assert_eq!(books[0].bid().best_price().unwrap().to_string(), "100");
    assert_eq!(books[0].ask().best_price().unwrap().to_string(), "102");
    assert_eq!(text(books[0].get_price()).as_deref(), Some("101"));
    assert_eq!(books[1].bid().best_price().unwrap().to_string(), "101");
    assert_eq!(books[1].ask().best_price().unwrap().to_string(), "102");
    assert_eq!(text(books[1].get_price()).as_deref(), Some("101.5"));
    assert_eq!(books[1].executions().len(), 1);
    assert_eq!(
        operation_of(books[1].bid().live().next().unwrap()).get_marketoperationid(),
        Some(3)
    );
    assert_eq!(books[1].executions()[0].get_marketoperationid(), Some(3));
}

#[test]
fn codec_book_admission_skips_noncontributing_records_between_market_events() {
    let codec = fixed_codec(committed_registry()).with_batch_row_size(1);
    let admitted = [
        b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|44=100|38=5|10=0|".as_slice(),
        b"8=FIX.4.4|35=S|52=20260921-10:00:01|117=Q1|55=AAPL|54=2|133=101|135=7|10=0|".as_slice(),
        b"8=FIX.4.4|35=8|52=20260921-10:00:02|17=E1|37=O1|55=AAPL|54=1|31=100|32=2|150=F|10=0|".as_slice(),
        b"8=FIX.4.4|35=W|52=20260921-10:00:03|55=AAPL|268=1|269=0|278=B1|270=99|271=10|10=0|".as_slice(),
        b"8=FIX.4.4|35=X|52=20260921-10:00:04|55=AAPL|268=1|279=1|269=0|278=B1|271=11|10=0|".as_slice(),
        b"8=FIX.4.4|35=AE|52=20260921-10:00:05|571=T1|487=0|55=AAPL|32=1|31=100|552=1|54=1|1427=E2|1009=1|10=0|".as_slice(),
    ]
    .map(|line| codec.parse_fix_line(line).unwrap());
    let ignored = [
        b"8=FIX.4.4|35=0|52=20260922-10:00:00|10=0|".as_slice(),
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
    let expected = books_of(codec.book_arrow_reader(admitted, 0, false).unwrap());
    assert_eq!(expected.len(), 6);
    assert_eq!(expected[0].bid().deltas()[0].kind(), MarketKind::OrderEvent);
    assert_eq!(expected[1].ask().deltas()[0].kind(), MarketKind::QuoteEvent);
    assert_eq!(expected[2].executions().len(), 1);
    assert_eq!(expected[5].executions().len(), 1);
    let actual = books_of(codec.book_arrow_reader(mixed, 0, false).unwrap());
    assert_eq!(actual, expected);

    let mut empty = codec.book_arrow_reader(ignored.clone(), 0, false).unwrap();
    assert!(empty.next().is_none());
    for source in ignored {
        assert!(MarketData::try_from(source.clone()).is_err());
        let mut strict = yggdryl::fix::FixMarketIterator::new([source].into_iter());
        assert!(strict.next().unwrap().is_err());
        assert!(strict.next().is_none());
    }
}

#[test]
fn codec_book_admission_preserves_source_errors_and_fuses() {
    let codec = fixed_codec(committed_registry()).with_batch_row_size(1);
    let source = [
        Ok(codec.parse_fix_line(b"8=FIX.4.4|35=0|10=0|").unwrap()),
        Err(Error::InvalidRecord {
            path: "$.intake".into(),
            reason: "source failed before the next message".into(),
        }),
        Ok(codec
            .parse_fix_line(b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|44=100|38=5|10=0|")
            .unwrap()),
    ];
    let mut reader = codec.book_arrow_reader(source, 0, false).unwrap();
    let error = reader.next().unwrap().unwrap_err().to_string();
    assert!(error.contains("$.intake"), "{error}");
    assert!(
        error.contains("source failed before the next message"),
        "{error}"
    );
    assert!(reader.next().is_none());
    assert!(reader.next().is_none());
}

#[test]
fn codec_book_admission_keeps_admitted_trade_and_book_refusals() {
    let codec = fixed_codec(committed_registry()).with_batch_row_size(1);
    for line in [
        b"8=FIX.4.4|35=AE|571=CANCEL|487=1|150=F|552=1|54=1|1427=E1|10=0|".as_slice(),
        b"8=FIX.4.4|35=AE|571=CORRECT|487=2|150=F|552=1|54=1|1427=E1|10=0|".as_slice(),
        b"8=FIX.4.4|35=AE|571=STATUS|487=0|150=I|552=1|54=1|1427=E1|10=0|".as_slice(),
        b"8=FIX.4.4|35=AE|571=MISSING-SIDE|487=0|552=1|1427=E1|10=0|".as_slice(),
        b"8=FIX.4.4|35=W|55=AAPL|10=0|".as_slice(),
        b"8=FIX.4.4|35=X|55=AAPL|268=1|279=9|269=0|278=B1|270=100|271=2|10=0|".as_slice(),
    ] {
        let invalid = codec.parse_fix_line(line).unwrap();
        let expected = invalid.market_operations().unwrap_err().to_string();
        let source = [
            codec.parse_fix_line(b"8=FIX.4.4|35=0|10=0|").unwrap(),
            invalid,
        ];
        let mut reader = codec.book_arrow_reader(source, 0, false).unwrap();
        let actual = reader.next().unwrap().unwrap_err().to_string();
        assert!(
            actual.contains(&expected),
            "expected {expected:?}, got {actual:?}"
        );
        assert!(reader.next().is_none());
    }
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
    let reader = codec.book_arrow_reader(messages, 0, false).unwrap();
    let books = books_of(reader);

    assert_eq!(books.len(), 3);
    assert_eq!(books[0].bid().len(), 1);
    assert_eq!(books[1].bid().len(), 1);
    assert_eq!(books[2].ask().len(), 1);
    assert!(books[0].ask().is_empty());
    assert!(books[1].ask().is_empty());
    assert!(books[2].bid().is_empty());
    let versions = [
        books[0].bid().live().next().unwrap(),
        books[1].bid().live().next().unwrap(),
        books[2].ask().live().next().unwrap(),
    ];
    for (index, (book, version)) in books.iter().zip(versions).enumerate() {
        assert_eq!(version.kind(), MarketKind::OrderEvent);
        let operation = operation_of(version);
        assert_eq!(book.get_ticker(), Some("AAPL"));
        assert_eq!(operation.get_ticker(), Some("AAPL"));
        assert_eq!(operation.get_altids().get(ENTRY_ID), Some("B1"));
        assert_eq!(operation.get_altids().get("ORDERID"), Some("O1"));
        assert_eq!(operation.get_seqnum(), index as u64);
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
            let bid = operation.get_bid().expect("the bid lane");
            assert_eq!(bid.price, operation.get_price());
            assert_eq!(bid.quantity, operation.get_quantity());
            assert_eq!(operation.get_ask(), None);
        } else {
            assert!(operation.get_side().is_ask());
            let ask = operation.get_ask().expect("the ask lane");
            assert_eq!(ask.price, operation.get_price());
            assert_eq!(ask.quantity, operation.get_quantity());
            assert_eq!(operation.get_bid(), None);
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
    // The book control each entry stated rides with it, typed: what the
    // partial update said, and only that.
    assert_eq!(
        versions[0].book().and_then(|book| book.action),
        Some(MdUpdateAction::Snapshot),
        "a full refresh entry"
    );
    let changed = versions[1].book().expect("the change's control");
    assert_eq!(changed.action, Some(MdUpdateAction::Change));
    assert_eq!(
        changed.entry_size.map(|held| held.to_string()),
        Some("11".to_owned())
    );
    assert_eq!(changed.entry_px, None, "the change restated no price");
    let overlaid = versions[2].book().expect("the overlay's control");
    assert_eq!(overlaid.action, Some(MdUpdateAction::Overlay));
    assert_eq!(
        overlaid.entry_px.map(|held| held.to_string()),
        Some("101".to_owned())
    );
    assert_eq!(overlaid.entry_size, None, "the overlay restated no size");
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
    let reader = codec.book_arrow_reader(messages, 0, false).unwrap();
    let books = books_of(reader);

    assert_eq!(books.len(), 2);
    let previous = operation_of(books[0].bid().live().next().unwrap());
    assert!(books[1].bid().is_empty());
    assert!(books[1].ask().is_empty());
    let [deleted] = books[1].bid().deltas() else {
        panic!("one terminal bid delta")
    };
    assert_eq!(deleted.kind(), MarketKind::OrderEvent);
    assert_eq!(
        deleted.book().and_then(|book| book.action),
        Some(MdUpdateAction::Delete)
    );
    let deleted = operation_of(deleted);
    assert!(!deleted.get_state().is_live());
    assert!(deleted.get_side().is_bid());
    assert_eq!(deleted.get_ticker(), Some("AAPL"));
    assert_eq!(deleted.get_altids().get(ENTRY_ID), Some("B1"));
    assert_eq!(deleted.get_altids().get("ORDERID"), Some("O1"));
    assert_eq!(deleted.get_prevuuid(), Some(previous.get_curruuid()));
    assert_eq!(deleted.get_prevunix(), Some(previous.get_currunix()));
    assert_eq!(deleted.get_seqnum(), previous.get_seqnum() + 1);
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

    let reader = codec.book_arrow_reader(messages, 0, false).unwrap();
    let books = books_of(reader);
    assert_eq!(books.len(), 2);
    for (index, book) in books.iter().enumerate() {
        assert_eq!(book.get_ticker(), Some("AAPL"));
        assert_eq!(book.bid().len(), 1);
        assert!(book.ask().is_empty());
        let operation = book.bid().live().next().unwrap();
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
    .market_operations()
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
    assert_eq!(operations[1].get_altids().get("ORDERID"), Some("O1"));
    assert_eq!(
        operations[1].get_altids().get("MDREQID"),
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
        assert!(operation.get_state().is_live());
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
    .into_market_operations()
    .unwrap();
    assert!(scope_of(&initial[0]).contains("MDReqID=REQ-1"));
    let mut book = BookEvent::new(currunix(&initial[0]), "AAPL");
    book.add_operations(initial).unwrap();

    let empty_other_request =
        message(b"8=FIX.4.4|35=W|52=20260921-10:00:01|55=AAPL|262=REQ-2|268=0|10=0|")
            .into_market_operations()
            .unwrap();
    assert!(scope_of(&empty_other_request[0]).contains("MDReqID=REQ-2"));
    assert!(is_full_snapshot(&empty_other_request[0]));
    book.add_operations(empty_other_request).unwrap();
    assert_eq!(book.bid().len(), 1);
}

#[test]
fn an_operation_names_its_entry_beside_the_message_identifiers_and_refuses_a_key_without_a_source()
{
    let mut source = message(b"8=FIX.4.4|35=X|55=AAPL|268=1|279=1|269=0|278=B1|271=11|10=0|");
    // A message's alternate identifiers are views of its fields: a key the
    // dictionary has a source field for is written there, one it has not is
    // a located refusal, never a private slot.
    let error = source.insert_altid("foreign", "kept").unwrap_err();
    assert!(
        matches!(&error, Error::InvalidRecord { path, .. } if path.contains("foreign")),
        "{error}"
    );
    assert!(source.insert_altid("MDREQID", "REQ-9").unwrap());
    assert_eq!(
        source.get_by_tag(262).as_ref().and_then(Scalar::as_str),
        Some("REQ-9")
    );

    let input = MarketData::try_from(source).unwrap();
    let operation = operation_of(&input);
    let altids = operation.get_altids();
    assert_eq!(altids.get(ENTRY_ID), Some("B1"));
    assert_eq!(altids.get(ENTRY_REF_ID), None);
    assert_eq!(altids.get("MDREQID"), Some("REQ-9"));
    assert_eq!(altids.get("foreign"), None);
    let book = input.book().expect("an entry states its control");
    assert_eq!(book.action, Some(MdUpdateAction::Change));
    assert_eq!(
        book.entry_size.map(|held| held.to_string()),
        Some("11".to_owned())
    );
    assert_eq!(book.entry_px, None, "the entry restated no price");
    assert!(scope_of(&input).contains("MDReqID=REQ-9"));
}

#[test]
fn incremental_actions_keep_the_wire_action_and_terminal_delete_state() {
    let inputs = message(
        b"8=FIX.4.4|35=X|83=7|1181=43|268=3|279=0|269=0|278=B1|55=AAPL|270=100|271=10|290=1|279=1|269=0|278=B1|55=AAPL|270=101|271=11|290=1|279=2|269=0|280=B1|55=AAPL|10=0|",
    )
    .into_market_operations()
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
    assert_eq!(operations[0].get_altids().get(ENTRY_ID), Some("B1"));
    assert_eq!(operations[2].get_altids().get(ENTRY_ID), None);
    assert_eq!(operations[2].get_altids().get(ENTRY_REF_ID), Some("B1"));
    assert_eq!(inputs[1].book().and_then(|book| book.position), Some(1));
}

#[test]
fn fallback_identity_is_scoped_typed_and_stable_across_price_changes() {
    let input = MarketData::try_from(message(
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

#[test]
fn unsupported_market_shapes_name_the_exact_fix_path() {
    let cases: &[(&[u8], &str)] = &[
        (
            b"8=FIX.4.4|35=X|268=1|269=0|278=B1|10=0|",
            "MDUpdateAction(279)",
        ),
        (
            b"8=FIX.4.4|35=X|268=1|279=0|278=B1|10=0|",
            "MDEntryType(269)",
        ),
        (b"8=FIX.4.4|35=8|17=E1|37=O1|150=0|10=0|", "MsgType(35)"),
    ];
    for (line, path_part) in cases {
        let error = message(line)
            .into_market_operations()
            .expect_err("the shape is unsupported");
        assert!(
            matches!(&error, Error::InvalidRecord { path, .. } if path.contains(path_part)),
            "{error}"
        );
    }
}

#[test]
fn singular_conversion_refuses_a_multi_entry_book_message() {
    let error = MarketData::try_from(message(
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
    .into_market_operations()
    .expect("entry clocks");

    assert_eq!(operations.len(), 2);
    assert_eq!(currunix(&operations[0]), 1_789_896_600_123_456_789);
    assert_eq!(currunix(&operations[1]), 1_789_896_600_223_456_789);
    assert_eq!(
        event_of(&operations[1]).get_execunix(),
        Some(1_789_896_600_223_456_789)
    );

    let snapshot = MarketData::try_from(message(
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
        source.market_operations().expect("borrowed expansion"),
        source
            .clone()
            .into_market_operations()
            .expect("owned expansion"),
    ] {
        assert_eq!(
            operations
                .iter()
                .map(|input| operation_of(input).get_altids().get(ENTRY_ID).unwrap())
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
        .into_market_operations()
        .unwrap();
    let mut book = BookEvent::new(currunix(&snapshot[0]), "AAPL");
    book.add_operations(snapshot).unwrap();

    let size_only = message(b"8=FIX.4.4|35=X|55=AAPL|268=1|279=1|269=0|278=B1|271=11|10=0|")
        .into_market_operations()
        .unwrap();
    book.add_operations(size_only).unwrap();
    let live = book.bid().live().next().unwrap();
    assert_eq!(text(live.get_price()).as_deref(), Some("100"));
    assert_eq!(text(live.get_quantity()).as_deref(), Some("11"));

    let price_only = message(b"8=FIX.4.4|35=X|55=AAPL|268=1|279=5|269=0|278=B1|270=101|10=0|")
        .into_market_operations()
        .unwrap();
    book.add_operations(price_only).unwrap();
    let live = book.bid().live().next().unwrap();
    assert_eq!(text(live.get_price()).as_deref(), Some("101"));
    assert_eq!(text(live.get_quantity()).as_deref(), Some("11"));
}

#[test]
fn anonymous_incremental_changes_use_stable_position_identity_or_refuse_ambiguity() {
    let snapshot = message(
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=1|269=0|290=1|270=100|271=10|10=0|",
    )
    .into_market_operations()
    .unwrap();
    let identity = snapshot[0].get_crosscode().to_owned();
    let mut book = BookEvent::new(currunix(&snapshot[0]), "AAPL");
    book.add_operations(snapshot).unwrap();

    let change = message(
        b"8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=1|279=1|269=0|290=1|271=11|10=0|",
    )
    .into_market_operations()
    .unwrap();
    assert_eq!(change[0].get_crosscode(), identity);
    book.add_operations(change).unwrap();
    assert_eq!(book.bid().len(), 1);
    assert_eq!(
        text(book.bid().live().next().unwrap().get_price()).as_deref(),
        Some("100")
    );
    assert_eq!(
        text(book.bid().live().next().unwrap().get_quantity()).as_deref(),
        Some("11")
    );

    let error = message(b"8=FIX.4.4|35=X|55=AAPL|268=1|279=1|269=0|271=12|10=0|")
        .into_market_operations()
        .expect_err("an anonymous update without a stable coordinate is ambiguous");
    assert!(error.to_string().contains("MDEntryID"), "{error}");
}

#[test]
fn an_empty_full_refresh_clears_its_scope() {
    let initial = message(
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=1|269=0|278=B1|270=100|271=10|10=0|",
    )
    .into_market_operations()
    .unwrap();
    let mut book = BookEvent::new(currunix(&initial[0]), "AAPL");
    book.add_operations(initial).unwrap();
    assert_eq!(book.bid().len(), 1);

    let empty = message(b"8=FIX.4.4|35=W|52=20260921-10:00:01|55=AAPL|268=0|10=0|")
        .into_market_operations()
        .unwrap();
    assert!(matches!(empty.as_slice(), [MarketData::SnapshotEvent(_)]));
    assert!(is_full_snapshot(&empty[0]));
    assert_eq!(empty[0].get_ticker(), Some("AAPL"));
    book.add_operations(empty).unwrap();
    assert!(book.bid().is_empty());
    assert!(book.ask().is_empty());
}

/// A FIX event always states an operation id (`derived_marketoperationid`),
/// but a snapshot control states no operation of its own: it is a
/// [`yggdryl::graph::SnapshotEvent`], which holds no operation fact, its row
/// states none, and an Arrow round trip returns exactly what went in.
#[test]
fn an_empty_full_refresh_states_no_operation_and_round_trips_through_arrow() {
    let expected = message(b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=0|10=0|")
        .into_market_operations()
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
        batch
            .column_by_name("marketoperationid")
            .unwrap()
            .null_count(),
        1,
        "the row states no operation id"
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
    let input = MarketData::try_from(message(
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

#[test]
fn a_typed_price_outside_decimal18_is_refused_instead_of_becoming_zero() {
    let msgtype = tagged("MsgType", 35, DataType::utf8());
    let entry_type = tagged("MDEntryType", 269, DataType::utf8());
    let price = tagged(
        "MDEntryPx",
        270,
        DataType::decimal256(39, 0).expect("a 39-digit decimal"),
    );
    let item = StructType::from_fields([entry_type, price])
        .map(DataType::from)
        .unwrap()
        .required_field("MDEntry");
    let mut entries = DataType::serie(item).required_field("MDEntries");
    entries.as_fix_mut().set_counter(268).unwrap();
    let counter = tagged("NoMDEntries", 268, DataType::Int32);
    let registry = Arc::new(
        FixRegistry::from_fields([counter, msgtype.clone(), entries.clone()])
            .expect("the minimal book registry"),
    );
    let root = StructType::from_fields([msgtype, entries])
        .map(DataType::from)
        .unwrap()
        .required_field("fix");
    let too_wide = Scalar::d256(
        "170141183460469231731687303715884105728".parse().unwrap(),
        0,
    );
    let value = Scalar::from_sequence([
        Scalar::from("W"),
        Scalar::from_sequence([Scalar::from_sequence([Scalar::from("0"), too_wide])]),
    ]);
    let error = FixMsg::with_registry(registry, root, value)
        .unwrap()
        .into_market_operations()
        .expect_err("the graph price cannot hold a 39-digit coefficient");
    assert!(
        matches!(&error, Error::InvalidRecord { path, reason }
            if path.contains("MDEntryPx(270)") && reason.contains("exact decimal")),
        "{error}"
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
    let expected = parsed.market_operations().expect("a full refresh");
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

    assert_eq!(run.market_operations().unwrap(), expected);
    assert_eq!(
        column
            .market_operations()
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

    // The execution reaches a book with its FX parts.
    let inputs = held.into_market_operations().expect("an execution");
    assert_eq!(inputs.len(), 1);
    let books = yggdryl::graph::BookIterator::new(inputs.into_iter(), 0, false)
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
}

#[test]
fn a_quotes_fx_parts_land_on_its_two_lanes() {
    let held = message(
        b"8=FIX.4.4|35=S|117=Q1|55=EURUSD|132=1.2|134=1000000|133=1.21|135=1000000|188=1.19|189=0.01|190=1.2|191=0.011|10=0|",
    );
    let bid = held.get_bid().expect("a bid lane");
    assert_eq!(text(bid.spotrate).as_deref(), Some("1.19"));
    assert_eq!(text(bid.forwardpoints).as_deref(), Some("0.01"));
    let ask = held.get_ask().expect("an ask lane");
    assert_eq!(text(ask.spotrate).as_deref(), Some("1.2"));
    assert_eq!(text(ask.forwardpoints).as_deref(), Some("0.011"));
    assert_eq!(
        held.get_spotrate(),
        None,
        "a two-lane quote states no last price of its own"
    );
}

#[test]
fn a_levels_fx_parts_read_onto_its_lane() {
    let held = message(
        b"8=FIX.4.4|35=W|55=EURUSD|268=1|269=0|278=B1|270=1.2|271=100|1026=1.19|1027=0.01|10=0|",
    );
    let inputs = held.into_market_operations().expect("one level");
    let level = operation_of(&inputs[0]);
    assert_eq!(text(level.get_spotrate()).as_deref(), Some("1.19"));
    assert_eq!(text(level.get_forwardpoints()).as_deref(), Some("0.01"));
    let bid = level.get_bid().expect("the level is a bid lane");
    assert_eq!(text(bid.price).as_deref(), Some("1.2"));
    assert_eq!(text(bid.spotrate).as_deref(), Some("1.19"));
    assert_eq!(text(bid.forwardpoints).as_deref(), Some("0.01"));
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
fn messages(codec: &yggdryl::FixCodec, lines: &[&[u8]]) -> Vec<FixMsg> {
    lines
        .iter()
        .map(|line| codec.sole_line(line).expect("one FIX message"))
        .collect()
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

    let actual = drained(codec.market_operations(unsorted.clone())).expect("the sorted door");
    assert_eq!(
        actual, expected,
        "the door answers the sorted capture's leaves"
    );

    // The book door stays strict: the capture it is handed is out of order.
    let refused = codec
        .book_arrow_reader(unsorted.clone(), 0, false)
        .expect("a book reader")
        .find_map(Result::err)
        .expect("the book door refuses the regression")
        .to_string();
    assert!(refused.contains("$.operations"), "{refused}");

    // The sorted operations fold through the stateful book, one per leaf.
    let books = yggdryl::graph::BookIterator::new(codec.market_operations(unsorted), 0, false)
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
    // Sorted messages are not sorted operations.
    let strict = drained(yggdryl::fix::FixMarketIterator::new(
        capture.clone().into_iter(),
    ))
    .expect_err("the entry regresses behind the order")
    .to_string();
    assert!(strict.contains("$.operations"), "{strict}");

    let operations = drained(codec.market_operations(capture)).expect("the sorted door");
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
    let books = yggdryl::graph::BookIterator::new(operations.into_iter().map(Ok), 0, false)
        .expect("a book iterator")
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("the operations fold");
    assert_eq!(books.len(), 2);
}

#[test]
fn intake_errors_come_first_and_the_arrow_reader_yields_nothing_else() {
    let codec = fixed_codec(committed_registry());
    let source = || {
        [
            Ok(codec
                .sole_line(
                    b"8=FIX.4.4|35=D|52=20260921-10:00:01|11=C1|55=AAPL|54=1|44=100|38=5|10=0|",
                )
                .unwrap()),
            Err(Error::InvalidRecord {
                path: "$.intake".into(),
                reason: "source failed between two messages".into(),
            }),
            Ok(codec
                .sole_line(
                    b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C2|55=AAPL|54=2|44=101|38=6|10=0|",
                )
                .unwrap()),
        ]
    };
    let mut operations = codec.market_operations(source());
    let error = operations
        .next()
        .expect("the failure")
        .expect_err("the source's failure leads")
        .to_string();
    assert!(error.contains("$.intake"), "{error}");
    let leaves = drained(operations.by_ref()).expect("then every operation");
    assert_eq!(leaves.len(), 2);
    assert_eq!(leaves[0].get_crosscode(), "C2", "sorted by instant");
    assert_eq!(leaves[1].get_crosscode(), "C1");
    assert!(operations.next().is_none(), "fused");

    // The writer meets the failure before any row, yields it and stops: one
    // intake error is the whole answer.
    let mut reader = codec.market_arrow_reader(source()).expect("a reader");
    let error = reader
        .next()
        .expect("the failure")
        .expect_err("no row precedes it")
        .to_string();
    assert!(error.contains("$.intake"), "{error}");
    assert!(reader.next().is_none(), "and nothing follows it");
}

#[test]
fn a_message_that_cannot_expand_is_an_intake_refusal_that_drops_only_itself() {
    let codec = fixed_codec(committed_registry());
    let capture = messages(
        &codec,
        &[
            b"8=FIX.4.4|35=D|52=20260921-10:00:01|11=C1|55=AAPL|54=1|44=100|38=5|10=0|",
            b"8=FIX.4.4|35=W|52=20260921-10:00:02|55=AAPL|10=0|",
            b"8=FIX.4.4|35=S|52=20260921-10:00:00|117=Q1|55=AAPL|54=2|133=101|135=7|10=0|",
        ],
    );
    let mut operations = codec.market_operations(capture);
    let error = operations
        .next()
        .expect("the refusal")
        .expect_err("a W with no entries group refuses")
        .to_string();
    assert!(error.contains("NoMDEntries(268)"), "{error}");
    let leaves = drained(operations).expect("the other messages stand");
    assert_eq!(
        leaves.iter().map(MarketData::kind).collect::<Vec<_>>(),
        [MarketKind::QuoteEvent, MarketKind::OrderEvent]
    );
}

/// Five messages, one of each dated leaf a capture expands into, in time
/// order.
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
    let expected = drained(codec.market_operations(capture.clone())).expect("the leaves");
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
            "execution_event",
            "trade_event",
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
    let direct = drained(codec.market_operations(capture.clone())).expect("the leaves");
    let rows = codec
        .arrow_reader(
            yggdryl::fix_schema(&registry, "fix").expect("the fixed row"),
            capture,
        )
        .expect("the FIX rows");
    let twin = codec
        .market_operations_arrow_reader(rows)
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
        .market_operations_arrow_reader(foreign())
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

/// An order stating four fields no typed column reads - `OrdType(40)`,
/// `ExecInst(18)`, `HandlInst(21)` and `DisplayQty(111)`, FIX 4.4's
/// `MaxFloor` - beside the header, the trailer and the typed facts.
const UNMAPPED_ORDER: &[u8] = b"8=FIX.4.4|9=120|35=D|49=BUYER|56=VENUE|34=12|52=20260921-10:00:00|11=C1|1=ACC1|55=AAPL|54=1|44=100.5|38=5|40=2|18=G|21=1|111=3|60=20260921-10:00:00|10=123|";

#[test]
fn a_leaf_carries_every_unmapped_field_and_no_typed_one() {
    let leaves = message(UNMAPPED_ORDER)
        .into_market_operations()
        .expect("an order");
    let [leaf] = leaves.as_slice() else {
        panic!("one order")
    };
    assert_eq!(
        metadata(leaf),
        [
            // A quantity spells its decimal at the scale it is stored at.
            ("displayqty", "3.000000000000000000"),
            ("execinst", "G"),
            ("handlinst", "1"),
            ("ordtype", "2"),
        ]
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
    );
    for typed in [
        "symbol",
        "side",
        "price",
        "orderqty",
        "clordid",
        "account",
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
    assert_eq!(order.get_accountids().get("ACCOUNT"), Some("ACC1"));
    assert_eq!(text(order.get_price()).as_deref(), Some("100.5"));
}

#[test]
fn a_group_occurrence_is_keyed_by_its_path_and_a_mapped_party_is_consumed() {
    let leaves = message(
        b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|38=5|453=2|448=TRADER1|447=D|452=11|448=ACC9|447=D|452=24|10=0|",
    )
    .into_market_operations()
    .expect("an order");
    let [leaf] = leaves.as_slice() else {
        panic!("one order")
    };
    // Role 11 is no identifier map's, so its occurrence lands member by
    // member under its path, at its own position.
    assert_eq!(
        metadata(leaf),
        [
            ("parties[0].partyid", "TRADER1"),
            ("parties[0].partyidsource", "D"),
            ("parties[0].partyrole", "11"),
        ]
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
    );
    // Role 24 is the customer account's: the occurrence landed there, whole.
    assert_eq!(
        operation_of(leaf).get_accountids().get("CUSTOMERACCOUNT"),
        Some("ACC9")
    );
    assert!(!keys(leaf).iter().any(|key| key.starts_with("parties[1]")));
    assert!(!keys(leaf).iter().any(|key| key == "nopartyids"));
}

#[test]
fn an_expanded_entry_carries_the_message_level_map_and_its_own_never_a_siblings() {
    let leaves = message(
        b"8=FIX.4.4|35=X|52=20260921-10:00:00|55=AAPL|1180=MDP|268=2|279=0|269=0|278=B1|270=100|271=10|83=7|279=0|269=1|278=A1|270=101|271=11|83=8|10=0|",
    )
    .into_market_operations()
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
    .into_market_operations()
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
fn a_trade_side_keeps_its_own_members_bare_and_the_order_independence_pins_hold() {
    let read = |line: &[u8]| {
        let leaves = message(line).into_market_operations().expect("a trade");
        let [MarketData::TradeEvent(trade)] = leaves.as_slice() else {
            panic!("one composite trade")
        };
        trade.clone()
    };
    let first = read(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|487=0|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|54=1|1427=BUY-EXEC|1009=4|528=A|54=2|1427=SELL-EXEC|1009=6|528=P|10=0|",
    );
    let second = read(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|487=0|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|54=2|1427=SELL-EXEC|1009=6|528=P|54=1|1427=BUY-EXEC|1009=4|528=A|10=0|",
    );
    assert_eq!(first, second, "the side order changes nothing");
    assert_eq!(first.get_curruuid(), second.get_curruuid());

    let [buy, sell] = first.executions() else {
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
    // The trade carries the message's own - `TradeReportID(571)`, which no
    // column reads, and not `TradeReportTransType(487)`, which says the
    // report executed; each side adds its own members, bare, and never its
    // sibling's. `OrderCapacity(528)` sits in the side's
    // `TradeReportOrderDetail` component, so its path opens there.
    assert_eq!(of(first.get_metadata()), owned(&[("tradereportid", "T1")]));
    assert_eq!(
        of(buy.get_metadata()),
        owned(&[
            ("tradereportid", "T1"),
            ("tradereportorderdetail.ordercapacity", "A")
        ])
    );
    assert_eq!(
        of(sell.get_metadata()),
        owned(&[
            ("tradereportid", "T1"),
            ("tradereportorderdetail.ordercapacity", "P")
        ])
    );
}

#[test]
fn the_option_turns_the_fill_off_and_the_identity_says_so() {
    let filled = fixed_codec(committed_registry());
    let bare = fixed_codec(committed_registry()).with_market_metadata(false);
    assert!(filled.market_metadata());
    assert!(!bare.market_metadata());
    let read = |codec: &yggdryl::FixCodec| {
        let capture = messages(codec, &[UNMAPPED_ORDER]);
        drained(codec.market_operations(capture)).expect("an order")
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
        drained(codec.market_operations(capture)).expect("an order")
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
        let books = books_of(
            codec
                .book_arrow_reader(capture, 0, false)
                .expect("a book reader"),
        );
        let [book] = books.as_slice() else {
            panic!("one book")
        };
        book.bid().live().next().expect("the order rests").clone()
    };
    let filled = live(fixed_codec(committed_registry()));
    let bare = live(fixed_codec(committed_registry()).with_market_metadata(false));
    assert_eq!(filled.get_metadata().len(), 4);
    assert!(bare.get_metadata().is_empty());
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
            b"8=FIX.4.4|35=D|34=7|52=20260921-10:00:00|65032=SESSION|65008=CONTEXT|11=C1|55=AAPL|54=1|44=100|38=5|21=1|18=G|10=0|",
            b"8=FIX.4.4|35=D|34=7|52=20260921-10:00:01|65032=SESSION|65008=CONTEXT|11=C1|55=AAPL|54=1|44=100|38=5|21=2|111=3|10=0|",
        ],
    );
    let walked = codec
        .lifecycle(capture)
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("the walk");
    assert_eq!(walked.len(), 1, "one event, observed twice");
    let leaves = drained(codec.market_operations(walked)).expect("the merged order");
    let [leaf] = leaves.as_slice() else {
        panic!("one order")
    };
    assert_eq!(
        metadata(leaf),
        [
            ("displayqty", "3.000000000000000000"),
            ("execinst", "G"),
            ("handlinst", "2"),
        ]
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
            b"8=FIX.4.4|35=D|52=20260921-10:00:02|11=C2|55=AAPL|54=1|38=5|453=1|448=TRADER1|447=D|452=11|10=0|",
        ],
    );
    let expected = drained(codec.market_operations(capture.clone())).expect("the leaves");
    assert!(expected.iter().all(|leaf| !leaf.get_metadata().is_empty()));
    let actual = drained(
        MarketData::from_arrow_reader(codec.market_arrow_reader(capture).expect("a reader"))
            .expect("the rows read back"),
    )
    .expect("every row");
    assert_eq!(actual, expected);
}
