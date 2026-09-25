use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use smol_str::SmolStr;
use yggdryl::graph::book::{ENTRY_ID, ENTRY_REF_ID};
use yggdryl::graph::{
    Book, BookControl, BookInput, BookIterator, BookRef, BookSide, Element, Event, GLOBAL_SYMBOL,
    Market, MarketEventData, MarketOperation, MdUpdateAction, Operation, OperationEventData,
    OperationKind, Trade,
};
use yggdryl::{Ccy, Decimal18, Side, State, Unit};

/// The alternate-identifier key an order's `OrderID(37)` is held under.
const ORDER_ID: &str = "ORDERID";

/// One synthetic operation of `kind` - `order`, `quote` or `execution` -
/// going by `identity` as its cross code and its `MDENTRYID`, finalized.
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
    let mut data = OperationEventData::at(unix);
    data.set_crosscode(identity.to_owned());
    data.set_ticker(Some(SmolStr::new(symbol)));
    data.set_side(Side::read(side).unwrap());
    data.set_price(Some(price.parse().unwrap()));
    data.set_quantity(Some(Decimal18::from_int(quantity)));
    data.set_currency(Ccy::new("USD").unwrap());
    data.set_unit(Unit::new("share").unwrap());
    data.set_state(State::read(state).unwrap());
    data.insert_altid(ENTRY_ID, identity).unwrap();
    let kind = OperationKind::read(kind).expect("an operation kind");
    let mut operation =
        MarketOperation::new(kind, data).expect("an order, a quote or an execution");
    operation.finalize();
    operation
}

/// `operation` carrying the book-control facts `book` states, finalized
/// again around them.
fn with_book(mut operation: MarketOperation, book: BookRef) -> MarketOperation {
    operation.set_book(Some(book));
    operation.finalize();
    operation
}

/// `operation` going by `altids` too, each replacing the key where it held
/// one already, finalized again around them.
fn with_altids(mut operation: MarketOperation, altids: &[(&str, &str)]) -> MarketOperation {
    for (key, value) in altids {
        operation.remove_altid(key).unwrap();
        operation.insert_altid(key, value).unwrap();
    }
    operation.finalize();
    operation
}

/// `operation` without its `MDENTRYID`: an anonymous entry.
fn anonymous(mut operation: MarketOperation) -> MarketOperation {
    operation.remove_altid(ENTRY_ID).unwrap();
    operation.finalize();
    operation
}

/// A book control naming only `scope`.
fn scoped(scope: &str) -> BookRef {
    BookRef {
        scope: Some(SmolStr::new(scope)),
        ..BookRef::default()
    }
}

/// A book control stating only `action`.
fn acting(action: MdUpdateAction) -> BookRef {
    BookRef {
        action: Some(action),
        ..BookRef::default()
    }
}

/// A book control stating `action` in `scope`.
fn acting_in(action: MdUpdateAction, scope: &str) -> BookRef {
    BookRef {
        action: Some(action),
        scope: Some(SmolStr::new(scope)),
        ..BookRef::default()
    }
}

fn decimal(text: &str) -> Decimal18 {
    text.parse().unwrap()
}

/// The operations as the inputs a book takes.
fn inputs<const N: usize>(operations: [MarketOperation; N]) -> [BookInput; N] {
    operations.map(BookInput::from)
}

/// The event of a full-snapshot control at `unix`, named `identity`, on
/// IBM.
fn reset_event(unix: i64, identity: &str) -> MarketEventData {
    let mut reset = MarketEventData::at(unix);
    reset.set_crosscode(identity.to_owned());
    reset.set_ticker(Some(SmolStr::new("IBM")));
    reset.set_state(State::read("New").unwrap());
    reset.finalize();
    reset
}

/// What a book states of one operation, compared never by identity: the
/// price, the quantity and the names it goes by.
type Facts = (Option<Decimal18>, Option<Decimal18>, Vec<(String, String)>);

/// [`Facts`] for each operation, in the order given.
fn facts<'a>(operations: impl Iterator<Item = &'a MarketOperation>) -> Vec<Facts> {
    operations
        .map(|operation| {
            (
                operation.get_price(),
                operation.get_quantity(),
                operation
                    .get_altids()
                    .iter()
                    .map(|(key, value)| (key.to_owned(), value.to_owned()))
                    .collect(),
            )
        })
        .collect()
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
        side.live().map(Market::get_price).collect::<Vec<_>>(),
        [
            Some(decimal("101")),
            Some(decimal("101")),
            Some(decimal("100"))
        ]
    );
    assert_eq!(side.best_price(), Some(decimal("101")));
    assert_eq!(side.best_quantity(), Some(Decimal18::from_int(7)));
    assert_eq!(side.deltas().len(), 3);

    side.add_operation(operation(
        "quote", "IBM", "Q-1", 4, "Buy", "101", 0, "Canceled",
    ))
    .unwrap();
    assert_eq!(side.best_quantity(), Some(Decimal18::from_int(4)));
}

#[test]
fn a_side_refuses_an_operation_stating_no_price() {
    // A level is a price: an operation stating none has no place on a
    // side, and nothing invents one for it - not a zero, not a last
    // executed price it may carry.
    let mut unpriced = operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New");
    unpriced.set_price(None);
    // The lane the fixture already filled would state the price back;
    // cleared, finalizing fills it from the facts left, and a price is not
    // among them.
    unpriced.set_bid(None);
    unpriced.set_lastpx(Some(decimal("100")));
    unpriced.finalize();
    assert_eq!(unpriced.get_price(), None);
    assert_eq!(unpriced.get_bid().and_then(|lane| lane.price), None);

    let mut side = BookSide::new(Side::read("Buy").unwrap()).unwrap();
    let error = side
        .add_operation(unpriced.clone())
        .unwrap_err()
        .to_string();
    assert!(error.contains("$.operation.price"), "{error}");
    assert!(error.contains("expected a price on a book side"), "{error}");
    assert!(
        side.is_empty(),
        "a refused operation leaves the side as it was"
    );

    let mut book = Book::new(1, "IBM");
    let error = book
        .add_operations(inputs([unpriced]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("expected a price on a book side"), "{error}");
    assert!(book.bid().is_empty() && book.ask().is_empty());
}

#[test]
fn book_exposes_bbo_midpoint_and_two_value_quantity_median() {
    let mut book = Book::new(10, "IBM");
    book.add_operations(inputs([
        operation("quote", "IBM", "B-1", 10, "Buy", "100", 8, "New"),
        operation("quote", "IBM", "A-1", 10, "Sell", "102", 4, "New"),
    ]))
    .unwrap();

    assert_eq!(book.bbo_midpoint(), Some(decimal("101")));
    assert_eq!(book.median_quantity(), Some(Decimal18::from_int(6)));
    assert_eq!(book.get_price(), Some(decimal("101")));
    assert_eq!(book.get_quantity(), Some(Decimal18::from_int(6)));
    assert_eq!(book.bid().best_quantity(), Some(Decimal18::from_int(8)));
    assert_eq!(book.ask().best_quantity(), Some(Decimal18::from_int(4)));

    book.add_operations(inputs([operation(
        "quote", "IBM", "B-2", 11, "Buy", "103", 1, "New",
    )]))
    .unwrap();
    assert!(book.is_crossed());
    assert_eq!(book.bbo_midpoint(), None);
    assert_eq!(
        book.get_price(),
        None,
        "a crossed book has no midpoint, so it states no price"
    );
}

#[test]
fn full_snapshot_replaces_only_its_scope_atomically() {
    let mut book = Book::new(1, "IBM");
    let first = with_book(
        operation("quote", "IBM", "B-1", 1, "Buy", "100", 2, "New"),
        scoped("PRIMARY"),
    );
    let other = with_book(
        operation("quote", "IBM", "B-X", 1, "Buy", "99", 9, "New"),
        scoped("OTHER"),
    );
    book.add_operations(inputs([first, other])).unwrap();
    let previous = book.clone();

    let replacement = with_book(
        operation("quote", "IBM", "B-2", 2, "Buy", "101", 3, "New"),
        acting_in(MdUpdateAction::Snapshot, "PRIMARY"),
    );
    book.add_operations(inputs([replacement.clone()])).unwrap();

    let identities: Vec<_> = book
        .bid()
        .live()
        .map(|operation| operation.get_crosscode())
        .collect();
    assert_eq!(identities, ["B-2", "B-X"]);

    let mut update = Book::new(2, "IBM");
    update.add_operations(inputs([replacement])).unwrap();
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
            .map(|book| (book.get_currunix(), book.get_ticker().unwrap()))
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
            .all(|book| book.get_ticker() == Some(GLOBAL_SYMBOL))
    );
}

#[test]
fn iterator_emits_a_completed_timestamp_before_a_source_error_then_fuses() {
    struct Counted {
        source: std::vec::IntoIter<yggdryl::Result<BookInput>>,
        pulled: Arc<AtomicUsize>,
    }

    impl Iterator for Counted {
        type Item = yggdryl::Result<BookInput>;

        fn next(&mut self) -> Option<Self::Item> {
            let item = self.source.next()?;
            self.pulled.fetch_add(1, Ordering::SeqCst);
            Some(item)
        }
    }

    let pulled = Arc::new(AtomicUsize::new(0));
    let source = vec![
        Ok(operation("quote", "IBM", "IBM-B", 1_000_000, "Buy", "100", 2, "New").into()),
        Ok(operation("quote", "IBM", "IBM-A", 1_000_000, "Sell", "102", 4, "New").into()),
        Err(yggdryl::Error::InvalidRecord {
            path: "$[2]".into(),
            reason: "broken operation source".into(),
        }),
        Ok(operation(
            "quote", "IBM", "IBM-LATE", 2_000_000, "Buy", "101", 1, "New",
        )
        .into()),
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
fn a_snapshot_is_the_same_book_with_every_living_order_and_nothing_else() {
    // Two orders live, then one dies before the next tick: the tick's book is
    // the same struct with every living order kept - the survivor - and
    // everything else purged: no deltas, no executions, and the dead order
    // nowhere. The book the order died in still carries it, as a delta.
    let operations = vec![
        operation("order", "IBM", "O-1", 1_000_000, "Buy", "100", 2, "New"),
        operation("order", "IBM", "O-2", 1_500_000, "Buy", "101", 3, "New"),
        operation(
            "order", "IBM", "O-2", 2_500_000, "Buy", "101", 3, "Canceled",
        ),
        // A later order, so the walk crosses the tick after the death.
        operation("order", "IBM", "O-3", 3_500_000, "Buy", "99", 1, "New"),
    ];
    let books = BookIterator::new(operations.clone().into_iter(), 1, false)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let at = |unix: i64| {
        books
            .iter()
            .find(|book| book.get_currunix() == unix)
            .unwrap_or_else(|| panic!("a book at {unix}"))
    };
    let died = at(2_500_000);
    assert_eq!(died.bid().live().count(), 1, "the survivor stays live");
    assert_eq!(
        died.bid().deltas().len(),
        1,
        "the cancel is the delta of the book it died in"
    );
    let snapshot = at(3_000_000);
    assert_eq!(snapshot.get_snapunix(), Some(3_000_000));
    assert_eq!(
        snapshot
            .bid()
            .live()
            .map(|held| held.get_crosscode().to_owned())
            .collect::<Vec<_>>(),
        ["O-1"],
        "every living order, and no dead one"
    );
    assert!(
        snapshot.bid().deltas().is_empty(),
        "nothing but the living orders"
    );
    assert!(snapshot.executions().is_empty());
    assert!(snapshot.ask().live().next().is_none());

    // With no grid, every emitted book keeps all living orders beside its
    // own deltas: nothing is ever purged between books.
    let books = BookIterator::new(operations.into_iter(), 0, false)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(books.len(), 4);
    assert!(books.iter().all(|book| book.get_snapunix().is_none()));
    assert_eq!(books[1].bid().live().count(), 2, "both live at 1.5 ms");
    assert_eq!(books[2].bid().live().count(), 1, "the survivor at 2.5 ms");
    assert_eq!(books[2].bid().deltas().len(), 1);
    assert_eq!(
        books[3].bid().live().count(),
        2,
        "the survivor and the newcomer at 3.5 ms"
    );
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
                book.get_ticker().unwrap(),
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
        .add_operations(inputs([operation(
            "quote", "IBM", "B", 1_000_001, "Buy", "100", 1, "New",
        )]))
        .unwrap();
    let mut second = first.clone();
    second.set_currunix(1_000_002);
    second.finalize();
    assert_ne!(first.get_curruuid(), second.get_curruuid());
}

#[test]
fn updates_follow_the_live_entry_and_atomic_failures_leave_the_book_unchanged() {
    let mut book = Book::new(1, "IBM");
    book.add_operations(inputs([operation(
        "order", "IBM", "O-1", 1, "Buy", "100", 2, "New",
    )]))
    .unwrap();
    let first = book.bid().live().next().unwrap().clone();

    book.add_operations(inputs([operation(
        "order", "IBM", "O-1", 2, "Buy", "101", 3, "Replaced",
    )]))
    .unwrap();
    let replacement = book.bid().live().next().unwrap();
    assert_eq!(replacement.get_seqnum(), 1);
    assert_eq!(replacement.get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(replacement.get_prevpx(), first.get_price());
    assert_eq!(replacement.get_prevqty(), first.get_quantity());
    assert_eq!(book.get_seqnum(), 1);

    let before = book.clone();
    let error = book
        .add_operations(inputs([
            operation("quote", "IBM", "B-2", 3, "Buy", "99", 1, "New"),
            operation("quote", "IBM", "B-3", 4, "Buy", "98", 1, "New"),
        ]))
        .expect_err("one atomic group has one timestamp");
    assert!(error.to_string().contains("same currunix"));
    assert_eq!(book, before);
}

#[test]
fn partial_updates_require_a_predecessor_or_complete_values_and_refs_precede_destination() {
    let mut book = Book::new(1, "IBM");
    let partial = with_book(
        operation("order", "IBM", "MISSING", 1, "Buy", "101", 3, "Replaced"),
        acting(MdUpdateAction::Change),
    );
    let error = book.add_operations(inputs([partial])).unwrap_err();
    assert!(
        matches!(&error, yggdryl::Error::InvalidRecord { path, .. } if path == "$.operation.book.mdentrypx"),
        "{error}"
    );
    assert!(book.bid().is_empty());

    book.add_operations(inputs([
        operation("order", "IBM", "X", 1, "Buy", "100", 1, "New"),
        operation("order", "IBM", "Y", 1, "Buy", "99", 2, "New"),
    ]))
    .unwrap();
    let before = book.clone();
    let collision = with_book(
        with_altids(
            operation("order", "IBM", "Y", 2, "Buy", "101", 4, "Replaced"),
            &[(ENTRY_REF_ID, "X")],
        ),
        BookRef {
            action: Some(MdUpdateAction::Change),
            entry_px: Some(decimal("101")),
            entry_size: Some(decimal("4")),
            ..BookRef::default()
        },
    );
    let error = book
        .add_operations(inputs([collision]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("MDENTRYID"), "{error}");
    assert_eq!(book, before);

    let unresolved = with_book(
        with_altids(
            operation("order", "IBM", "Z", 2, "Buy", "102", 5, "Replaced"),
            &[(ENTRY_REF_ID, "ABSENT")],
        ),
        BookRef {
            action: Some(MdUpdateAction::Change),
            entry_px: Some(decimal("102")),
            entry_size: Some(decimal("5")),
            ..BookRef::default()
        },
    );
    let error = book
        .add_operations(inputs([unresolved]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("MDENTRYREFID"), "{error}");
    assert_eq!(book, before);
}

#[test]
fn partial_market_updates_continue_orders_without_restating_order_id() {
    let mut book = Book::new(1, "IBM");
    book.add_operations(inputs([with_book(
        with_altids(
            operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New"),
            &[(ORDER_ID, "ORDER-1")],
        ),
        acting(MdUpdateAction::New),
    )]))
    .unwrap();
    let previous = book.bid().live().next().unwrap().clone();

    book.add_operations(inputs([with_book(
        operation("quote", "IBM", "O-1", 2, "Buy", "0", 3, "Replaced"),
        BookRef {
            action: Some(MdUpdateAction::Change),
            entry_size: Some(decimal("3")),
            ..BookRef::default()
        },
    )]))
    .unwrap();

    let live = book.bid().live().next().unwrap();
    assert_eq!(live.kind(), OperationKind::Order);
    assert_eq!(live.get_altids().get(ORDER_ID), Some("ORDER-1"));
    assert_eq!(live.get_price(), previous.get_price());
    assert_eq!(live.get_quantity(), Some(Decimal18::from_int(3)));
    assert_eq!(live.get_prevuuid(), Some(previous.get_curruuid()));
    assert_eq!(live.get_prevpx(), previous.get_price());
    assert_eq!(live.get_prevqty(), previous.get_quantity());
    assert_eq!(live.get_seqnum(), previous.get_seqnum() + 1);
    let mut finalized = live.clone();
    finalized.finalize();
    assert_eq!(*live, finalized);
}

#[test]
fn referenced_market_update_refuses_an_occupied_destination_on_the_other_side() {
    let mut book = Book::new(1, "IBM");
    book.add_operations(inputs([
        operation("quote", "IBM", "X", 1, "Buy", "100", 2, "New"),
        operation("quote", "IBM", "Y", 1, "Sell", "101", 3, "New"),
    ]))
    .unwrap();
    let before = book.clone();
    let error = book
        .add_operations(inputs([with_book(
            with_altids(
                operation("quote", "IBM", "Y", 2, "Buy", "99", 4, "Replaced"),
                &[(ENTRY_REF_ID, "X")],
            ),
            BookRef {
                action: Some(MdUpdateAction::Change),
                entry_px: Some(decimal("99")),
                entry_size: Some(decimal("4")),
                ..BookRef::default()
            },
        )]))
        .expect_err("a reference cannot replace a different live destination on either side");
    assert!(
        matches!(&error, yggdryl::Error::InvalidRecord { path, .. } if path == "$.operation.altids.MDENTRYID"),
        "{error}"
    );
    assert_eq!(book, before);
}

#[test]
fn partial_market_updates_move_between_sides_with_their_predecessor() {
    let mut book = Book::new(1, "IBM");
    book.add_operations(inputs([with_book(
        with_altids(
            operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New"),
            &[(ORDER_ID, "ORDER-1")],
        ),
        acting(MdUpdateAction::New),
    )]))
    .unwrap();
    let previous = book.bid().live().next().unwrap().clone();

    book.add_operations(inputs([with_book(
        with_altids(
            operation("quote", "IBM", "O-2", 2, "Sell", "0", 3, "Replaced"),
            &[(ENTRY_REF_ID, "O-1")],
        ),
        BookRef {
            action: Some(MdUpdateAction::Change),
            entry_size: Some(decimal("3")),
            ..BookRef::default()
        },
    )]))
    .unwrap();
    assert!(book.bid().is_empty());
    let live = book.ask().live().next().unwrap();
    assert_eq!(live.kind(), OperationKind::Order);
    assert_eq!(live.get_price(), previous.get_price());
    assert_eq!(live.get_quantity(), Some(Decimal18::from_int(3)));
    assert_eq!(live.get_prevuuid(), Some(previous.get_curruuid()));
    assert_eq!(live.get_prevpx(), previous.get_price());
    assert_eq!(live.get_prevqty(), previous.get_quantity());
    assert_eq!(live.get_seqnum(), previous.get_seqnum() + 1);
    assert_eq!(live.get_altids().get(ENTRY_ID), Some("O-2"));
    let previous = live.clone();

    book.add_operations(inputs([with_book(
        operation("quote", "IBM", "O-2", 3, "Buy", "101", 0, "Replaced"),
        BookRef {
            action: Some(MdUpdateAction::Overlay),
            entry_px: Some(decimal("101")),
            ..BookRef::default()
        },
    )]))
    .unwrap();
    assert!(book.ask().is_empty());
    let live = book.bid().live().next().unwrap();
    assert_eq!(live.kind(), OperationKind::Order);
    assert_eq!(live.get_price(), Some(Decimal18::from_int(101)));
    assert_eq!(live.get_quantity(), previous.get_quantity());
    assert_eq!(live.get_prevuuid(), Some(previous.get_curruuid()));
    assert_eq!(live.get_seqnum(), previous.get_seqnum() + 1);
}

#[test]
fn renamed_market_entry_expires_using_its_current_identity() {
    let mut initial = with_book(
        with_altids(
            operation("order", "IBM", "X", 1, "Buy", "100", 2, "New"),
            &[(ORDER_ID, "ORDER-1")],
        ),
        acting(MdUpdateAction::New),
    );
    initial.set_exprtime(Some(4));
    initial.finalize();
    let renamed = with_book(
        with_altids(
            operation("quote", "IBM", "Y", 2, "Sell", "101", 3, "Replaced"),
            &[(ENTRY_REF_ID, "X")],
        ),
        BookRef {
            action: Some(MdUpdateAction::Change),
            entry_px: Some(decimal("101")),
            entry_size: Some(decimal("3")),
            ..BookRef::default()
        },
    );
    let books = BookIterator::new([initial, renamed].into_iter(), 0, false)
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("expiry deletes the current entry after its reference has been resolved");
    assert_eq!(books.len(), 3);
    let previous = books[1].ask().live().next().unwrap();
    assert_eq!(previous.get_altids().get(ENTRY_ID), Some("Y"));
    assert!(books[2].bid().is_empty());
    assert!(books[2].ask().is_empty());
    assert_eq!(books[2].get_currunix(), 4);
    let expired = &books[2].ask().deltas()[0];
    assert_eq!(expired.kind(), OperationKind::Order);
    assert_eq!(expired.get_prevuuid(), Some(previous.get_curruuid()));
    assert_eq!(expired.get_altids().get(ENTRY_ID), Some("Y"));
    assert!(!expired.get_state().is_live());
}

#[test]
fn partial_market_updates_promote_quotes_when_the_order_id_becomes_known() {
    let mut book = Book::new(1, "IBM");
    book.add_operations(inputs([with_book(
        operation("quote", "IBM", "Q-1", 1, "Buy", "100", 2, "New"),
        acting(MdUpdateAction::New),
    )]))
    .unwrap();
    let previous = book.bid().live().next().unwrap().clone();
    book.add_operations(inputs([with_book(
        with_altids(
            operation("order", "IBM", "Q-1", 2, "Buy", "0", 3, "Replaced"),
            &[(ORDER_ID, "ORDER-1")],
        ),
        BookRef {
            action: Some(MdUpdateAction::Change),
            entry_size: Some(decimal("3")),
            ..BookRef::default()
        },
    )]))
    .unwrap();
    let live = book.bid().live().next().unwrap();
    assert_eq!(live.kind(), OperationKind::Order);
    assert_eq!(live.get_price(), previous.get_price());
    assert_eq!(live.get_altids().get(ORDER_ID), Some("ORDER-1"));
    assert_eq!(live.get_prevuuid(), Some(previous.get_curruuid()));
    assert_eq!(live.get_seqnum(), previous.get_seqnum() + 1);
}

#[test]
fn contradictory_market_update_order_ids_refuse_without_removing_the_predecessor() {
    let mut book = Book::new(1, "IBM");
    book.add_operations(inputs([with_book(
        with_altids(
            operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New"),
            &[(ORDER_ID, "ORDER-1")],
        ),
        acting(MdUpdateAction::New),
    )]))
    .unwrap();
    let before = book.clone();
    let error = book
        .add_operations(inputs([with_book(
            with_altids(
                operation("order", "IBM", "O-1", 2, "Sell", "101", 3, "Replaced"),
                &[(ORDER_ID, "ORDER-2")],
            ),
            BookRef {
                action: Some(MdUpdateAction::Change),
                entry_px: Some(decimal("101")),
                entry_size: Some(decimal("3")),
                ..BookRef::default()
            },
        )]))
        .expect_err("a continuation cannot replace a known order identity");
    assert!(
        matches!(&error, yggdryl::Error::InvalidRecord { path, .. } if path == "$.operation.altids.ORDERID"),
        "{error}"
    );
    assert_eq!(book, before);
}

#[test]
fn executions_are_reported_beside_depth_and_propagate_the_latest_execution_clock() {
    let mut first = OperationEventData::at(10);
    first.set_crosscode("E-1".to_owned());
    first.set_ticker(Some(SmolStr::new("IBM")));
    first.set_price(Some(decimal("100")));
    first.set_quantity(Some(Decimal18::from_int(2)));
    first.set_state(State::read("Filled").unwrap());
    first.set_execunix(Some(7));
    first.finalize();
    let mut second = first.clone();
    second.set_crosscode("E-2".to_owned());
    second.set_execunix(Some(9));
    second.finalize();

    let mut book = Book::new(10, "IBM");
    book.add_operations([
        MarketOperation::execution(first).into(),
        MarketOperation::execution(second).into(),
    ])
    .unwrap();
    assert_eq!(book.executions().len(), 2);
    assert!(book.bid().is_empty());
    assert!(book.ask().is_empty());
    assert_eq!(book.get_execunix(), Some(9));
}

#[test]
fn a_trade_flattens_its_sorted_executions_without_entering_depth() {
    let buy = operation("execution", "IBM", "E-BUY", 10, "Buy", "100", 2, "Filled");
    let sell = operation("execution", "IBM", "E-SELL", 10, "Sell", "101", 3, "Filled");
    let mut event = OperationEventData::at(10);
    event.set_crosscode("T-1".to_owned());
    event.set_ticker(Some(SmolStr::new("IBM")));
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
    let mut execution = OperationEventData::at(1);
    execution.set_crosscode("E-1".to_owned());
    execution.set_ticker(Some(SmolStr::new("IBM")));
    execution.set_state(State::read("Filled").unwrap());
    execution.set_exprtime(Some(2));
    execution.finalize();

    let books = BookIterator::new(
        [
            live,
            MarketOperation::execution(execution),
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
    assert_eq!(replacement.get_price(), Some(decimal("101")));
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
    let mut expiring = with_book(
        operation("quote", "IBM", "EXPIRING", 1, "Buy", "101", 1, "New"),
        acting_in(MdUpdateAction::Snapshot, "PRIMARY"),
    );
    expiring.set_exprtime(Some(2));
    expiring.finalize();
    let standing = with_book(
        operation("quote", "IBM", "STANDING", 1, "Buy", "100", 1, "New"),
        acting_in(MdUpdateAction::Snapshot, "PRIMARY"),
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
    let other_scope = with_book(
        operation("order", "IBM", "O-OTHER", 1, "Buy", "98", 4, "New"),
        scoped("OTHER"),
    );
    let mut snapshot = first.clone();
    snapshot.set_price(Some(decimal("101")));
    snapshot.set_quantity(Some(Decimal18::from_int(5)));
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
    assert_eq!(first.get_price(), Some(decimal("101")));
    assert_eq!(first.get_quantity(), Some(Decimal18::from_int(5)));
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
        Some(decimal("101"))
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
    let mut buy = operation("execution", "IBM", "E-BUY", 2, "Buy", "101", 4, "Trade");
    buy.set_execunix(Some(1));
    buy.finalize();
    let mut sell = operation("execution", "IBM", "E-SELL", 2, "Sell", "101", 4, "Trade");
    sell.set_execunix(Some(2));
    sell.finalize();
    let mut root = OperationEventData::at(2);
    root.set_crosscode("T-1".to_owned());
    root.set_ticker(Some(SmolStr::new("IBM")));
    root.set_snapunix(Some(3));
    root.set_state(State::read("Trade").unwrap());
    root.finalize();
    let trade = Trade::from_parts(root, vec![sell, buy]).unwrap();

    let book = BookIterator::new([BookInput::Trade(trade)].into_iter(), 0, false)
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
    let error = direct
        .add_operations(inputs([execution]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("operation[0].snapunix"), "{error}");

    let mut reset = MarketEventData::at(3);
    reset.set_crosscode("RESET-FUTURE".to_owned());
    reset.set_ticker(Some(SmolStr::new("IBM")));
    reset.set_snapunix(Some(2));
    reset.finalize();
    let control = BookControl::snapshot(reset, Some(SmolStr::new("PRIMARY")));
    let error = BookIterator::new([BookInput::Snapshot(control)].into_iter(), 0, false)
        .unwrap()
        .next()
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(error.contains("snapshot.controls[0].currunix"), "{error}");
}

#[test]
fn an_explicit_empty_snapshot_replaces_only_its_partition() {
    let primary = with_book(
        operation("quote", "IBM", "PRIMARY", 1, "Buy", "100", 1, "New"),
        scoped("PRIMARY"),
    );
    let other = with_book(
        operation("quote", "IBM", "OTHER", 1, "Buy", "99", 1, "New"),
        scoped("OTHER"),
    );
    let mut reset = reset_event(2, "RESET-OTHER");
    reset.set_seqnum(9);
    reset.set_snapunix(Some(2));
    reset.finalize();
    let control = BookControl::snapshot(reset, Some(SmolStr::new("OTHER")));

    let books = BookIterator::new(
        [primary.into(), other.into(), BookInput::Snapshot(control)].into_iter(),
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
    let book = |identity: &str, side: &str, price: &str, recdunix: i64, feed: &str| {
        let mut book = Book::new(100, "IBM");
        book.add_operations(inputs([operation(
            "quote", "IBM", identity, 100, side, price, 2, "New",
        )]))
        .unwrap();
        book.set_recdunix(Some(recdunix));
        book.set_metadata(Some(BTreeMap::from([(
            SmolStr::new("Feed"),
            SmolStr::new(feed),
        )])));
        book.finalize();
        book
    };
    let mut older = book("B-1", "Buy", "100", 20, "OLDER");
    older.set_execunix(Some(7));
    older.finalize();
    let mut latest = book("A-1", "Sell", "102", 30, "LATEST");
    latest.set_execunix(Some(9));
    latest.finalize();

    let left = older.clone().merge_with(&latest).unwrap();
    let right = latest.clone().merge_with(&older).unwrap();
    for merged in [&left, &right] {
        assert_eq!(merged.bid().len(), 1);
        assert_eq!(merged.ask().len(), 1);
        // The later recording (30) is the reference and has the word on a
        // conflict; the clocks fold to the earliest either book knows.
        assert_eq!(merged.get_metadata()["Feed"], "LATEST");
        assert_eq!(merged.get_execunix(), Some(7));
        assert_eq!(merged.get_recdunix(), Some(20));
    }
    assert_eq!(left.get_curruuid(), right.get_curruuid());

    // No clock keeps the reference's own 30, so the merged book ranks by the
    // earliest recording (20) it holds: a third book recorded at 25 leads
    // it, although it would not lead `latest` alone.
    let between = book("B-2", "Buy", "99", 25, "BETWEEN");
    let alone = latest.merge_with(&between).unwrap();
    assert_eq!(alone.get_metadata()["Feed"], "LATEST");
    assert_eq!(alone.get_recdunix(), Some(25));
    let folded = left.merge_with(&between).unwrap();
    assert_eq!(folded.get_metadata()["Feed"], "BETWEEN");
    assert_eq!(folded.get_recdunix(), Some(20));
    assert_eq!(folded.bid().len(), 2);
    assert_eq!(folded.ask().len(), 1);
}

#[test]
fn operation_kind_participates_in_book_identity() {
    let mut order_book = Book::new(1, "IBM");
    order_book
        .add_operations(inputs([operation(
            "order", "IBM", "B-1", 1, "Buy", "100", 1, "New",
        )]))
        .unwrap();
    let mut quote_book = Book::new(1, "IBM");
    quote_book
        .add_operations(inputs([operation(
            "quote", "IBM", "B-1", 1, "Buy", "100", 1, "New",
        )]))
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
    left.add_operations(inputs([left_operation])).unwrap();
    let mut right = Book::new(1, "IBM");
    right.add_operations(inputs([right_operation])).unwrap();

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
    live.add_operations(inputs([
        operation("quote", "IBM", "B-1", 1, "Buy", "100", 2, "New"),
        operation("quote", "IBM", "A-1", 1, "Sell", "102", 4, "New"),
    ]))
    .unwrap();
    live.set_recdunix(Some(12));
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
    book.add_operations(inputs([
        with_book(
            operation("quote", "IBM", "SAME", 1, "Buy", "100", 1, "New"),
            scoped("PRIMARY"),
        ),
        with_book(
            operation("quote", "MSFT", "SAME", 1, "Buy", "200", 2, "New"),
            scoped("PRIMARY"),
        ),
        with_book(
            operation("quote", "IBM", "SAME", 1, "Buy", "99", 3, "New"),
            scoped("OTHER"),
        ),
    ]))
    .unwrap();
    assert_eq!(book.bid().len(), 3);
}

#[test]
fn a_global_snapshot_replaces_only_its_source_symbol_and_scope() {
    let mut book = Book::new(1, GLOBAL_SYMBOL);
    book.add_operations(inputs([
        with_book(
            operation("quote", "IBM", "IBM-OLD", 1, "Buy", "100", 1, "New"),
            scoped("PRIMARY"),
        ),
        with_book(
            operation("quote", "MSFT", "MS-LIVE", 1, "Buy", "200", 2, "New"),
            scoped("PRIMARY"),
        ),
    ]))
    .unwrap();
    book.add_operations(inputs([with_book(
        operation("quote", "IBM", "IBM-NEW", 2, "Buy", "101", 3, "New"),
        acting_in(MdUpdateAction::Snapshot, "PRIMARY"),
    )]))
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
    book.add_operations(inputs([
        with_book(
            operation("quote", "IBM", "P-1", 1, "Buy", "101", 1, "New"),
            scoped("PRIMARY"),
        ),
        with_book(
            operation("quote", "IBM", "O-1", 1, "Buy", "100", 2, "New"),
            scoped("OTHER"),
        ),
        with_book(
            operation("quote", "IBM", "P-2", 1, "Buy", "99", 3, "New"),
            scoped("PRIMARY"),
        ),
    ]))
    .unwrap();

    let deletion = with_book(
        operation("quote", "IBM", "DELETE", 2, "Buy", "0", 0, "Canceled"),
        BookRef {
            action: Some(MdUpdateAction::DeleteThru),
            scope: Some(SmolStr::new("PRIMARY")),
            position: Some(1),
            ..BookRef::default()
        },
    );
    book.add_operations(inputs([deletion])).unwrap();
    let remaining = book
        .bid()
        .live()
        .map(Element::get_crosscode)
        .collect::<Vec<_>>();
    assert_eq!(remaining, ["O-1", "P-2"]);

    let before = book.clone();
    let invalid = with_book(
        operation("quote", "IBM", "DELETE", 3, "Buy", "0", 0, "Canceled"),
        BookRef {
            action: Some(MdUpdateAction::DeleteFrom),
            scope: Some(SmolStr::new("PRIMARY")),
            position: Some(0),
            ..BookRef::default()
        },
    );
    assert!(book.add_operations(inputs([invalid])).is_err());
    assert_eq!(book, before);
}

#[test]
fn an_anonymous_new_entry_refuses_to_replace_an_occupied_position() {
    let positioned = || BookRef {
        action: Some(MdUpdateAction::New),
        position: Some(1),
        ..BookRef::default()
    };
    let mut book = Book::new(1, "IBM");
    let first = anonymous(with_book(
        operation("quote", "IBM", "POSITION-1", 1, "Buy", "100", 1, "New"),
        positioned(),
    ));
    book.add_operations(inputs([first])).unwrap();

    let collision = anonymous(with_book(
        operation("quote", "IBM", "POSITION-1", 2, "Buy", "101", 2, "New"),
        positioned(),
    ));
    let before = book.clone();
    let error = book
        .add_operations(inputs([collision]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("existing position"), "{error}");
    assert_eq!(book, before);

    let mut book = Book::new(1, "IBM");
    let explicit = with_book(
        operation("quote", "IBM", "EXPLICIT", 1, "Buy", "100", 1, "New"),
        positioned(),
    );
    book.add_operations(inputs([explicit])).unwrap();
    let anonymous_entry = anonymous(with_book(
        operation("quote", "IBM", "ANONYMOUS", 2, "Buy", "101", 1, "New"),
        positioned(),
    ));
    let before = book.clone();
    let error = book
        .add_operations(inputs([anonymous_entry]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("existing position"), "{error}");
    assert_eq!(book, before);
}

#[test]
fn advancing_time_clears_previous_deltas_and_executions_and_rejects_regression() {
    let mut execution = OperationEventData::at(1);
    execution.set_crosscode("E-1".to_owned());
    execution.set_ticker(Some(SmolStr::new("IBM")));
    execution.set_state(State::read("Filled").unwrap());
    execution.finalize();
    let mut book = Book::new(1, "IBM");
    book.add_operations([
        operation("quote", "IBM", "B-1", 1, "Buy", "100", 1, "New").into(),
        MarketOperation::execution(execution).into(),
    ])
    .unwrap();
    assert_eq!(book.executions().len(), 1);
    book.set_snapunix(Some(1));
    book.finalize();

    book.add_operations(inputs([operation(
        "quote", "IBM", "A-1", 2, "Sell", "102", 2, "New",
    )]))
    .unwrap();
    assert_eq!(book.get_snapunix(), None);
    assert!(book.executions().is_empty());
    assert!(book.bid().deltas().is_empty());
    assert_eq!(book.ask().deltas().len(), 1);
    let mut canonical_bid = book.bid().clone();
    canonical_bid.finalize();
    assert_eq!(book.bid(), &canonical_bid);

    book.add_operations(inputs([operation(
        "execution",
        "IBM",
        "E-2",
        3,
        "Buy",
        "101",
        1,
        "Filled",
    )]))
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
        book.add_operations(inputs([operation(
            "quote", "IBM", "OLD", 1, "Buy", "99", 1, "New"
        )]))
        .is_err()
    );
}

#[test]
fn moving_one_identity_between_sides_retires_the_old_side() {
    let mut book = Book::new(1, "IBM");
    book.add_operations(inputs([operation(
        "order", "IBM", "O-1", 1, "Buy", "100", 1, "New",
    )]))
    .unwrap();
    book.add_operations(inputs([operation(
        "order", "IBM", "O-1", 2, "Sell", "101", 1, "Replaced",
    )]))
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
    book.add_operations(inputs([operation(
        "quote", "IBM", "B-1", 1, "Buy", "100", 1, "New",
    )]))
    .unwrap();
    assert!(book.clone().merge_with(&book).is_none());

    let final_live = operation("quote", "IBM", "B", 2, "Buy", "100", 1, "New");
    let mut direct = Book::new(2, "IBM");
    direct.add_operations(inputs([final_live.clone()])).unwrap();
    let mut with_history = Book::new(2, "IBM");
    with_history
        .add_operations(inputs([
            operation("quote", "IBM", "TRANSIENT", 2, "Buy", "99", 1, "New"),
            operation("quote", "IBM", "TRANSIENT", 2, "Buy", "99", 0, "Canceled"),
            final_live,
        ]))
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
    reference.finalize();

    let mut supplement = Book::new(2_000_000, "IBM");
    supplement
        .add_operations(inputs([operation(
            "quote", "IBM", "A-X", 2_000_000, "Sell", "103", 1, "New",
        )]))
        .unwrap();
    supplement.set_recdunix(Some(10));
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
        .add_operations(inputs([
            with_book(
                operation("quote", "IBM", "B-1", 2, "Buy", "100", 1, "New"),
                scoped("PRIMARY"),
            ),
            with_book(
                operation("quote", "IBM", "B-X", 2, "Buy", "99", 2, "New"),
                scoped("OTHER"),
            ),
        ]))
        .unwrap();
    older.set_recdunix(Some(10));
    older.finalize();

    let reset = reset_event(2, "RESET");
    let control = BookControl::snapshot(reset, Some(SmolStr::new("PRIMARY")));
    let mut latest = Book::new(2, "IBM");
    latest
        .add_operations([BookInput::Snapshot(control)])
        .unwrap();
    latest.set_recdunix(Some(20));
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
    bid_operation.set_price(Some(bid));
    bid_operation.set_quantity(Some(Decimal18::MAX));
    bid_operation.finalize();
    let mut ask_operation = operation("quote", "IBM", "A", 1, "Sell", "0", 1, "New");
    ask_operation.set_price(Some(ask));
    ask_operation.set_quantity(Some(Decimal18::MAX));
    ask_operation.finalize();
    let mut book = Book::new(1, "IBM");
    book.add_operations(inputs([bid_operation, ask_operation]))
        .unwrap();
    assert_eq!(
        book.bbo_midpoint(),
        Decimal18::from_units(Decimal18::MAX.units() - 1)
    );
    assert_eq!(book.median_quantity(), Some(Decimal18::MAX));
}

#[test]
fn a_failed_iterator_group_emits_only_the_error() {
    let mut invalid = operation("quote", "IBM", "BAD", 1, "Buy", "99", 1, "New");
    invalid.set_side(Side::Unknown);
    // The bid lane its buy filled would name that side again: an operation
    // with no side is one quoting no single lane either.
    invalid.set_bid(None);
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
    assert_eq!(later.get_ticker(), Some("MSFT"));
    assert_eq!(later.bid().live().next().unwrap().get_crosscode(), "LATER");
}

/// One book from the synthetic operations, however they arrive: each as a
/// `BookInput::MarketOperation` in a group of its own, or all in one
/// `add_operations` group, the depth, the deltas and the executions state
/// the same prices, quantities and names - never compared by identity, which
/// the grouping is allowed to move.
#[test]
fn one_by_one_and_grouped_operations_build_the_same_depth_deltas_and_executions() {
    let operations = || {
        [
            with_book(
                operation("order", "IBM", "O-1", 5, "Buy", "100", 2, "New"),
                scoped("PRIMARY"),
            ),
            with_altids(
                operation("quote", "IBM", "Q-1", 5, "Buy", "101", 3, "New"),
                &[(ORDER_ID, "ORDER-Q1")],
            ),
            operation("quote", "IBM", "A-1", 5, "Sell", "103", 4, "New"),
            with_book(
                operation("order", "IBM", "A-2", 5, "Sell", "102", 1, "New"),
                scoped("PRIMARY"),
            ),
            operation("execution", "IBM", "E-1", 5, "Buy", "101", 1, "Filled"),
            operation("quote", "IBM", "Q-1", 5, "Buy", "101", 0, "Canceled"),
            operation("execution", "IBM", "E-2", 5, "Sell", "102", 1, "Filled"),
        ]
    };

    let mut one_by_one = Book::new(5, "IBM");
    for operation in operations() {
        one_by_one
            .add_operations([BookInput::MarketOperation(operation)])
            .unwrap();
    }
    let mut grouped = Book::new(5, "IBM");
    grouped.add_operations(inputs(operations())).unwrap();

    assert_eq!(one_by_one.bid().len(), 1);
    assert_eq!(one_by_one.ask().len(), 2);
    assert_eq!(one_by_one.bid().deltas().len(), 3);
    assert_eq!(one_by_one.ask().deltas().len(), 2);
    assert_eq!(one_by_one.executions().len(), 2);
    for (arrived, in_group) in [
        (facts(one_by_one.bid().live()), facts(grouped.bid().live())),
        (facts(one_by_one.ask().live()), facts(grouped.ask().live())),
        (
            facts(one_by_one.bid().deltas().iter()),
            facts(grouped.bid().deltas().iter()),
        ),
        (
            facts(one_by_one.ask().deltas().iter()),
            facts(grouped.ask().deltas().iter()),
        ),
        (
            facts(one_by_one.executions().iter()),
            facts(grouped.executions().iter()),
        ),
    ] {
        assert_eq!(arrived, in_group);
    }
    assert_eq!(one_by_one.bid().best_price(), grouped.bid().best_price());
    assert_eq!(one_by_one.ask().best_price(), grouped.ask().best_price());
    assert_eq!(one_by_one.bbo_midpoint(), grouped.bbo_midpoint());
}
