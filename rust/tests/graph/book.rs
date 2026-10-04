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
    book.alive_on(if bid { Side::Buy } else { Side::Sell })
        .collect()
}

/// A quote of `code` at `unix` on IBM tagging no side, holding the bid and
/// the ask given - each `(price, quantity)`, `None` a leg it states nothing
/// of - in `state`, finalized.
fn two_sided(
    code: &str,
    unix: i64,
    bid: Option<(&str, i64)>,
    ask: Option<(&str, i64)>,
    state: &str,
) -> MarketData {
    let mut quote = QuoteEvent::at(unix);
    quote.set_crosscode(code.to_owned());
    quote.set_ticker(Some(SmolStr::new("IBM")), true);
    if let Some((price, quantity)) = bid {
        quote.set_bidpx(Some(decimal(price)), true);
        quote.set_bidqty(Some(Decimal::from_int(quantity)), true);
    }
    if let Some((price, quantity)) = ask {
        quote.set_askpx(Some(decimal(price)), true);
        quote.set_askqty(Some(Decimal::from_int(quantity)), true);
    }
    quote.set_currency(Ccy::new("USD").unwrap(), true);
    quote.set_state(State::read(state).unwrap());
    quote.set_tradable(Some(true), true);
    quote.finalize();
    MarketData::from(quote)
}

/// The bid and the ask `entry` states, each `(price, quantity)`.
type Legs = (
    (Option<Decimal>, Option<Decimal>),
    (Option<Decimal>, Option<Decimal>),
);

fn legs(entry: &MarketData) -> Legs {
    (
        (entry.get_bidpx(), entry.get_bidqty()),
        (entry.get_askpx(), entry.get_askqty()),
    )
}

/// `(Some(price), Some(quantity))` of one leg.
fn leg(price: &str, quantity: i64) -> (Option<Decimal>, Option<Decimal>) {
    (Some(decimal(price)), Some(Decimal::from_int(quantity)))
}

/// The deltas applied to one side of `book`, in the order they were.
fn deltas(book: &BookEvent, bid: bool) -> Vec<&MarketData> {
    book.deltas()
        .filter(|entry| entry.get_side().is_bid() == bid)
        .collect()
}

/// Every book of `books` whole, as a reader folding a walk's books holds
/// it: a complete one as it is, and one stating its deltas alone rebuilt
/// over the last whole book of its code before it - the empty book a walk
/// starts from where none is.
fn whole(books: &[BookEvent]) -> Vec<BookEvent> {
    let mut last: BTreeMap<String, BookEvent> = BTreeMap::new();
    books
        .iter()
        .map(|book| {
            let whole = if book.is_complete() {
                book.clone()
            } else {
                let origin = BookEvent::new(book.get_currunix(), book.get_crosscode());
                book.clone()
                    .with_previous(last.get(book.get_crosscode()).unwrap_or(&origin))
                    .expect("a book stating its deltas rebuilds over the book before it")
            };
            last.insert(book.get_crosscode().to_owned(), whole.clone());
            whole
        })
        .collect()
}

/// The cross codes `entries` go by, in their order.
fn codes<'a>(entries: impl IntoIterator<Item = &'a MarketData>) -> Vec<&'a str> {
    entries.into_iter().map(Element::get_crosscode).collect()
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
    assert_eq!(book.deltas().len(), 3);

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

/// How many limits `book` states on `side`: the count its size hint
/// answers before a limit is read, counted down as each one is, and equal
/// to the distinct prices its entries rest at - read off the entries, not
/// the levels.
fn counted_levels(book: &BookEvent, side: Side) -> usize {
    let mut limits = book.limits(side);
    let (count, exact) = limits.size_hint();
    assert_eq!(exact, Some(count), "an exact count on {side:?}");
    for left in (0..count).rev() {
        assert!(limits.next().is_some(), "{count} limits on {side:?}");
        assert_eq!(limits.size_hint(), (left, Some(left)), "on {side:?}");
    }
    assert!(limits.next().is_none(), "{count} limits on {side:?}");
    let mut prices: Vec<_> = book.alive_on(side).map(Market::get_price).collect();
    prices.dedup();
    assert_eq!(prices.len(), count, "the distinct prices on {side:?}");
    count
}

/// A side keeps how many levels it holds through every change - an entry
/// opening a level or joining one, leaving one others hold or the last
/// one, moving price, a refused group rolled back and a snapshot clearing
/// its scope - so a `take` or a collection of its limits is sized by a
/// read rather than a walk of the side.
#[test]
fn a_side_counts_its_levels_through_every_change() {
    let buy = |code: &str, price: &str, state: &str| {
        operation("order", "IBM", code, 1, "Buy", price, 1, state)
    };
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([
        buy("B-1", "101", "New"),
        buy("B-2", "101", "New"),
        buy("B-3", "99", "New"),
        unpriced(buy("B-4", "0", "New")),
        operation("order", "IBM", "A-1", 1, "Sell", "102", 1, "New"),
    ])
    .unwrap();
    assert_eq!(counted_levels(&book, Side::Buy), 3);
    assert_eq!(counted_levels(&book, Side::Sell), 1);
    assert_eq!(counted_levels(&book, Side::Unknown), 0);
    // A level between two opens one; an entry at a held level opens none.
    book.add_operations([buy("B-5", "100", "New")]).unwrap();
    book.add_operations([buy("B-6", "99", "New")]).unwrap();
    assert_eq!(counted_levels(&book, Side::Buy), 4);
    // Leaving a level another entry holds keeps it; the last one closes it.
    book.add_operations([buy("B-1", "101", "Canceled")])
        .unwrap();
    assert_eq!(counted_levels(&book, Side::Buy), 4);
    book.add_operations([buy("B-2", "101", "Canceled")])
        .unwrap();
    assert_eq!(counted_levels(&book, Side::Buy), 3);
    // Moving price closes the entry's level and opens another.
    book.add_operations([buy("B-5", "98", "Replaced")]).unwrap();
    assert_eq!(counted_levels(&book, Side::Buy), 3);
    // A group refused after it applied - a level past decimal - rolls back
    // the level it opened with the rest.
    let huge = |code: &str| sized(buy(code, "97", "New"), Decimal::MAX);
    book.add_operations([huge("H-1")]).unwrap();
    assert_eq!(counted_levels(&book, Side::Buy), 4);
    let before = book.clone();
    book.add_operations([buy("B-7", "96", "New"), huge("H-2")])
        .unwrap_err();
    assert_eq!(book, before);
    assert_eq!(counted_levels(&book, Side::Buy), 4);
    // A snapshot clears its scope's levels and opens its own.
    let primary = |code: &str, price: &str, unix: i64, book: BookRef| {
        with_book(
            operation("order", "IBM", code, unix, "Buy", price, 1, "New"),
            book,
        )
    };
    book.add_operations([
        primary("P-1", "95", 1, scoped("PRIMARY")),
        primary("P-2", "94", 1, scoped("PRIMARY")),
    ])
    .unwrap();
    assert_eq!(counted_levels(&book, Side::Buy), 6);
    book.add_operations([primary(
        "P-3",
        "93",
        2,
        acting_in(MdUpdateAction::Snapshot, "PRIMARY"),
    )])
    .unwrap();
    assert_eq!(counted_levels(&book, Side::Buy), 5);
    assert_eq!(counted_levels(&book, Side::Sell), 1);
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
    assert_eq!(identities, ["14:0:B-2", "14:0:B-X"]);
    // A group replacing membership is a snapshot: it states its instant.
    assert_eq!(book.get_snapunix(), Some(2));
    assert!(previous.get_snapunix().is_none());
}

/// A walk emits a book whole at a full refresh, a `W` replacing one scope,
/// beside every entry of the scopes it left alone, and states the deltas
/// alone between: the book after it holds only what changed.
#[test]
fn a_full_refresh_emits_a_complete_book() {
    let first = with_book(
        operation("quote", "IBM", "B-1", 1, "Buy", "100", 2, "New"),
        scoped("PRIMARY"),
    );
    let other = with_book(
        operation("quote", "IBM", "B-X", 2, "Buy", "99", 9, "New"),
        scoped("OTHER"),
    );
    let replacement = with_book(
        operation("quote", "IBM", "B-2", 3, "Buy", "101", 3, "New"),
        acting_in(MdUpdateAction::Snapshot, "PRIMARY"),
    );
    let after = with_book(
        operation("quote", "IBM", "B-X", 4, "Buy", "99", 8, "Replaced"),
        scoped("OTHER"),
    );
    let books = books_of(vec![first, other, replacement, after]);
    // The first book of the code states its delta alone, following no book.
    assert_eq!(
        books.iter().map(BookEvent::is_complete).collect::<Vec<_>>(),
        [false, false, true, false]
    );
    assert_eq!(books[0].get_prevuuid(), None);
    assert_eq!(books[2].get_snapunix(), Some(3));
    assert_eq!(codes(alive(&books[2], true)), ["14:0:B-2", "14:0:B-X"]);
    assert_eq!(codes(books[2].deltas()), ["14:0:B-2"]);
    assert_eq!(books[3].alive().count(), 0);
    assert_eq!(codes(books[3].deltas()), ["14:0:B-X"]);
    assert_eq!(
        codes(alive(&whole(&books)[3], true)),
        ["14:0:B-2", "14:0:B-X"]
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

/// An ISIN an ISIN registry filled moves the book an element without one
/// stands in: unfilled, the second quote - stating the ticker alone -
/// stands in the ticker's book while the first stands in the instrument's;
/// filled from the first quote's row through the ticker index, both stand
/// in the instrument's.
#[test]
fn a_ticker_a_registry_filled_files_the_element_under_the_instruments_book() {
    let first = edited(
        operation("quote", "HOLN", "Q-1", 1_000_000, "Buy", "100", 2, "New"),
        |operation| {
            operation
                .insert_securityid(identifier(&IdType::Isin, "CH0012214059"))
                .unwrap();
        },
    );
    let second = operation("quote", "HOLN", "Q-2", 1_000_000, "Sell", "101", 3, "New");
    let books = |values: Vec<MarketData>| {
        BookIterator::new(values.into_iter(), 0)
            .unwrap()
            .map(|book| {
                let book = book.unwrap();
                (
                    book.book_crosscode().to_owned(),
                    book.get_ticker().map(str::to_owned),
                    book.get_isincode().map(str::to_owned),
                )
            })
            .collect::<Vec<_>>()
    };
    let keyed = |key: &str, isin: Option<&str>| {
        (
            key.to_owned(),
            Some("HOLN".to_owned()),
            isin.map(str::to_owned),
        )
    };
    assert_eq!(
        books(vec![first.clone(), second.clone()]),
        [
            keyed("CH0012214059", Some("CH0012214059")),
            keyed("HOLN", None)
        ]
    );

    let mut registry = yggdryl::IsinRegistry::new();
    let filled = [first, second]
        .into_iter()
        .map(|mut value| {
            on_operation!(&mut value, operation => registry.enrich(operation));
            value
        })
        .collect::<Vec<_>>();
    assert_eq!(op(&filled[1]).get_isincode(), Some("CH0012214059"));
    assert_eq!(books(filled), [keyed("CH0012214059", Some("CH0012214059"))]);
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
    assert_eq!(codes(book.deltas()), ["14:0:IBM-B", "14:0:IBM-A"]);
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
    // everything else purged: no deltas, and the dead order
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
    assert!(
        !died.is_complete(),
        "between ticks a book states its deltas"
    );
    assert_eq!(
        deltas(died, true).len(),
        1,
        "the cancel is the delta of the book it died in"
    );
    assert_eq!(
        alive(&died.clone().with_previous(at(2_000_000)).unwrap(), true).len(),
        1,
        "the survivor stays live"
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
    assert!(alive(snapshot, false).into_iter().next().is_none());
}

/// With no grid and no snapshot input, no book is emitted whole - its
/// first appearance included, which follows no book: every book states its
/// own deltas alone, its alive entries purged, and is whole again over the
/// book before it, the first over the empty book a walk starts from.
#[test]
fn only_snapshot_books_carry_their_alive_entries() {
    let operations = vec![
        operation("order", "IBM", "O-1", 1_000_000, "Buy", "100", 2, "New"),
        operation("order", "IBM", "O-2", 1_500_000, "Buy", "101", 3, "New"),
        operation(
            "order", "IBM", "O-2", 2_500_000, "Buy", "101", 3, "Canceled",
        ),
        operation("order", "IBM", "O-3", 3_500_000, "Buy", "99", 1, "New"),
    ];
    let books = books_of(operations);
    assert_eq!(books.len(), 4);
    assert_eq!(books[0].get_prevuuid(), None);
    for book in &books {
        assert!(!book.is_complete());
        assert!(book.get_snapunix().is_none());
        assert_eq!(book.alive().count(), 0);
        assert_eq!(book.limits(Side::Buy).count(), 0);
        assert_eq!(book.depth(Side::Buy, 1), None);
        assert_eq!(book.deltas().len(), 1);
    }
    let whole = whole(&books);
    assert_eq!(
        whole
            .iter()
            .map(|book| alive(book, true).len())
            .collect::<Vec<_>>(),
        [1, 2, 1, 2],
        "both live at 1.5 ms, the survivor at 2.5 ms, and the newcomer beside it at 3.5 ms"
    );
    // Each rebuilt book is the one the walk stated, whole.
    for (book, rebuilt) in books.iter().zip(&whole) {
        assert_eq!(rebuilt.get_curruuid(), book.get_curruuid());
        assert_eq!(rebuilt.get_bidpx(), book.get_bidpx());
        assert_eq!(codes(rebuilt.deltas()), codes(book.deltas()));
    }
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
    let whole = whole(&books);
    let previous = alive(&whole[1], false).into_iter().next().unwrap();
    assert_eq!(identifiers_of(previous).get(&ENTRY_ID), Some("Y"));
    assert!(alive(&whole[2], true).is_empty());
    assert!(alive(&whole[2], false).is_empty());
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

/// A book folds what `MarketDataKind::is_booked` admits into its sides and
/// records what `is_recorded` admits beyond it among its deltas: a trade
/// is pruned before the fold, so a group of nothing else changes nothing -
/// the instant does not advance and the deltas stand - and an execution
/// moves no entry and no level - its fill moved the book through its
/// order's own report - but advances the instant, stands among the deltas
/// and dates the book's last execution.
#[test]
fn an_execution_is_recorded_among_the_deltas_and_a_trade_is_pruned() {
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New")])
        .unwrap();
    let before = book.clone();

    let execution = operation("execution", "IBM", "E-1", 10, "Buy", "100", 2, "Filled");
    let mut root = ExecutionEvent::at(10);
    root.set_crosscode("T-1".to_owned());
    root.set_ticker(Some(SmolStr::new("IBM")), true);
    root.set_state(State::read("Filled").unwrap());
    root.finalize();
    let fill = ExecutionEvent::try_from(execution.clone()).unwrap();
    let trade = TradeEvent::from_parts(&root, vec![fill]).unwrap();

    // A trade alone is pruned: nothing changes, the instant stands.
    book.add_operations([MarketData::from(trade)]).unwrap();
    assert_eq!(book, before);
    assert_eq!(book.get_currunix(), 1);
    assert_eq!(codes(book.deltas()), ["10:1:O-1"]);

    // An execution alone is recorded: the instant advances to it, the
    // deltas are the execution, and the entries and the levels stand.
    book.add_operations([execution.clone()]).unwrap();
    assert_eq!(book.get_currunix(), 10);
    assert_eq!(codes(book.deltas()), ["8:1:E-1"]);
    assert_eq!(codes(book.alive()), ["10:1:O-1"]);
    assert_eq!(book.best_price(Side::Buy), before.best_price(Side::Buy));
    assert_eq!(
        book.best_quantity(Side::Buy),
        before.best_quantity(Side::Buy)
    );
    assert_eq!(book.get_execunix(), execution.get_execunix());

    // Beside an order, both stand among the deltas in the order applied,
    // the order alone moves a side, and the execution's own instant dates
    // the book's last execution.
    let mut fill = operation("execution", "IBM", "E-2", 11, "Buy", "100", 1, "Filled");
    op_mut(&mut fill).set_execunix(Some(11), true);
    fill.finalize();
    book.add_operations([
        fill,
        operation("order", "IBM", "O-2", 11, "Sell", "101", 3, "New"),
    ])
    .unwrap();
    assert_eq!(book.get_currunix(), 11);
    assert_eq!(codes(book.deltas()), ["8:1:E-2", "10:2:O-2"]);
    assert_eq!(book.alive().count(), 2);
    assert_eq!(book.get_execunix(), Some(11));
}

#[test]
fn expiry_precedes_an_equal_time_source_and_an_execution_never_enters_live_expiry() {
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
    // The execution was recorded at its instant, resting on no side and
    // scheduling no expiry: the first book states it among its deltas,
    // and nothing holds it alive.
    assert_eq!(codes(books[0].deltas()), ["10:1:O-1", "8:0:E-1"]);
    let whole = whole(&books);
    assert_eq!(codes(whole[0].alive()), ["10:1:O-1"]);
    let replacement = alive(&whole[1], true).into_iter().next().unwrap();
    assert_eq!(replacement.get_price(), Some(decimal("101")));
    assert_eq!(
        (op(replacement).get_seqnum(), op(replacement).get_prevuuid()),
        (0, None)
    );
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
    let whole = whole(&books);
    assert_eq!(alive(&whole[1], true).len(), 1);
    assert_eq!(
        alive(&whole[1], true)
            .into_iter()
            .next()
            .unwrap()
            .get_crosscode(),
        "14:0:STANDING"
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
    let later = operation("order", "IBM", "LATER", 3, "Sell", "100", 1, "New");

    let results = BookIterator::new(
        [initial, raw, duplicate.clone(), duplicate, later].into_iter(),
        0,
    )
    .unwrap()
    .collect::<Vec<_>>();
    assert_eq!(results.len(), 2);
    let books = results.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
    let whole = whole(&books);
    let after = &whole[1];
    assert_eq!(alive(after, true).len(), 1);
    assert_eq!(
        alive(after, true)
            .into_iter()
            .next()
            .unwrap()
            .get_crosscode(),
        "14:0:INITIAL"
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
    assert!(!books[0].is_complete());
    assert_eq!(alive(&whole(&books)[0], true).len(), 3);
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

/// A snapshot component dated after the snapshot it stands in is never
/// backdated: the walk leaves it out with a warning, and a direct
/// `add_operations`, an explicit call, refuses it.
#[test]
fn future_snapshot_components_are_refused_instead_of_backdated() {
    let mut future = operation("order", "IBM", "O-FUTURE", 3, "Buy", "101", 1, "New");
    op_mut(&mut future).set_snapunix(Some(2));
    future.finalize();
    assert!(
        BookIterator::new([future.clone()].into_iter(), 0)
            .unwrap()
            .next()
            .is_none()
    );

    op_mut(&mut future).set_snapunix(Some(4));
    future.finalize();
    let mut direct = BookEvent::new(3, "IBM");
    let error = direct.add_operations([future]).unwrap_err().to_string();
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

/// A book is emitted where it holds a delta, or at a snapshot tick where
/// it holds an entry: a grid tick finding a book empty that no group
/// changed there emits nothing - the next book follows the last one
/// emitted - while a snapshot emptying a book is emitted, empty and whole,
/// for the books after it to rebuild over.
#[test]
fn a_book_is_emitted_where_it_holds_a_delta_or_a_snapshot_holds_an_entry() {
    let ms = |at: i64| at * 1_000_000;
    let books = BookIterator::new(
        [
            operation("order", "IBM", "O-1", ms(1), "Buy", "100", 1, "New"),
            operation("order", "IBM", "O-1", ms(2), "Buy", "100", 0, "Canceled"),
            operation("order", "IBM", "GONE", ms(5), "Buy", "100", 0, "Canceled"),
        ]
        .into_iter(),
        1,
    )
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
    assert_eq!(
        books.iter().map(Event::get_currunix).collect::<Vec<_>>(),
        [ms(1), ms(2), ms(5)],
        "the empty ticks at 3 and 4 ms are left out"
    );
    assert!(books.iter().all(BookEvent::is_complete));
    assert_eq!(books[2].get_prevuuid(), Some(books[1].get_curruuid()));
    assert_eq!(books[2].get_prevunix(), Some(ms(2)));
    assert_eq!(codes(books[2].deltas()), ["10:1:GONE"]);

    let mut reset = reset_event(2, "RESET");
    reset.set_snapunix(Some(2));
    reset.finalize();
    let control = SnapshotEvent::snapshot(&reset, None);
    let books = books_of(vec![
        operation("order", "IBM", "O-1", 1, "Buy", "100", 1, "New"),
        MarketData::from(control),
        operation("order", "IBM", "O-2", 3, "Buy", "101", 1, "New"),
    ]);
    assert_eq!(
        books.iter().map(Event::get_currunix).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert!(books[1].is_complete());
    assert_eq!(books[1].alive().count(), 0);
    assert_eq!(books[1].deltas().len(), 0);
    assert_eq!(books[2].get_prevuuid(), Some(books[1].get_curruuid()));
    assert_eq!(codes(whole(&books)[2].alive()), ["10:1:O-2"]);
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
        "14:0:PRIMARY"
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
    assert_eq!(remaining, ["14:0:O-1", "14:0:P-2"]);

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
fn advancing_time_clears_previous_deltas_and_rejects_regression() {
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([operation("quote", "IBM", "B-1", 1, "Buy", "100", 1, "New")])
        .unwrap();
    book.set_snapunix(Some(1));
    book.finalize();

    book.add_operations([operation("quote", "IBM", "A-1", 2, "Sell", "102", 2, "New")])
        .unwrap();
    assert_eq!(book.get_snapunix(), None);
    assert!(deltas(&book, true).is_empty());
    assert_eq!(deltas(&book, false).len(), 1);
    let mut canonical = book.clone();
    canonical.finalize();
    assert_eq!(book, canonical);

    // An execution alone is recorded: the instant advances to it, the
    // deltas of the instant before go, and it stands as the one delta,
    // resting on no side.
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
    assert_eq!(book.get_currunix(), 3);
    assert_eq!(codes(book.deltas()), ["8:1:E-2"]);
    assert_eq!(deltas(&book, true).len(), 1);
    assert!(deltas(&book, false).is_empty());
    book.add_operations([operation("quote", "IBM", "B-2", 3, "Buy", "101", 1, "New")])
        .unwrap();
    assert_eq!(codes(book.deltas()), ["8:1:E-2", "14:0:B-2"]);
    let mut canonical_book = book.clone();
    canonical_book.finalize();
    assert_eq!(book.get_curruuid(), canonical_book.get_curruuid());
    assert_eq!(book.get_currhashcode(), canonical_book.get_currhashcode());
    assert!(
        book.add_operations([operation("quote", "IBM", "OLD", 1, "Buy", "99", 1, "New")])
            .is_err()
    );
}

/// FIX scopes an `MDEntryID(278)` by its `MDEntryType(269)`: an entry of
/// the other side going by one id is another entry, and only a change of
/// the id finding none of its own side moves the entry it names there,
/// retiring the old side.
#[test]
fn an_entry_id_names_one_entry_per_side_and_a_change_moves_it() {
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([operation("order", "IBM", "O-1", 1, "Buy", "100", 1, "New")])
        .unwrap();
    let mut other = book.clone();
    other
        .add_operations([operation(
            "order", "IBM", "O-1", 2, "Sell", "101", 1, "Replaced",
        )])
        .unwrap();
    assert_eq!(alive(&other, true).len(), 1);
    assert_eq!(alive(&other, false).len(), 1);

    book.add_operations([with_book(
        operation("order", "IBM", "O-1", 2, "Sell", "101", 1, "Replaced"),
        BookRef {
            action: Some(MdUpdateAction::Change),
            entry_px: Some(decimal("101")),
            entry_size: Some(decimal("1")),
            ..BookRef::default()
        },
    )])
    .unwrap();
    assert!(alive(&book, true).is_empty());
    assert_eq!(alive(&book, false).len(), 1);
}

/// A bid and an offer going by one `MDEntryID` and no order are two quotes
/// tagging their sides: each rests on its own side, and a change to the
/// offer moves the offer alone.
#[test]
fn a_bid_and_an_offer_going_by_one_entry_id_are_two_entries() {
    let level = |code: &str, unix: i64, side: &str, price: &str, quantity: i64, action| {
        with_identifiers(
            with_book(
                operation("quote", "IBM", code, unix, side, price, quantity, "New"),
                BookRef {
                    action: Some(action),
                    entry_px: Some(decimal(price)),
                    entry_size: Some(Decimal::from_int(quantity)),
                    ..BookRef::default()
                },
            ),
            &[(ENTRY_ID, "E1")],
        )
    };
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([
        level("BID-E1", 1, "Buy", "99", 5, MdUpdateAction::New),
        level("ASK-E1", 1, "Sell", "101", 6, MdUpdateAction::New),
    ])
    .unwrap();
    assert_eq!(codes(book.alive_on(Side::Buy)), ["14:0:BID-E1"]);
    assert_eq!(codes(book.alive_on(Side::Sell)), ["14:0:ASK-E1"]);
    book.add_operations([level("ASK-E1", 2, "Sell", "102", 7, MdUpdateAction::Change)])
        .unwrap();
    assert_eq!(codes(book.alive_on(Side::Buy)), ["14:0:BID-E1"]);
    assert_eq!(book.best_price(Side::Buy), Some(decimal("99")));
    assert_eq!(codes(book.alive_on(Side::Sell)), ["14:0:ASK-E1"]);
    assert_eq!(book.best_price(Side::Sell), Some(decimal("102")));
    assert_eq!(book.best_quantity(Side::Sell), Some(Decimal::from_int(7)));
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
            // Another instrument's quote reaches the grid tick, which
            // emits every book as of it.
            operation("quote", "MSFT", "M-1", 2_000_000, "Buy", "200", 1, "New"),
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
    // The same book, its PRIMARY scope then emptied by a snapshot: a later
    // statement of it, and a snapshot, which a merge takes whole.
    let mut latest = older.clone();
    latest.add_operations([MarketData::from(control)]).unwrap();
    latest.set_recdunix(Some(20));
    latest.finalize();
    assert_eq!(latest.get_snapunix(), Some(2));
    assert_eq!(codes(alive(&latest, true)), ["14:0:B-X"]);

    let merged = older.merge_with(&latest).unwrap();
    assert_eq!(alive(&merged, true).len(), 1);
    assert_eq!(
        alive(&merged, true)
            .into_iter()
            .next()
            .unwrap()
            .get_crosscode(),
        "14:0:B-X"
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

/// One book from the synthetic operations, however they arrive: each as a
/// operation in a group of its own, or all in one `add_operations` group,
/// the depth and the deltas state the same prices, quantities and names in
/// the same order - never compared by identity, which the grouping is
/// allowed to move - and the executions among them stand among the deltas
/// and reach no side.
#[test]
fn one_by_one_and_grouped_operations_build_the_same_depth_and_deltas() {
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
    assert_eq!(deltas(&one_by_one, true).len(), 4);
    assert_eq!(deltas(&one_by_one, false).len(), 3);
    assert_eq!(
        codes(one_by_one.deltas()),
        [
            "10:1:O-1", "14:0:Q-1", "14:0:A-1", "10:2:A-2", "8:1:E-1", "14:0:Q-1", "8:2:E-2"
        ]
    );
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
        (facts(one_by_one.deltas()), facts(grouped.deltas())),
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

/// A quote of `identity` stating the ISIN `isin` and the ticker `ticker`,
/// none for an empty one, finalized.
fn listed(identity: &str, unix: i64, isin: &str, ticker: &str) -> MarketData {
    let mut input = operation("quote", ticker, identity, unix, "Buy", "100", 1, "New");
    if ticker.is_empty() {
        input.set_ticker(None, true);
    }
    op_mut(&mut input)
        .insert_securityid(identifier(&IdType::Isin, isin))
        .unwrap();
    input.finalize();
    input
}

fn books_of(inputs: Vec<MarketData>) -> Vec<BookEvent> {
    BookIterator::new(inputs.into_iter(), 0)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

/// An input is booked by its instrument's ISIN where it holds one, else by
/// its ticker, else under the number that states none, `XX0000000000`,
/// whatever market and classification it names; the book states the
/// ticker and the ISIN its first input stated, so an ISIN book carries its
/// ISIN and a book keyed by nothing states neither.
#[test]
fn an_input_is_booked_by_its_isin_else_its_ticker_else_the_default() {
    let books = books_of(vec![
        unticked("Q-1", 1, None, None),
        unticked("Q-2", 1, Some("XPAR"), Some("ESVUFR")),
        operation("quote", "ACME", "Q-3", 1, "Buy", "100", 1, "New"),
        listed("Q-4", 1, "US0378331005", ""),
        listed("Q-5", 1, "CH0012214059", "HOLN"),
    ]);
    let keys: Vec<(&str, Option<&str>, Option<&str>)> = books
        .iter()
        .map(|book| (book.get_crosscode(), book.get_ticker(), book.get_isincode()))
        .collect();
    assert_eq!(
        keys,
        [
            ("3:0:ACME", Some("ACME"), None),
            ("3:0:CH0012214059", Some("HOLN"), Some("CH0012214059")),
            ("3:0:US0378331005", None, Some("US0378331005")),
            ("3:0:XX0000000000", None, None),
        ]
    );
    let whole = whole(&books);
    assert_eq!(
        whole
            .iter()
            .map(|book| alive(book, true).len())
            .collect::<Vec<_>>(),
        [1, 1, 1, 2],
        "the two inputs stating neither ISIN nor ticker share the default book"
    );
    for book in &whole {
        assert_eq!(
            Some(book.book_crosscode()),
            book.get_crosscode().strip_prefix("3:0:"),
            "a book's own facts spell its key"
        );
    }
}

/// A chain stated first by its ticker alone and then under its
/// instrument's ISIN is one entry in one book: the restatement withdraws it
/// from the ticker book - one delta there, removing it and reporting no
/// fill - and opens it in the ISIN book, where the instrument's other
/// entries go; across two instants or within one.
#[test]
fn a_chain_restated_under_its_isin_leaves_its_ticker_book_for_the_instruments() {
    const MS: i64 = 1_000_000;
    let with_isin = |mut input: MarketData| {
        op_mut(&mut input)
            .insert_securityid(identifier(&IdType::Isin, "US0378331005"))
            .unwrap();
        input.finalize();
        input
    };
    for (second, third) in [(2 * MS, 3 * MS), (MS, 2 * MS)] {
        let books = books_of(vec![
            operation("order", "ACME", "C1", MS, "Buy", "100", 10, "New"),
            with_isin(operation(
                "order", "ACME", "C1", second, "Buy", "101", 10, "Replaced",
            )),
            with_isin(operation(
                "order", "ACME", "C9", third, "Sell", "105", 1, "New",
            )),
        ]);
        let whole = whole(&books);
        let keyed: Vec<(i64, &str, usize, usize)> = whole
            .iter()
            .map(|book| {
                (
                    book.get_currunix(),
                    book.book_crosscode(),
                    alive(book, true).len(),
                    alive(book, false).len(),
                )
            })
            .collect();
        let expected: Vec<(i64, &str, usize, usize)> = if second == MS {
            vec![
                (MS, "ACME", 0, 0),
                (MS, "US0378331005", 1, 0),
                (third, "US0378331005", 1, 1),
            ]
        } else {
            vec![
                (MS, "ACME", 1, 0),
                (second, "ACME", 0, 0),
                (second, "US0378331005", 1, 0),
                (third, "US0378331005", 1, 1),
            ]
        };
        assert_eq!(keyed, expected, "{second}");
        let ticker_book = whole
            .iter()
            .rfind(|book| book.book_crosscode() == "ACME")
            .unwrap();
        let withdrawn = ticker_book
            .deltas()
            .last()
            .expect("the withdrawal is the ticker book's last delta");
        assert_eq!(*op(withdrawn).get_state(), State::Removed, "{second}");
        assert_eq!(
            withdrawn.book().and_then(|book| book.action),
            Some(MdUpdateAction::Delete),
            "{second}"
        );
        assert_eq!(
            (op(withdrawn).get_lastpx(), op(withdrawn).get_lastqty()),
            (None, None),
            "{second}"
        );
        let instrument = whole.last().unwrap();
        assert_eq!(
            op(alive(instrument, true)[0]).get_price(),
            Some(decimal("101")),
            "{second}: the restatement stands in the instrument's book"
        );
    }
}

/// A chain stated by its ticker alone and then restated under its
/// instrument's ISIN by a snapshot's member leaves its ticker book too:
/// the member withdraws it there - one delta, removing it - as any
/// restatement does, so the entry rests in the instrument's book alone.
#[test]
fn a_chain_restated_by_a_snapshot_under_its_isin_leaves_its_ticker_book() {
    const MS: i64 = 1_000_000;
    let first = operation("quote", "ACME", "C1", MS, "Buy", "100", 1, "New");
    let mut restated = listed("C1", 2 * MS, "US0378331005", "ACME");
    op_mut(&mut restated).set_snapunix(Some(2 * MS));
    let books = whole(&books_of(vec![first, restated]));
    let keyed: Vec<(i64, &str, usize)> = books
        .iter()
        .map(|book| {
            (
                book.get_currunix(),
                book.book_crosscode(),
                alive(book, true).len(),
            )
        })
        .collect();
    assert_eq!(
        keyed,
        [
            (MS, "ACME", 1),
            (2 * MS, "ACME", 0),
            (2 * MS, "US0378331005", 1),
        ],
        "the snapshot's member withdraws the entry from the ticker book"
    );
    let withdrawn = books[1]
        .deltas()
        .last()
        .expect("the withdrawal is the ticker book's delta");
    assert_eq!(*op(withdrawn).get_state(), State::Removed);
    assert_eq!(
        (op(withdrawn).get_lastpx(), op(withdrawn).get_lastqty()),
        (None, None),
        "a withdrawal reports no fill"
    );
}

/// A snapshot member of the ticker book and the chain restated under its
/// ISIN at one instant, the member first: the member leaves that snapshot,
/// so the replaced membership does not put back the entry the restatement
/// moved, and the entry rests in the instrument's book alone.
#[test]
fn a_snapshot_member_then_a_restatement_at_one_instant_rests_in_one_book() {
    const MS: i64 = 1_000_000;
    let first = operation("quote", "ACME", "C1", MS, "Buy", "100", 1, "New");
    let mut member = operation("quote", "ACME", "C1", 2 * MS, "Buy", "100", 1, "New");
    op_mut(&mut member).set_snapunix(Some(2 * MS));
    let restated = listed("C1", 2 * MS, "US0378331005", "ACME");
    let books = whole(&books_of(vec![first, member, restated]));
    let keyed: Vec<(i64, &str, usize)> = books
        .iter()
        .map(|book| {
            (
                book.get_currunix(),
                book.book_crosscode(),
                alive(book, true).len(),
            )
        })
        .collect();
    assert_eq!(
        keyed,
        [
            (MS, "ACME", 1),
            (2 * MS, "ACME", 0),
            (2 * MS, "US0378331005", 1),
        ],
        "the member leaves the ticker book's snapshot with the entry it states"
    );
}

/// A leg stating a negative quantity is refused at `$.quantity` and the
/// group stands as it was: a level adds only what rests at it, so taking an
/// entry off can never push the level past decimal.
#[test]
fn a_negative_leg_quantity_is_refused_and_leaves_the_book_as_it_was() {
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([operation("order", "IBM", "B-1", 1, "Buy", "100", 1, "New")])
        .unwrap();
    let before = book.clone();
    let error = book
        .add_operations([operation("order", "IBM", "B-2", 2, "Buy", "100", -1, "New")])
        .unwrap_err()
        .to_string();
    assert!(error.contains("quantity"), "{error}");
    assert_eq!(book, before);
}

/// An entry the walk expires reports no fill: the delta ending it at its
/// deadline states its last price and quantity and the parts of a fill's
/// price as none, as the lifecycle's own expiration does.
#[test]
fn an_expiration_reports_no_fill() {
    const MS: i64 = 1_000_000;
    let mut partial = operation(
        "order",
        "ACME",
        "O-1",
        MS,
        "Buy",
        "100",
        10,
        "PartiallyFilled",
    );
    {
        let order = op_mut(&mut partial);
        order.set_lastqty(Some(Decimal::from_int(2)), true);
        order.set_lastpx(Some(Decimal::from_int(100)), true);
        order.set_exprunix(Some(5 * MS));
    }
    partial.finalize();
    assert_eq!(op(&partial).get_lastqty(), Some(Decimal::from_int(2)));
    let books = books_of(vec![partial]);
    assert_eq!(books.len(), 2, "the entry, then its expiry");
    assert_eq!(books[1].get_currunix(), 5 * MS);
    let expired = books[1].deltas().next().expect("the expiry is the delta");
    assert_eq!(*op(expired).get_state(), State::Expired);
    assert_eq!(
        (op(expired).get_lastpx(), op(expired).get_lastqty()),
        (None, None),
        "an expiry reports no fill"
    );
    assert_eq!(
        (op(expired).get_spotrate(), op(expired).get_forwardpoints()),
        (None, None),
        "nor the parts of a fill's price"
    );
}

/// A book takes only what is keyed to it - an input whose ISIN, else
/// ticker, spells its key - and any input stating neither; the refusal
/// names the key and the input's.
#[test]
fn each_book_refuses_an_input_keyed_elsewhere() {
    let mut instrument = whole(&books_of(vec![listed("Q-1", 1, "CH0012214059", "HOLN")]))
        .pop()
        .unwrap();
    let error = instrument
        .add_operations([operation("quote", "ACME", "Q-2", 1, "Buy", "100", 1, "New")])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(r#"expected book crosscode "CH0012214059", got "ACME""#),
        "{error}"
    );
    assert!(error.contains("$.operation.ticker"), "{error}");
    let error = instrument
        .add_operations([listed("Q-3", 1, "US0378331005", "HOLN")])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(r#"expected book crosscode "CH0012214059", got "US0378331005""#),
        "{error}"
    );
    assert!(error.contains("$.operation.isincode"), "{error}");
    // Two listings of one instrument stand in its book, apart by their
    // partition, and the book keeps the ticker its first input stated.
    instrument
        .add_operations([listed("Q-4", 1, "CH0012214059", "HOLN.L")])
        .expect("another listing of the book's own instrument");
    instrument
        .add_operations([unticked("Q-5", 1, Some("XPAR"), Some("ESVUFR"))])
        .expect("an input stating neither ISIN nor ticker");
    assert_eq!(instrument.get_ticker(), Some("HOLN"));
    assert_eq!(alive(&instrument, true).len(), 3);

    let mut ticker = BookEvent::new(1, "ACME");
    ticker
        .add_operations([unticked("Q-6", 1, None, None)])
        .expect("a ticker book takes a ticker-less input");
    let error = ticker
        .add_operations([operation("quote", "MSFT", "Q-7", 1, "Buy", "100", 1, "New")])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(r#"expected book crosscode "ACME", got "MSFT""#),
        "{error}"
    );
    let error = ticker
        .add_operations([listed("Q-8", 1, "US0378331005", "ACME")])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(r#"expected book crosscode "ACME", got "US0378331005""#),
        "{error}"
    );

    // The empty book keyed by nothing takes only inputs stating neither.
    let mut none = BookEvent::new(1, "");
    assert_eq!(none.get_crosscode(), "3:0:XX0000000000");
    assert_eq!(none.get_ticker(), None);
    none.add_operations([unticked("Q-9", 1, Some("XPAR"), Some("ESVUFR"))])
        .expect("an input stating neither ISIN nor ticker");
    let error = none
        .add_operations([operation(
            "quote", "ACME", "Q-10", 1, "Buy", "100", 1, "New",
        )])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(r#"expected book crosscode "XX0000000000", got "ACME""#),
        "{error}"
    );
}

/// An entry of the default book expires in that book, at its deadline.
#[test]
fn a_default_books_entry_expires_in_its_own_book() {
    let mut live = unticked("Q-1", 1, Some("XPAR"), Some("ESVUFR"));
    op_mut(&mut live).set_exprunix(Some(3));
    live.finalize();
    let books = whole(&books_of(vec![live, listed("Q-2", 5, "US0378331005", "")]));
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
            (1, "3:0:XX0000000000", 1),
            (3, "3:0:XX0000000000", 0),
            (5, "3:0:US0378331005", 1),
        ]
    );
    assert!(books[1].get_ticker().is_none());
    assert!(books[1].get_isincode().is_none());
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::Side;
    use yggdryl::graph::{BookIterator, Element, Event, MarketData};
    use yggdryl::internals::graph_book::{scheduled_expirations, side_store};
    use yggdryl::internals::logging_warning::count;

    use super::{op_mut, operation};

    /// A walk emits a book whole sharing its sides' stores rather than
    /// copying them, and between ticks states only deltas, which hold no
    /// store: it copies a side's store at its next change only while a whole
    /// book it emitted still holds it, and changes a store it alone holds in
    /// place, however many books stating their deltas a consumer holds.
    #[test]
    fn a_walk_shares_each_side_s_store_with_the_books_it_emits() {
        const MS: i64 = 1_000_000;
        let inputs = || {
            [
                operation("order", "IBM", "B-1", MS, "Buy", "100", 1, "New"),
                operation("order", "IBM", "A-1", MS, "Sell", "101", 1, "New"),
                operation(
                    "order",
                    "IBM",
                    "B-1",
                    MS + 200_000,
                    "Buy",
                    "100",
                    2,
                    "Replaced",
                ),
                operation(
                    "order",
                    "IBM",
                    "B-1",
                    MS + 400_000,
                    "Buy",
                    "100",
                    3,
                    "Replaced",
                ),
                operation(
                    "order",
                    "IBM",
                    "A-2",
                    2 * MS + 500_000,
                    "Sell",
                    "102",
                    1,
                    "New",
                ),
            ]
        };
        // Every book held: whole at 1 ms, its deltas at 1.2 and 1.4 ms,
        // whole at the 2 ms tick, its delta at 2.5 ms.
        let books = BookIterator::new(inputs().into_iter(), 1)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            books
                .iter()
                .map(|book| book.is_complete())
                .collect::<Vec<_>>(),
            [true, false, false, true, false]
        );
        assert_eq!(side_store(&books[1], Side::Buy), None);
        assert_eq!(side_store(&books[4], Side::Sell), None);
        assert_eq!(side_store(&books[0], Side::Unknown), None);
        // The bid changed while the first book held it: copied, the tick
        // holding the walk's copy; the ask, unchanged until after the tick,
        // is the one store both whole books hold.
        let (first_bid, held) = side_store(&books[0], Side::Buy).unwrap();
        assert_eq!(held, 1);
        let (tick_bid, _) = side_store(&books[3], Side::Buy).unwrap();
        assert_ne!(tick_bid, first_bid);
        let (ask, held) = side_store(&books[0], Side::Sell).unwrap();
        assert_eq!((ask, held), side_store(&books[3], Side::Sell).unwrap());
        assert_eq!(held, 2);

        // Nothing but the walk holds a store once its whole book is dropped:
        // the walk changes it in place through every delta, and the tick
        // shares the very store the first book did.
        let mut walk = BookIterator::new(inputs().into_iter(), 1).unwrap();
        let first = walk.next().unwrap().unwrap();
        let (bid, held) = side_store(&first, Side::Buy).unwrap();
        assert_eq!(held, 2, "the walk and its first book");
        drop(first);
        let deltas = [walk.next().unwrap().unwrap(), walk.next().unwrap().unwrap()];
        assert!(deltas.iter().all(|book| !book.is_complete()));
        let tick = walk.next().unwrap().unwrap();
        assert_eq!(side_store(&tick, Side::Buy), Some((bid, 2)));
    }

    /// What the walk places nowhere it says, once per kind and then
    /// counted: a live entry resting on no side and continuing nothing,
    /// under its kind - still the delta of its book.
    #[test]
    fn what_the_walk_places_nowhere_is_warned_about_under_its_kind() {
        let nowhere = operation("quote", "IBM", "NOWHERE", 1, "Unknown", "101", 1, "New");
        let books = BookIterator::new([nowhere].into_iter(), 0)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].deltas().len(), 1);
        assert!(
            count(
                "yggdryl::graph::book",
                "book entry placed nowhere: it rests on neither the bid nor the ask and continues no live entry",
                "quote_event",
            ) >= 1
        );
    }

    /// The walk schedules one deadline per identity: a statement withdraws
    /// the one its predecessor was scheduled for, so amending an entry a
    /// hundred times while an earlier deadline of another stands holds two
    /// deadlines, not a hundred and one.
    #[test]
    fn amending_an_expiring_entry_keeps_one_deadline_scheduled() {
        let expiring = |code: &str, unix: i64, quantity: i64, deadline: i64| {
            let mut entry = operation("order", "IBM", code, unix, "Buy", "100", quantity, "New");
            op_mut(&mut entry).set_exprunix(Some(deadline));
            entry.finalize();
            entry
        };
        let inputs: Vec<MarketData> = std::iter::once(expiring("NEAR", 1, 1, 10_000))
            .chain((0..100).map(|step| expiring("FAR", 2 + step, 1 + step, 20_000)))
            .collect();
        let mut walk = BookIterator::new(inputs.into_iter(), 0).unwrap();
        let mut last = 0;
        while last < 101 {
            last = walk.next().unwrap().unwrap().get_currunix();
        }
        assert_eq!(scheduled_expirations(&walk), 2);
    }
}

/// A book's deltas are the orders and quotes it applied since the book
/// before it, in the order applied across both sides - a cancel among
/// them, though it leaves nothing alive.
#[test]
fn deltas_are_held_in_the_order_applied_across_both_sides() {
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([
        operation("order", "IBM", "A-1", 1, "Sell", "102", 1, "New"),
        operation("order", "IBM", "B-1", 1, "Buy", "100", 2, "New"),
    ])
    .unwrap();
    book.add_operations([operation("quote", "IBM", "A-2", 1, "Sell", "103", 1, "New")])
        .unwrap();
    book.add_operations([operation(
        "order", "IBM", "B-1", 1, "Buy", "100", 2, "Canceled",
    )])
    .unwrap();
    assert_eq!(
        codes(book.deltas()),
        ["10:2:A-1", "10:1:B-1", "14:0:A-2", "10:1:B-1"]
    );
    assert_eq!(codes(book.alive()), ["10:2:A-1", "14:0:A-2"]);

    // The digest reads the deltas in that order: the same deltas applied
    // in another order are another book.
    let mut reordered = BookEvent::new(1, "IBM");
    reordered
        .add_operations([
            operation("order", "IBM", "B-1", 1, "Buy", "100", 2, "New"),
            operation("order", "IBM", "A-1", 1, "Sell", "102", 1, "New"),
        ])
        .unwrap();
    let mut ordered = BookEvent::new(1, "IBM");
    ordered
        .add_operations([
            operation("order", "IBM", "A-1", 1, "Sell", "102", 1, "New"),
            operation("order", "IBM", "B-1", 1, "Buy", "100", 2, "New"),
        ])
        .unwrap();
    assert_eq!(codes(reordered.alive()), codes(ordered.alive()));
    assert_ne!(reordered.get_curruuid(), ordered.get_curruuid());
}

/// `alive_on` reads one side's store best first, the unpriced entry last,
/// and `alive` is the bid side's then the ask side's.
#[test]
fn alive_on_reads_one_side_best_first() {
    let book = book_of(
        &[
            ("B-1", Some("100"), 1),
            ("B-M", None, 3),
            ("B-2", Some("101"), 2),
        ],
        &[("A-1", Some("102"), 1)],
    );
    let bids = book.alive_on(Side::Buy);
    assert_eq!(bids.len(), 3);
    assert_eq!(codes(bids), ["10:1:B-2", "10:1:B-1", "10:1:B-M"]);
    assert_eq!(codes(book.alive_on(Side::Sell)), ["10:2:A-1"]);
    assert_eq!(book.alive_on(Side::Unknown).len(), 0);
    assert_eq!(
        codes(book.alive()),
        codes(book.alive_on(Side::Buy).chain(book.alive_on(Side::Sell)))
    );
}

/// A walk prunes an execution or a trade where it pulls it: an instant only
/// an execution reached emits the book of its instrument stating the
/// execution among its deltas and moving nothing, and a trade is pruned,
/// so an instrument only a trade names opens no book.
#[test]
fn a_book_walk_records_an_execution_in_its_book_and_prunes_a_trade() {
    let mut fill = operation("execution", "IBM", "E-1", 2, "Buy", "100", 1, "Filled");
    op_mut(&mut fill).set_execunix(Some(2), true);
    fill.finalize();
    let mut root = ExecutionEvent::at(3);
    root.set_crosscode("T-1".to_owned());
    root.set_ticker(Some(SmolStr::new("MSFT")), true);
    root.set_state(State::read("Filled").unwrap());
    root.finalize();
    let traded = operation("execution", "MSFT", "E-2", 3, "Sell", "200", 1, "Filled");
    let trade =
        TradeEvent::from_parts(&root, vec![ExecutionEvent::try_from(traded).unwrap()]).unwrap();
    let inputs = vec![
        operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New"),
        fill,
        MarketData::from(trade),
        operation("order", "IBM", "O-2", 4, "Sell", "101", 1, "New"),
    ];
    let books = BookIterator::new(inputs.into_iter(), 0)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        books
            .iter()
            .map(|book| (book.get_currunix(), book.get_ticker().unwrap()))
            .collect::<Vec<_>>(),
        [(1, "IBM"), (2, "IBM"), (4, "IBM")]
    );
    // The execution's book states it alone, as a delta book, dated by it;
    // rebuilt over the book before it, it holds the order as before.
    assert_eq!(codes(books[1].deltas()), ["8:1:E-1"]);
    assert_eq!(books[1].get_execunix(), Some(2));
    assert!(!books[1].is_complete());
    let whole = whole(&books);
    assert_eq!(codes(whole[1].alive()), ["10:1:O-1"]);
    assert_eq!(codes(whole[2].alive()), ["10:1:O-1", "10:2:O-2"]);
    assert!(
        books
            .iter()
            .flat_map(|book| book.alive())
            .all(|entry| entry.kind() == MarketKind::OrderEvent)
    );
    assert_eq!(whole[2].get_execunix(), Some(2));
}

/// A filter narrows what a walk folds after the kind rule pruned it, so it
/// never admits a trade, and it keeps or drops an execution as it keeps or
/// drops an order; a filter keeping every row is no filter, and one naming
/// a column the row does not carry, or answering no boolean, is refused
/// where it is bound.
#[test]
fn a_filter_narrows_the_walk_and_never_widens_it() {
    let inputs = || {
        vec![
            operation("order", "IBM", "B-1", 1, "Buy", "100", 2, "New"),
            operation("order", "IBM", "A-1", 2, "Sell", "102", 1, "New"),
            operation("execution", "IBM", "E-1", 3, "Buy", "100", 1, "Filled"),
            operation("quote", "IBM", "B-2", 4, "Buy", "101", 1, "New"),
        ]
    };
    let walk = |filter: &str| {
        BookIterator::new(inputs().into_iter(), 0)
            .unwrap()
            .with_filter(filter)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let instants = |books: &[BookEvent]| books.iter().map(Event::get_currunix).collect::<Vec<_>>();

    let buys = walk("side = 'BUYS'");
    assert_eq!(instants(&buys), [1, 3, 4]);
    assert_eq!(codes(buys[1].deltas()), ["8:1:E-1"]);
    assert_eq!(codes(whole(&buys)[2].alive()), ["14:0:B-2", "10:1:B-1"]);
    assert_eq!(instants(&walk("marketdatakind = 'EXEC'")), [3]);
    assert_eq!(instants(&walk("marketdatakind = 'QUOT'")), [4]);
    assert_eq!(instants(&walk("true")), [1, 2, 3, 4]);

    for refused in ["nope = 1", "price + 1"] {
        assert!(
            BookIterator::new(inputs().into_iter(), 0)
                .unwrap()
                .with_filter(refused)
                .is_err(),
            "{refused}"
        );
    }
}

/// A book following the one before it replays its deltas over that book
/// in the order applied: an entry that moved sides leaves the side it
/// stood on, rather than coming back on both.
#[test]
fn following_replays_a_side_change_in_the_order_applied() {
    let mut previous = BookEvent::new(1, "IBM");
    previous
        .add_operations([operation("order", "IBM", "O-1", 1, "Buy", "100", 1, "New")])
        .unwrap();

    let mut update = BookEvent::new(2, "IBM");
    update
        .add_operations([
            operation("order", "IBM", "B-2", 2, "Buy", "99", 1, "New"),
            operation("order", "IBM", "O-1", 2, "Sell", "101", 1, "Replaced"),
        ])
        .unwrap();
    let continued = update.with_previous(&previous).unwrap();
    assert_eq!(codes(continued.alive_on(Side::Buy)), ["10:1:B-2"]);
    assert_eq!(codes(continued.alive_on(Side::Sell)), ["10:2:O-1"]);
    assert_eq!(codes(continued.deltas()), ["10:1:B-2", "10:2:O-1"]);
}

/// A quote tagging no side rests on every side it states a leg for, as one
/// entry: the one element on both sides, listed once by `alive`, each side
/// reading its own leg - and a row reads back the same way.
#[test]
fn a_two_sided_quote_rests_on_both_sides_as_one_entry() {
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([two_sided(
        "Q-1",
        1,
        Some(("99", 2)),
        Some(("101", 3)),
        "New",
    )])
    .unwrap();
    assert_eq!(codes(book.alive_on(Side::Buy)), ["14:0:Q-1"]);
    assert_eq!(codes(book.alive_on(Side::Sell)), ["14:0:Q-1"]);
    let (bid, ask) = (
        book.alive_on(Side::Buy).next().unwrap(),
        book.alive_on(Side::Sell).next().unwrap(),
    );
    assert!(std::ptr::eq(bid, ask), "one entry on both sides");
    assert_eq!(codes(book.alive()), ["14:0:Q-1"]);
    assert_eq!(codes(book.deltas()), ["14:0:Q-1"]);
    assert_eq!(
        book.limits(Side::Buy).collect::<Vec<_>>(),
        [limit(&book, Some("99"), 2, &["Q-1"])]
    );
    assert_eq!(
        book.limits(Side::Sell).collect::<Vec<_>>(),
        [limit(&book, Some("101"), 3, &["Q-1"])]
    );
    assert_eq!(
        (book.best_price(Side::Buy), book.best_price(Side::Sell)),
        (Some(decimal("99")), Some(decimal("101")))
    );
    assert_eq!(
        (book.get_bidpx(), book.get_askpx()),
        (Some(decimal("99")), Some(decimal("101")))
    );
    assert_eq!(book.get_currency().as_str(), "USD");
    assert_eq!(book.spread(), Some(decimal("2")));

    // The row states the entry once and reads it back on both sides.
    let written =
        MarketData::arrow_reader(vec![MarketData::from(book.clone())], None, None).unwrap();
    let read = MarketData::from_arrow_reader(written)
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    let MarketData::BookEvent(read) = &read[0] else {
        panic!("a book row reads back as a book");
    };
    assert_eq!(**read, book);
    assert_eq!(codes(read.alive_on(Side::Buy)), ["14:0:Q-1"]);
    assert_eq!(codes(read.alive_on(Side::Sell)), ["14:0:Q-1"]);
}

/// A quote follower stating one leg with a zero quantity withdraws that
/// leg: the quote leaves that side, rests on the other with the leg it
/// carried, and is one delta.
#[test]
fn a_quote_withdrawing_a_leg_leaves_that_side_alone() {
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([two_sided(
        "Q-1",
        1,
        Some(("99", 2)),
        Some(("101", 3)),
        "New",
    )])
    .unwrap();
    book.add_operations([two_sided("Q-1", 2, Some(("99", 0)), None, "Replaced")])
        .unwrap();
    assert_eq!(book.alive_on(Side::Buy).len(), 0);
    assert_eq!(codes(book.alive_on(Side::Sell)), ["14:0:Q-1"]);
    let live = book.alive_on(Side::Sell).next().unwrap();
    assert_eq!(legs(live), (leg("99", 0), leg("101", 3)));
    assert_eq!(codes(book.deltas()), ["14:0:Q-1"]);
    assert_eq!(
        (book.get_bidpx(), book.get_askpx()),
        (None, Some(decimal("101")))
    );
}

/// A quote follower stating one leg moves that leg and keeps the other,
/// carried from its chain: still one entry on both sides.
#[test]
fn a_quote_follower_stating_one_leg_keeps_the_other() {
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([two_sided(
        "Q-1",
        1,
        Some(("99", 2)),
        Some(("101", 3)),
        "New",
    )])
    .unwrap();
    let first = live_uuid(&book, "Q-1");
    book.add_operations([two_sided("Q-1", 2, Some(("100", 5)), None, "Replaced")])
        .unwrap();
    let (bid, ask) = (
        book.alive_on(Side::Buy).next().unwrap(),
        book.alive_on(Side::Sell).next().unwrap(),
    );
    assert!(std::ptr::eq(bid, ask));
    assert_eq!(legs(bid), (leg("100", 5), leg("101", 3)));
    assert_eq!(op(bid).get_prevuuid(), Some(first));
    assert_eq!(book.alive().count(), 1);
    assert_eq!(
        (book.best_price(Side::Buy), book.best_quantity(Side::Buy)),
        (Some(decimal("100")), Some(Decimal::from_int(5)))
    );
    assert_eq!(
        (book.best_price(Side::Sell), book.best_quantity(Side::Sell)),
        (Some(decimal("101")), Some(Decimal::from_int(3)))
    );
}

/// A cancel naming a live two-sided quote and quoting nothing takes it off
/// both sides: one delta, carrying the legs it takes out.
#[test]
fn a_quote_cancel_naming_a_live_quote_takes_it_off_both_sides() {
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([
        two_sided("Q-1", 1, Some(("99", 2)), Some(("101", 3)), "New"),
        operation("order", "IBM", "O-1", 1, "Buy", "98", 1, "New"),
    ])
    .unwrap();
    book.add_operations([two_sided("Q-1", 2, None, None, "Canceled")])
        .unwrap();
    assert_eq!(codes(book.alive_on(Side::Buy)), ["10:1:O-1"]);
    assert_eq!(book.alive_on(Side::Sell).len(), 0);
    assert_eq!(codes(book.deltas()), ["14:0:Q-1"]);
    let canceled = book.deltas().next().unwrap();
    assert!(!op(canceled).get_state().is_live());
    assert_eq!(legs(canceled), (leg("99", 2), leg("101", 3)));
    assert_eq!(
        (book.get_bidpx(), book.get_askpx()),
        (Some(decimal("98")), None)
    );
}

/// Every order and quote a book folds is a delta, resting anywhere or not:
/// one stating no leg - no side, no bid, no ask - rests on neither side and
/// places nothing, warned of where it is live, and one first seen ended
/// places nothing either, yet each is its book's delta, so the instant it
/// stands at emits a book holding it - while an expiration due then still
/// fires. A cancel stating a zero quantity of a new at its own instant takes
/// it off again, and both are deltas.
#[test]
fn an_entry_resting_nowhere_is_still_a_delta() {
    let nowhere =
        |code: &str, unix: i64| operation("quote", "IBM", code, unix, "Unknown", "101", 1, "New");
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New")])
        .unwrap();
    book.add_operations([nowhere("NOWHERE", 2)]).unwrap();
    assert_eq!(book.get_currunix(), 2);
    assert_eq!(codes(book.deltas()), ["14:0:NOWHERE"]);
    assert_eq!(book.alive().count(), 1);
    book.add_operations([operation("order", "IBM", "O-2", 2, "Sell", "101", 1, "New")])
        .unwrap();
    assert_eq!(codes(book.deltas()), ["14:0:NOWHERE", "10:2:O-2"]);
    assert_eq!(book.alive().count(), 2);

    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([
        operation("quote", "IBM", "Q-1", 1, "Buy", "99", 1, "New"),
        operation("quote", "IBM", "Q-1", 1, "Buy", "99", 0, "Canceled"),
    ])
    .unwrap();
    assert_eq!(book.alive().count(), 0);
    assert_eq!(codes(book.deltas()), ["14:0:Q-1", "14:0:Q-1"]);

    let mut live = operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New");
    op_mut(&mut live).set_exprunix(Some(3));
    live.finalize();
    let books = books_of(vec![
        live,
        nowhere("NOWHERE", 2),
        nowhere("LATE", 3),
        operation("quote", "MSFT", "LATER", 4, "Buy", "200", 1, "New"),
        operation("order", "IBM", "GONE", 5, "Buy", "100", 0, "Canceled"),
    ]);
    assert_eq!(
        books
            .iter()
            .map(|book| (book.get_currunix(), book.get_ticker().unwrap()))
            .collect::<Vec<_>>(),
        [(1, "IBM"), (2, "IBM"), (3, "IBM"), (4, "MSFT"), (5, "IBM")]
    );
    let whole = whole(&books);
    assert_eq!(codes(books[1].deltas()), ["14:0:NOWHERE"]);
    assert_eq!(whole[1].alive().count(), 1);
    assert_eq!(codes(books[2].deltas()), ["10:1:O-1", "14:0:LATE"]);
    assert_eq!(whole[2].alive().count(), 0);
    assert_eq!(codes(whole[3].alive()), ["14:0:LATER"]);
    // An order first seen ended is the one delta of its instant.
    assert_eq!(codes(books[4].deltas()), ["10:1:GONE"]);
    assert_eq!(whole[4].alive().count(), 0);
}

/// A two-sided quote's deadline is scheduled once: it expires off both
/// sides at once, as one delta carrying both legs.
#[test]
fn a_two_sided_quote_expires_once_off_both_sides() {
    let mut quote = two_sided("Q-1", 1, Some(("99", 2)), Some(("101", 3)), "New");
    op_mut(&mut quote).set_exprunix(Some(3));
    quote.finalize();
    let books = books_of(vec![quote]);
    assert_eq!(
        books.iter().map(Event::get_currunix).collect::<Vec<_>>(),
        [1, 3]
    );
    let first = &whole(&books)[0];
    assert_eq!(
        (
            first.alive_on(Side::Buy).len(),
            first.alive_on(Side::Sell).len()
        ),
        (1, 1)
    );
    assert_eq!(books[1].alive().count(), 0);
    assert_eq!(codes(books[1].deltas()), ["14:0:Q-1"]);
    let expired = books[1].deltas().next().unwrap();
    assert_eq!(*op(expired).get_state(), State::Expired);
    assert_eq!(legs(expired), (leg("99", 2), leg("101", 3)));
}

/// A statement repeating the live entry it continues - every fact the same
/// but where it stands in its chain - is no change: the book records no
/// delta for it, and a group of nothing else neither advances the book nor
/// emits one; a statement changing a fact is the next book's delta.
#[test]
fn a_restatement_records_no_delta_and_emits_no_book() {
    let books = books_of(vec![
        operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New"),
        operation("order", "IBM", "O-1", 2, "Buy", "100", 2, "New"),
        operation("order", "IBM", "O-1", 3, "Buy", "100", 3, "New"),
    ]);
    assert_eq!(
        books.iter().map(Event::get_currunix).collect::<Vec<_>>(),
        [1, 3]
    );
    assert_eq!(codes(books[1].deltas()), ["10:1:O-1"]);
    assert_eq!(
        books[1].deltas().next().unwrap().get_quantity(),
        Some(Decimal::from_int(3))
    );
    // The change follows the entry the repeat left standing.
    assert_eq!(
        op(books[1].deltas().next().unwrap()).get_prevuuid(),
        Some(live_uuid(&whole(&books)[0], "O-1"))
    );

    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New")])
        .unwrap();
    let before = book.clone();
    book.add_operations([operation("order", "IBM", "O-1", 2, "Buy", "100", 2, "New")])
        .unwrap();
    assert_eq!(
        book, before,
        "the instant does not advance and the deltas stand"
    );
}

/// Deduplication is idempotent: a statement applied twice - in one group or
/// in two - leaves the book it leaves applied once, its two legs and all.
#[test]
fn applying_a_statement_twice_changes_the_book_once() {
    let quote = two_sided("Q-1", 1, Some(("99", 2)), Some(("101", 3)), "New");
    let mut once = BookEvent::new(1, "IBM");
    once.add_operations([quote.clone()]).unwrap();
    let mut twice = BookEvent::new(1, "IBM");
    twice
        .add_operations([quote.clone(), quote.clone()])
        .unwrap();
    assert_eq!(twice, once);
    twice.add_operations([quote]).unwrap();
    assert_eq!(twice, once);
    assert_eq!(twice.deltas().len(), 1);
    assert_eq!(
        (
            twice.alive_on(Side::Buy).len(),
            twice.alive_on(Side::Sell).len()
        ),
        (1, 1)
    );
}

/// A full refresh restating an entry its scope held exactly as it was keeps
/// that entry, identity and all, and records no delta for it: only what
/// changed is the refresh's delta.
#[test]
fn an_unchanged_level_keeps_its_identity_across_full_refreshes() {
    let refresh = |unix: i64, code: &str, price: &str, quantity: i64| {
        with_book(
            operation("quote", "IBM", code, unix, "Buy", price, quantity, "New"),
            acting_in(MdUpdateAction::Snapshot, "PRIMARY"),
        )
    };
    let books = books_of(vec![
        refresh(1, "B-1", "100", 2),
        refresh(1, "B-2", "99", 1),
        refresh(2, "B-1", "100", 2),
        refresh(2, "B-2", "99", 5),
    ]);
    assert_eq!(books.len(), 2);
    assert!(books.iter().all(BookEvent::is_complete));
    assert_eq!(books[1].get_snapunix(), Some(2));
    assert_eq!(live_uuid(&books[1], "B-1"), live_uuid(&books[0], "B-1"));
    assert_ne!(live_uuid(&books[1], "B-2"), live_uuid(&books[0], "B-2"));
    assert_eq!(codes(books[1].deltas()), ["14:0:B-2"]);
    assert_eq!(
        books[1].limits(Side::Buy).collect::<Vec<_>>(),
        [
            limit(&books[1], Some("100"), 2, &["B-1"]),
            limit(&books[1], Some("99"), 5, &["B-2"]),
        ]
    );
}

/// Every book a walk emits names the book of its code before it - its
/// identity, its instant, its price and quantity - whether it is whole at a
/// grid tick or states its deltas alone, so a reader holding one can tell
/// whether it holds the book the next one follows.
#[test]
fn the_books_of_one_key_chain_by_prevuuid() {
    let books = BookIterator::new(
        vec![
            operation("order", "IBM", "O-1", 1_000_000, "Buy", "100", 2, "New"),
            operation("order", "MSFT", "M-1", 1_000_000, "Buy", "200", 1, "New"),
            operation("order", "IBM", "O-2", 1_500_000, "Buy", "101", 3, "New"),
            operation(
                "order", "IBM", "O-1", 3_500_000, "Buy", "100", 2, "Canceled",
            ),
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
                book.get_ticker().unwrap(),
                book.get_currunix(),
                book.is_complete()
            ))
            .collect::<Vec<_>>(),
        [
            ("IBM", 1_000_000, true),
            ("MSFT", 1_000_000, true),
            ("IBM", 1_500_000, false),
            ("IBM", 2_000_000, true),
            ("MSFT", 2_000_000, true),
            ("IBM", 3_000_000, true),
            ("MSFT", 3_000_000, true),
            ("IBM", 3_500_000, false),
        ]
    );
    let mut last: BTreeMap<&str, &BookEvent> = BTreeMap::new();
    for book in &books {
        match last.get(book.get_crosscode()) {
            Some(previous) => {
                assert_eq!(book.get_prevuuid(), Some(previous.get_curruuid()));
                assert_eq!(book.get_prevunix(), Some(previous.get_currunix()));
                assert_eq!(book.get_prevpx(), previous.get_price());
                assert_eq!(book.get_prevqty(), previous.get_quantity());
            }
            None => assert_eq!(book.get_prevuuid(), None),
        }
        last.insert(book.get_crosscode(), book);
    }
}

/// A book stating its deltas alone is rebuilt over the book it names as its
/// `prevuuid` and over nothing else: not over an earlier book - a gap in
/// the chain - a later one, another code's, or one holding only its deltas
/// too; and a complete book already linked to the one given moves nothing.
#[test]
fn a_delta_book_follows_only_the_book_it_names() {
    let books = books_of(vec![
        operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New"),
        operation("order", "IBM", "O-2", 2, "Buy", "101", 3, "New"),
        operation("order", "IBM", "O-3", 3, "Buy", "102", 1, "New"),
    ]);
    let whole = whole(&books);
    let rebuilt = books[2].clone().with_previous(&whole[1]).unwrap();
    assert_eq!(rebuilt, whole[2]);
    assert_eq!(rebuilt.get_curruuid(), books[2].get_curruuid());
    assert!(
        books[2].clone().with_previous(&whole[0]).is_none(),
        "a gap in the chain"
    );
    assert!(books[2].clone().with_previous(&books[1]).is_none());
    assert!(books[1].clone().with_previous(&whole[2]).is_none());
    assert!(
        books[1]
            .clone()
            .with_previous(&BookEvent::new(1, "MSFT"))
            .is_none()
    );
    assert!(whole[2].clone().with_previous(&whole[1]).is_none());
}

/// A book stating its deltas alone answers its top of book from the facts
/// it settled on - the best bid and ask, their quantities, the spread, the
/// midpoint and the median quantity, what a candle reads - and no depth,
/// which only its sides hold; rebuilt, it answers all of them alike.
#[test]
fn a_delta_book_reads_its_best_prices_from_its_own_facts() {
    let books = books_of(vec![
        operation("order", "IBM", "B-1", 1, "Buy", "100", 2, "New"),
        operation("order", "IBM", "A-1", 1, "Sell", "102", 4, "New"),
        operation("order", "IBM", "B-2", 2, "Buy", "101", 6, "New"),
    ]);
    let delta = &books[1];
    assert!(!delta.is_complete());
    assert_eq!(delta.best_price(Side::Buy), Some(decimal("101")));
    assert_eq!(delta.best_quantity(Side::Buy), Some(Decimal::from_int(6)));
    assert_eq!(delta.best_price(Side::Sell), Some(decimal("102")));
    assert_eq!(delta.best_quantity(Side::Sell), Some(Decimal::from_int(4)));
    assert_eq!(delta.spread(), Some(decimal("1")));
    assert_eq!(delta.bbo_midpoint(), Some(decimal("101.5")));
    assert_eq!(delta.median_quantity(), Some(Decimal::from_int(5)));
    assert!(!delta.is_crossed() && !delta.is_locked());
    assert_eq!(delta.depth(Side::Buy, 1), None);
    assert_eq!(delta.imbalance(1), None);

    let rebuilt = &whole(&books)[1];
    for side in [Side::Buy, Side::Sell] {
        assert_eq!(rebuilt.best_price(side), delta.best_price(side));
        assert_eq!(rebuilt.best_quantity(side), delta.best_quantity(side));
    }
    assert_eq!(rebuilt.spread(), delta.spread());
    assert_eq!(rebuilt.bbo_midpoint(), delta.bbo_midpoint());
    assert_eq!(rebuilt.median_quantity(), delta.median_quantity());
    assert_eq!(rebuilt.imbalance(1), Some(decimal("0.2")));
}

/// A book stating its deltas alone holds no side to fold an operation
/// into: it is refused, the book unchanged, until it is rebuilt.
#[test]
fn a_delta_book_takes_no_operations() {
    let books = books_of(vec![
        operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New"),
        operation("order", "IBM", "O-2", 2, "Buy", "101", 3, "New"),
    ]);
    let mut delta = books[1].clone();
    let error = delta
        .add_operations([operation("order", "IBM", "O-3", 3, "Buy", "102", 1, "New")])
        .unwrap_err();
    assert!(
        matches!(&error, yggdryl::Error::InvalidRecord { path, reason }
            if path == "$.alive" && reason.contains("rebuild it with with_previous first")),
        "{error}"
    );
    assert_eq!(delta, books[1]);
    let mut rebuilt = whole(&books).pop().unwrap();
    rebuilt
        .add_operations([operation("order", "IBM", "O-3", 3, "Buy", "102", 1, "New")])
        .unwrap();
    assert_eq!(rebuilt.alive().count(), 3);
}

/// A deterministic churn of `count` inputs on IBM, four to an instant half
/// a millisecond apart, over six entries - the even ones orders, the odd
/// ones quotes - each newly stated, restated as it was, moved or canceled:
/// orders on either side, quotes on one leg, both or a leg sized zero.
fn churn(seed: u64, count: usize) -> Vec<MarketData> {
    let mut state = seed;
    let mut next = move |bound: u64| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) % bound
    };
    let mut last: Vec<Option<MarketData>> = vec![None; 6];
    let mut inputs = Vec::with_capacity(count);
    let mut base = 0;
    for at in 0..count {
        let unix = i64::try_from(at / 4 + 1).unwrap() * 500_000;
        // Four distinct entries an instant: no entry twice at one.
        if at % 4 == 0 {
            base = usize::try_from(next(6)).unwrap();
        }
        let slot = (base + at % 4) % 6;
        let code = format!("E-{slot}");
        let input = match (next(10), &last[slot]) {
            (0..=2, Some(held)) => {
                let mut held = held.clone();
                op_mut(&mut held).set_currunix(unix);
                held.finalize();
                held
            }
            (3, Some(held)) => edited(held.clone(), |operation| {
                operation.set_currunix(unix);
                operation.set_state(State::Canceled);
            }),
            _ => {
                let price = |offset: u64| (98 + offset).to_string();
                let quantity = i64::try_from(next(4)).unwrap() + 1;
                if slot % 2 == 0 {
                    let side = if next(2) == 0 { "Buy" } else { "Sell" };
                    let price = if side == "Buy" {
                        price(next(2))
                    } else {
                        price(2 + next(2))
                    };
                    operation("order", "IBM", &code, unix, side, &price, quantity, "New")
                } else {
                    let bid = price(next(2));
                    let ask = price(2 + next(2));
                    let (bid, ask) = match next(4) {
                        0 => (Some((bid.as_str(), quantity)), None),
                        1 => (None, Some((ask.as_str(), quantity))),
                        2 => (Some((bid.as_str(), 0)), Some((ask.as_str(), quantity))),
                        _ => (
                            Some((bid.as_str(), quantity)),
                            Some((ask.as_str(), quantity + 1)),
                        ),
                    };
                    two_sided(&code, unix, bid, ask, "New")
                }
            }
        };
        last[slot] = Some(input.clone());
        inputs.push(input);
    }
    inputs
}

/// Book state is the last complete book with every delta after it replayed
/// in the order applied: over a churn of orders and two-sided quotes -
/// restated, repeated, moved, withdrawn by a leg sized zero, canceled -
/// every book a walk emits, rebuilt over the one before it, holds what one
/// book folding each instant's inputs in turn holds at that instant, and
/// keeps the identity the walk gave it, with and without a grid.
#[test]
fn folding_with_previous_over_the_emitted_books_rebuilds_every_book_the_walk_held() {
    for seed in [1, 7, 42] {
        let inputs = churn(seed, 96);
        // The oracle: one book, each instant's group added in turn.
        let mut oracle = BookEvent::new(500_000, "IBM");
        let mut held: Vec<(i64, BookEvent)> = Vec::new();
        let mut group: Vec<MarketData> = Vec::new();
        for input in inputs
            .iter()
            .cloned()
            .chain([MarketData::from(OrderEvent::at(i64::MAX))])
        {
            let unix = op(&input).get_currunix();
            if group
                .first()
                .is_some_and(|first| op(first).get_currunix() != unix)
            {
                let at = op(&group[0]).get_currunix();
                oracle.add_operations(std::mem::take(&mut group)).unwrap();
                held.push((at, oracle.clone()));
            }
            group.push(input);
        }
        let oracle_at = |unix: i64| {
            &held
                .iter()
                .rev()
                .find(|(at, _)| *at <= unix)
                .expect("a group at or before every book")
                .1
        };
        for grid in [0, 1] {
            let books = BookIterator::new(inputs.clone().into_iter(), grid)
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(
                books.iter().any(|book| !book.is_complete()),
                "{seed}: deltas"
            );
            for (book, rebuilt) in books.iter().zip(whole(&books)) {
                let expected = oracle_at(book.get_currunix());
                let context = format!("seed {seed}, grid {grid}, book at {}", book.get_currunix());
                assert_eq!(rebuilt.get_curruuid(), book.get_curruuid(), "{context}");
                assert_eq!(
                    rebuilt
                        .alive()
                        .map(Element::get_curruuid)
                        .collect::<Vec<_>>(),
                    expected
                        .alive()
                        .map(Element::get_curruuid)
                        .collect::<Vec<_>>(),
                    "{context}"
                );
                for side in [Side::Buy, Side::Sell] {
                    assert_eq!(
                        rebuilt.limits(side).collect::<Vec<_>>(),
                        expected.limits(side).collect::<Vec<_>>(),
                        "{context}"
                    );
                    assert_eq!(
                        book.best_price(side),
                        expected.best_price(side),
                        "{context}"
                    );
                    assert_eq!(
                        book.best_quantity(side),
                        expected.best_quantity(side),
                        "{context}"
                    );
                }
                assert_eq!(book.get_price(), expected.get_price(), "{context}");
                assert_eq!(book.get_quantity(), expected.get_quantity(), "{context}");
            }
            // Written as rows and read back, every book rebuilds as it did:
            // a two-sided quote's place on the ask side included, which the
            // row's price levels state.
            let read_back: Vec<BookEvent> = MarketData::from_arrow_reader(
                MarketData::arrow_reader(
                    books
                        .iter()
                        .cloned()
                        .map(MarketData::from)
                        .collect::<Vec<_>>(),
                    None,
                    None,
                )
                .unwrap(),
            )
            .unwrap()
            .map(|value| value.unwrap().as_book_event().unwrap().clone())
            .collect();
            assert_eq!(read_back.len(), books.len(), "{seed}");
            for (read, walked) in whole(&read_back).iter().zip(&whole(&books)) {
                let context = format!("seed {seed}, grid {grid}, read at {}", read.get_currunix());
                assert_eq!(read.get_curruuid(), walked.get_curruuid(), "{context}");
                for side in [Side::Buy, Side::Sell] {
                    assert_eq!(
                        read.limits(side).collect::<Vec<_>>(),
                        walked.limits(side).collect::<Vec<_>>(),
                        "{context}"
                    );
                }
            }
        }
    }
}

/// A deadline a later statement outdated moves the walk nowhere: an order
/// expiring far ahead and canceled before then leaves no grid tick past the
/// last input, however far its deadline was.
#[test]
fn an_outdated_deadline_emits_no_tick_past_the_last_input() {
    const MS: i64 = 1_000_000;
    let mut order = operation("order", "IBM", "O-1", MS, "Buy", "100", 1, "New");
    op_mut(&mut order).set_exprunix(Some(100 * MS));
    order.finalize();
    let canceled = edited(order.clone(), |operation| {
        operation.set_currunix(2 * MS);
        operation.set_state(State::Canceled);
    });
    let books = BookIterator::new([order, canceled].into_iter(), 1)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        books.iter().map(Event::get_currunix).collect::<Vec<_>>(),
        [MS, 2 * MS]
    );
    assert_eq!(books[1].alive().count(), 0);
    assert_eq!(codes(books[1].deltas()), ["10:1:O-1"]);
}

/// A group refused after restating an entry where it stood - its quantity
/// changed at the same price, which moves nothing - leaves that entry as it
/// was: the group is undone whole.
#[test]
fn a_refused_group_undoes_an_entry_it_restated_in_place() {
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations([
        with_identifiers(
            operation("order", "IBM", "O-1", 1, "Buy", "100", 2, "New"),
            &[(ORDER_ID, "ORDER-1")],
        ),
        with_identifiers(
            operation("order", "IBM", "O-2", 1, "Buy", "99", 2, "New"),
            &[(ORDER_ID, "ORDER-2")],
        ),
    ])
    .unwrap();
    let before = book.clone();
    let error = book
        .add_operations([
            with_identifiers(
                operation("order", "IBM", "O-1", 2, "Buy", "100", 5, "Replaced"),
                &[(ORDER_ID, "ORDER-1")],
            ),
            with_identifiers(
                operation("order", "IBM", "O-2", 2, "Buy", "99", 3, "Replaced"),
                &[(ORDER_ID, "ORDER-X")],
            ),
        ])
        .unwrap_err();
    assert!(
        matches!(&error, yggdryl::Error::InvalidRecord { path, .. } if path == "$.operation.identifiers.orderid"),
        "{error}"
    );
    assert_eq!(book, before);
    assert_eq!(
        book.limits(Side::Buy).next().unwrap().quantity,
        Decimal::from_int(2)
    );
    // Applied alone, the restatement stands where the entry stood.
    book.add_operations([with_identifiers(
        operation("order", "IBM", "O-1", 2, "Buy", "100", 5, "Replaced"),
        &[(ORDER_ID, "ORDER-1")],
    )])
    .unwrap();
    assert_eq!(codes(alive(&book, true)), ["10:1:O-1", "10:1:O-2"]);
    assert_eq!(
        book.limits(Side::Buy).next().unwrap().quantity,
        Decimal::from_int(5)
    );
}

/// The leg `entry` rests on `side` with: a quote's bid or ask, an order's
/// price and quantity.
fn leg_on(entry: &MarketData, side: Side) -> (Option<Decimal>, Option<Decimal>) {
    match entry {
        MarketData::QuoteEvent(_) if side.is_bid() => (entry.get_bidpx(), entry.get_bidqty()),
        MarketData::QuoteEvent(_) => (entry.get_askpx(), entry.get_askqty()),
        _ => (entry.get_price(), entry.get_quantity()),
    }
}

/// `side` of `book` summed from the entries resting there, level by level
/// in book order: what `BookEvent::limits` answers, found without it.
fn summed_levels(book: &BookEvent, side: Side) -> Vec<(Option<Decimal>, Decimal, bool)> {
    let mut levels: Vec<(Option<Decimal>, Decimal, bool)> = Vec::new();
    for entry in book.alive_on(side) {
        let (price, quantity) = leg_on(entry, side);
        let quantity = quantity.unwrap_or(Decimal::ZERO);
        let trades = op(entry).get_tradable() != Some(false);
        match levels.last_mut() {
            Some((at, sum, any)) if *at == price => {
                *sum = sum.checked_add(quantity).unwrap();
                *any |= trades;
            }
            _ => levels.push((price, quantity, trades)),
        }
    }
    levels
}

/// A level's quantity and whether it trades, and the best bid and ask they
/// settle, stay what the entries resting there sum to through every change a
/// deep level takes - an entry moved to the back with a new size, a better
/// level that cannot trade, an entry halting and resizing where it stands,
/// entries leaving, a two-sided quote on both deep levels and its bid
/// withdrawn, a refused group rolled back, a snapshot replacing its scope -
/// on the side a change touches and on the one it leaves alone.
#[test]
fn a_deep_level_states_what_its_entries_sum_to_through_every_change() {
    const DEPTH: usize = 64;
    let check = |book: &BookEvent, step: &str| {
        for side in [Side::Buy, Side::Sell] {
            let summed = summed_levels(book, side);
            let limits: Vec<(Option<Decimal>, Decimal, bool)> = book
                .limits(side)
                .map(|limit| (limit.price, limit.quantity, limit.tradable))
                .collect();
            assert_eq!(limits, summed, "{step}: {}", side.as_str());
            let best = summed
                .iter()
                .find(|(price, _, trades)| price.is_some() && *trades);
            assert_eq!(
                (book.best_price(side), book.best_quantity(side)),
                (
                    best.and_then(|(price, ..)| *price),
                    best.map(|(_, quantity, _)| *quantity)
                ),
                "{step}: {}",
                side.as_str()
            );
        }
    };
    let bid = |code: &str, unix: i64, price: &str, quantity: i64, state: &str| {
        operation("order", "IBM", code, unix, "Buy", price, quantity, state)
    };
    let halted = |entry: MarketData| edited(entry, |held| held.set_tradable(Some(false), true));
    let mut book = BookEvent::new(1, "IBM");
    book.add_operations((0..DEPTH).flat_map(|at| {
        [
            bid(&format!("B-{at}"), 1, "100", 1, "New"),
            operation(
                "order",
                "IBM",
                &format!("A-{at}"),
                1,
                "Sell",
                "101",
                2,
                "New",
            ),
        ]
    }))
    .unwrap();
    check(&book, "built");
    assert_eq!(book.best_quantity(Side::Buy), Some(Decimal::from_int(64)));
    assert_eq!(book.best_quantity(Side::Sell), Some(Decimal::from_int(128)));
    book.add_operations([bid("B-7", 2, "100", 5, "Replaced")])
        .unwrap();
    check(&book, "moved to the back, resized");
    book.add_operations([halted(bid("H-1", 3, "100.5", 3, "New"))])
        .unwrap();
    check(&book, "a better level that cannot trade");
    assert_eq!(book.best_price(Side::Buy), Some(decimal("100")));
    // B-7 is now the level's last entry: these restate it where it stands.
    book.add_operations([halted(bid("B-7", 4, "100", 5, "Replaced"))])
        .unwrap();
    check(&book, "halted where it stands");
    book.add_operations([bid("B-7", 5, "100", 6, "Replaced")])
        .unwrap();
    check(&book, "resized where it stands");
    book.add_operations([
        bid("B-3", 6, "100", 1, "Canceled"),
        bid("H-1", 6, "100.5", 3, "Canceled"),
    ])
    .unwrap();
    check(&book, "entries left");
    book.add_operations([two_sided(
        "Q-1",
        7,
        Some(("100", 7)),
        Some(("101", 9)),
        "New",
    )])
    .unwrap();
    check(&book, "a two-sided quote");
    book.add_operations([two_sided(
        "Q-1",
        8,
        Some(("100", 0)),
        Some(("101", 9)),
        "Replaced",
    )])
    .unwrap();
    check(&book, "its bid withdrawn");
    let before = book.clone();
    book.add_operations([
        bid("B-11", 9, "100", 4, "Replaced"),
        sized(bid("X-1", 9, "100", 1, "New"), Decimal::MAX),
    ])
    .unwrap_err();
    assert_eq!(book, before);
    check(&book, "a refused group");
    let scoped_bid = |code: &str, unix: i64, quantity: i64, control: BookRef| {
        with_book(bid(code, unix, "100", quantity, "New"), control)
    };
    book.add_operations([
        scoped_bid("P-1", 10, 11, scoped("PRIMARY")),
        scoped_bid("P-2", 10, 13, scoped("PRIMARY")),
    ])
    .unwrap();
    check(&book, "a scope joined");
    book.add_operations([scoped_bid(
        "P-3",
        11,
        17,
        acting_in(MdUpdateAction::Snapshot, "PRIMARY"),
    )])
    .unwrap();
    check(&book, "a snapshot of the scope");
}
