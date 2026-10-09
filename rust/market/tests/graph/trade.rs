//! `rust/market/src/graph/trade.rs`: a canonical composite trade and its executions.

use smol_str::SmolStr;
use yggdryl::Decimal;
use yggdryl::graph::{Element, Event};
use yggdryl_market::Side;
use yggdryl_market::graph::{
    ExecutionEvent, Market, MarketData, MarketKind, OperationEvent, OperationKind, OrderEvent,
    TradeEvent,
};

fn event<K: OperationKind>(
    unix: i64,
    crosscode: &str,
    symbol: Option<&str>,
    sendunix: Option<i64>,
) -> OperationEvent<K> {
    let mut event = OperationEvent::<K>::at(unix);
    event.set_crosscode(crosscode.to_owned());
    event.set_ticker(symbol.map(SmolStr::new), true);
    event.set_sendunix(sendunix);
    event.finalize();
    event
}

/// The trade's root: the facts it states of its own, an order's.
fn root(unix: i64, crosscode: &str, symbol: Option<&str>, sendunix: Option<i64>) -> OrderEvent {
    event(unix, crosscode, symbol, sendunix)
}

#[allow(clippy::too_many_arguments)]
fn execution(
    unix: i64,
    crosscode: &str,
    symbol: Option<&str>,
    side: &str,
    price: i64,
    seqnum: u64,
    creaunix: Option<i64>,
    sendunix: Option<i64>,
    execunix: Option<i64>,
) -> ExecutionEvent {
    let mut event: ExecutionEvent = event(unix, crosscode, symbol, sendunix);
    event.set_side(Side::read(side).unwrap(), true);
    event.set_price(Some(Decimal::from_int(price)), true);
    event.set_seqnum(seqnum);
    event.set_creaunix(creaunix);
    event.set_execunix(execunix, true);
    event.finalize();
    event
}

#[test]
fn construction_refuses_invalid_composite_parts_at_the_child() {
    crate::install::installed();
    let root = root(10, "T-1", Some("IBM"), None);
    let error = TradeEvent::from_parts(&root, Vec::new()).unwrap_err();
    assert!(error.to_string().contains("$.executions"), "{error}");

    // A side nobody stated is still a fill: any side stands.
    let unknown = execution(10, "E-1", Some("IBM"), "Unknown", 100, 0, None, None, None);
    let trade = TradeEvent::from_parts(&root, vec![unknown]).expect("an unsided fill");
    assert_eq!(trade.executions()[0].get_side(), Side::Unknown);

    let late = execution(11, "E-1", Some("IBM"), "Buy", 100, 0, None, None, None);
    let error = TradeEvent::from_parts(&root, vec![late]).unwrap_err();
    assert!(
        error.to_string().contains("executions[0].transunix"),
        "{error}"
    );

    let other_symbol = execution(10, "E-1", Some("MSFT"), "Buy", 100, 0, None, None, None);
    let error = TradeEvent::from_parts(&root, vec![other_symbol]).unwrap_err();
    assert!(
        error.to_string().contains("executions[0].ticker"),
        "{error}"
    );

    // Two executions of one side under one code share a cross code; a buy
    // and a sell under one code are two, their codes carrying the side.
    let first = execution(10, "E-1", Some("IBM"), "Buy", 100, 0, None, None, None);
    let second = execution(10, "E-1", Some("IBM"), "Buy", 101, 0, None, None, None);
    let error = TradeEvent::from_parts(&root, vec![first, second]).unwrap_err();
    assert!(
        error.to_string().contains("executions[1].crosscode"),
        "{error}"
    );
}

#[test]
fn construction_orders_children_and_derives_one_content_identity_and_bounds() {
    crate::install::installed();
    let mut root = root(20, "T-1", Some("IBM"), Some(18));
    root.set_seqnum(2);
    root.set_creaunix(Some(15));
    root.set_execunix(Some(17), true);
    root.finalize();
    let buy_b = execution(
        20,
        "E-B",
        Some("IBM"),
        "Buy",
        101,
        7,
        Some(12),
        Some(16),
        Some(19),
    );
    let sell = execution(
        20,
        "E-S",
        Some("IBM"),
        "Sell",
        102,
        5,
        Some(14),
        Some(17),
        Some(20),
    );
    let buy_a = execution(
        20,
        "E-A",
        Some("IBM"),
        "Buy",
        100,
        3,
        Some(13),
        None,
        Some(18),
    );

    let first =
        TradeEvent::from_parts(&root, vec![sell.clone(), buy_b.clone(), buy_a.clone()]).unwrap();
    let second = TradeEvent::from_parts(&root, vec![buy_a, sell, buy_b]).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.get_uuid(), second.get_uuid());
    assert_eq!(first.get_hashcode(), second.get_hashcode());
    assert_eq!(
        first
            .executions()
            .iter()
            .map(|held| (held.get_side().as_str(), held.get_crosscode()))
            .collect::<Vec<_>>(),
        [
            ("BUYS", "8:1:E-A"),
            ("BUYS", "8:1:E-B"),
            ("SELL", "8:2:E-S")
        ]
    );
    assert!(
        first
            .executions()
            .iter()
            .all(|held| held.kind() == MarketKind::Execution)
    );
    // The root is not sided: the code it was given, under the trade's
    // category and side 0.
    assert_eq!(first.get_crosscode(), "21:0:T-1");
    assert_eq!(first.get_seqnum(), 7);
    assert_eq!(first.get_creaunix(), Some(12));
    assert_eq!(first.get_sendunix(), Some(16));
    assert_eq!(first.get_execunix(), Some(20));
    assert!(first.is_execution());
}

#[test]
fn merge_deduplicates_by_crosscode_and_the_statement_sent_last_leads() {
    crate::install::installed();
    let left_root = root(30, "T-1", Some("IBM"), Some(10));
    let right_root = root(30, "T-1", Some("IBM"), Some(20));
    let left_e1 = execution(
        30,
        "E-1",
        Some("IBM"),
        "Buy",
        100,
        1,
        Some(9),
        Some(30),
        Some(28),
    );
    let right_e1 = execution(
        30,
        "E-1",
        Some("IBM"),
        "Buy",
        101,
        4,
        Some(8),
        Some(40),
        Some(29),
    );
    let right_e2 = execution(
        30,
        "E-2",
        Some("IBM"),
        "Sell",
        102,
        3,
        Some(7),
        Some(35),
        Some(30),
    );
    let left = TradeEvent::from_parts(&left_root, vec![left_e1]).unwrap();
    let right = TradeEvent::from_parts(&right_root, vec![right_e2, right_e1]).unwrap();

    let merged = left.clone().merge_with(&right).unwrap();
    assert_eq!(merged.executions().len(), 2);
    assert_eq!(
        merged
            .executions()
            .iter()
            .filter(|held| held.get_crosscode() == "8:1:E-1")
            .count(),
        1
    );
    assert_eq!(
        merged
            .executions()
            .iter()
            .find(|held| held.get_crosscode() == "8:1:E-1")
            .unwrap()
            .get_price(),
        Some(Decimal::from_int(101)),
        "the later-recorded child leads its merge"
    );
    assert_eq!(merged.get_seqnum(), 4);
    assert_eq!(merged.get_creaunix(), Some(7));
    assert_eq!(merged.get_sendunix(), Some(10));
    assert_eq!(merged.get_execunix(), Some(30));
    assert_ne!(merged.get_uuid(), left.get_uuid());

    // A merged trade and each merged child keep only the earliest `sendunix`
    // their statements know - the trade 10, its E-1 child 30 - so against a
    // third statement they rank by it: a trade sent at 15 whose E-1 was
    // sent at 35 leads the merged trade and its child, although it
    // leads neither `right` (20) nor `right`'s own E-1 (40).
    let third = TradeEvent::from_parts(
        &root(30, "T-1", Some("IBM"), Some(15)),
        vec![execution(
            30,
            "E-1",
            Some("IBM"),
            "Buy",
            105,
            1,
            Some(9),
            Some(35),
            Some(29),
        )],
    )
    .unwrap();
    let e1_price = |trade: &TradeEvent| {
        trade
            .executions()
            .iter()
            .find(|held| held.get_crosscode() == "8:1:E-1")
            .unwrap()
            .get_price()
    };
    let alone = right.merge_with(&third).unwrap();
    assert_eq!(e1_price(&alone), Some(Decimal::from_int(101)));
    assert_eq!(alone.get_sendunix(), Some(15));
    let folded = merged.merge_with(&third).unwrap();
    assert_eq!(e1_price(&folded), Some(Decimal::from_int(105)));
    assert_eq!(folded.get_sendunix(), Some(10));
}

#[test]
fn following_combines_distinct_executions_and_preserves_composite_identity() {
    crate::install::installed();
    let previous = TradeEvent::from_parts(
        &root(40, "T-1", Some("IBM"), Some(10)),
        vec![execution(
            40,
            "E-1",
            Some("IBM"),
            "Buy",
            100,
            1,
            Some(9),
            Some(11),
            Some(38),
        )],
    )
    .unwrap();
    let current = TradeEvent::from_parts(
        &root(41, "T-1", Some("IBM"), Some(12)),
        vec![execution(
            41,
            "E-2",
            Some("IBM"),
            "Sell",
            101,
            2,
            Some(8),
            Some(13),
            Some(39),
        )],
    )
    .unwrap();

    let followed = current.with_previous(&previous).unwrap();
    assert_eq!(followed.executions().len(), 2);
    assert!(
        followed
            .executions()
            .iter()
            .all(|execution| execution.get_transunix() == 41),
        "carried executions are observations at the successor trade instant"
    );
    assert_eq!(
        followed.executions()[0].get_execunix(),
        Some(38),
        "rebasing retains the precise execution instant"
    );
    assert_eq!(followed.get_prevuuid(), Some(previous.get_uuid()));
    assert_eq!(followed.get_seqnum(), 2);
    assert_eq!(followed.get_creaunix(), Some(8));
    assert_eq!(followed.get_sendunix(), Some(11));
    assert_eq!(followed.get_execunix(), Some(39));
    let mut finalized = followed.clone();
    finalized.finalize();
    assert_eq!(finalized.get_uuid(), followed.get_uuid());
    assert_eq!(finalized.get_hashcode(), followed.get_hashcode());
}

#[test]
fn restating_rederives_the_composite_identity_after_holder_restatement() {
    crate::install::installed();
    let live = TradeEvent::from_parts(
        &root(45, "T-1", Some("IBM"), Some(12)),
        vec![execution(
            45,
            "E-1",
            Some("IBM"),
            "Buy",
            100,
            1,
            Some(10),
            Some(11),
            Some(44),
        )],
    )
    .unwrap();
    let mut repeated = live.clone();
    repeated.set_sendunix(Some(8));

    let restated = repeated.restating(&live);
    assert_eq!(restated.get_sendunix(), Some(8));
    let mut canonical = restated.clone();
    canonical.finalize();
    assert_eq!(restated.get_uuid(), canonical.get_uuid());
    assert_eq!(restated.get_hashcode(), canonical.get_hashcode());
    assert_eq!(restated.executions(), canonical.executions());
}

#[test]
fn timestamp_mutation_rebases_children_and_remains_arrow_valid() {
    crate::install::installed();
    let mut trade = TradeEvent::from_parts(
        &root(50, "T-1", Some("IBM"), Some(49)),
        vec![execution(
            50,
            "E-1",
            Some("IBM"),
            "Buy",
            100,
            1,
            Some(48),
            Some(49),
            Some(47),
        )],
    )
    .unwrap();
    trade.set_transunix(51);
    assert_eq!(trade.get_transunix(), 51);
    assert_eq!(trade.executions()[0].get_transunix(), 51);
    assert_eq!(trade.executions()[0].get_execunix(), Some(47));

    trade.set_transunix(52);
    let input = MarketData::from(trade);
    let MarketData::TradeEvent(trade) = &input else {
        panic!("the value remains a trade")
    };
    assert!(
        trade
            .executions()
            .iter()
            .all(|execution| execution.get_transunix() == 52)
    );
    assert_eq!(trade.executions()[0].get_execunix(), Some(47));
    assert_eq!(trade.get_transunix(), 52);
    assert_eq!(input.book(), None, "a trade carries no book control");

    let encoded = MarketData::arrow_reader([input.clone()], Some(1), None).unwrap();
    let decoded = MarketData::from_arrow_reader(encoded)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(decoded, input);
}
