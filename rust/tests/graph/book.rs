use std::collections::BTreeMap;

use yggdryl::graph::{
    Book, BookIterator, BookSide, Element, Event, Execution, GLOBAL_SYMBOL, MarketElement,
    MarketEventData, MarketOperation, Order, Quote,
};
use yggdryl::{Currency, Decimal18, Side, State};

#[allow(clippy::too_many_arguments)]
fn operation(
    kind: &str,
    symbol: &str,
    identity: &str,
    unix: i64,
    side: &str,
    px: &str,
    qty: i64,
    state: &str,
) -> MarketOperation {
    let mut event = MarketEventData::at(unix);
    event.set_crosscode(identity.to_owned());
    event.set_symbolticker(Some(symbol.to_owned()));
    event.set_side(Side::read(side).unwrap());
    event.set_px(px.parse().unwrap());
    event.set_qty(Decimal18::from_int(qty));
    event.set_currency(Currency::new("USD").unwrap());
    event.set_unit("share".to_owned());
    event.set_state(State::read(state).unwrap());
    event.set_identifiers(BTreeMap::from([(
        "MDEntryID".to_owned(),
        identity.to_owned(),
    )]));
    event.finalize();
    match kind {
        "order" => Order::from(event).into(),
        "quote" => Quote::from(event).into(),
        "execution" => Execution::from(event).into(),
        _ => panic!("unknown operation kind"),
    }
}

fn with_identifiers(
    mut operation: MarketOperation,
    identifiers: &[(&str, &str)],
) -> MarketOperation {
    let mut held = operation.get_identifiers().clone();
    held.extend(
        identifiers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
    );
    operation.set_identifiers(held);
    operation.finalize();
    operation
}

#[test]
fn side_keeps_best_price_order_and_aggregates_exact_level_quantity() {
    let mut side = BookSide::new(Side::read("Buy").unwrap()).unwrap();
    side.add_operation(operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New"))
        .unwrap();
    side.add_operation(operation("quote", "IBM", "Q-1", 2, "Buy", "101", 3, "New"))
        .unwrap();
    side.add_operation(operation("order", "IBM", "O-2", 3, "Buy", "101", 4, "New"))
        .unwrap();

    assert_eq!(
        side.live().map(MarketElement::get_px).collect::<Vec<_>>(),
        [
            "101".parse().unwrap(),
            "101".parse().unwrap(),
            "100".parse().unwrap()
        ]
    );
    assert_eq!(side.best_price(), Some("101".parse().unwrap()));
    assert_eq!(side.best_quantity(), Some(Decimal18::from_int(7)));
    assert_eq!(side.deltas().len(), 3);

    side.add_operation(operation(
        "quote", "IBM", "Q-1", 4, "Buy", "101", 0, "Canceled",
    ))
    .unwrap();
    assert_eq!(side.best_quantity(), Some(Decimal18::from_int(4)));
}

#[test]
fn book_exposes_bbo_midpoint_and_two_value_quantity_median() {
    let mut book = Book::new(10, "IBM");
    book.add_operations([
        operation("quote", "IBM", "B-1", 10, "Buy", "100", 8, "New"),
        operation("quote", "IBM", "A-1", 10, "Sell", "102", 4, "New"),
    ])
    .unwrap();

    assert_eq!(book.bbo_midpoint(), Some("101".parse().unwrap()));
    assert_eq!(book.median_quantity(), Some(Decimal18::from_int(6)));
    assert_eq!(book.get_px(), "101".parse().unwrap());
    assert_eq!(book.get_qty(), Decimal18::from_int(6));
    assert_eq!(book.get_bidqty(), Some(Decimal18::from_int(8)));
    assert_eq!(book.get_askqty(), Some(Decimal18::from_int(4)));

    book.add_operations([operation("quote", "IBM", "B-2", 11, "Buy", "103", 1, "New")])
        .unwrap();
    assert!(book.is_crossed());
    assert_eq!(book.bbo_midpoint(), None);
    assert_eq!(book.get_px(), Decimal18::ZERO);
}

#[test]
fn full_snapshot_replaces_only_its_scope_atomically() {
    let mut book = Book::new(1, "IBM");
    let mut first = operation("quote", "IBM", "B-1", 1, "Buy", "100", 2, "New");
    let mut other = operation("quote", "IBM", "B-X", 1, "Buy", "99", 9, "New");
    let mut ids = first.get_identifiers().clone();
    ids.insert("BookScope".to_owned(), "PRIMARY".to_owned());
    first.set_identifiers(ids);
    first.finalize();
    let mut ids = other.get_identifiers().clone();
    ids.insert("BookScope".to_owned(), "OTHER".to_owned());
    other.set_identifiers(ids);
    other.finalize();
    book.add_operations([first, other]).unwrap();
    let previous = book.clone();

    let mut replacement = operation("quote", "IBM", "B-2", 2, "Buy", "101", 3, "New");
    let mut ids = replacement.get_identifiers().clone();
    ids.insert("BookScope".to_owned(), "PRIMARY".to_owned());
    ids.insert("MDUpdateAction".to_owned(), "SNAPSHOT".to_owned());
    replacement.set_identifiers(ids);
    replacement.finalize();
    book.add_operations([replacement.clone()]).unwrap();

    let identities: Vec<_> = book
        .bid()
        .live()
        .map(|operation| operation.get_crosscode())
        .collect();
    assert_eq!(identities, ["B-2", "B-X"]);

    let mut update = Book::new(2, "IBM");
    update.add_operations([replacement]).unwrap();
    let continued = update.with_previous(&previous).unwrap();
    assert_eq!(
        continued
            .bid()
            .live()
            .map(Element::get_crosscode)
            .collect::<Vec<_>>(),
        ["B-2", "B-X"]
    );
}

#[test]
fn iterator_emits_one_book_per_symbol_and_timestamp_or_one_global_book() {
    let operations = vec![
        operation("quote", "IBM", "IBM-B", 1_000_000, "Buy", "100", 2, "New"),
        operation("quote", "MSFT", "MS-B", 1_000_000, "Buy", "200", 3, "New"),
        operation("quote", "IBM", "IBM-A", 3_000_000, "Sell", "102", 4, "New"),
    ];
    let books = BookIterator::new(operations.clone().into_iter(), 0, false)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(books.len(), 3);
    assert_eq!(
        books
            .iter()
            .map(|book| (book.get_currunix(), book.get_symbolticker().unwrap()))
            .collect::<Vec<_>>(),
        [(1_000_000, "IBM"), (1_000_000, "MSFT"), (3_000_000, "IBM")]
    );

    let global = BookIterator::new(operations.into_iter(), 0, true)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(global.len(), 2);
    assert!(
        global
            .iter()
            .all(|book| book.get_symbolticker() == Some(GLOBAL_SYMBOL))
    );
}

#[test]
fn iterator_grid_emits_complete_snapshots_without_losing_live_orders() {
    let operations = vec![
        operation("order", "IBM", "O-1", 1_000_000, "Buy", "100", 2, "New"),
        operation("quote", "IBM", "A-1", 3_000_000, "Sell", "102", 4, "New"),
    ];
    let books = BookIterator::new(operations.into_iter(), 1, false)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        books.iter().map(Event::get_currunix).collect::<Vec<_>>(),
        [1_000_000, 2_000_000, 3_000_000]
    );
    assert_eq!(books[1].get_snapunix(), Some(2_000_000));
    assert_eq!(books[0].get_snapunix(), Some(1_000_000));
    assert_eq!(books[2].get_snapunix(), Some(3_000_000));
    assert_eq!(books[1].bid().len(), 1);
    assert_eq!(books[2].ask().len(), 1);
    assert_eq!(books[0].bid().deltas().len(), 1);
    assert!(books[1].bid().deltas().is_empty());
    assert_ne!(books[0].bid().get_curruuid(), books[1].bid().get_curruuid());
}

#[test]
fn an_exact_grid_tick_emits_every_symbol_after_the_equal_time_source() {
    let books = BookIterator::new(
        [
            operation("quote", "IBM", "IBM-B", 1_000_000, "Buy", "100", 1, "New"),
            operation("quote", "MSFT", "MS-B", 1_000_000, "Buy", "200", 1, "New"),
            operation("quote", "IBM", "IBM-A", 2_000_000, "Sell", "102", 1, "New"),
        ]
        .into_iter(),
        1,
        false,
    )
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();

    assert_eq!(
        books
            .iter()
            .map(|book| (
                book.get_currunix(),
                book.get_snapunix(),
                book.get_symbolticker().unwrap(),
            ))
            .collect::<Vec<_>>(),
        [
            (1_000_000, Some(1_000_000), "IBM"),
            (1_000_000, Some(1_000_000), "MSFT"),
            (2_000_000, Some(2_000_000), "IBM"),
            (2_000_000, Some(2_000_000), "MSFT"),
        ]
    );
    assert_eq!(books[3].bid().len(), 1);
}

#[test]
fn exact_nanoseconds_participate_in_book_identity_within_one_millisecond() {
    let mut first = Book::new(1_000_001, "IBM");
    first
        .add_operations([operation(
            "quote", "IBM", "B", 1_000_001, "Buy", "100", 1, "New",
        )])
        .unwrap();
    let mut second = first.clone();
    second.set_currunix(1_000_002);
    second.finalize();
    assert_ne!(first.get_curruuid(), second.get_curruuid());
}

#[test]
fn updates_follow_the_live_entry_and_atomic_failures_leave_the_book_unchanged() {
    let mut book = Book::new(1, "IBM");
    book.add_operations([operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New")])
        .unwrap();
    let first = book.bid().live().next().unwrap().clone();

    book.add_operations([operation(
        "order", "IBM", "O-1", 2, "Buy", "101", 3, "Replaced",
    )])
    .unwrap();
    let replacement = book.bid().live().next().unwrap();
    assert_eq!(replacement.get_seqnum(), 1);
    assert_eq!(replacement.get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(replacement.get_prevpx(), Some(first.get_px()));
    assert_eq!(replacement.get_prevqty(), Some(first.get_qty()));
    assert_eq!(book.get_seqnum(), 1);

    let before = book.clone();
    let error = book
        .add_operations([
            operation("quote", "IBM", "B-2", 3, "Buy", "99", 1, "New"),
            operation("quote", "IBM", "B-3", 4, "Buy", "98", 1, "New"),
        ])
        .expect_err("one atomic group has one timestamp");
    assert!(error.to_string().contains("same currunix"));
    assert_eq!(book, before);
}

#[test]
fn partial_updates_require_a_predecessor_or_complete_values_and_refs_precede_destination() {
    let mut book = Book::new(1, "IBM");
    let partial = with_identifiers(
        operation("order", "IBM", "MISSING", 1, "Buy", "101", 3, "Replaced"),
        &[("MDUpdateAction", "1")],
    );
    let error = book.add_operations([partial]).unwrap_err().to_string();
    assert!(error.contains("MDEntryPx"), "{error}");
    assert!(book.bid().is_empty());

    book.add_operations([
        operation("order", "IBM", "X", 1, "Buy", "100", 1, "New"),
        operation("order", "IBM", "Y", 1, "Buy", "99", 2, "New"),
    ])
    .unwrap();
    let before = book.clone();
    let collision = with_identifiers(
        operation("order", "IBM", "Y", 2, "Buy", "101", 4, "Replaced"),
        &[
            ("MDUpdateAction", "1"),
            ("MDEntryRefID", "X"),
            ("MDEntryPx", "101"),
            ("MDEntrySize", "4"),
        ],
    );
    let error = book.add_operations([collision]).unwrap_err().to_string();
    assert!(error.contains("MDEntryID"), "{error}");
    assert_eq!(book, before);

    let unresolved = with_identifiers(
        operation("order", "IBM", "Z", 2, "Buy", "102", 5, "Replaced"),
        &[
            ("MDUpdateAction", "1"),
            ("MDEntryRefID", "ABSENT"),
            ("MDEntryPx", "102"),
            ("MDEntrySize", "5"),
        ],
    );
    let error = book.add_operations([unresolved]).unwrap_err().to_string();
    assert!(error.contains("MDEntryRefID"), "{error}");
    assert_eq!(book, before);
}

#[test]
fn executions_are_reported_beside_depth_and_propagate_the_latest_execution_clock() {
    let mut first = MarketEventData::at(10);
    first.set_crosscode("E-1".to_owned());
    first.set_symbolticker(Some("IBM".to_owned()));
    first.set_px("100".parse().unwrap());
    first.set_qty(Decimal18::from_int(2));
    first.set_state(State::read("Filled").unwrap());
    first.set_execunix(Some(7));
    first.finalize();
    let mut second = first.clone();
    second.set_crosscode("E-2".to_owned());
    second.set_execunix(Some(9));
    second.finalize();

    let mut book = Book::new(10, "IBM");
    book.add_operations([
        Execution::from(first).into(),
        Execution::from(second).into(),
    ])
    .unwrap();
    assert_eq!(book.executions().len(), 2);
    assert!(book.bid().is_empty());
    assert!(book.ask().is_empty());
    assert_eq!(book.get_execunix(), Some(9));
}

#[test]
fn expiry_precedes_an_equal_time_source_and_executions_do_not_enter_live_expiry() {
    let mut live = operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New");
    live.set_exprtime(Some(3));
    live.finalize();
    let mut execution = MarketEventData::at(1);
    execution.set_crosscode("E-1".to_owned());
    execution.set_symbolticker(Some("IBM".to_owned()));
    execution.set_state(State::read("Filled").unwrap());
    execution.set_exprtime(Some(2));
    execution.finalize();

    let books = BookIterator::new(
        [
            live,
            Execution::from(execution).into(),
            operation("order", "IBM", "O-1", 3, "Buy", "101", 3, "New"),
        ]
        .into_iter(),
        0,
        false,
    )
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();

    assert_eq!(
        books.iter().map(Event::get_currunix).collect::<Vec<_>>(),
        [1, 3]
    );
    assert_eq!(books[0].executions().len(), 1);
    assert!(books[1].executions().is_empty());
    let replacement = books[1].bid().live().next().unwrap();
    assert_eq!(replacement.get_px(), "101".parse().unwrap());
    assert_eq!(
        (replacement.get_seqnum(), replacement.get_prevuuid()),
        (0, None)
    );
}

#[test]
fn a_failed_equal_time_source_does_not_consume_the_pending_expiration() {
    let mut live = operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New");
    live.set_exprtime(Some(3));
    live.finalize();
    let invalid = operation("quote", "IBM", "INVALID", 3, "Unknown", "101", 1, "New");
    let mut books = BookIterator::new([live, invalid].into_iter(), 0, false).unwrap();

    assert_eq!(books.next().unwrap().unwrap().bid().len(), 1);
    assert!(books.next().unwrap().is_err());
    let expired = books.next().unwrap().unwrap();
    assert_eq!(expired.get_currunix(), 3);
    assert!(expired.bid().is_empty());
    assert!(books.next().is_none());
}

#[test]
fn expiring_one_snapshot_entry_keeps_the_rest_of_its_partition() {
    let mut expiring = with_identifiers(
        operation("quote", "IBM", "EXPIRING", 1, "Buy", "101", 1, "New"),
        &[("BookScope", "PRIMARY"), ("MDUpdateAction", "SNAPSHOT")],
    );
    expiring.set_exprtime(Some(2));
    expiring.finalize();
    let standing = with_identifiers(
        operation("quote", "IBM", "STANDING", 1, "Buy", "100", 1, "New"),
        &[("BookScope", "PRIMARY"), ("MDUpdateAction", "SNAPSHOT")],
    );

    let books = BookIterator::new([expiring, standing].into_iter(), 0, false)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(books.len(), 2);
    assert_eq!(books[0].bid().len(), 2);
    assert_eq!(books[1].bid().len(), 1);
    assert_eq!(
        books[1].bid().live().next().unwrap().get_crosscode(),
        "STANDING"
    );
}

#[test]
fn a_failed_mixed_snapshot_group_commits_none_of_its_raw_updates() {
    let initial = operation("quote", "IBM", "INITIAL", 1, "Buy", "100", 1, "New");
    let raw = operation("quote", "IBM", "RAW", 2, "Buy", "99", 1, "New");
    let mut duplicate = operation("quote", "IBM", "DUPLICATE", 2, "Buy", "101", 1, "New");
    duplicate.set_snapunix(Some(2));
    duplicate.finalize();
    let later = operation("execution", "IBM", "LATER", 3, "Unknown", "100", 1, "Trade");

    let results = BookIterator::new(
        [initial, raw, duplicate.clone(), duplicate, later].into_iter(),
        0,
        false,
    )
    .unwrap()
    .collect::<Vec<_>>();
    assert_eq!(results.len(), 3);
    assert!(results[0].is_ok());
    assert!(results[1].is_err());
    let after = results[2].as_ref().unwrap();
    assert_eq!(after.bid().len(), 1);
    assert_eq!(
        after.bid().live().next().unwrap().get_crosscode(),
        "INITIAL"
    );
}

#[test]
fn a_snapshot_view_purges_live_entries_absent_from_that_view() {
    let first = operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New");
    let second = operation("order", "IBM", "O-2", 1, "Buy", "99", 3, "New");
    let other_scope = with_identifiers(
        operation("order", "IBM", "O-OTHER", 1, "Buy", "98", 4, "New"),
        &[("BookScope", "OTHER")],
    );
    let mut snapshot = first.clone();
    let mut identifiers = snapshot.get_identifiers().clone();
    identifiers.insert("SnapshotView".to_owned(), "2".to_owned());
    snapshot.set_identifiers(identifiers);
    snapshot.set_px("101".parse().unwrap());
    snapshot.set_qty(Decimal18::from_int(5));
    snapshot.finalize();
    snapshot.set_snapunix(Some(2));
    let snapshot_only = snapshot.clone();

    let books = BookIterator::new([first, second, other_scope, snapshot].into_iter(), 0, false)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(books.len(), 2);
    assert_eq!(books[0].bid().len(), 3);
    assert_eq!(books[1].get_snapunix(), Some(2));
    assert_eq!(books[1].bid().len(), 2);
    let mut live = books[1].bid().live();
    let first = live.next().unwrap();
    assert_eq!(first.get_crosscode(), "O-1");
    assert_eq!(first.get_px(), "101".parse().unwrap());
    assert_eq!(first.get_qty(), Decimal18::from_int(5));
    assert_eq!(live.next().unwrap().get_crosscode(), "O-OTHER");

    let only = BookIterator::new([snapshot_only].into_iter(), 0, false)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(only.get_snapunix(), Some(2));
    assert_eq!(only.bid().len(), 1);
    assert_eq!(
        only.bid().live().next().unwrap().get_px(),
        "101".parse().unwrap()
    );
}

#[test]
fn snapshotted_execution_is_reported_without_replacing_live_depth() {
    let live = operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New");
    let mut execution = operation("execution", "IBM", "E-1", 2, "Unknown", "101", 1, "Trade");
    execution.set_execunix(Some(2));
    execution.set_snapunix(Some(3));
    execution.finalize();

    let books = BookIterator::new([live, execution].into_iter(), 0, false)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(books.len(), 2);
    assert_eq!(books[1].get_currunix(), 3);
    assert_eq!(books[1].get_snapunix(), Some(3));
    assert_eq!(books[1].bid().len(), 1);
    assert_eq!(books[1].executions().len(), 1);
    assert_eq!(books[1].executions()[0].get_currunix(), 3);
    assert_eq!(books[1].executions()[0].get_execunix(), Some(2));
}

#[test]
fn future_snapshot_components_are_refused_instead_of_backdated() {
    let mut execution = operation(
        "execution",
        "IBM",
        "E-FUTURE",
        3,
        "Unknown",
        "101",
        1,
        "Trade",
    );
    execution.set_snapunix(Some(2));
    execution.finalize();
    let error = BookIterator::new([execution.clone()].into_iter(), 0, false)
        .unwrap()
        .next()
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(error.contains("snapshot.executions[0].currunix"), "{error}");

    execution.set_snapunix(Some(4));
    execution.finalize();
    let mut direct = Book::new(3, "IBM");
    let error = direct.add_operations([execution]).unwrap_err().to_string();
    assert!(error.contains("operation[0].snapunix"), "{error}");

    let mut reset = MarketEventData::at(3);
    reset.set_crosscode("RESET-FUTURE".to_owned());
    reset.set_symbolticker(Some("IBM".to_owned()));
    reset.set_snapunix(Some(2));
    reset.set_identifiers(BTreeMap::from([
        ("BookScope".to_owned(), "PRIMARY".to_owned()),
        ("MDUpdateAction".to_owned(), "SNAPSHOT".to_owned()),
    ]));
    reset.finalize();
    let error = BookIterator::new([MarketOperation::Snapshot(reset)].into_iter(), 0, false)
        .unwrap()
        .next()
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(error.contains("snapshot.controls[0].currunix"), "{error}");
}

#[test]
fn an_explicit_empty_snapshot_replaces_only_its_partition() {
    let primary = with_identifiers(
        operation("quote", "IBM", "PRIMARY", 1, "Buy", "100", 1, "New"),
        &[("BookScope", "PRIMARY")],
    );
    let other = with_identifiers(
        operation("quote", "IBM", "OTHER", 1, "Buy", "99", 1, "New"),
        &[("BookScope", "OTHER")],
    );
    let mut reset = MarketEventData::at(2);
    reset.set_crosscode("RESET-OTHER".to_owned());
    reset.set_symbolticker(Some("IBM".to_owned()));
    reset.set_seqnum(9);
    reset.set_snapunix(Some(2));
    reset.set_state(State::read("New").unwrap());
    reset.set_identifiers(BTreeMap::from([
        ("BookScope".to_owned(), "OTHER".to_owned()),
        ("MDUpdateAction".to_owned(), "SNAPSHOT".to_owned()),
    ]));
    reset.finalize();

    let books = BookIterator::new(
        [primary, other, MarketOperation::Snapshot(reset)].into_iter(),
        0,
        false,
    )
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
    assert_eq!(books.len(), 2);
    assert_eq!(books[1].get_seqnum(), 9);
    assert_eq!(books[1].get_snapunix(), Some(2));
    assert_eq!(books[1].bid().len(), 1);
    assert_eq!(
        books[1].bid().live().next().unwrap().get_crosscode(),
        "PRIMARY"
    );
}

#[test]
fn merging_books_uses_the_latest_recording_as_reference_and_keeps_earliest_clocks() {
    let mut older = Book::new(100, "IBM");
    older
        .add_operations([operation(
            "quote", "IBM", "B-1", 100, "Buy", "100", 2, "New",
        )])
        .unwrap();
    older.set_execunix(Some(7));
    older.set_recdunix(Some(20));
    older.set_refrecdunix(Some(20));
    older.finalize();

    let mut latest = Book::new(100, "IBM");
    latest
        .add_operations([operation(
            "quote", "IBM", "A-1", 100, "Sell", "102", 4, "New",
        )])
        .unwrap();
    latest.set_execunix(Some(9));
    latest.set_recdunix(Some(30));
    latest.set_refrecdunix(Some(30));
    latest.finalize();

    let left = older.clone().merge_with(&latest).unwrap();
    let right = latest.merge_with(&older).unwrap();
    for merged in [&left, &right] {
        assert_eq!(merged.bid().len(), 1);
        assert_eq!(merged.ask().len(), 1);
        assert_eq!(merged.get_execunix(), Some(7));
        assert_eq!(merged.get_recdunix(), Some(20));
        assert_eq!(merged.get_refrecdunix(), Some(30));
    }
    assert_eq!(left.get_curruuid(), right.get_curruuid());
}

#[test]
fn operation_kind_participates_in_book_identity() {
    let mut order_book = Book::new(1, "IBM");
    order_book
        .add_operations([operation("order", "IBM", "B-1", 1, "Buy", "100", 1, "New")])
        .unwrap();
    let mut quote_book = Book::new(1, "IBM");
    quote_book
        .add_operations([operation("quote", "IBM", "B-1", 1, "Buy", "100", 1, "New")])
        .unwrap();
    assert_ne!(order_book.get_curruuid(), quote_book.get_curruuid());
}

#[test]
fn global_depth_keys_include_symbol_scope_and_identity() {
    let mut book = Book::new(1, GLOBAL_SYMBOL);
    book.add_operations([
        with_identifiers(
            operation("quote", "IBM", "SAME", 1, "Buy", "100", 1, "New"),
            &[("BookScope", "PRIMARY")],
        ),
        with_identifiers(
            operation("quote", "MSFT", "SAME", 1, "Buy", "200", 2, "New"),
            &[("BookScope", "PRIMARY")],
        ),
        with_identifiers(
            operation("quote", "IBM", "SAME", 1, "Buy", "99", 3, "New"),
            &[("BookScope", "OTHER")],
        ),
    ])
    .unwrap();
    assert_eq!(book.bid().len(), 3);
}

#[test]
fn a_global_snapshot_replaces_only_its_source_symbol_and_scope() {
    let mut book = Book::new(1, GLOBAL_SYMBOL);
    book.add_operations([
        with_identifiers(
            operation("quote", "IBM", "IBM-OLD", 1, "Buy", "100", 1, "New"),
            &[("BookScope", "PRIMARY")],
        ),
        with_identifiers(
            operation("quote", "MSFT", "MS-LIVE", 1, "Buy", "200", 2, "New"),
            &[("BookScope", "PRIMARY")],
        ),
    ])
    .unwrap();
    book.add_operations([with_identifiers(
        operation("quote", "IBM", "IBM-NEW", 2, "Buy", "101", 3, "New"),
        &[("BookScope", "PRIMARY"), ("MDUpdateAction", "SNAPSHOT")],
    )])
    .unwrap();

    assert_eq!(
        book.bid()
            .live()
            .map(Element::get_crosscode)
            .collect::<Vec<_>>(),
        ["MS-LIVE", "IBM-NEW"]
    );
}

#[test]
fn range_deletes_are_positive_in_range_and_scope_local() {
    let mut book = Book::new(1, "IBM");
    book.add_operations([
        with_identifiers(
            operation("quote", "IBM", "P-1", 1, "Buy", "101", 1, "New"),
            &[("BookScope", "PRIMARY")],
        ),
        with_identifiers(
            operation("quote", "IBM", "O-1", 1, "Buy", "100", 2, "New"),
            &[("BookScope", "OTHER")],
        ),
        with_identifiers(
            operation("quote", "IBM", "P-2", 1, "Buy", "99", 3, "New"),
            &[("BookScope", "PRIMARY")],
        ),
    ])
    .unwrap();

    let deletion = with_identifiers(
        operation("quote", "IBM", "DELETE", 2, "Buy", "0", 0, "Canceled"),
        &[
            ("BookScope", "PRIMARY"),
            ("MDUpdateAction", "3"),
            ("MDEntryPositionNo", "1"),
        ],
    );
    book.add_operations([deletion]).unwrap();
    let remaining = book
        .bid()
        .live()
        .map(Element::get_crosscode)
        .collect::<Vec<_>>();
    assert_eq!(remaining, ["O-1", "P-2"]);

    let before = book.clone();
    let invalid = with_identifiers(
        operation("quote", "IBM", "DELETE", 3, "Buy", "0", 0, "Canceled"),
        &[
            ("BookScope", "PRIMARY"),
            ("MDUpdateAction", "4"),
            ("MDEntryPositionNo", "0"),
        ],
    );
    assert!(book.add_operations([invalid]).is_err());
    assert_eq!(book, before);
}

#[test]
fn an_anonymous_new_entry_refuses_to_replace_an_occupied_position() {
    let mut book = Book::new(1, "IBM");
    let first = with_identifiers(
        operation("quote", "IBM", "POSITION-1", 1, "Buy", "100", 1, "New"),
        &[("MDUpdateAction", "0"), ("MDEntryPositionNo", "1")],
    );
    let mut first_ids = first.get_identifiers().clone();
    first_ids.remove("MDEntryID");
    let mut first = first;
    first.set_identifiers(first_ids);
    first.finalize();
    book.add_operations([first]).unwrap();

    let mut collision = with_identifiers(
        operation("quote", "IBM", "POSITION-1", 2, "Buy", "101", 2, "New"),
        &[("MDUpdateAction", "0"), ("MDEntryPositionNo", "1")],
    );
    let mut collision_ids = collision.get_identifiers().clone();
    collision_ids.remove("MDEntryID");
    collision.set_identifiers(collision_ids);
    collision.finalize();
    let before = book.clone();
    let error = book.add_operations([collision]).unwrap_err().to_string();
    assert!(error.contains("existing position"), "{error}");
    assert_eq!(book, before);

    let mut book = Book::new(1, "IBM");
    let explicit = with_identifiers(
        operation("quote", "IBM", "EXPLICIT", 1, "Buy", "100", 1, "New"),
        &[("MDUpdateAction", "0"), ("MDEntryPositionNo", "1")],
    );
    book.add_operations([explicit]).unwrap();
    let mut anonymous = with_identifiers(
        operation("quote", "IBM", "ANONYMOUS", 2, "Buy", "101", 1, "New"),
        &[("MDUpdateAction", "0"), ("MDEntryPositionNo", "1")],
    );
    let mut identifiers = anonymous.get_identifiers().clone();
    identifiers.remove("MDEntryID");
    anonymous.set_identifiers(identifiers);
    anonymous.finalize();
    let before = book.clone();
    let error = book.add_operations([anonymous]).unwrap_err().to_string();
    assert!(error.contains("existing position"), "{error}");
    assert_eq!(book, before);
}

#[test]
fn advancing_time_clears_previous_deltas_and_executions_and_rejects_regression() {
    let mut execution = MarketEventData::at(1);
    execution.set_crosscode("E-1".to_owned());
    execution.set_symbolticker(Some("IBM".to_owned()));
    execution.set_state(State::read("Filled").unwrap());
    execution.finalize();
    let mut book = Book::new(1, "IBM");
    book.add_operations([
        operation("quote", "IBM", "B-1", 1, "Buy", "100", 1, "New"),
        Execution::from(execution).into(),
    ])
    .unwrap();
    assert_eq!(book.executions().len(), 1);
    book.set_snapunix(Some(1));
    book.finalize();

    book.add_operations([operation("quote", "IBM", "A-1", 2, "Sell", "102", 2, "New")])
        .unwrap();
    assert_eq!(book.get_snapunix(), None);
    assert!(book.executions().is_empty());
    assert!(book.bid().deltas().is_empty());
    assert_eq!(book.ask().deltas().len(), 1);
    assert!(
        book.add_operations([operation("quote", "IBM", "OLD", 1, "Buy", "99", 1, "New")])
            .is_err()
    );
}

#[test]
fn moving_one_identity_between_sides_retires_the_old_side() {
    let mut book = Book::new(1, "IBM");
    book.add_operations([operation("order", "IBM", "O-1", 1, "Buy", "100", 1, "New")])
        .unwrap();
    book.add_operations([operation(
        "order", "IBM", "O-1", 2, "Sell", "101", 1, "Replaced",
    )])
    .unwrap();
    assert!(book.bid().is_empty());
    assert_eq!(book.ask().len(), 1);
}

#[test]
fn side_identity_includes_deeper_levels_and_book_merge_is_idempotent() {
    let mut shallow = BookSide::new(Side::read("Buy").unwrap()).unwrap();
    shallow
        .add_operation(operation("quote", "IBM", "B-1", 1, "Buy", "100", 1, "New"))
        .unwrap();
    let mut deep = shallow.clone();
    deep.add_operation(operation("quote", "IBM", "B-2", 1, "Buy", "99", 1, "New"))
        .unwrap();
    assert_ne!(shallow.get_curruuid(), deep.get_curruuid());

    let mut book = Book::new(1, "IBM");
    book.add_operations([operation("quote", "IBM", "B-1", 1, "Buy", "100", 1, "New")])
        .unwrap();
    assert!(book.clone().merge_with(&book).is_none());

    let final_live = operation("quote", "IBM", "B", 2, "Buy", "100", 1, "New");
    let mut direct = Book::new(2, "IBM");
    direct.add_operations([final_live.clone()]).unwrap();
    let mut with_history = Book::new(2, "IBM");
    with_history
        .add_operations([
            operation("quote", "IBM", "TRANSIENT", 2, "Buy", "99", 1, "New"),
            operation("quote", "IBM", "TRANSIENT", 2, "Buy", "99", 0, "Canceled"),
            final_live,
        ])
        .unwrap();
    assert_eq!(
        direct.bid().live().collect::<Vec<_>>(),
        with_history.bid().live().collect::<Vec<_>>()
    );
    assert_ne!(direct.bid().deltas(), with_history.bid().deltas());
    assert_ne!(direct.get_curruuid(), with_history.get_curruuid());
}

#[test]
fn merging_treats_a_grid_snapshot_as_authoritative() {
    let views = BookIterator::new(
        [
            operation("quote", "IBM", "B-1", 1_000_000, "Buy", "100", 1, "New"),
            operation(
                "execution",
                "IBM",
                "T-1",
                2_000_000,
                "Unknown",
                "101",
                1,
                "Trade",
            ),
        ]
        .into_iter(),
        2,
        false,
    )
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
    let mut reference = views[1].clone();
    assert!(reference.bid().deltas().is_empty());
    reference.set_recdunix(Some(20));
    reference.set_refrecdunix(Some(20));
    reference.finalize();

    let mut supplement = Book::new(2_000_000, "IBM");
    supplement
        .add_operations([operation(
            "quote", "IBM", "A-X", 2_000_000, "Sell", "103", 1, "New",
        )])
        .unwrap();
    supplement.set_recdunix(Some(10));
    supplement.set_refrecdunix(Some(10));
    supplement.finalize();
    let merged = supplement.merge_with(&reference).unwrap();
    assert_eq!(merged.bid().len(), 1);
    assert!(merged.ask().is_empty());
    assert!(merged.ask().deltas().is_empty());
}

#[test]
fn an_empty_snapshot_reference_does_not_refill_replaced_scope_on_merge() {
    let mut older = Book::new(2, "IBM");
    older
        .add_operations([
            with_identifiers(
                operation("quote", "IBM", "B-1", 2, "Buy", "100", 1, "New"),
                &[("BookScope", "PRIMARY")],
            ),
            with_identifiers(
                operation("quote", "IBM", "B-X", 2, "Buy", "99", 2, "New"),
                &[("BookScope", "OTHER")],
            ),
        ])
        .unwrap();
    older.set_recdunix(Some(10));
    older.set_refrecdunix(Some(10));
    older.finalize();

    let mut reset = MarketEventData::at(2);
    reset.set_crosscode("RESET".to_owned());
    reset.set_symbolticker(Some("IBM".to_owned()));
    reset.set_state(State::read("New").unwrap());
    reset.set_identifiers(BTreeMap::from([
        ("BookScope".to_owned(), "PRIMARY".to_owned()),
        ("MDUpdateAction".to_owned(), "SNAPSHOT".to_owned()),
    ]));
    reset.finalize();
    let mut latest = Book::new(2, "IBM");
    latest
        .add_operations([MarketOperation::Snapshot(reset)])
        .unwrap();
    latest.set_recdunix(Some(20));
    latest.set_refrecdunix(Some(20));
    latest.finalize();

    let continued = latest.clone().with_previous(&older).unwrap();
    assert_eq!(continued.bid().len(), 1);
    assert_eq!(
        continued.bid().live().next().unwrap().get_crosscode(),
        "B-X"
    );

    let merged = older.merge_with(&latest).unwrap();
    assert_eq!(merged.bid().len(), 1);
    assert_eq!(merged.bid().live().next().unwrap().get_crosscode(), "B-X");
    assert!(merged.ask().is_empty());
}

#[test]
fn decimal_means_do_not_overflow_representable_results() {
    let bid = Decimal18::from_units(Decimal18::MAX.units() - 2).unwrap();
    let ask = Decimal18::MAX;
    let mut bid_operation = operation("quote", "IBM", "B", 1, "Buy", "0", 1, "New");
    bid_operation.set_px(bid);
    bid_operation.set_qty(Decimal18::MAX);
    bid_operation.finalize();
    let mut ask_operation = operation("quote", "IBM", "A", 1, "Sell", "0", 1, "New");
    ask_operation.set_px(ask);
    ask_operation.set_qty(Decimal18::MAX);
    ask_operation.finalize();
    let mut book = Book::new(1, "IBM");
    book.add_operations([bid_operation, ask_operation]).unwrap();
    assert_eq!(
        book.bbo_midpoint(),
        Decimal18::from_units(Decimal18::MAX.units() - 1)
    );
    assert_eq!(book.median_quantity(), Some(Decimal18::MAX));
}

#[test]
fn a_failed_iterator_group_emits_only_the_error() {
    let mut invalid = operation("quote", "IBM", "BAD", 1, "Buy", "99", 1, "New");
    invalid.set_side(Side::unknown());
    invalid.finalize();
    let results = BookIterator::new(
        [
            operation("quote", "IBM", "GOOD", 1, "Buy", "100", 1, "New"),
            invalid,
            operation("quote", "MSFT", "LATER", 2_000_000, "Buy", "200", 1, "New"),
        ]
        .into_iter(),
        1,
        false,
    )
    .unwrap()
    .collect::<Vec<_>>();
    assert_eq!(results.len(), 2);
    assert!(results[0].is_err());
    let later = results[1].as_ref().unwrap();
    assert_eq!(later.get_symbolticker(), Some("MSFT"));
    assert_eq!(later.bid().live().next().unwrap().get_crosscode(), "LATER");
}
