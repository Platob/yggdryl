//! `rust/src/fix/market.rs`: the typed FIX boundary into graph operations.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::{SoleMessage, committed_registry, fixed_codec};
use yggdryl::graph::{Book, Element, Event, MarketElement, MarketOperation};
use yggdryl::{DataType, Error, Field, FixMsg, FixRegistry, Scalar, StructType};

fn message(line: &[u8]) -> FixMsg {
    fixed_codec(committed_registry())
        .sole_line(line)
        .expect("one FIX message")
}

#[test]
fn direct_market_categories_move_into_their_operation_kind_and_arrow_round_trip() {
    type Case = (&'static [u8], fn(&MarketOperation) -> bool);
    let cases: &[Case] = &[
        (
            b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|44=100|38=5|10=0|",
            |operation| matches!(operation, MarketOperation::Order(_)),
        ),
        (
            b"8=FIX.4.4|35=S|117=Q1|55=AAPL|132=99|134=7|133=101|135=8|10=0|",
            |operation| matches!(operation, MarketOperation::Quote(_)),
        ),
        (
            b"8=FIX.4.4|35=8|17=E1|37=O1|55=AAPL|31=100|32=2|150=F|10=0|",
            |operation| matches!(operation, MarketOperation::Execution(_)),
        ),
    ];

    for (line, expected) in cases {
        let source = message(line);
        let operations = source.into_market_operations().expect("a market category");
        assert_eq!(operations.len(), 1);
        assert!(expected(&operations[0]), "{line:?}");
        let expected = operations[0].clone();
        let encoded = MarketOperation::arrow_reader(operations, Some(1), None).unwrap();
        let actual = MarketOperation::from_arrow_reader(encoded)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(actual, expected);
    }
}

#[test]
fn full_refresh_expands_entries_in_source_order_and_types_each_one() {
    let operations = message(
        b"8=FIX.4.4|35=W|55=AAPL|262=REQ-1|1021=2|1180=MDP|1181=42|268=3|269=0|278=B1|270=100|271=10|290=1|269=1|278=A1|37=O1|270=101|271=12|290=1|269=2|278=T1|270=100.5|271=2|10=0|",
    )
    .market_operations()
    .expect("a full refresh");

    assert_eq!(operations.len(), 3);
    assert!(matches!(operations[0], MarketOperation::Quote(_)));
    assert!(matches!(operations[1], MarketOperation::Order(_)));
    assert!(matches!(operations[2], MarketOperation::Execution(_)));
    assert!(operations[0].get_side().is_bid());
    assert!(operations[1].get_side().is_ask());
    assert!(operations[0].get_crosscode().ends_with("|MDEntryID=B1"));
    assert!(operations[1].get_crosscode().ends_with("|MDEntryID=A1"));
    assert!(operations[2].get_crosscode().ends_with("|MDEntryID=T1"));
    assert_eq!(operations[0].get_px().to_string(), "100");
    assert_eq!(operations[1].get_qty().to_string(), "12");
    assert_eq!(operations[0].get_symbolticker(), Some("AAPL"));
    for operation in &operations {
        assert_eq!(operation.get_identifiers()["MDUpdateAction"], "SNAPSHOT");
        assert_eq!(operation.get_identifiers()["ApplID"], "MDP");
        assert_eq!(operation.get_identifiers()["ApplSeqNum"], "42");
        assert!(operation.get_identifiers()["BookScope"].contains("Symbol=AAPL"));
        assert!(operation.get_identifiers()["BookScope"].contains("MDReqID=REQ-1"));
        assert!(operation.get_state().is_live());
    }
}

#[test]
fn lifted_request_id_keeps_full_snapshot_partitions_distinct() {
    let initial = message(
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|262=REQ-1|268=1|269=0|278=B1|270=100|271=10|10=0|",
    )
    .into_market_operations()
    .unwrap();
    assert!(initial[0].get_identifiers()["BookScope"].contains("MDReqID=REQ-1"));
    let mut book = Book::new(initial[0].get_currunix(), "AAPL");
    book.add_operations(initial).unwrap();

    let empty_other_request =
        message(b"8=FIX.4.4|35=W|52=20260921-10:00:01|55=AAPL|262=REQ-2|268=0|10=0|")
            .into_market_operations()
            .unwrap();
    assert!(empty_other_request[0].get_identifiers()["BookScope"].contains("MDReqID=REQ-2"));
    book.add_operations(empty_other_request).unwrap();
    assert_eq!(book.bid().len(), 1);
}

#[test]
fn occurrence_identifiers_replace_stale_protocol_keys_and_keep_foreign_keys() {
    let mut source = message(b"8=FIX.4.4|35=X|55=AAPL|268=1|279=1|269=0|278=B1|271=11|10=0|");
    source.set_identifiers(BTreeMap::from([
        ("foreign".to_owned(), "kept".to_owned()),
        ("MDEntryRefID".to_owned(), "STALE".to_owned()),
        ("MDEntryPx".to_owned(), "999".to_owned()),
        ("MDPriceLevel".to_owned(), "88".to_owned()),
        ("ApplSeqNum".to_owned(), "77".to_owned()),
    ]));

    let operation = MarketOperation::try_from(source).unwrap();
    let identifiers = operation.get_identifiers();
    assert_eq!(identifiers.get("foreign").map(String::as_str), Some("kept"));
    assert_eq!(identifiers.get("MDEntryID").map(String::as_str), Some("B1"));
    assert_eq!(
        identifiers.get("MDEntrySize").map(String::as_str),
        Some("11")
    );
    for absent in ["MDEntryRefID", "MDEntryPx", "MDPriceLevel", "ApplSeqNum"] {
        assert!(!identifiers.contains_key(absent), "stale {absent} survived");
    }
}

#[test]
fn incremental_actions_keep_the_wire_action_and_terminal_delete_state() {
    let operations = message(
        b"8=FIX.4.4|35=X|83=7|1181=43|268=3|279=0|269=0|278=B1|55=AAPL|270=100|271=10|290=1|279=1|269=0|278=B1|55=AAPL|270=101|271=11|290=1|279=2|269=0|280=B1|55=AAPL|10=0|",
    )
    .into_market_operations()
    .expect("incremental operations");

    assert_eq!(operations.len(), 3);
    assert_eq!(operations[0].get_identifiers()["MDUpdateAction"], "0");
    assert_eq!(operations[1].get_identifiers()["MDUpdateAction"], "1");
    assert_eq!(operations[2].get_identifiers()["MDUpdateAction"], "2");
    assert!(operations[0].get_state().is_live());
    assert!(operations[1].get_state().is_live());
    assert!(!operations[2].get_state().is_live());
    assert_eq!(operations[2].get_crosscode(), operations[0].get_crosscode());
    assert_eq!(operations[0].get_identifiers()["RptSeq"], "7");
    assert_eq!(operations[0].get_identifiers()["ApplSeqNum"], "43");
}

#[test]
fn fallback_identity_is_scoped_typed_and_stable_across_price_changes() {
    let operation = MarketOperation::try_from(message(
        b"8=FIX.4.4|35=X|1301=XNAS|1300=NASDAQ|268=1|279=0|269=1|55=AAPL|1023=2|290=3|270=101.25|271=4|10=0|",
    ))
    .expect("one operation");
    let code = operation.get_crosscode();
    assert!(code.contains("Symbol=AAPL"), "{code}");
    assert!(code.contains("MarketID=XNAS"), "{code}");
    assert!(code.contains("MDEntryType=1"), "{code}");
    assert!(code.contains("MDEntryPositionNo=3"), "{code}");
    assert!(code.contains("MDPriceLevel=2"), "{code}");
    assert!(!code.contains("MDEntryPx="), "{code}");
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
    let error = MarketOperation::try_from(message(
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
    assert_eq!(operations[0].get_currunix(), 1_789_896_600_123_456_789);
    assert_eq!(operations[1].get_currunix(), 1_789_896_600_223_456_789);
    assert_eq!(
        operations[1].get_execunix(),
        Some(1_789_896_600_223_456_789)
    );

    let snapshot = MarketOperation::try_from(message(
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=1|269=0|278=B1|270=100|271=10|272=20260920|273=09:30:00.123456789|10=0|",
    ))
    .expect("one full-snapshot entry");
    assert_eq!(snapshot.get_currunix(), 1_789_984_800_000_000_000);
    assert_eq!(snapshot.get_creaunix(), Some(1_789_896_600_123_456_789));
}

#[test]
fn incremental_changes_inherit_price_or_size_the_fix_entry_did_not_restate() {
    let snapshot = message(b"8=FIX.4.4|35=W|55=AAPL|268=1|269=0|278=B1|270=100|271=10|10=0|")
        .into_market_operations()
        .unwrap();
    let mut book = Book::new(snapshot[0].get_currunix(), "AAPL");
    book.add_operations(snapshot).unwrap();

    let size_only = message(b"8=FIX.4.4|35=X|55=AAPL|268=1|279=1|269=0|278=B1|271=11|10=0|")
        .into_market_operations()
        .unwrap();
    book.add_operations(size_only).unwrap();
    let live = book.bid().live().next().unwrap();
    assert_eq!(live.get_px().to_string(), "100");
    assert_eq!(live.get_qty().to_string(), "11");

    let price_only = message(b"8=FIX.4.4|35=X|55=AAPL|268=1|279=5|269=0|278=B1|270=101|10=0|")
        .into_market_operations()
        .unwrap();
    book.add_operations(price_only).unwrap();
    let live = book.bid().live().next().unwrap();
    assert_eq!(live.get_px().to_string(), "101");
    assert_eq!(live.get_qty().to_string(), "11");
}

#[test]
fn anonymous_incremental_changes_use_stable_position_identity_or_refuse_ambiguity() {
    let snapshot = message(
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=1|269=0|290=1|270=100|271=10|10=0|",
    )
    .into_market_operations()
    .unwrap();
    let identity = snapshot[0].get_crosscode().to_owned();
    let mut book = Book::new(snapshot[0].get_currunix(), "AAPL");
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
        book.bid().live().next().unwrap().get_px().to_string(),
        "100"
    );
    assert_eq!(
        book.bid().live().next().unwrap().get_qty().to_string(),
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
    let mut book = Book::new(initial[0].get_currunix(), "AAPL");
    book.add_operations(initial).unwrap();
    assert_eq!(book.bid().len(), 1);

    let empty = message(b"8=FIX.4.4|35=W|52=20260921-10:00:01|55=AAPL|268=0|10=0|")
        .into_market_operations()
        .unwrap();
    assert!(matches!(empty.as_slice(), [MarketOperation::Snapshot(_)]));
    book.add_operations(empty).unwrap();
    assert!(book.bid().is_empty());
    assert!(book.ask().is_empty());
}

#[test]
fn book_scope_escapes_external_delimiters_injectively() {
    let operation = MarketOperation::try_from(message(
        b"8=FIX.4.4|35=W|55=A=B%X|268=1|269=0|278=B1|270=100|271=1|10=0|",
    ))
    .unwrap();
    assert!(operation.get_identifiers()["BookScope"].contains("Symbol=A%3DB%25X"));
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
    let mut entries = DataType::list(item).required_field("MDEntries");
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
