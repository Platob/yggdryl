//! `rust/src/fix/market.rs`: the typed FIX boundary into graph operations.

use std::sync::Arc;

use super::{SoleMessage, committed_registry, fixed_codec};
use yggdryl::graph::book::{ENTRY_ID, ENTRY_REF_ID};
use yggdryl::graph::{
    Book, BookInput, Element, Event, Market, MarketOperation, MdUpdateAction, Operation,
    OperationKind,
};
use yggdryl::{DataType, Error, Field, FixCode, FixMsg, FixRegistry, Scalar, StructType};

fn message(line: &[u8]) -> FixMsg {
    fixed_codec(committed_registry())
        .sole_line(line)
        .expect("one FIX message")
}

/// The operation an input is; a trade or a control is a fixture mistake.
fn operation_of(input: &BookInput) -> &Operation {
    match input {
        BookInput::Operation(operation) => operation,
        other => panic!("expected an operation, got {other:?}"),
    }
}

/// The book scope an input states.
fn scope_of(input: &BookInput) -> &str {
    input
        .book()
        .and_then(|book| book.scope.as_deref())
        .expect("a scoped entry")
}

#[test]
fn direct_market_categories_move_into_their_operation_kind_and_arrow_round_trip() {
    let cases: &[(&[u8], i32, OperationKind)] = &[
        (
            b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|44=100|38=5|10=0|",
            10,
            OperationKind::Order,
        ),
        (
            b"8=FIX.4.4|35=S|117=Q1|55=AAPL|132=99|134=7|133=101|135=8|10=0|",
            14,
            OperationKind::Quote,
        ),
        (
            b"8=FIX.4.4|35=8|17=E1|37=O1|55=AAPL|31=100|32=2|150=F|10=0|",
            8,
            OperationKind::Execution,
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
        assert_eq!(operation.kind(), *kind, "{line:?}");
        assert_eq!(operation.get_marketoperationid(), Some(*operation_id));
        assert_eq!(operation.book(), None, "a direct message is no book entry");
        let expected = operations[0].clone();
        let encoded = BookInput::arrow_reader(operations, Some(1), None).unwrap();
        let actual = BookInput::from_arrow_reader(encoded)
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

    let [BookInput::Trade(trade)] = operations.as_slice() else {
        panic!("one composite trade")
    };
    assert_eq!(trade.get_marketoperationid(), Some(21));
    assert_eq!(trade.get_price().to_string(), "101.25");
    assert_eq!(trade.get_quantity().to_string(), "10");

    let [buy, sell] = trade.executions() else {
        panic!("one execution per stated trade side")
    };
    assert!(buy.get_side().is_bid());
    assert!(sell.get_side().is_ask());
    assert_eq!(buy.kind(), OperationKind::Execution);
    assert_eq!(buy.get_price().to_string(), "101.25");
    assert_eq!(sell.get_price().to_string(), "101.25");
    assert_eq!(buy.get_quantity().to_string(), "4");
    assert_eq!(sell.get_quantity().to_string(), "6");
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
    let encoded = BookInput::arrow_reader(operations, Some(1), None).unwrap();
    let actual = BookInput::from_arrow_reader(encoded)
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
    let [BookInput::Trade(trade)] = operations.as_slice() else {
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

    let ([BookInput::Trade(first)], [BookInput::Trade(second)]) =
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
    let ([BookInput::Trade(first)], [BookInput::Trade(second)]) =
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
    let [BookInput::Trade(tagged)] = tagged.as_slice() else {
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
    let books = Book::from_arrow_reader(reader)
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();

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
        [BookInput::Operation(operation)] if operation.kind() == OperationKind::Quote
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
    let books = Book::from_arrow_reader(reader)
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();

    assert_eq!(books.len(), 2);
    assert_eq!(books[0].bid().best_price().unwrap().to_string(), "100");
    assert_eq!(books[0].ask().best_price().unwrap().to_string(), "102");
    assert_eq!(books[0].get_price().to_string(), "101");
    assert_eq!(books[1].bid().best_price().unwrap().to_string(), "101");
    assert_eq!(books[1].ask().best_price().unwrap().to_string(), "102");
    assert_eq!(books[1].get_price().to_string(), "101.5");
    assert_eq!(books[1].executions().len(), 1);
    assert_eq!(
        books[1]
            .bid()
            .live()
            .next()
            .unwrap()
            .get_marketoperationid(),
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
    let expected = Book::from_arrow_reader(codec.book_arrow_reader(admitted, 0, false).unwrap())
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(expected.len(), 6);
    assert_eq!(expected[0].bid().deltas()[0].kind(), OperationKind::Order);
    assert_eq!(expected[1].ask().deltas()[0].kind(), OperationKind::Quote);
    assert_eq!(expected[2].executions().len(), 1);
    assert_eq!(expected[5].executions().len(), 1);
    let actual = Book::from_arrow_reader(codec.book_arrow_reader(mixed, 0, false).unwrap())
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(actual, expected);

    let mut empty = codec.book_arrow_reader(ignored.clone(), 0, false).unwrap();
    assert!(empty.next().is_none());
    for source in ignored {
        assert!(BookInput::try_from(source.clone()).is_err());
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
    let books = Book::from_arrow_reader(reader)
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();

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
    for (index, (book, operation)) in books.iter().zip(versions).enumerate() {
        assert_eq!(operation.kind(), OperationKind::Order);
        assert_eq!(book.get_ticker(), Some("AAPL"));
        assert_eq!(operation.get_ticker(), Some("AAPL"));
        assert_eq!(operation.get_altids().get(ENTRY_ID), Some("B1"));
        assert_eq!(operation.get_altids().get("ORDERID"), Some("O1"));
        assert_eq!(operation.get_seqnum(), index as u64);
        assert_eq!(
            operation.get_price().to_string(),
            ["100", "100", "101"][index]
        );
        assert_eq!(
            operation.get_quantity().to_string(),
            ["10", "11", "11"][index]
        );
        if index < 2 {
            assert!(operation.get_side().is_bid());
            let bid = operation.get_bid().expect("the bid lane");
            assert_eq!(bid.price, Some(operation.get_price()));
            assert_eq!(bid.quantity, Some(operation.get_quantity()));
            assert_eq!(operation.get_ask(), None);
        } else {
            assert!(operation.get_side().is_ask());
            let ask = operation.get_ask().expect("the ask lane");
            assert_eq!(ask.price, Some(operation.get_price()));
            assert_eq!(ask.quantity, Some(operation.get_quantity()));
            assert_eq!(operation.get_bid(), None);
        }
    }
    for pair in versions.windows(2) {
        assert_eq!(pair[1].get_prevuuid(), Some(pair[0].get_curruuid()));
        assert_eq!(pair[1].get_prevunix(), Some(pair[0].get_currunix()));
        assert_eq!(pair[1].get_prevpx(), Some(pair[0].get_price()));
        assert_eq!(pair[1].get_prevqty(), Some(pair[0].get_quantity()));
    }
    assert_eq!(versions[0].get_prevuuid(), None);
    // The book control each entry stated rides with it, typed: what the
    // partial update said, and only that.
    assert_eq!(
        versions[0].action(),
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
    let messages = [
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=1|269=0|278=B1|37=O1|270=100|271=10|10=0|".as_slice(),
        b"8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=1|279=2|269=0|278=B1|10=0|".as_slice(),
    ]
    .map(|line| codec.sole_line(line).unwrap());
    let reader = codec.book_arrow_reader(messages, 0, false).unwrap();
    let books = Book::from_arrow_reader(reader)
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();

    assert_eq!(books.len(), 2);
    let previous = books[0].bid().live().next().unwrap();
    assert!(books[1].bid().is_empty());
    assert!(books[1].ask().is_empty());
    let [deleted] = books[1].bid().deltas() else {
        panic!("one terminal bid delta")
    };
    assert_eq!(deleted.kind(), OperationKind::Order);
    assert!(!deleted.get_state().is_live());
    assert!(deleted.get_side().is_bid());
    assert_eq!(deleted.get_ticker(), Some("AAPL"));
    assert_eq!(deleted.get_altids().get(ENTRY_ID), Some("B1"));
    assert_eq!(deleted.action(), Some(MdUpdateAction::Delete));
    assert_eq!(deleted.get_altids().get("ORDERID"), Some("O1"));
    assert_eq!(deleted.get_prevuuid(), Some(previous.get_curruuid()));
    assert_eq!(deleted.get_prevunix(), Some(previous.get_currunix()));
    assert_eq!(deleted.get_seqnum(), previous.get_seqnum() + 1);
    assert_eq!(deleted.get_prevpx(), Some(previous.get_price()));
    assert_eq!(deleted.get_prevqty(), Some(previous.get_quantity()));
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
    let books = Book::from_arrow_reader(reader)
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(books.len(), 2);
    for (index, book) in books.iter().enumerate() {
        assert_eq!(book.get_ticker(), Some("AAPL"));
        assert_eq!(book.bid().len(), 1);
        assert!(book.ask().is_empty());
        let operation = book.bid().live().next().unwrap();
        assert_eq!(operation.kind(), OperationKind::Order);
        assert_eq!(operation.get_ticker(), Some("AAPL"));
        assert_eq!(operation.get_price().to_string(), ["100", "101"][index]);
        assert_eq!(operation.get_quantity().to_string(), ["5", "6"][index]);
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
    let operations: Vec<&Operation> = inputs.iter().map(operation_of).collect();
    assert_eq!(operations[0].kind(), OperationKind::Quote);
    assert_eq!(operations[1].kind(), OperationKind::Order);
    assert_eq!(operations[2].kind(), OperationKind::Execution);
    assert!(operations[0].get_side().is_bid());
    assert!(operations[1].get_side().is_ask());
    assert!(operations[0].get_crosscode().ends_with("|MDEntryID=B1"));
    assert!(operations[1].get_crosscode().ends_with("|MDEntryID=A1"));
    assert!(operations[2].get_crosscode().ends_with("|MDEntryID=T1"));
    assert_eq!(operations[0].get_price().to_string(), "100");
    assert_eq!(operations[1].get_quantity().to_string(), "12");
    assert_eq!(operations[0].get_ticker(), Some("AAPL"));
    assert_eq!(operations[1].get_altids().get("ORDERID"), Some("O1"));
    assert_eq!(
        operations[1].get_altids().get("MDREQID"),
        Some("REQ-1"),
        "the request the message answers names every entry"
    );
    for (index, operation) in operations.iter().enumerate() {
        let book = operation
            .book()
            .expect("a full refresh entry is a book entry");
        assert_eq!(book.action, Some(MdUpdateAction::Snapshot));
        assert!(operation.is_full_snapshot());
        assert_eq!(
            book.position,
            (index < 2).then_some(1),
            "the trade states no position"
        );
        assert!(operation.scope().contains("Symbol=AAPL"));
        assert!(operation.scope().contains("MDReqID=REQ-1"));
        assert!(operation.get_state().is_live());
    }
    assert_eq!(
        operations[0].book().and_then(|book| book.entry_px),
        Some("100".parse().unwrap())
    );
    assert_eq!(
        operations[0].book().and_then(|book| book.entry_size),
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
    let mut book = Book::new(initial[0].currunix(), "AAPL");
    book.add_operations(initial).unwrap();

    let empty_other_request =
        message(b"8=FIX.4.4|35=W|52=20260921-10:00:01|55=AAPL|262=REQ-2|268=0|10=0|")
            .into_market_operations()
            .unwrap();
    assert!(scope_of(&empty_other_request[0]).contains("MDReqID=REQ-2"));
    assert!(empty_other_request[0].is_full_snapshot());
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

    let input = BookInput::try_from(source).unwrap();
    let operation = operation_of(&input);
    let altids = operation.get_altids();
    assert_eq!(altids.get(ENTRY_ID), Some("B1"));
    assert_eq!(altids.get(ENTRY_REF_ID), None);
    assert_eq!(altids.get("MDREQID"), Some("REQ-9"));
    assert_eq!(altids.get("foreign"), None);
    let book = operation.book().expect("an entry states its control");
    assert_eq!(book.action, Some(MdUpdateAction::Change));
    assert_eq!(
        book.entry_size.map(|held| held.to_string()),
        Some("11".to_owned())
    );
    assert_eq!(book.entry_px, None, "the entry restated no price");
    assert!(operation.scope().contains("MDReqID=REQ-9"));
}

#[test]
fn incremental_actions_keep_the_wire_action_and_terminal_delete_state() {
    let inputs = message(
        b"8=FIX.4.4|35=X|83=7|1181=43|268=3|279=0|269=0|278=B1|55=AAPL|270=100|271=10|290=1|279=1|269=0|278=B1|55=AAPL|270=101|271=11|290=1|279=2|269=0|280=B1|55=AAPL|10=0|",
    )
    .into_market_operations()
    .expect("incremental operations");

    assert_eq!(inputs.len(), 3);
    let operations: Vec<&Operation> = inputs.iter().map(operation_of).collect();
    assert_eq!(operations[0].action(), Some(MdUpdateAction::New));
    assert_eq!(operations[1].action(), Some(MdUpdateAction::Change));
    assert_eq!(operations[2].action(), Some(MdUpdateAction::Delete));
    assert!(
        operations
            .iter()
            .all(|operation| !operation.is_full_snapshot())
    );
    assert!(operations[0].get_state().is_live());
    assert!(operations[1].get_state().is_live());
    assert!(!operations[2].get_state().is_live());
    assert_eq!(operations[2].get_crosscode(), operations[0].get_crosscode());
    assert_eq!(operations[0].get_altids().get(ENTRY_ID), Some("B1"));
    assert_eq!(operations[2].get_altids().get(ENTRY_ID), None);
    assert_eq!(operations[2].get_altids().get(ENTRY_REF_ID), Some("B1"));
    assert_eq!(operations[1].book().and_then(|book| book.position), Some(1));
}

#[test]
fn fallback_identity_is_scoped_typed_and_stable_across_price_changes() {
    let input = BookInput::try_from(message(
        b"8=FIX.4.4|35=X|1301=XNAS|1300=NASDAQ|268=1|279=0|269=1|55=AAPL|1023=2|290=3|270=101.25|271=4|10=0|",
    ))
    .expect("one operation");
    let code = input.event().get_crosscode();
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
    let error = BookInput::try_from(message(
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
    assert_eq!(operations[0].currunix(), 1_789_896_600_123_456_789);
    assert_eq!(operations[1].currunix(), 1_789_896_600_223_456_789);
    assert_eq!(
        operations[1].event().get_execunix(),
        Some(1_789_896_600_223_456_789)
    );

    let snapshot = BookInput::try_from(message(
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=1|269=0|278=B1|270=100|271=10|272=20260920|273=09:30:00.123456789|10=0|",
    ))
    .expect("one full-snapshot entry");
    assert_eq!(snapshot.currunix(), 1_789_984_800_000_000_000);
    assert_eq!(
        snapshot.event().get_creaunix(),
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
                .all(|pair| { pair[0].currunix() <= pair[1].currunix() })
        );
    }
}

#[test]
fn incremental_changes_inherit_price_or_size_the_fix_entry_did_not_restate() {
    let snapshot = message(b"8=FIX.4.4|35=W|55=AAPL|268=1|269=0|278=B1|270=100|271=10|10=0|")
        .into_market_operations()
        .unwrap();
    let mut book = Book::new(snapshot[0].currunix(), "AAPL");
    book.add_operations(snapshot).unwrap();

    let size_only = message(b"8=FIX.4.4|35=X|55=AAPL|268=1|279=1|269=0|278=B1|271=11|10=0|")
        .into_market_operations()
        .unwrap();
    book.add_operations(size_only).unwrap();
    let live = book.bid().live().next().unwrap();
    assert_eq!(live.get_price().to_string(), "100");
    assert_eq!(live.get_quantity().to_string(), "11");

    let price_only = message(b"8=FIX.4.4|35=X|55=AAPL|268=1|279=5|269=0|278=B1|270=101|10=0|")
        .into_market_operations()
        .unwrap();
    book.add_operations(price_only).unwrap();
    let live = book.bid().live().next().unwrap();
    assert_eq!(live.get_price().to_string(), "101");
    assert_eq!(live.get_quantity().to_string(), "11");
}

#[test]
fn anonymous_incremental_changes_use_stable_position_identity_or_refuse_ambiguity() {
    let snapshot = message(
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=1|269=0|290=1|270=100|271=10|10=0|",
    )
    .into_market_operations()
    .unwrap();
    let identity = snapshot[0].event().get_crosscode().to_owned();
    let mut book = Book::new(snapshot[0].currunix(), "AAPL");
    book.add_operations(snapshot).unwrap();

    let change = message(
        b"8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=1|279=1|269=0|290=1|271=11|10=0|",
    )
    .into_market_operations()
    .unwrap();
    assert_eq!(change[0].event().get_crosscode(), identity);
    book.add_operations(change).unwrap();
    assert_eq!(book.bid().len(), 1);
    assert_eq!(
        book.bid().live().next().unwrap().get_price().to_string(),
        "100"
    );
    assert_eq!(
        book.bid().live().next().unwrap().get_quantity().to_string(),
        "11"
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
    let mut book = Book::new(initial[0].currunix(), "AAPL");
    book.add_operations(initial).unwrap();
    assert_eq!(book.bid().len(), 1);

    let empty = message(b"8=FIX.4.4|35=W|52=20260921-10:00:01|55=AAPL|268=0|10=0|")
        .into_market_operations()
        .unwrap();
    assert!(matches!(empty.as_slice(), [BookInput::Snapshot(_)]));
    assert!(empty[0].is_full_snapshot());
    assert_eq!(empty[0].event().get_ticker(), Some("AAPL"));
    book.add_operations(empty).unwrap();
    assert!(book.bid().is_empty());
    assert!(book.ask().is_empty());
}

#[test]
fn book_scope_escapes_external_delimiters_injectively() {
    let input = BookInput::try_from(message(
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
