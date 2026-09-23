use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use yggdryl::graph::{
    Book, BookIterator, BookSide, Element, Event, Execution, GLOBAL_SYMBOL, MarketElement,
    MarketEventData, MarketOperation, Order, Quote, Trade,
};
use yggdryl::{Currency, Decimal18, Side, State};

#[allow(clippy::too_many_arguments)]
fn operation(
    kind: &str,
    symbol: &str,
    identity: &str,
    unix: i64,
    side: &str,
    price: &str,
    quantity: i64,
    state: &str,
) -> MarketOperation {
    let mut event = MarketEventData::at(unix);
    event.set_crosscode(identity.to_owned());
    event.set_symbolticker(Some(symbol.to_owned()));
    event.set_side(Side::read(side).unwrap());
    event.set_price(price.parse().unwrap());
    event.set_quantity(Decimal18::from_int(quantity));
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
        side.live()
            .map(MarketElement::get_price)
            .collect::<Vec<_>>(),
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
    assert_eq!(book.get_price(), "101".parse().unwrap());
    assert_eq!(book.get_quantity(), Decimal18::from_int(6));
    assert_eq!(book.get_bidqty(), Some(Decimal18::from_int(8)));
    assert_eq!(book.get_askqty(), Some(Decimal18::from_int(4)));

    book.add_operations([operation("quote", "IBM", "B-2", 11, "Buy", "103", 1, "New")])
        .unwrap();
    assert!(book.is_crossed());
    assert_eq!(book.bbo_midpoint(), None);
    assert_eq!(book.get_price(), Decimal18::ZERO);
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
fn iterator_emits_a_completed_timestamp_before_a_source_error_then_fuses() {
    struct Counted {
        source: std::vec::IntoIter<yggdryl::Result<MarketOperation>>,
        pulled: Arc<AtomicUsize>,
    }

    impl Iterator for Counted {
        type Item = yggdryl::Result<MarketOperation>;

        fn next(&mut self) -> Option<Self::Item> {
            let item = self.source.next()?;
            self.pulled.fetch_add(1, Ordering::SeqCst);
            Some(item)
        }
    }

    let pulled = Arc::new(AtomicUsize::new(0));
    let source = vec![
        Ok(operation(
            "quote", "IBM", "IBM-B", 1_000_000, "Buy", "100", 2, "New",
        )),
        Ok(operation(
            "quote", "IBM", "IBM-A", 1_000_000, "Sell", "102", 4, "New",
        )),
        Err(yggdryl::Error::InvalidRecord {
            path: "$[2]".into(),
            reason: "broken operation source".into(),
        }),
        Ok(operation(
            "quote", "IBM", "IBM-LATE", 2_000_000, "Buy", "101", 1, "New",
        )),
    ];
    let mut books = BookIterator::new(
        Counted {
            source: source.into_iter(),
            pulled: Arc::clone(&pulled),
        },
        0,
        false,
    )
    .unwrap();

    assert_eq!(pulled.load(Ordering::SeqCst), 0);
    let book = books.next().unwrap().unwrap();
    assert_eq!(book.get_currunix(), 1_000_000);
    assert_eq!((book.bid().len(), book.ask().len()), (1, 1));
    assert_eq!(pulled.load(Ordering::SeqCst), 3);

    let error = books.next().unwrap().unwrap_err();
    assert!(
        matches!(&error, yggdryl::Error::InvalidRecord { path, reason }
            if path == "$[2]" && reason == "broken operation source"),
        "{error}"
    );
    assert!(books.next().is_none());
    assert!(books.next().is_none());
    assert_eq!(pulled.load(Ordering::SeqCst), 3);
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
    assert_eq!(replacement.get_prevpx(), Some(first.get_price()));
    assert_eq!(replacement.get_prevqty(), Some(first.get_quantity()));
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
fn partial_market_updates_continue_orders_without_restating_order_id() {
    let mut book = Book::new(1, "IBM");
    book.add_operations([with_identifiers(
        operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New"),
        &[("OrderID", "ORDER-1"), ("MDUpdateAction", "0")],
    )])
    .unwrap();
    let previous = book.bid().live().next().unwrap().clone();

    book.add_operations([with_identifiers(
        operation("quote", "IBM", "O-1", 2, "Buy", "0", 3, "Replaced"),
        &[("MDUpdateAction", "1"), ("MDEntrySize", "3")],
    )])
    .unwrap();

    let live = book.bid().live().next().unwrap();
    assert!(matches!(live, MarketOperation::Order(_)));
    assert_eq!(live.get_identifiers()["OrderID"], "ORDER-1");
    assert_eq!(live.get_price(), previous.get_price());
    assert_eq!(live.get_quantity(), Decimal18::from_int(3));
    assert_eq!(live.get_prevuuid(), Some(previous.get_curruuid()));
    assert_eq!(live.get_prevpx(), Some(previous.get_price()));
    assert_eq!(live.get_prevqty(), Some(previous.get_quantity()));
    assert_eq!(live.get_seqnum(), previous.get_seqnum() + 1);
    let mut finalized = live.clone();
    finalized.finalize();
    assert_eq!(*live, finalized);
}

#[test]
fn referenced_market_update_refuses_an_occupied_destination_on_the_other_side() {
    let mut book = Book::new(1, "IBM");
    book.add_operations([
        operation("quote", "IBM", "X", 1, "Buy", "100", 2, "New"),
        operation("quote", "IBM", "Y", 1, "Sell", "101", 3, "New"),
    ])
    .unwrap();
    let before = book.clone();
    let error = book
        .add_operations([with_identifiers(
            operation("quote", "IBM", "Y", 2, "Buy", "99", 4, "Replaced"),
            &[
                ("MDUpdateAction", "1"),
                ("MDEntryRefID", "X"),
                ("MDEntryPx", "99"),
                ("MDEntrySize", "4"),
            ],
        )])
        .expect_err("a reference cannot replace a different live destination on either side");
    assert!(
        matches!(&error, yggdryl::Error::InvalidRecord { path, .. } if path == "$.operation.identifiers.MDEntryID"),
        "{error}"
    );
    assert_eq!(book, before);
}

#[test]
fn partial_market_updates_move_between_sides_with_their_predecessor() {
    let mut book = Book::new(1, "IBM");
    book.add_operations([with_identifiers(
        operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New"),
        &[("OrderID", "ORDER-1"), ("MDUpdateAction", "0")],
    )])
    .unwrap();
    let previous = book.bid().live().next().unwrap().clone();

    book.add_operations([with_identifiers(
        operation("quote", "IBM", "O-2", 2, "Sell", "0", 3, "Replaced"),
        &[
            ("MDUpdateAction", "1"),
            ("MDEntryRefID", "O-1"),
            ("MDEntrySize", "3"),
        ],
    )])
    .unwrap();
    assert!(book.bid().is_empty());
    let live = book.ask().live().next().unwrap();
    assert!(matches!(live, MarketOperation::Order(_)));
    assert_eq!(live.get_price(), previous.get_price());
    assert_eq!(live.get_quantity(), Decimal18::from_int(3));
    assert_eq!(live.get_prevuuid(), Some(previous.get_curruuid()));
    assert_eq!(live.get_prevpx(), Some(previous.get_price()));
    assert_eq!(live.get_prevqty(), Some(previous.get_quantity()));
    assert_eq!(live.get_seqnum(), previous.get_seqnum() + 1);
    assert_eq!(live.get_identifiers()["MDEntryID"], "O-2");
    let previous = live.clone();

    book.add_operations([with_identifiers(
        operation("quote", "IBM", "O-2", 3, "Buy", "101", 0, "Replaced"),
        &[("MDUpdateAction", "5"), ("MDEntryPx", "101")],
    )])
    .unwrap();
    assert!(book.ask().is_empty());
    let live = book.bid().live().next().unwrap();
    assert!(matches!(live, MarketOperation::Order(_)));
    assert_eq!(live.get_price(), Decimal18::from_int(101));
    assert_eq!(live.get_quantity(), previous.get_quantity());
    assert_eq!(live.get_prevuuid(), Some(previous.get_curruuid()));
    assert_eq!(live.get_seqnum(), previous.get_seqnum() + 1);
}

#[test]
fn renamed_market_entry_expires_using_its_current_identity() {
    let mut initial = with_identifiers(
        operation("order", "IBM", "X", 1, "Buy", "100", 2, "New"),
        &[("MDUpdateAction", "0"), ("OrderID", "ORDER-1")],
    );
    initial.set_exprtime(Some(4));
    initial.finalize();
    let renamed = with_identifiers(
        operation("quote", "IBM", "Y", 2, "Sell", "101", 3, "Replaced"),
        &[
            ("MDUpdateAction", "1"),
            ("MDEntryRefID", "X"),
            ("MDEntryPx", "101"),
            ("MDEntrySize", "3"),
        ],
    );
    let books = BookIterator::new([initial, renamed].into_iter(), 0, false)
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("expiry deletes the current entry after its reference has been resolved");
    assert_eq!(books.len(), 3);
    let previous = books[1].ask().live().next().unwrap();
    assert_eq!(previous.get_identifiers()["MDEntryID"], "Y");
    assert!(books[2].bid().is_empty());
    assert!(books[2].ask().is_empty());
    assert_eq!(books[2].get_currunix(), 4);
    let expired = &books[2].ask().deltas()[0];
    assert!(matches!(expired, MarketOperation::Order(_)));
    assert_eq!(expired.get_prevuuid(), Some(previous.get_curruuid()));
    assert_eq!(expired.get_identifiers()["MDEntryID"], "Y");
    assert!(!expired.get_state().is_live());
}

#[test]
fn partial_market_updates_promote_quotes_when_the_order_id_becomes_known() {
    let mut book = Book::new(1, "IBM");
    book.add_operations([with_identifiers(
        operation("quote", "IBM", "Q-1", 1, "Buy", "100", 2, "New"),
        &[("MDUpdateAction", "0")],
    )])
    .unwrap();
    let previous = book.bid().live().next().unwrap().clone();
    book.add_operations([with_identifiers(
        operation("order", "IBM", "Q-1", 2, "Buy", "0", 3, "Replaced"),
        &[
            ("MDUpdateAction", "1"),
            ("OrderID", "ORDER-1"),
            ("MDEntrySize", "3"),
        ],
    )])
    .unwrap();
    let live = book.bid().live().next().unwrap();
    assert!(matches!(live, MarketOperation::Order(_)));
    assert_eq!(live.get_price(), previous.get_price());
    assert_eq!(live.get_identifiers()["OrderID"], "ORDER-1");
    assert_eq!(live.get_prevuuid(), Some(previous.get_curruuid()));
    assert_eq!(live.get_seqnum(), previous.get_seqnum() + 1);
}

#[test]
fn contradictory_market_update_order_ids_refuse_without_removing_the_predecessor() {
    let mut book = Book::new(1, "IBM");
    book.add_operations([with_identifiers(
        operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New"),
        &[("OrderID", "ORDER-1"), ("MDUpdateAction", "0")],
    )])
    .unwrap();
    let before = book.clone();
    let error = book
        .add_operations([with_identifiers(
            operation("order", "IBM", "O-1", 2, "Sell", "101", 3, "Replaced"),
            &[
                ("MDUpdateAction", "1"),
                ("OrderID", "ORDER-2"),
                ("MDEntryPx", "101"),
                ("MDEntrySize", "3"),
            ],
        )])
        .expect_err("a continuation cannot replace a known order identity");
    assert!(
        matches!(&error, yggdryl::Error::InvalidRecord { path, .. } if path == "$.operation.identifiers.OrderID"),
        "{error}"
    );
    assert_eq!(book, before);
}

#[test]
fn executions_are_reported_beside_depth_and_propagate_the_latest_execution_clock() {
    let mut first = MarketEventData::at(10);
    first.set_crosscode("E-1".to_owned());
    first.set_symbolticker(Some("IBM".to_owned()));
    first.set_price("100".parse().unwrap());
    first.set_quantity(Decimal18::from_int(2));
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
fn a_trade_flattens_its_sorted_executions_without_entering_depth() {
    let buy = Execution::try_from(operation(
        "execution",
        "IBM",
        "E-BUY",
        10,
        "Buy",
        "100",
        2,
        "Filled",
    ))
    .unwrap();
    let sell = Execution::try_from(operation(
        "execution",
        "IBM",
        "E-SELL",
        10,
        "Sell",
        "101",
        3,
        "Filled",
    ))
    .unwrap();
    let mut event = MarketEventData::at(10);
    event.set_crosscode("T-1".to_owned());
    event.set_symbolticker(Some("IBM".to_owned()));
    event.set_state(State::read("Filled").unwrap());
    event.finalize();
    let trade = Trade::from_parts(event, vec![sell, buy]).unwrap();
    assert_eq!(
        trade
            .executions()
            .iter()
            .map(Element::get_crosscode)
            .collect::<Vec<_>>(),
        ["E-BUY", "E-SELL"]
    );
    let mut expected = trade.executions().to_vec();
    expected.sort_by_key(Element::get_curruuid);

    let mut book = Book::new(10, "IBM");
    book.add_operations([trade.into()]).unwrap();

    assert!(book.bid().is_empty());
    assert!(book.ask().is_empty());
    assert_eq!(book.executions(), expected);
    assert!(
        book.executions()
            .windows(2)
            .all(|pair| pair[0].get_curruuid() < pair[1].get_curruuid())
    );
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
    assert_eq!(replacement.get_price(), "101".parse().unwrap());
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
    snapshot.set_price("101".parse().unwrap());
    snapshot.set_quantity(Decimal18::from_int(5));
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
    assert_eq!(first.get_price(), "101".parse().unwrap());
    assert_eq!(first.get_quantity(), Decimal18::from_int(5));
    assert_eq!(live.next().unwrap().get_crosscode(), "O-OTHER");

    let only = BookIterator::new([snapshot_only].into_iter(), 0, false)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(only.get_snapunix(), Some(2));
    assert_eq!(only.bid().len(), 1);
    assert_eq!(
        only.bid().live().next().unwrap().get_price(),
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
fn snapshotted_trade_rebases_every_child_and_preserves_execution_time() {
    let mut buy = Execution::try_from(operation(
        "execution",
        "IBM",
        "E-BUY",
        2,
        "Buy",
        "101",
        4,
        "Trade",
    ))
    .unwrap();
    buy.set_execunix(Some(1));
    buy.finalize();
    let mut sell = Execution::try_from(operation(
        "execution",
        "IBM",
        "E-SELL",
        2,
        "Sell",
        "101",
        4,
        "Trade",
    ))
    .unwrap();
    sell.set_execunix(Some(2));
    sell.finalize();
    let mut root = MarketEventData::at(2);
    root.set_crosscode("T-1".to_owned());
    root.set_symbolticker(Some("IBM".to_owned()));
    root.set_snapunix(Some(3));
    root.set_state(State::read("Trade").unwrap());
    root.finalize();
    let trade = Trade::from_parts(root, vec![sell, buy]).unwrap();

    let book = BookIterator::new([MarketOperation::Trade(trade)].into_iter(), 0, false)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();

    assert_eq!(book.get_currunix(), 3);
    assert_eq!(book.get_snapunix(), Some(3));
    assert_eq!(book.executions().len(), 2);
    assert!(
        book.executions()
            .iter()
            .all(|execution| execution.get_currunix() == 3)
    );
    let mut execunix = book
        .executions()
        .iter()
        .map(Event::get_execunix)
        .collect::<Vec<_>>();
    execunix.sort_unstable();
    assert_eq!(execunix, [Some(1), Some(2)]);
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
fn composite_identity_consumes_nested_uuid_without_rehashing_nested_content() {
    let canonical = operation("quote", "IBM", "B-1", 1, "Buy", "100", 1, "New");
    let mut left_operation = canonical.clone();
    let mut right_operation = canonical;
    let operation_uuid = left_operation.get_curruuid();
    left_operation.set_currhashcode(11);
    right_operation.set_currhashcode(29);
    left_operation.set_curruuid(operation_uuid);
    right_operation.set_curruuid(operation_uuid);
    assert_eq!(
        left_operation.get_curruuid(),
        right_operation.get_curruuid()
    );
    assert_ne!(
        left_operation.get_currhashcode(),
        right_operation.get_currhashcode()
    );

    let mut left = Book::new(1, "IBM");
    left.add_operations([left_operation]).unwrap();
    let mut right = Book::new(1, "IBM");
    right.add_operations([right_operation]).unwrap();

    assert_eq!(left.bid().get_curruuid(), right.bid().get_curruuid());
    assert_eq!(
        left.bid().get_currhashcode(),
        right.bid().get_currhashcode()
    );
    assert_eq!(left.get_curruuid(), right.get_curruuid());
    assert_eq!(left.get_currhashcode(), right.get_currhashcode());
}

#[test]
fn restating_rederives_the_book_identity_after_holder_restatement() {
    let mut live = Book::new(1, "IBM");
    live.add_operations([
        operation("quote", "IBM", "B-1", 1, "Buy", "100", 2, "New"),
        operation("quote", "IBM", "A-1", 1, "Sell", "102", 4, "New"),
    ])
    .unwrap();
    live.set_recdunix(Some(12));
    live.set_refrecdunix(Some(12));
    live.finalize();
    let mut repeated = live.clone();
    repeated.set_recdunix(Some(8));

    let restated = repeated.restating(&live);
    assert_eq!(restated.get_recdunix(), Some(8));
    let mut canonical = restated.clone();
    canonical.finalize();
    assert_eq!(restated.get_curruuid(), canonical.get_curruuid());
    assert_eq!(restated.get_currhashcode(), canonical.get_currhashcode());
    assert_eq!(restated.bid(), canonical.bid());
    assert_eq!(restated.ask(), canonical.ask());
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
    let mut canonical_bid = book.bid().clone();
    canonical_bid.finalize();
    assert_eq!(book.bid(), &canonical_bid);

    book.add_operations([operation(
        "execution",
        "IBM",
        "E-2",
        3,
        "Buy",
        "101",
        1,
        "Filled",
    )])
    .unwrap();
    assert!(book.bid().deltas().is_empty());
    assert!(book.ask().deltas().is_empty());
    let mut canonical_bid = book.bid().clone();
    canonical_bid.finalize();
    let mut canonical_ask = book.ask().clone();
    canonical_ask.finalize();
    assert_eq!(book.bid(), &canonical_bid);
    assert_eq!(book.ask(), &canonical_ask);
    let mut canonical_book = book.clone();
    canonical_book.finalize();
    assert_eq!(book.get_curruuid(), canonical_book.get_curruuid());
    assert_eq!(book.get_currhashcode(), canonical_book.get_currhashcode());
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
    bid_operation.set_price(bid);
    bid_operation.set_quantity(Decimal18::MAX);
    bid_operation.finalize();
    let mut ask_operation = operation("quote", "IBM", "A", 1, "Sell", "0", 1, "New");
    ask_operation.set_price(ask);
    ask_operation.set_quantity(Decimal18::MAX);
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
