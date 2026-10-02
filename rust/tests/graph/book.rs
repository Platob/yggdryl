//! `rust/src/graph/book.rs`: the typed market operations a book folds, its
//! price-ordered sides and their readings, and the walk that emits books.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use smol_str::SmolStr;
use yggdryl::IdKey;
use yggdryl::graph::book::{ENTRY_ID, ENTRY_REF_ID};
use yggdryl::graph::{
    BookEvent, BookIterator, BookRef, Element, Event, ExecutionEvent, Market, MarketData,
    MarketKind, MdUpdateAction, Operation, OrderEvent, QuoteEvent, SnapshotEvent, TradeEvent,
};
use yggdryl::{
    Ccy, Decimal, IdSource, IdType, Identifier, Identifiers, Limit, Side, State, Unit, Uuid,
};

/// The identifier type an order's `OrderID(37)` is held under.
const ORDER_ID: IdType = IdType::OrderId;

/// The source the fixtures state their identifiers from.
const SOURCE: IdSource = IdSource::Base;

/// One identifier of a plain holder: a value of `kind` from `fix`.
fn identifier(kind: &IdType, value: &str) -> Identifier {
    Identifier::new(IdKey::new(SOURCE, kind.clone()), value).unwrap()
}

/// One synthetic operation of `kind` - `order`, `quote` or `execution` -
/// going by `identity` as its cross code and its `mdentryid`, finalized.
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
) -> MarketData {
    let mut data = OrderEvent::at(unix);
    data.set_crosscode(identity.to_owned());
    data.set_ticker(Some(SmolStr::new(symbol)), true);
    data.set_side(Side::read(side).unwrap(), true);
    data.set_price(Some(price.parse().unwrap()), true);
    data.set_quantity(Some(Decimal::from_int(quantity)), true);
    data.set_currency(Ccy::new("USD").unwrap(), true);
    data.set_unit(Unit::new("share").unwrap(), true);
    data.set_state(State::read(state).unwrap());
    // Every fixture entry can trade, so a side's best is its first priced
    // level; a test of an untradable level states it.
    data.set_tradable(Some(true), true);
    data.insert_identifier(identifier(&ENTRY_ID, identity))
        .unwrap();
    let mut operation = match kind {
        "order" => MarketData::from(data),
        "quote" => MarketData::from(QuoteEvent::from(&data)),
        "execution" => MarketData::from(ExecutionEvent::from(&data)),
        other => panic!("{other} is not an operation kind"),
    };
    operation.finalize();
    operation
}

/// What a dated operation answers, whichever kind it is: the view the
/// fixtures read a book's entries through.
trait EventOperation: Event + Operation {}
impl<T: Event + Operation + ?Sized> EventOperation for T {}

/// Binds `$operation` to the dated order, quote or execution `$value` holds.
macro_rules! on_operation {
    ($value:expr, $operation:ident => $body:expr) => {
        match $value {
            MarketData::OrderEvent($operation) => $body,
            MarketData::QuoteEvent($operation) => $body,
            MarketData::ExecutionEvent($operation) => $body,
            other => panic!("{} is not a dated operation", other.kind().as_str()),
        }
    };
}

/// The dated operation `value` holds.
fn op(value: &MarketData) -> &dyn EventOperation {
    on_operation!(value, operation => operation)
}

/// The dated operation `value` holds, mutably.
fn op_mut(value: &mut MarketData) -> &mut dyn EventOperation {
    on_operation!(value, operation => operation)
}

/// `value`'s operation edited by `edit`, finalized again around it.
fn edited(mut value: MarketData, edit: impl FnOnce(&mut dyn EventOperation)) -> MarketData {
    on_operation!(&mut value, operation => edit(operation));
    value.finalize();
    value
}

/// The executions `values` hold, each refused unless it is one.
fn executions<const N: usize>(values: [MarketData; N]) -> Vec<ExecutionEvent> {
    values
        .into_iter()
        .map(|value| ExecutionEvent::try_from(value).expect("an execution"))
        .collect()
}

/// The identifiers the dated operation `value` holds goes by.
fn identifiers_of(value: &MarketData) -> &Identifiers {
    op(value).get_identifiers()
}

/// `operation` carrying the book-control facts `book` states, finalized
/// again around them.
fn with_book(mut operation: MarketData, book: BookRef) -> MarketData {
    on_operation!(&mut operation, held => held.set_book(Some(book)));
    operation.finalize();
    operation
}

/// `operation` going by `identifiers` too, each replacing the value its type
/// held already, finalized again around them.
fn with_identifiers(operation: MarketData, identifiers: &[(IdType, &str)]) -> MarketData {
    edited(operation, |held| {
        for (kind, value) in identifiers {
            held.remove_identifier(&IdKey::new(SOURCE, (kind).clone()))
                .unwrap();
            held.insert_identifier(identifier(kind, value)).unwrap();
        }
    })
}

/// `operation` without its `mdentryid`: an anonymous entry.
fn anonymous(operation: MarketData) -> MarketData {
    edited(operation, |held| {
        held.remove_identifier(&IdKey::new(SOURCE, ENTRY_ID))
            .unwrap();
    })
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

fn decimal(text: &str) -> Decimal {
    text.parse().unwrap()
}

/// `operation` stating no price - a market order - and a last executed
/// price it carries left out of the price.
fn unpriced(mut operation: MarketData) -> MarketData {
    operation.set_price(None, true);
    operation.set_lastpx(Some(decimal("100")), true);
    operation.finalize();
    assert_eq!(operation.get_price(), None);
    operation
}

/// `operation` stating `quantity`, finalized again around it.
fn sized(mut operation: MarketData, quantity: Decimal) -> MarketData {
    operation.set_quantity(Some(quantity), true);
    operation.finalize();
    operation
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

/// The `curruuid` of the live entry going by `code` on its side.
fn live_uuid(book: &BookEvent, code: &str) -> Uuid {
    book.alive()
        .find(|entry| entry.get_crosscode() == entry.stored_crosscode(code))
        .unwrap_or_else(|| panic!("{code} is live"))
        .get_curruuid()
}

/// One limit at `price` - `None` for the unpriced one - of `quantity`
/// over the live entries `codes` go by, in that order; tradable, since
/// every fixture entry states it can trade.
fn limit(book: &BookEvent, price: Option<&str>, quantity: i64, codes: &[&str]) -> Limit {
    Limit {
        price: price.map(decimal),
        quantity: Decimal::from_int(quantity),
        uuids: codes.iter().map(|code| live_uuid(book, code)).collect(),
        tradable: true,
    }
}

/// The entry `code` resting on `side` at `price` - `None` unpriced - of
/// `quantity`.
fn resting(side: &str, code: &str, price: Option<&str>, quantity: i64) -> MarketData {
    let entry = operation(
        "order",
        "IBM",
        code,
        1,
        side,
        price.unwrap_or("0"),
        quantity,
        "New",
    );
    if price.is_some() {
        entry
    } else {
        unpriced(entry)
    }
}

/// A book of `bid` and `ask`, each `(code, price, quantity)`, `None` an
/// unpriced entry.
fn book_of(bid: &[(&str, Option<&str>, i64)], ask: &[(&str, Option<&str>, i64)]) -> BookEvent {
    let mut book = BookEvent::new(1, "IBM");
    let mut operations: Vec<MarketData> = bid
        .iter()
        .map(|(code, price, quantity)| resting("Buy", code, *price, *quantity))
        .collect();
    operations.extend(
        ask.iter()
            .map(|(code, price, quantity)| resting("Sell", code, *price, *quantity)),
    );
    book.add_operations(operations).unwrap();
    book
}

/// A book holding `levels` on `side` alone.
fn side_of(side: &str, levels: &[(&str, Option<&str>, i64)]) -> BookEvent {
    if side == "Buy" {
        book_of(levels, &[])
    } else {
        book_of(&[], levels)
    }
}

/// The event of a full-snapshot control at `unix`, named `identity`, on
/// IBM.
fn reset_event(unix: i64, identity: &str) -> OrderEvent {
    let mut reset = OrderEvent::at(unix);
    reset.set_crosscode(identity.to_owned());
    reset.set_ticker(Some(SmolStr::new("IBM")), true);
    reset.set_state(State::read("New").unwrap());
    reset.finalize();
    reset
}

/// What a book states of one operation, compared never by identity: the
/// price, the quantity and the names it goes by.
type Facts = (Option<Decimal>, Option<Decimal>, Vec<(String, String)>);

/// [`Facts`] for each operation, in the order given.
fn facts<'a>(operations: impl IntoIterator<Item = &'a MarketData>) -> Vec<Facts> {
    operations
        .into_iter()
        .map(|operation| fact(op(operation)))
        .collect()
}

/// [`Facts`] for one operation.
fn fact(operation: &(impl Operation + ?Sized)) -> Facts {
    (
        operation.get_price(),
        operation.get_quantity(),
        operation
            .get_identifiers()
            .iter()
            .map(|id| (id.kind().as_str().to_owned(), id.value().to_owned()))
            .collect(),
    )
}

#[test]
fn a_side_keeps_best_price_order_and_aggregates_exact_level_quantity() {
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([
        operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New"),
        operation("quote", "IBM", "Q-1", 1, "Buy", "101", 3, "New"),
        operation("order", "IBM", "O-2", 1, "Buy", "101", 4, "New"),
    ])
    .unwrap();

    assert_eq!(
        alive(&book, true)
            .into_iter()
            .map(Market::get_price)
            .collect::<Vec<_>>(),
        [
            Some(decimal("101")),
            Some(decimal("101")),
            Some(decimal("100"))
        ]
    );
    assert_eq!(book.best_price(Side::Buy), Some(decimal("101")));
    assert_eq!(book.best_quantity(Side::Buy), Some(Decimal::from_int(7)));
    assert_eq!(book.deltas().count(), 3);

    book.add_operations([operation(
        "quote", "IBM", "Q-1", 4, "Buy", "101", 0, "Canceled",
    )])
    .unwrap();
    assert_eq!(book.best_quantity(Side::Buy), Some(Decimal::from_int(4)));
    // A side that is neither a bid nor an ask reads nothing.
    assert_eq!(book.best_price(Side::Unknown), None);
    assert_eq!(book.limits(Side::Unknown).count(), 0);
    assert_eq!(book.depth(Side::Unknown, 1), None);
}

#[test]
fn an_unpriced_entry_lands_last_as_one_limit() {
    // Nothing invents a price for a market order - not a zero, not the last
    // executed price it carries: it rests after every priced level.
    for (name, better, worse) in [("Buy", "101", "100"), ("Sell", "100", "101")] {
        let bid = name == "Buy";
        let side = Side::read(name).unwrap();
        let book = side_of(
            name,
            &[
                ("O-1", Some(worse), 2),
                ("M-1", None, 5),
                ("O-2", Some(better), 3),
            ],
        );
        let limits = [
            limit(&book, Some(better), 3, &["O-2"]),
            limit(&book, Some(worse), 2, &["O-1"]),
            limit(&book, None, 5, &["M-1"]),
        ];
        assert_eq!(book.limits(side).collect::<Vec<_>>(), limits, "{name}");
        assert_eq!(book.best_price(side), Some(decimal(better)), "{name}");
        assert_eq!(book.best_quantity(side), Some(Decimal::from_int(3)));
        assert_eq!(
            alive(&book, bid)
                .into_iter()
                .map(Element::get_crosscode)
                .collect::<Vec<_>>(),
            ["O-2", "O-1", "M-1"].map(|code| format!("10:{}:{code}", side.code()))
        );
        // The other side states no level.
        let other = if bid { Side::Sell } else { Side::Buy };
        assert_eq!(book.limits(other).count(), 0, "{name}");
    }

    // A book folds one the same way, on either side.
    let book = book_of(
        &[("B-1", Some("100"), 1), ("B-M", None, 4)],
        &[("A-M", None, 6), ("A-1", Some("102"), 2)],
    );
    assert_eq!(
        alive(&book, true).last().unwrap().get_crosscode(),
        "10:1:B-M"
    );
    assert_eq!(
        alive(&book, false).last().unwrap().get_crosscode(),
        "10:2:A-M"
    );
    assert_eq!(book.bbo_midpoint(), Some(decimal("101")));
    assert_eq!(book.limits(Side::Buy).last().unwrap().price, None);
}

#[test]
fn a_book_level_trades_unless_every_entry_there_states_it_cannot() {
    let stating = |code: &str, price: Option<&str>, tradable: Option<bool>| {
        edited(resting("Buy", code, price, 1), |held| {
            held.set_tradable(tradable, true);
        })
    };
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([
        stating("A", Some("100"), Some(false)),
        stating("B", Some("100"), None),
        stating("C", Some("101"), None),
        stating("D", Some("101"), Some(true)),
        stating("E", Some("101"), Some(false)),
        stating("F", Some("99"), Some(false)),
        stating("G", Some("99"), Some(false)),
        stating("M", None, Some(true)),
    ])
    .unwrap();
    // An entry stating nothing trades, so one beside a `false` keeps its
    // level tradable; only a level every entry of which states `false`
    // cannot trade.
    let untradable = |limit: Limit| Limit {
        tradable: false,
        ..limit
    };
    let limits = [
        limit(&book, Some("101"), 3, &["C", "D", "E"]),
        limit(&book, Some("100"), 2, &["A", "B"]),
        untradable(limit(&book, Some("99"), 2, &["F", "G"])),
        limit(&book, None, 1, &["M"]),
    ];
    assert_eq!(book.limits(Side::Buy).collect::<Vec<_>>(), limits);
}

/// The best bid and ask are the best levels that can trade: a better level
/// every entry of which states it cannot trade is skipped, one stating
/// nothing trades, and a side none of whose levels can trade states no
/// best - the book's `bidpx`/`askpx` included.
#[test]
fn the_best_price_is_the_best_tradable_level() {
    let stating = |side: &str, code: &str, price: &str, quantity: i64, tradable: Option<bool>| {
        edited(resting(side, code, Some(price), quantity), |held| {
            held.set_tradable(tradable, true);
        })
    };
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([
        stating("Buy", "B-TOP", "101", 5, Some(false)),
        stating("Buy", "B-NEXT", "100", 3, None),
        stating("Sell", "A-TOP", "102", 4, Some(false)),
        stating("Sell", "A-NEXT", "103", 2, Some(false)),
    ])
    .unwrap();
    // The top bid states it cannot trade, so the next level - stating
    // nothing - is the best.
    assert_eq!(book.best_price(Side::Buy), Some(decimal("100")));
    assert_eq!(book.best_quantity(Side::Buy), Some(Decimal::from_int(3)));
    assert_eq!(book.get_bidpx(), Some(decimal("100")));
    assert_eq!(book.get_bidqty(), Some(Decimal::from_int(3)));
    assert_eq!(book.get_bidccy().map(Ccy::as_str), Some("USD"));
    // No ask level can trade: no best, and nothing stated for the ask.
    assert_eq!(book.best_price(Side::Sell), None);
    assert_eq!(book.best_quantity(Side::Sell), None);
    assert_eq!(
        (book.get_askpx(), book.get_askqty(), book.get_askccy()),
        (None, None, None)
    );
    assert_eq!(book.spread(), None);
    // The limits still state every level, tradable or not.
    assert_eq!(book.limits(Side::Sell).count(), 2);
}

#[test]
fn two_entries_at_one_price_are_one_limit_with_both_uuids() {
    // Position order first, then arrival order among the unpositioned.
    let positioned = |position| BookRef {
        position: Some(position),
        ..BookRef::default()
    };
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([
        operation("order", "IBM", "C", 1, "Buy", "100", 4, "New"),
        with_book(
            operation("order", "IBM", "A", 1, "Buy", "100", 2, "New"),
            positioned(2),
        ),
        operation("order", "IBM", "D", 1, "Buy", "100", 8, "New"),
        with_book(
            operation("order", "IBM", "B", 1, "Buy", "100", 1, "New"),
            positioned(1),
        ),
    ])
    .unwrap();
    assert_eq!(
        book.limits(Side::Buy).collect::<Vec<_>>(),
        [limit(&book, Some("100"), 15, &["B", "A", "C", "D"])]
    );
    assert_eq!(book.best_quantity(Side::Buy), Some(Decimal::from_int(15)));
}

#[test]
fn a_side_holding_only_market_orders_states_no_best() {
    let book = side_of("Sell", &[("M-1", None, 2), ("M-2", None, 3)]);
    assert_eq!(
        (book.best_price(Side::Sell), book.best_quantity(Side::Sell)),
        (None, None)
    );
    assert_eq!(
        book.limits(Side::Sell).collect::<Vec<_>>(),
        [limit(&book, None, 5, &["M-1", "M-2"])]
    );
    assert_eq!(book.depth(Side::Sell, 1), Some(Decimal::from_int(5)));
    // No best states no currency and no unit for the book.
    assert_eq!(book.get_currency(), &Ccy::none());
    assert_eq!(book.get_unit(), &Unit::none());

    let book = book_of(&[("B-M", None, 1)], &[("A-1", Some("102"), 2)]);
    assert_eq!(book.spread(), None);
    assert!(!book.is_crossed() && !book.is_locked());
    assert_eq!(book.bbo_midpoint(), None);
    // The book's summary is its bests': a side stating none leaves the
    // other's price, quantity, currency and unit standing alone.
    assert_eq!(book.get_price(), Some(decimal("102")));
    assert_eq!(book.get_quantity(), Some(Decimal::from_int(2)));
    assert_eq!(book.get_currency(), &Ccy::new("USD").unwrap());
    assert_eq!(book.get_unit(), &Unit::new("share").unwrap());
}

#[test]
fn a_locked_book_is_not_crossed() {
    let book = book_of(&[("B-1", Some("100"), 1)], &[("A-1", Some("100"), 2)]);
    assert!(book.is_locked());
    assert!(!book.is_crossed());
    assert_eq!(book.spread(), Some(Decimal::ZERO));
    assert_eq!(book.bbo_midpoint(), Some(decimal("100")));
}

#[test]
fn spread_is_negative_when_crossed_and_none_when_one_sided() {
    let book = book_of(&[("B-1", Some("100"), 1)], &[("A-1", Some("102.5"), 2)]);
    assert_eq!(book.spread(), Some(decimal("2.5")));
    assert!(!book.is_crossed() && !book.is_locked());

    let crossed = book_of(&[("B-1", Some("103"), 1)], &[("A-1", Some("102"), 2)]);
    assert!(crossed.is_crossed() && !crossed.is_locked());
    assert_eq!(crossed.spread(), Some(decimal("-1")));

    let one_sided = book_of(&[("B-1", Some("103"), 1)], &[]);
    assert_eq!(one_sided.spread(), None);
    assert!(!one_sided.is_locked());
    assert_eq!(BookEvent::new(1, "IBM").spread(), None);
}

#[test]
fn imbalance_at_one_level_of_a_one_sided_book_is_one_or_minus_one() {
    let bid = book_of(&[("B-1", Some("100"), 3)], &[]);
    assert_eq!(bid.imbalance(1), Some(Decimal::ONE));
    let ask = book_of(&[], &[("A-1", Some("100"), 3)]);
    assert_eq!(ask.imbalance(1), Some(-Decimal::ONE));
}

#[test]
fn imbalance_is_none_when_both_sides_are_empty_or_levels_is_zero() {
    assert_eq!(BookEvent::new(1, "IBM").imbalance(5), None);
    let book = book_of(&[("B-1", Some("100"), 3)], &[("A-1", Some("101"), 1)]);
    assert_eq!(book.imbalance(0), None);
    assert_eq!(book.imbalance(1), Some(decimal("0.5")));
}

#[test]
fn imbalance_reads_the_first_levels_only() {
    let book = book_of(
        &[
            ("B-1", Some("101"), 3),
            ("B-2", Some("100"), 2),
            ("B-3", Some("99"), 100),
        ],
        &[
            ("A-1", Some("102"), 1),
            ("A-2", Some("103"), 4),
            ("A-3", Some("104"), 10),
        ],
    );
    assert_eq!(book.imbalance(1), Some(decimal("0.5")));
    assert_eq!(book.imbalance(2), Some(Decimal::ZERO));
    assert_eq!(book.imbalance(3), Some(decimal("0.75")));
}

#[test]
fn depth_sums_the_first_levels_and_counts_the_unpriced_limit_last() {
    let book = side_of(
        "Buy",
        &[
            ("M-1", None, 5),
            ("B-2", Some("100"), 2),
            ("B-1", Some("101"), 3),
        ],
    );
    let depths: Vec<_> = [0, 1, 2, 3, 10]
        .into_iter()
        .map(|levels| book.depth(Side::Buy, levels))
        .collect();
    assert_eq!(
        depths,
        [0, 3, 5, 10, 10].map(|sum| Some(Decimal::from_int(sum)))
    );
    assert_eq!(book.depth(Side::Sell, 3), Some(Decimal::ZERO));
    assert_eq!(book.limits(Side::Sell).count(), 0);
}

#[test]
fn a_level_whose_quantity_would_overflow_is_refused_at_the_write() {
    let huge = |code: &str, price: &str| {
        sized(
            operation("order", "IBM", code, 1, "Buy", price, 1, "New"),
            Decimal::MAX,
        )
    };
    // At the best level, at a deeper one and at the unpriced one alike:
    // every level a refreshed side holds fits decimal18.
    for (price, named, above) in [
        (Some("101"), "101", 0),
        (Some("100"), "100", 1),
        (None, "unpriced", 1),
    ] {
        let price_text = price.unwrap_or("0");
        let entry = |code: &str| {
            let held = huge(code, price_text);
            if price.is_some() {
                held
            } else {
                unpriced(held)
            }
        };
        let mut book = side_of("Buy", &[("TOP", Some("102"), 1)][..above]);
        book.add_operations([entry("H-1")]).unwrap();
        let before = book.clone();
        let error = book.add_operations([entry("H-2")]).unwrap_err();
        let yggdryl::Error::InvalidRecord { path, reason } = &error else {
            panic!("expected a located refusal, got {error}");
        };
        assert_eq!(path, "$.quantity", "{error}");
        assert!(reason.contains(named), "{error}");
        assert_eq!(book, before, "the book is unchanged");
    }
}

#[test]
fn a_delete_from_reaches_an_unpriced_entry_after_every_priced_level() {
    let mut book = book_of(
        &[
            ("M-1", None, 5),
            ("P-2", Some("100"), 2),
            ("P-1", Some("101"), 3),
        ],
        &[],
    );
    let deletion = with_book(
        operation("quote", "IBM", "DELETE", 2, "Buy", "0", 0, "Canceled"),
        BookRef {
            action: Some(MdUpdateAction::DeleteFrom),
            position: Some(3),
            ..BookRef::default()
        },
    );
    book.add_operations([deletion]).unwrap();
    assert_eq!(
        alive(&book, true)
            .into_iter()
            .map(Element::get_crosscode)
            .collect::<Vec<_>>(),
        ["10:1:P-1", "10:1:P-2"]
    );
    assert!(book.limits(Side::Buy).all(|limit| limit.price.is_some()));
}

#[test]
fn book_exposes_bbo_midpoint_and_two_value_quantity_median() {
    let mut book = BookEvent::new(10, "IBM");
    book.add_operations([
        operation("quote", "IBM", "B-1", 10, "Buy", "100", 8, "New"),
        operation("quote", "IBM", "A-1", 10, "Sell", "102", 4, "New"),
    ])
    .unwrap();

    assert_eq!(book.bbo_midpoint(), Some(decimal("101")));
    assert_eq!(book.median_quantity(), Some(Decimal::from_int(6)));
    assert_eq!(book.get_price(), Some(decimal("101")));
    assert_eq!(book.get_quantity(), Some(Decimal::from_int(6)));
    assert_eq!(book.best_quantity(Side::Buy), Some(Decimal::from_int(8)));
    assert_eq!(book.best_quantity(Side::Sell), Some(Decimal::from_int(4)));

    book.add_operations([operation("quote", "IBM", "B-2", 11, "Buy", "103", 1, "New")])
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
    let mut book = BookEvent::new(1, "IBM");
    let first = with_book(
        operation("quote", "IBM", "B-1", 1, "Buy", "100", 2, "New"),
        scoped("PRIMARY"),
    );
    let other = with_book(
        operation("quote", "IBM", "B-X", 1, "Buy", "99", 9, "New"),
        scoped("OTHER"),
    );
    book.add_operations([first, other]).unwrap();
    let previous = book.clone();

    let replacement = with_book(
        operation("quote", "IBM", "B-2", 2, "Buy", "101", 3, "New"),
        acting_in(MdUpdateAction::Snapshot, "PRIMARY"),
    );
    book.add_operations([replacement.clone()]).unwrap();

    let identities: Vec<_> = alive(&book, true)
        .into_iter()
        .map(|operation| operation.get_crosscode())
        .collect();
    assert_eq!(identities, ["14:1:B-2", "14:1:B-X"]);

    let mut update = BookEvent::new(2, "IBM");
    update.add_operations([replacement]).unwrap();
    let continued = update.with_previous(&previous).unwrap();
    assert_eq!(
        alive(&continued, true)
            .into_iter()
            .map(Element::get_crosscode)
            .collect::<Vec<_>>(),
        ["14:1:B-2", "14:1:B-X"]
    );
}

#[test]
fn iterator_emits_one_book_per_symbol_and_timestamp() {
    let operations = vec![
        operation("quote", "IBM", "IBM-B", 1_000_000, "Buy", "100", 2, "New"),
        operation("quote", "MSFT", "MS-B", 1_000_000, "Buy", "200", 3, "New"),
        operation("quote", "IBM", "IBM-A", 3_000_000, "Sell", "102", 4, "New"),
    ];
    let books = BookIterator::new(operations.into_iter(), 0)
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
}

/// A ticker an ISIN registry filled moves the book an element without one
/// stands in: unfilled, the second quote stands in the book of its market
/// and classification; filled from the first quote's row, in the ticker's.
#[test]
fn a_ticker_a_registry_filled_files_the_element_under_the_ticker_s_book() {
    let holcim = |value: MarketData, ticker: bool| {
        edited(value, |operation| {
            operation
                .insert_securityid(identifier(&IdType::Isin, "CH0012214059"))
                .unwrap();
            if !ticker {
                operation.set_ticker(None, true);
            }
        })
    };
    let first = holcim(
        operation("quote", "HOLN", "Q-1", 1_000_000, "Buy", "100", 2, "New"),
        true,
    );
    let second = holcim(
        operation("quote", "HOLN", "Q-2", 1_000_000, "Sell", "101", 3, "New"),
        false,
    );
    let books = |values: Vec<MarketData>| {
        BookIterator::new(values.into_iter(), 0)
            .unwrap()
            .map(|book| book.unwrap().book_crosscode().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        books(vec![first.clone(), second.clone()]),
        ["HOLN", "XXXX:XXXXXX"]
    );

    let mut registry = yggdryl::IsinRegistry::new();
    let filled = [first, second]
        .into_iter()
        .map(|mut value| {
            on_operation!(&mut value, operation => registry.enrich(operation));
            value
        })
        .collect::<Vec<_>>();
    assert_eq!(op(&filled[1]).get_ticker(), Some("HOLN"));
    assert_eq!(books(filled), ["HOLN"]);
}

#[test]
fn iterator_emits_a_completed_timestamp_before_a_source_error_then_fuses() {
    struct Counted {
        source: std::vec::IntoIter<yggdryl::Result<MarketData>>,
        pulled: Arc<AtomicUsize>,
    }

    impl Iterator for Counted {
        type Item = yggdryl::Result<MarketData>;

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
    )
    .unwrap();

    assert_eq!(pulled.load(Ordering::SeqCst), 0);
    let book = books.next().unwrap().unwrap();
    assert_eq!(book.get_currunix(), 1_000_000);
    assert_eq!(
        (alive(&book, true).len(), alive(&book, false).len()),
        (1, 1)
    );
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
    let books = BookIterator::new(operations.into_iter(), 1)
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
    assert_eq!(alive(&books[1], true).len(), 1);
    assert_eq!(alive(&books[2], false).len(), 1);
    assert_eq!(deltas(&books[0], true).len(), 1);
    assert!(deltas(&books[1], true).is_empty());
    // The grid tick restates the same level: the same limits, another book.
    assert_eq!(
        books[0].limits(Side::Buy).collect::<Vec<_>>(),
        books[1].limits(Side::Buy).collect::<Vec<_>>()
    );
    assert_ne!(books[0].get_curruuid(), books[1].get_curruuid());
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
    let books = BookIterator::new(operations.clone().into_iter(), 1)
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
    assert_eq!(alive(died, true).len(), 1, "the survivor stays live");
    assert_eq!(
        deltas(died, true).len(),
        1,
        "the cancel is the delta of the book it died in"
    );
    let snapshot = at(3_000_000);
    assert_eq!(snapshot.get_snapunix(), Some(3_000_000));
    assert_eq!(
        alive(snapshot, true)
            .into_iter()
            .map(|held| held.get_crosscode().to_owned())
            .collect::<Vec<_>>(),
        ["10:1:O-1"],
        "every living order, and no dead one"
    );
    assert!(
        deltas(snapshot, true).is_empty(),
        "nothing but the living orders"
    );
    assert!(snapshot.executions().is_empty());
    assert!(alive(snapshot, false).into_iter().next().is_none());

    // With no grid, every emitted book keeps all living orders beside its
    // own deltas: nothing is ever purged between books.
    let books = BookIterator::new(operations.into_iter(), 0)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(books.len(), 4);
    assert!(books.iter().all(|book| book.get_snapunix().is_none()));
    assert_eq!(alive(&books[1], true).len(), 2, "both live at 1.5 ms");
    assert_eq!(alive(&books[2], true).len(), 1, "the survivor at 2.5 ms");
    assert_eq!(deltas(&books[2], true).len(), 1);
    assert_eq!(
        alive(&books[3], true).len(),
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
    assert_eq!(alive(&books[3], true).len(), 1);
}

#[test]
fn exact_nanoseconds_participate_in_book_identity_within_one_millisecond() {
    let mut first = BookEvent::new(1_000_001, "IBM");
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
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New")])
        .unwrap();
    let first = alive(&book, true).into_iter().next().unwrap().clone();

    book.add_operations([operation(
        "order", "IBM", "O-1", 2, "Buy", "101", 3, "Replaced",
    )])
    .unwrap();
    let replacement = alive(&book, true).into_iter().next().unwrap();
    // A later instant keeps its own place, and so does the book that
    // reset to it.
    assert_eq!(op(replacement).get_seqnum(), 0);
    assert_eq!(op(replacement).get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(replacement.get_prevpx(), first.get_price());
    assert_eq!(replacement.get_prevqty(), first.get_quantity());
    assert_eq!(book.get_seqnum(), 0);

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
    let mut book = BookEvent::new(1, "IBM");
    let partial = with_book(
        operation("order", "IBM", "MISSING", 1, "Buy", "101", 3, "Replaced"),
        acting(MdUpdateAction::Change),
    );
    let error = book.add_operations([partial]).unwrap_err();
    assert!(
        matches!(&error, yggdryl::Error::InvalidRecord { path, .. } if path == "$.operation.book.entry_px"),
        "{error}"
    );
    assert!(alive(&book, true).is_empty());

    book.add_operations([
        operation("order", "IBM", "X", 1, "Buy", "100", 1, "New"),
        operation("order", "IBM", "Y", 1, "Buy", "99", 2, "New"),
    ])
    .unwrap();
    let before = book.clone();
    let collision = with_book(
        with_identifiers(
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
    let error = book.add_operations([collision]).unwrap_err().to_string();
    assert!(error.contains("mdentryid"), "{error}");
    assert_eq!(book, before);

    let unresolved = with_book(
        with_identifiers(
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
    let error = book.add_operations([unresolved]).unwrap_err().to_string();
    assert!(error.contains("mdentryrefid"), "{error}");
    assert_eq!(book, before);
}

#[test]
fn partial_market_updates_continue_orders_without_restating_order_id() {
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([with_book(
        with_identifiers(
            operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New"),
            &[(ORDER_ID, "ORDER-1")],
        ),
        acting(MdUpdateAction::New),
    )])
    .unwrap();
    let previous = alive(&book, true).into_iter().next().unwrap().clone();

    book.add_operations([with_book(
        operation("quote", "IBM", "O-1", 2, "Buy", "0", 3, "Replaced"),
        BookRef {
            action: Some(MdUpdateAction::Change),
            entry_size: Some(decimal("3")),
            ..BookRef::default()
        },
    )])
    .unwrap();

    let live = alive(&book, true).into_iter().next().unwrap();
    assert_eq!(live.kind(), MarketKind::OrderEvent);
    assert_eq!(identifiers_of(live).get(&ORDER_ID), Some("ORDER-1"));
    assert_eq!(live.get_price(), previous.get_price());
    assert_eq!(live.get_quantity(), Some(Decimal::from_int(3)));
    assert_eq!(op(live).get_prevuuid(), Some(previous.get_curruuid()));
    assert_eq!(live.get_prevpx(), previous.get_price());
    assert_eq!(live.get_prevqty(), previous.get_quantity());
    // A later instant keeps its own place.
    assert_eq!(op(live).get_seqnum(), 0);
    let mut finalized = live.clone();
    finalized.finalize();
    assert_eq!(*live, finalized);
}

#[test]
fn referenced_market_update_refuses_an_occupied_destination_on_the_other_side() {
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([
        operation("quote", "IBM", "X", 1, "Buy", "100", 2, "New"),
        operation("quote", "IBM", "Y", 1, "Sell", "101", 3, "New"),
    ])
    .unwrap();
    let before = book.clone();
    let error = book
        .add_operations([with_book(
            with_identifiers(
                operation("quote", "IBM", "Y", 2, "Buy", "99", 4, "Replaced"),
                &[(ENTRY_REF_ID, "X")],
            ),
            BookRef {
                action: Some(MdUpdateAction::Change),
                entry_px: Some(decimal("99")),
                entry_size: Some(decimal("4")),
                ..BookRef::default()
            },
        )])
        .expect_err("a reference cannot replace a different live destination on either side");
    assert!(
        matches!(&error, yggdryl::Error::InvalidRecord { path, .. } if path == "$.operation.identifiers.mdentryid"),
        "{error}"
    );
    assert_eq!(book, before);
}

#[test]
fn partial_market_updates_move_between_sides_with_their_predecessor() {
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([with_book(
        with_identifiers(
            operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New"),
            &[(ORDER_ID, "ORDER-1")],
        ),
        acting(MdUpdateAction::New),
    )])
    .unwrap();
    let previous = alive(&book, true).into_iter().next().unwrap().clone();

    book.add_operations([with_book(
        with_identifiers(
            operation("quote", "IBM", "O-2", 2, "Sell", "0", 3, "Replaced"),
            &[(ENTRY_REF_ID, "O-1")],
        ),
        BookRef {
            action: Some(MdUpdateAction::Change),
            entry_size: Some(decimal("3")),
            ..BookRef::default()
        },
    )])
    .unwrap();
    assert!(alive(&book, true).is_empty());
    let live = alive(&book, false).into_iter().next().unwrap();
    assert_eq!(live.kind(), MarketKind::OrderEvent);
    assert_eq!(live.get_price(), previous.get_price());
    assert_eq!(live.get_quantity(), Some(Decimal::from_int(3)));
    assert_eq!(op(live).get_prevuuid(), Some(previous.get_curruuid()));
    assert_eq!(live.get_prevpx(), previous.get_price());
    assert_eq!(live.get_prevqty(), previous.get_quantity());
    // A later instant keeps its own place.
    assert_eq!(op(live).get_seqnum(), 0);
    assert_eq!(identifiers_of(live).get(&ENTRY_ID), Some("O-2"));
    let previous = live.clone();

    book.add_operations([with_book(
        operation("quote", "IBM", "O-2", 3, "Buy", "101", 0, "Replaced"),
        BookRef {
            action: Some(MdUpdateAction::Overlay),
            entry_px: Some(decimal("101")),
            ..BookRef::default()
        },
    )])
    .unwrap();
    assert!(alive(&book, false).is_empty());
    let live = alive(&book, true).into_iter().next().unwrap();
    assert_eq!(live.kind(), MarketKind::OrderEvent);
    assert_eq!(live.get_price(), Some(Decimal::from_int(101)));
    assert_eq!(live.get_quantity(), previous.get_quantity());
    assert_eq!(op(live).get_prevuuid(), Some(previous.get_curruuid()));
    // A later instant keeps its own place.
    assert_eq!(op(live).get_seqnum(), 0);
}

#[test]
fn renamed_market_entry_expires_using_its_current_identity() {
    let mut initial = with_book(
        with_identifiers(
            operation("order", "IBM", "X", 1, "Buy", "100", 2, "New"),
            &[(ORDER_ID, "ORDER-1")],
        ),
        acting(MdUpdateAction::New),
    );
    op_mut(&mut initial).set_exprunix(Some(4));
    initial.finalize();
    let renamed = with_book(
        with_identifiers(
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
    let books = BookIterator::new([initial, renamed].into_iter(), 0)
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("expiry deletes the current entry after its reference has been resolved");
    assert_eq!(books.len(), 3);
    let previous = alive(&books[1], false).into_iter().next().unwrap();
    assert_eq!(identifiers_of(previous).get(&ENTRY_ID), Some("Y"));
    assert!(alive(&books[2], true).is_empty());
    assert!(alive(&books[2], false).is_empty());
    assert_eq!(books[2].get_currunix(), 4);
    let expired = &deltas(&books[2], false)[0];
    assert_eq!(expired.kind(), MarketKind::OrderEvent);
    assert_eq!(op(expired).get_prevuuid(), Some(previous.get_curruuid()));
    assert_eq!(identifiers_of(expired).get(&ENTRY_ID), Some("Y"));
    assert!(!op(expired).get_state().is_live());
}

#[test]
fn partial_market_updates_promote_quotes_when_the_order_id_becomes_known() {
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([with_book(
        operation("quote", "IBM", "Q-1", 1, "Buy", "100", 2, "New"),
        acting(MdUpdateAction::New),
    )])
    .unwrap();
    let previous = alive(&book, true).into_iter().next().unwrap().clone();
    book.add_operations([with_book(
        with_identifiers(
            operation("order", "IBM", "Q-1", 2, "Buy", "0", 3, "Replaced"),
            &[(ORDER_ID, "ORDER-1")],
        ),
        BookRef {
            action: Some(MdUpdateAction::Change),
            entry_size: Some(decimal("3")),
            ..BookRef::default()
        },
    )])
    .unwrap();
    let live = alive(&book, true).into_iter().next().unwrap();
    assert_eq!(live.kind(), MarketKind::OrderEvent);
    assert_eq!(live.get_price(), previous.get_price());
    assert_eq!(identifiers_of(live).get(&ORDER_ID), Some("ORDER-1"));
    assert_eq!(op(live).get_prevuuid(), Some(previous.get_curruuid()));
    // A later instant keeps its own place.
    assert_eq!(op(live).get_seqnum(), 0);
}

#[test]
fn contradictory_market_update_order_ids_refuse_without_removing_the_predecessor() {
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([with_book(
        with_identifiers(
            operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New"),
            &[(ORDER_ID, "ORDER-1")],
        ),
        acting(MdUpdateAction::New),
    )])
    .unwrap();
    let before = book.clone();
    let error = book
        .add_operations([with_book(
            with_identifiers(
                operation("order", "IBM", "O-1", 2, "Sell", "101", 3, "Replaced"),
                &[(ORDER_ID, "ORDER-2")],
            ),
            BookRef {
                action: Some(MdUpdateAction::Change),
                entry_px: Some(decimal("101")),
                entry_size: Some(decimal("3")),
                ..BookRef::default()
            },
        )])
        .expect_err("a continuation cannot replace a known order identity");
    assert!(
        matches!(&error, yggdryl::Error::InvalidRecord { path, .. } if path == "$.operation.identifiers.orderid"),
        "{error}"
    );
    assert_eq!(book, before);
}

#[test]
fn executions_are_reported_beside_depth_and_propagate_the_latest_execution_clock() {
    let mut first = ExecutionEvent::at(10);
    first.set_crosscode("E-1".to_owned());
    first.set_ticker(Some(SmolStr::new("IBM")), true);
    first.set_price(Some(decimal("100")), true);
    first.set_quantity(Some(Decimal::from_int(2)), true);
    first.set_state(State::read("Filled").unwrap());
    first.set_execunix(Some(7), true);
    first.finalize();
    let mut second = first.clone();
    second.set_crosscode("E-2".to_owned());
    second.set_execunix(Some(9), true);
    second.finalize();

    let mut book = BookEvent::new(10, "IBM");
    book.add_operations([MarketData::from(first), MarketData::from(second)])
        .unwrap();
    assert_eq!(book.executions().len(), 2);
    assert!(alive(&book, true).is_empty());
    assert!(alive(&book, false).is_empty());
    assert_eq!(book.get_execunix(), Some(9));
}

#[test]
fn a_trade_flattens_its_sorted_executions_without_entering_depth() {
    let buy = operation("execution", "IBM", "E-BUY", 10, "Buy", "100", 2, "Filled");
    let sell = operation("execution", "IBM", "E-SELL", 10, "Sell", "101", 3, "Filled");
    let mut event = ExecutionEvent::at(10);
    event.set_crosscode("T-1".to_owned());
    event.set_ticker(Some(SmolStr::new("IBM")), true);
    event.set_state(State::read("Filled").unwrap());
    event.finalize();
    let trade = TradeEvent::from_parts(&event, executions([sell, buy])).unwrap();
    assert_eq!(
        trade
            .executions()
            .iter()
            .map(Element::get_crosscode)
            .collect::<Vec<_>>(),
        ["8:1:E-BUY", "8:2:E-SELL"]
    );
    let mut expected = trade.executions().to_vec();
    expected.sort_by_key(Element::get_curruuid);

    let mut book = BookEvent::new(10, "IBM");
    book.add_operations([MarketData::from(trade)]).unwrap();

    assert!(alive(&book, true).is_empty());
    assert!(alive(&book, false).is_empty());
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
    op_mut(&mut live).set_exprunix(Some(3));
    live.finalize();
    let mut execution = ExecutionEvent::at(1);
    execution.set_crosscode("E-1".to_owned());
    execution.set_ticker(Some(SmolStr::new("IBM")), true);
    execution.set_state(State::read("Filled").unwrap());
    execution.set_exprunix(Some(2));
    execution.finalize();

    let books = BookIterator::new(
        [
            live,
            MarketData::from(execution),
            operation("order", "IBM", "O-1", 3, "Buy", "101", 3, "New"),
        ]
        .into_iter(),
        0,
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
    let replacement = alive(&books[1], true).into_iter().next().unwrap();
    assert_eq!(replacement.get_price(), Some(decimal("101")));
    assert_eq!(
        (op(replacement).get_seqnum(), op(replacement).get_prevuuid()),
        (0, None)
    );
}

/// A quote stating neither the bid nor the ask rests on no side: it is
/// left out of the book with a warning, and the expiration due at its
/// instant still fires.
#[test]
fn an_unsided_quote_is_left_out_and_the_pending_expiration_still_fires() {
    let mut live = operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New");
    op_mut(&mut live).set_exprunix(Some(3));
    live.finalize();
    let unsided = operation("quote", "IBM", "UNSIDED", 3, "Unknown", "101", 1, "New");
    let mut books = BookIterator::new([live, unsided].into_iter(), 0).unwrap();

    assert_eq!(alive(&books.next().unwrap().unwrap(), true).len(), 1);
    let expired = books.next().unwrap().unwrap();
    assert_eq!(expired.get_currunix(), 3);
    assert!(alive(&expired, true).is_empty());
    assert!(books.next().is_none());
}

#[test]
fn expiring_one_snapshot_entry_keeps_the_rest_of_its_partition() {
    let mut expiring = with_book(
        operation("quote", "IBM", "EXPIRING", 1, "Buy", "101", 1, "New"),
        acting_in(MdUpdateAction::Snapshot, "PRIMARY"),
    );
    op_mut(&mut expiring).set_exprunix(Some(2));
    expiring.finalize();
    let standing = with_book(
        operation("quote", "IBM", "STANDING", 1, "Buy", "100", 1, "New"),
        acting_in(MdUpdateAction::Snapshot, "PRIMARY"),
    );

    let books = BookIterator::new([expiring, standing].into_iter(), 0)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(books.len(), 2);
    assert_eq!(alive(&books[0], true).len(), 2);
    assert_eq!(alive(&books[1], true).len(), 1);
    assert_eq!(
        alive(&books[1], true)
            .into_iter()
            .next()
            .unwrap()
            .get_crosscode(),
        "14:1:STANDING"
    );
}

/// A group the book refuses - here a snapshot stating one entry twice - is
/// left out whole, with a warning: none of its raw updates lands, the book
/// stands as it was, and the walk goes on.
#[test]
fn a_failed_mixed_snapshot_group_commits_none_of_its_raw_updates() {
    let initial = operation("quote", "IBM", "INITIAL", 1, "Buy", "100", 1, "New");
    let raw = operation("quote", "IBM", "RAW", 2, "Buy", "99", 1, "New");
    let mut duplicate = operation("quote", "IBM", "DUPLICATE", 2, "Buy", "101", 1, "New");
    op_mut(&mut duplicate).set_snapunix(Some(2));
    duplicate.finalize();
    let later = operation("execution", "IBM", "LATER", 3, "Unknown", "100", 1, "Trade");

    let results = BookIterator::new(
        [initial, raw, duplicate.clone(), duplicate, later].into_iter(),
        0,
    )
    .unwrap()
    .collect::<Vec<_>>();
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(Result::is_ok));
    let after = results[1].as_ref().unwrap();
    assert_eq!(alive(after, true).len(), 1);
    assert_eq!(
        alive(after, true)
            .into_iter()
            .next()
            .unwrap()
            .get_crosscode(),
        "14:1:INITIAL"
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
    snapshot.set_price(Some(decimal("101")), true);
    snapshot.set_quantity(Some(Decimal::from_int(5)), true);
    snapshot.finalize();
    op_mut(&mut snapshot).set_snapunix(Some(2));
    let snapshot_only = snapshot.clone();

    let books = BookIterator::new([first, second, other_scope, snapshot].into_iter(), 0)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(books.len(), 2);
    assert_eq!(alive(&books[0], true).len(), 3);
    assert_eq!(books[1].get_snapunix(), Some(2));
    assert_eq!(alive(&books[1], true).len(), 2);
    let mut live = alive(&books[1], true).into_iter();
    let first = live.next().unwrap();
    assert_eq!(first.get_crosscode(), "10:1:O-1");
    assert_eq!(first.get_price(), Some(decimal("101")));
    assert_eq!(first.get_quantity(), Some(Decimal::from_int(5)));
    assert_eq!(live.next().unwrap().get_crosscode(), "10:1:O-OTHER");

    let only = BookIterator::new([snapshot_only].into_iter(), 0)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(only.get_snapunix(), Some(2));
    assert_eq!(alive(&only, true).len(), 1);
    assert_eq!(
        alive(&only, true).into_iter().next().unwrap().get_price(),
        Some(decimal("101"))
    );
}

#[test]
fn snapshotted_execution_is_reported_without_replacing_live_depth() {
    let live = operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New");
    let mut execution = operation("execution", "IBM", "E-1", 2, "Unknown", "101", 1, "Trade");
    op_mut(&mut execution).set_execunix(Some(2), true);
    op_mut(&mut execution).set_snapunix(Some(3));
    execution.finalize();

    let books = BookIterator::new([live, execution].into_iter(), 0)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(books.len(), 2);
    assert_eq!(books[1].get_currunix(), 3);
    assert_eq!(books[1].get_snapunix(), Some(3));
    assert_eq!(alive(&books[1], true).len(), 1);
    assert_eq!(books[1].executions().len(), 1);
    assert_eq!(books[1].executions()[0].get_currunix(), 3);
    assert_eq!(books[1].executions()[0].get_execunix(), Some(2));
}

#[test]
fn snapshotted_trade_rebases_every_child_and_preserves_execution_time() {
    let mut buy = operation("execution", "IBM", "E-BUY", 2, "Buy", "101", 4, "Trade");
    op_mut(&mut buy).set_execunix(Some(1), true);
    buy.finalize();
    let mut sell = operation("execution", "IBM", "E-SELL", 2, "Sell", "101", 4, "Trade");
    op_mut(&mut sell).set_execunix(Some(2), true);
    sell.finalize();
    let mut root = ExecutionEvent::at(2);
    root.set_crosscode("T-1".to_owned());
    root.set_ticker(Some(SmolStr::new("IBM")), true);
    root.set_snapunix(Some(3));
    root.set_state(State::read("Trade").unwrap());
    root.finalize();
    let trade = TradeEvent::from_parts(&root, executions([sell, buy])).unwrap();

    let book = BookIterator::new([MarketData::from(trade)].into_iter(), 0)
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
        .map(Market::get_execunix)
        .collect::<Vec<_>>();
    execunix.sort_unstable();
    assert_eq!(execunix, [Some(1), Some(2)]);
}

/// A snapshot component dated after the snapshot it stands in is never
/// backdated: the walk leaves it out with a warning, and a direct
/// `add_operations`, an explicit call, refuses it.
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
    op_mut(&mut execution).set_snapunix(Some(2));
    execution.finalize();
    assert!(
        BookIterator::new([execution.clone()].into_iter(), 0)
            .unwrap()
            .next()
            .is_none()
    );

    op_mut(&mut execution).set_snapunix(Some(4));
    execution.finalize();
    let mut direct = BookEvent::new(3, "IBM");
    let error = direct.add_operations([execution]).unwrap_err().to_string();
    assert!(error.contains("operation[0].snapunix"), "{error}");

    let mut reset = ExecutionEvent::at(3);
    reset.set_crosscode("RESET-FUTURE".to_owned());
    reset.set_ticker(Some(SmolStr::new("IBM")), true);
    reset.set_snapunix(Some(2));
    reset.finalize();
    let control = SnapshotEvent::snapshot(&reset, Some(SmolStr::new("PRIMARY")));
    assert!(
        BookIterator::new([MarketData::from(control)].into_iter(), 0)
            .unwrap()
            .next()
            .is_none()
    );
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
    let control = SnapshotEvent::snapshot(&reset, Some(SmolStr::new("OTHER")));

    let books = BookIterator::new([primary, other, MarketData::from(control)].into_iter(), 0)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(books.len(), 2);
    assert_eq!(books[1].get_seqnum(), 9);
    assert_eq!(books[1].get_snapunix(), Some(2));
    assert_eq!(alive(&books[1], true).len(), 1);
    assert_eq!(
        alive(&books[1], true)
            .into_iter()
            .next()
            .unwrap()
            .get_crosscode(),
        "14:1:PRIMARY"
    );
}

#[test]
fn merging_books_uses_the_latest_recording_as_reference_and_keeps_earliest_clocks() {
    let book = |identity: &str, side: &str, price: &str, recdunix: i64, feed: &str| {
        let mut book = BookEvent::new(100, "IBM");
        book.add_operations([operation(
            "quote", "IBM", identity, 100, side, price, 2, "New",
        )])
        .unwrap();
        book.set_recdunix(Some(recdunix));
        book.set_metadata(
            Some(BTreeMap::from([(SmolStr::new("Feed"), SmolStr::new(feed))])),
            true,
        );
        book.finalize();
        book
    };
    let mut older = book("B-1", "Buy", "100", 20, "OLDER");
    older.set_execunix(Some(7), true);
    older.finalize();
    let mut latest = book("A-1", "Sell", "102", 30, "LATEST");
    latest.set_execunix(Some(9), true);
    latest.finalize();

    let left = older.clone().merge_with(&latest).unwrap();
    let right = latest.clone().merge_with(&older).unwrap();
    for merged in [&left, &right] {
        assert_eq!(alive(merged, true).len(), 1);
        assert_eq!(alive(merged, false).len(), 1);
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
    assert_eq!(alive(&folded, true).len(), 2);
    assert_eq!(alive(&folded, false).len(), 1);
}

#[test]
fn operation_kind_participates_in_book_identity() {
    let mut order_book = BookEvent::new(1, "IBM");
    order_book
        .add_operations([operation("order", "IBM", "B-1", 1, "Buy", "100", 1, "New")])
        .unwrap();
    let mut quote_book = BookEvent::new(1, "IBM");
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

    let mut left = BookEvent::new(1, "IBM");
    left.add_operations([left_operation]).unwrap();
    let mut right = BookEvent::new(1, "IBM");
    right.add_operations([right_operation]).unwrap();

    assert_eq!(
        left.limits(Side::Buy).collect::<Vec<_>>(),
        right.limits(Side::Buy).collect::<Vec<_>>()
    );
    assert_eq!(left.get_curruuid(), right.get_curruuid());
    assert_eq!(left.get_currhashcode(), right.get_currhashcode());
}

#[test]
fn restating_rederives_the_book_identity_after_holder_restatement() {
    let mut live = BookEvent::new(1, "IBM");
    live.add_operations([
        operation("quote", "IBM", "B-1", 1, "Buy", "100", 2, "New"),
        operation("quote", "IBM", "A-1", 1, "Sell", "102", 4, "New"),
    ])
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
    for side in [Side::Buy, Side::Sell] {
        assert_eq!(
            restated.limits(side).collect::<Vec<_>>(),
            canonical.limits(side).collect::<Vec<_>>()
        );
    }
}

#[test]
fn range_deletes_are_positive_in_range_and_scope_local() {
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([
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
    ])
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
    book.add_operations([deletion]).unwrap();
    let remaining = alive(&book, true)
        .into_iter()
        .map(Element::get_crosscode)
        .collect::<Vec<_>>();
    assert_eq!(remaining, ["14:1:O-1", "14:1:P-2"]);

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
    assert!(book.add_operations([invalid]).is_err());
    assert_eq!(book, before);
}

#[test]
fn an_anonymous_new_entry_refuses_to_replace_an_occupied_position() {
    let positioned = || BookRef {
        action: Some(MdUpdateAction::New),
        position: Some(1),
        ..BookRef::default()
    };
    let mut book = BookEvent::new(1, "IBM");
    let first = anonymous(with_book(
        operation("quote", "IBM", "POSITION-1", 1, "Buy", "100", 1, "New"),
        positioned(),
    ));
    book.add_operations([first]).unwrap();

    let collision = anonymous(with_book(
        operation("quote", "IBM", "POSITION-1", 2, "Buy", "101", 2, "New"),
        positioned(),
    ));
    let before = book.clone();
    let error = book.add_operations([collision]).unwrap_err().to_string();
    assert!(error.contains("existing position"), "{error}");
    assert_eq!(book, before);

    let mut book = BookEvent::new(1, "IBM");
    let explicit = with_book(
        operation("quote", "IBM", "EXPLICIT", 1, "Buy", "100", 1, "New"),
        positioned(),
    );
    book.add_operations([explicit]).unwrap();
    let anonymous_entry = anonymous(with_book(
        operation("quote", "IBM", "ANONYMOUS", 2, "Buy", "101", 1, "New"),
        positioned(),
    ));
    let before = book.clone();
    let error = book
        .add_operations([anonymous_entry])
        .unwrap_err()
        .to_string();
    assert!(error.contains("existing position"), "{error}");
    assert_eq!(book, before);
}

#[test]
fn advancing_time_clears_previous_deltas_and_executions_and_rejects_regression() {
    let mut execution = ExecutionEvent::at(1);
    execution.set_crosscode("E-1".to_owned());
    execution.set_ticker(Some(SmolStr::new("IBM")), true);
    execution.set_state(State::read("Filled").unwrap());
    execution.finalize();
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([
        operation("quote", "IBM", "B-1", 1, "Buy", "100", 1, "New"),
        MarketData::from(execution),
    ])
    .unwrap();
    assert_eq!(book.executions().len(), 1);
    book.set_snapunix(Some(1));
    book.finalize();

    book.add_operations([operation("quote", "IBM", "A-1", 2, "Sell", "102", 2, "New")])
        .unwrap();
    assert_eq!(book.get_snapunix(), None);
    assert!(book.executions().is_empty());
    assert!(deltas(&book, true).is_empty());
    assert_eq!(deltas(&book, false).len(), 1);
    let mut canonical = book.clone();
    canonical.finalize();
    assert_eq!(book, canonical);

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
    assert!(deltas(&book, true).is_empty());
    assert!(deltas(&book, false).is_empty());
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
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([operation("order", "IBM", "O-1", 1, "Buy", "100", 1, "New")])
        .unwrap();
    book.add_operations([operation(
        "order", "IBM", "O-1", 2, "Sell", "101", 1, "Replaced",
    )])
    .unwrap();
    assert!(alive(&book, true).is_empty());
    assert_eq!(alive(&book, false).len(), 1);
}

#[test]
fn book_identity_includes_deeper_levels_and_book_merge_is_idempotent() {
    let mut shallow = BookEvent::new(1, "IBM");
    shallow
        .add_operations([operation("quote", "IBM", "B-1", 1, "Buy", "100", 1, "New")])
        .unwrap();
    let mut deep = shallow.clone();
    deep.add_operations([operation("quote", "IBM", "B-2", 1, "Buy", "99", 1, "New")])
        .unwrap();
    // The same best on both, one level deeper on the second.
    assert_eq!(shallow.best_price(Side::Buy), deep.best_price(Side::Buy));
    assert_ne!(shallow.get_curruuid(), deep.get_curruuid());

    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([operation("quote", "IBM", "B-1", 1, "Buy", "100", 1, "New")])
        .unwrap();
    assert!(book.clone().merge_with(&book).is_none());

    let final_live = operation("quote", "IBM", "B", 2, "Buy", "100", 1, "New");
    let mut direct = BookEvent::new(2, "IBM");
    direct.add_operations([final_live.clone()]).unwrap();
    let mut with_history = BookEvent::new(2, "IBM");
    with_history
        .add_operations([
            operation("quote", "IBM", "TRANSIENT", 2, "Buy", "99", 1, "New"),
            operation("quote", "IBM", "TRANSIENT", 2, "Buy", "99", 0, "Canceled"),
            final_live,
        ])
        .unwrap();
    assert_eq!(
        alive(&direct, true).into_iter().collect::<Vec<_>>(),
        alive(&with_history, true).into_iter().collect::<Vec<_>>()
    );
    assert_ne!(deltas(&direct, true), deltas(&with_history, true));
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
    )
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
    let mut reference = views[1].clone();
    assert!(deltas(&reference, true).is_empty());
    reference.set_recdunix(Some(20));
    reference.finalize();

    let mut supplement = BookEvent::new(2_000_000, "IBM");
    supplement
        .add_operations([operation(
            "quote", "IBM", "A-X", 2_000_000, "Sell", "103", 1, "New",
        )])
        .unwrap();
    supplement.set_recdunix(Some(10));
    supplement.finalize();
    let merged = supplement.merge_with(&reference).unwrap();
    assert_eq!(alive(&merged, true).len(), 1);
    assert!(alive(&merged, false).is_empty());
    assert!(deltas(&merged, false).is_empty());
}

#[test]
fn an_empty_snapshot_reference_does_not_refill_replaced_scope_on_merge() {
    let mut older = BookEvent::new(2, "IBM");
    older
        .add_operations([
            with_book(
                operation("quote", "IBM", "B-1", 2, "Buy", "100", 1, "New"),
                scoped("PRIMARY"),
            ),
            with_book(
                operation("quote", "IBM", "B-X", 2, "Buy", "99", 2, "New"),
                scoped("OTHER"),
            ),
        ])
        .unwrap();
    older.set_recdunix(Some(10));
    older.finalize();

    let reset = reset_event(2, "RESET");
    let control = SnapshotEvent::snapshot(&reset, Some(SmolStr::new("PRIMARY")));
    let mut latest = BookEvent::new(2, "IBM");
    latest.add_operations([MarketData::from(control)]).unwrap();
    latest.set_recdunix(Some(20));
    latest.finalize();

    let continued = latest.clone().with_previous(&older).unwrap();
    assert_eq!(alive(&continued, true).len(), 1);
    assert_eq!(
        alive(&continued, true)
            .into_iter()
            .next()
            .unwrap()
            .get_crosscode(),
        "14:1:B-X"
    );

    let merged = older.merge_with(&latest).unwrap();
    assert_eq!(alive(&merged, true).len(), 1);
    assert_eq!(
        alive(&merged, true)
            .into_iter()
            .next()
            .unwrap()
            .get_crosscode(),
        "14:1:B-X"
    );
    assert!(alive(&merged, false).is_empty());
}

#[test]
fn decimal_means_do_not_overflow_representable_results() {
    let bid = Decimal::from_units(Decimal::MAX.units() - 2).unwrap();
    let ask = Decimal::MAX;
    let mut bid_operation = operation("quote", "IBM", "B", 1, "Buy", "0", 1, "New");
    bid_operation.set_price(Some(bid), true);
    bid_operation.set_quantity(Some(Decimal::MAX), true);
    bid_operation.finalize();
    let mut ask_operation = operation("quote", "IBM", "A", 1, "Sell", "0", 1, "New");
    ask_operation.set_price(Some(ask), true);
    ask_operation.set_quantity(Some(Decimal::MAX), true);
    ask_operation.finalize();
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([bid_operation, ask_operation]).unwrap();
    assert_eq!(
        book.bbo_midpoint(),
        Decimal::from_units(Decimal::MAX.units() - 1)
    );
    assert_eq!(book.median_quantity(), Some(Decimal::MAX));
}

/// A quote stating no side is left out of its book - the book it would
/// have folded into still stands with what else its instant stated - and
/// the walk goes on to the next book.
#[test]
fn an_unsided_quote_leaves_its_group_and_every_later_book_standing() {
    let mut invalid = operation("quote", "IBM", "BAD", 1, "Buy", "99", 1, "New");
    invalid.set_side(Side::Unknown, true);
    invalid.finalize();
    let results = BookIterator::new(
        [
            operation("quote", "IBM", "GOOD", 1, "Buy", "100", 1, "New"),
            invalid,
            operation("quote", "MSFT", "LATER", 2_000_000, "Buy", "200", 1, "New"),
        ]
        .into_iter(),
        1,
    )
    .unwrap()
    .collect::<Vec<_>>();
    assert!(results.iter().all(Result::is_ok));
    let first = results[0].as_ref().unwrap();
    assert_eq!(first.get_ticker(), Some("IBM"));
    assert_eq!(alive(first, true).len(), 1, "the good quote alone");
    let later = results.last().unwrap().as_ref().unwrap();
    assert_eq!(later.get_ticker(), Some("MSFT"));
    assert_eq!(
        alive(later, true)
            .into_iter()
            .next()
            .unwrap()
            .get_crosscode(),
        "14:1:LATER"
    );
}

/// One book from the synthetic operations, however they arrive: each as a
/// operation in a group of its own, or all in one
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
            with_identifiers(
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

    let mut one_by_one = BookEvent::new(5, "IBM");
    for operation in operations() {
        one_by_one.add_operations([operation]).unwrap();
    }
    let mut grouped = BookEvent::new(5, "IBM");
    grouped.add_operations(operations()).unwrap();

    assert_eq!(alive(&one_by_one, true).len(), 1);
    assert_eq!(alive(&one_by_one, false).len(), 2);
    assert_eq!(deltas(&one_by_one, true).len(), 3);
    assert_eq!(deltas(&one_by_one, false).len(), 2);
    assert_eq!(one_by_one.executions().len(), 2);
    for (arrived, in_group) in [
        (
            facts(alive(&one_by_one, true)),
            facts(alive(&grouped, true)),
        ),
        (
            facts(alive(&one_by_one, false)),
            facts(alive(&grouped, false)),
        ),
        (
            facts(deltas(&one_by_one, true)),
            facts(deltas(&grouped, true)),
        ),
        (
            facts(deltas(&one_by_one, false)),
            facts(deltas(&grouped, false)),
        ),
        (
            one_by_one.executions().iter().map(fact).collect::<Vec<_>>(),
            grouped.executions().iter().map(fact).collect::<Vec<_>>(),
        ),
    ] {
        assert_eq!(arrived, in_group);
    }
    assert_eq!(
        one_by_one.best_price(Side::Buy),
        grouped.best_price(Side::Buy)
    );
    assert_eq!(
        one_by_one.best_price(Side::Sell),
        grouped.best_price(Side::Sell)
    );
    assert_eq!(one_by_one.bbo_midpoint(), grouped.bbo_midpoint());
}

/// A quote of `identity` stating no ticker, of the market and the
/// classification given, finalized.
fn unticked(identity: &str, unix: i64, mic: Option<&str>, cfi: Option<&str>) -> MarketData {
    let mut input = operation("quote", "X", identity, unix, "Buy", "100", 1, "New");
    input.set_ticker(None, true);
    input.set_miccode(mic.map(|code| yggdryl::Mic::new(code).unwrap()), true);
    input.set_cficode(cfi.map(|code| yggdryl::Cfi::new(code).unwrap()), true);
    input.finalize();
    input
}

fn books_of(inputs: Vec<MarketData>) -> Vec<BookEvent> {
    BookIterator::new(inputs.into_iter(), 0)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

/// An input stating no ticker is booked under its category -
/// `{miccode}:{cficode}`, `XXXX` and `XXXXXX` for what it does not name -
/// and the book states no ticker; a ticker input keeps its ticker book.
#[test]
fn a_tickerless_input_is_booked_under_its_market_and_classification() {
    let books = books_of(vec![
        unticked("Q-1", 1, None, None),
        unticked("Q-2", 1, Some("XPAR"), Some("ESVUFR")),
        operation("quote", "ACME", "Q-3", 1, "Buy", "100", 1, "New"),
    ]);
    let keys: Vec<(&str, Option<&str>)> = books
        .iter()
        .map(|book| (book.get_crosscode(), book.get_ticker()))
        .collect();
    assert_eq!(
        keys,
        [
            ("3:0:ACME", Some("ACME")),
            ("3:0:XPAR:ESVUFR", None),
            ("3:0:XXXX:XXXXXX", None),
        ]
    );
    assert!(books.iter().all(|book| alive(book, true).len() == 1));
}

/// A categorized book takes only what is keyed to it; a ticker book takes
/// a ticker-less input and refuses another ticker.
#[test]
fn each_book_shape_refuses_an_input_keyed_elsewhere() {
    let mut categorized = books_of(vec![unticked("Q-1", 1, Some("XPAR"), Some("ESVUFR"))])
        .pop()
        .unwrap();
    let error = categorized
        .add_operations([operation("quote", "ACME", "Q-2", 1, "Buy", "100", 1, "New")])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(r#"expected book crosscode "XPAR:ESVUFR", got ticker "ACME""#),
        "{error}"
    );
    let error = categorized
        .add_operations([unticked("Q-3", 1, Some("XNAS"), Some("ESVUFR"))])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(r#"expected book crosscode "XPAR:ESVUFR", got "XNAS:ESVUFR""#),
        "{error}"
    );
    categorized
        .add_operations([unticked("Q-4", 1, Some("XPAR"), Some("ESVUFR"))])
        .expect("an input of the book's own category");

    let mut ticker = BookEvent::new(1, "ACME");
    ticker
        .add_operations([unticked("Q-5", 1, None, None)])
        .expect("a ticker book takes a ticker-less input");
    let error = ticker
        .add_operations([operation("quote", "MSFT", "Q-6", 1, "Buy", "100", 1, "New")])
        .unwrap_err()
        .to_string();
    assert!(error.contains(r#"expected "ACME", got "MSFT""#), "{error}");
}

/// A ticker can spell a category key: the input stating it and a
/// ticker-less input of that category share one book, in either order,
/// and the first to arrive decides whether the book states a ticker.
#[test]
fn a_ticker_spelling_a_category_shares_its_book_in_either_order() {
    let spelled = || operation("quote", "XPAR:ESVUFR", "Q-1", 1, "Buy", "100", 1, "New");
    let categorized = || unticked("Q-2", 2, Some("XPAR"), Some("ESVUFR"));

    let books = books_of(vec![spelled(), categorized()]);
    assert_eq!(books.len(), 2);
    assert!(
        books
            .iter()
            .all(|book| book.get_crosscode() == "3:0:XPAR:ESVUFR")
    );
    assert!(
        books
            .iter()
            .all(|book| book.get_ticker() == Some("XPAR:ESVUFR"))
    );
    assert_eq!(alive(&books[1], true).len(), 2);

    let later = |unix: i64| operation("quote", "XPAR:ESVUFR", "Q-1", unix, "Buy", "100", 1, "New");
    let books = books_of(vec![
        unticked("Q-2", 1, Some("XPAR"), Some("ESVUFR")),
        later(2),
    ]);
    assert_eq!(books.len(), 2);
    assert!(
        books
            .iter()
            .all(|book| book.get_crosscode() == "3:0:XPAR:ESVUFR")
    );
    assert!(books.iter().all(|book| book.get_ticker().is_none()));
    assert_eq!(alive(&books[1], true).len(), 2);
}

/// An entry of a categorized book expires in that book, at its deadline.
#[test]
fn a_categorized_entry_expires_in_its_own_book() {
    let mut live = unticked("Q-1", 1, Some("XPAR"), Some("ESVUFR"));
    op_mut(&mut live).set_exprunix(Some(3));
    live.finalize();
    let books = books_of(vec![live, unticked("Q-2", 5, None, None)]);
    let keyed: Vec<(i64, &str, usize)> = books
        .iter()
        .map(|book| {
            (
                book.get_currunix(),
                book.get_crosscode(),
                alive(book, true).len(),
            )
        })
        .collect();
    assert_eq!(
        keyed,
        [
            (1, "3:0:XPAR:ESVUFR", 1),
            (3, "3:0:XPAR:ESVUFR", 0),
            (5, "3:0:XXXX:XXXXXX", 1),
        ]
    );
    assert!(books[1].get_ticker().is_none());
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::graph::BookIterator;
    use yggdryl::internals::warning::count;

    use super::operation;

    /// What the walk leaves out it says, once per kind of refusal and then
    /// counted: an unsided entry under its kind.
    #[test]
    fn what_the_walk_leaves_out_is_warned_about_under_its_kind() {
        let unsided = operation("quote", "IBM", "UNSIDED", 1, "Unknown", "101", 1, "New");
        let books = BookIterator::new([unsided].into_iter(), 0)
            .unwrap()
            .collect::<Vec<_>>();
        assert!(books.is_empty());
        assert!(
            count(
                "yggdryl::graph::book",
                "book entry excluded: it states no bid or ask side",
                "quote_event",
            ) >= 1
        );
    }
}
