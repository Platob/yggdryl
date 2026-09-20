//! The book iterator over hand-built statements, no protocol in sight:
//! one book per symbol per instant touched, the live layer kept per
//! symbol, the prints counted, the expiries swept, the global book, the
//! grid, and what it refuses.

use std::hash::Hasher;
use std::num::NonZeroU32;

use yggdryl::graph::{Element, Event, MarketElement};
use yggdryl::market::{
    Book, BookData, BookIterator, ExecutionData, Level, Order, OrderData, QuoteData, Statement,
    Symbol, TradeData,
};
use yggdryl::{Currency, Decimal18, Isin, Mic, Side, State, Uuid};

fn decimal(text: &str) -> Decimal18 {
    text.parse().expect("a number")
}

fn level(px: &str, qty: i64, count: u64) -> Level {
    Level {
        px: decimal(px),
        qty: Decimal18::from_int(qty),
        count,
    }
}

fn depth(levels: u32) -> NonZeroU32 {
    NonZeroU32::new(levels).expect("a depth")
}

/// A live limit order, named by its ticker, priced in USD, sourced from
/// one message named after its code and instant.
fn order(unix: i64, code: &str, ticker: &str, side: &str, px: &str, qty: i64) -> OrderData {
    let mut order = OrderData::at(unix);
    order.set_crosscode(code.to_owned());
    if !ticker.is_empty() {
        order.set_symbolticker(Some(ticker.to_owned()));
    }
    order.set_side(Side::read(side).expect("a side"));
    order.set_px(decimal(px));
    order.set_qty(Decimal18::from_int(qty));
    order.set_state(State::read("New").expect("a state"));
    order.set_currency(Currency::new("USD").expect("a currency"));
    order.set_srcuuids(vec![source(code, unix)]);
    order.finalize();
    order
}

/// A two-sided quote where both lanes are given, one-sided where one is
/// zero.
fn quote(unix: i64, code: &str, ticker: &str, bid: (&str, i64), ask: (&str, i64)) -> QuoteData {
    let mut quote = QuoteData::at(unix);
    quote.set_crosscode(code.to_owned());
    quote.set_symbolticker(Some(ticker.to_owned()));
    quote.set_bidpx(Some(decimal(bid.0)));
    quote.set_bidqty(Some(Decimal18::from_int(bid.1)));
    quote.set_askpx(Some(decimal(ask.0)));
    quote.set_askqty(Some(Decimal18::from_int(ask.1)));
    quote.set_state(State::read("New").expect("a state"));
    quote.set_currency(Currency::new("USD").expect("a currency"));
    quote.set_srcuuids(vec![source(code, unix)]);
    quote.finalize();
    quote
}

/// A print against an order's chain.
fn print(unix: i64, code: &str, ticker: &str, px: &str, qty: i64, state: &str) -> ExecutionData {
    let mut fill = ExecutionData::at(unix);
    fill.set_crosscode(code.to_owned());
    fill.set_symbolticker(Some(ticker.to_owned()));
    fill.set_px(decimal(px));
    fill.set_qty(Decimal18::from_int(qty));
    fill.set_state(State::read(state).expect("a state"));
    fill.set_srcuuids(vec![source(&format!("{code}/{px}"), unix)]);
    fill.finalize();
    fill
}

/// One message identity per statement, named after what stated it.
fn source(name: &str, unix: i64) -> Uuid {
    let mut seed = yggdryl::xxhash::Xxh3::default();
    seed.write(name.as_bytes());
    seed.write(&unix.to_le_bytes());
    Uuid::from_v8(u128::from(seed.as_u64()))
}

fn books<I>(statements: I, levels: u32) -> Vec<BookData>
where
    I: IntoIterator<Item = Statement>,
{
    BookIterator::new(statements.into_iter().map(Ok), depth(levels), true)
        .collect::<yggdryl::Result<_>>()
        .expect("every statement applies")
}

#[test]
fn one_instant_is_one_book_naming_every_statement_of_it() {
    let books = books(
        [
            Statement::from(order(10, "A1", "AAPL", "Buy", "10.5", 100)),
            Statement::from(quote(10, "Q1", "AAPL", ("10.4", 30), ("10.6", 40))),
            Statement::from(print(10, "A1", "AAPL", "10.55", 20, "PartiallyFilled")),
        ],
        5,
    );
    assert_eq!(books.len(), 1);
    let book = &books[0];
    assert_eq!(book.get_crosscode(), "AAPL");
    assert_eq!(book.get_currunix(), 10);
    assert_eq!(
        book.get_bids(),
        [level("10.5", 100, 1), level("10.4", 30, 1)]
    );
    assert_eq!(book.get_asks(), [level("10.6", 40, 1)]);
    assert_eq!(
        book.get_updates(),
        3,
        "the print counts among the statements"
    );
    assert_eq!(book.get_lastpx(), Some(decimal("10.55")));
    assert_eq!(book.get_cumqty(), Some(Decimal18::from_int(20)));
    assert_eq!(
        book.get_srcuuids().len(),
        3,
        "every statement of the instant is a source"
    );
    assert_eq!(book.get_currency().as_str(), "USD");
    assert_eq!(book.get_px(), decimal("10.55"), "the mid");
    assert_eq!(book.get_seqnum(), 0);
    assert_eq!(book.get_snapunix(), None);
}

#[test]
fn two_symbols_in_one_instant_are_two_books_in_symbol_order() {
    let books = books(
        [
            Statement::from(order(10, "M1", "MSFT", "Buy", "400", 10)),
            Statement::from(order(10, "A1", "AAPL", "Buy", "10.5", 100)),
        ],
        5,
    );
    assert_eq!(
        books.iter().map(Element::get_crosscode).collect::<Vec<_>>(),
        ["AAPL", "MSFT"]
    );
    assert!(
        books.iter().all(|book| book.get_seqnum() == 0),
        "two chains"
    );
    assert_ne!(books[0].get_crossuuid(), books[1].get_crossuuid());
}

#[test]
fn the_global_book_keys_every_statement_under_one_symbol() {
    let mut euro = order(10, "E1", "SAP", "Sell", "5", 5);
    euro.set_currency(Currency::new("EUR").expect("a currency"));
    euro.set_miccode(Some(Mic::new("XETR").expect("a MIC")));
    euro.finalize();
    let mut apple = order(10, "A1", "AAPL", "Buy", "10.5", 100);
    apple.set_miccode(Some(Mic::new("XNAS").expect("a MIC")));
    apple.finalize();
    let books: Vec<BookData> = BookIterator::new(
        [Statement::from(apple), Statement::from(euro)].map(Ok),
        depth(5),
        true,
    )
    .with_symbol(Symbol::default())
    .collect::<yggdryl::Result<_>>()
    .expect("every statement applies");
    assert_eq!(books.len(), 1, "one book for the whole stream");
    let book = &books[0];
    assert_eq!(book.get_crosscode(), "GLOBAL");
    assert_eq!(book.get_bids(), [level("10.5", 100, 1)]);
    assert_eq!(book.get_asks(), [level("5", 5, 1)]);
    assert_eq!(
        book.get_currency(),
        &Currency::none(),
        "two currencies price it in none"
    );
    assert_eq!(book.get_miccode(), None, "two markets name no one MIC");
    assert_eq!(
        book.get_symbolticker(),
        None,
        "two tickers name no one ticker"
    );
    assert!(
        book.is_crossed(),
        "a bid above an ask, stated and not refused"
    );
}

#[test]
fn a_statement_naming_no_instrument_keys_the_global_symbol_beside_the_named() {
    let books = books(
        [
            Statement::from(order(10, "A1", "AAPL", "Buy", "10.5", 100)),
            Statement::from(order(10, "X1", "", "Buy", "1", 1)),
        ],
        5,
    );
    assert_eq!(
        books.iter().map(Element::get_crosscode).collect::<Vec<_>>(),
        ["AAPL", "GLOBAL"]
    );
}

#[test]
fn a_restated_order_moves_its_level_and_a_dead_or_expired_one_leaves() {
    let mut cancelled = order(30, "A1", "AAPL", "Buy", "10.6", 60);
    cancelled.set_state(State::read("Canceled").expect("a state"));
    cancelled.finalize();
    let mut filled = order(40, "A2", "AAPL", "Buy", "10.5", 50);
    filled.set_leavesqty(Some(Decimal18::ZERO));
    filled.set_state(State::read("Filled").expect("a state"));
    filled.finalize();
    let mut expiring = order(40, "A3", "AAPL", "Buy", "10.4", 70);
    expiring.set_expirunix(Some(45));
    expiring.finalize();
    let books = books(
        [
            Statement::from(order(10, "A1", "AAPL", "Buy", "10.5", 100)),
            Statement::from(order(10, "A2", "AAPL", "Buy", "10.5", 50)),
            // The first order restated: a new price and size.
            Statement::from(order(20, "A1", "AAPL", "Buy", "10.6", 60)),
            Statement::from(cancelled),
            Statement::from(filled),
            Statement::from(expiring),
            // Nothing of AAPL at 50: the expiry leaves on its own.
            Statement::from(order(50, "M1", "MSFT", "Buy", "400", 10)),
        ],
        5,
    );
    assert_eq!(books.len(), 6);
    assert_eq!(books[0].get_bids(), [level("10.5", 150, 2)]);
    assert_eq!(
        books[1].get_bids(),
        [level("10.6", 60, 1), level("10.5", 50, 1)],
        "moved"
    );
    assert_eq!(
        books[2].get_bids(),
        [level("10.5", 50, 1)],
        "cancelled, gone"
    );
    assert_eq!(
        books[3].get_bids(),
        [level("10.4", 70, 1)],
        "filled, gone; the expiring one rests"
    );
    // The expiry leaves at the first instant closed at or after it, with
    // no statement of its own, the book dated at that instant.
    let expired = &books[4];
    assert_eq!(expired.get_crosscode(), "AAPL");
    assert_eq!(expired.get_currunix(), 50);
    assert_eq!(expired.get_bids(), []);
    assert_eq!(
        expired.get_srcuuids(),
        [source("A3", 40)],
        "names what rested the order it retired"
    );
    assert_eq!(books[5].get_crosscode(), "MSFT");
}

#[test]
fn a_live_order_stating_nothing_of_the_ladder_keeps_its_lanes() {
    let mut request = OrderData::at(20);
    request.set_crosscode("A1".to_owned());
    request.set_symbolticker(Some("AAPL".to_owned()));
    request.set_side(Side::read("Buy").expect("a side"));
    request.set_state(State::read("PendingCancel").expect("a state"));
    request.set_srcuuids(vec![source("A1/cancel", 20)]);
    request.finalize();
    let mut market = order(30, "A1", "AAPL", "Buy", "0", 100);
    market.set_ordtype(Some("1".to_owned()));
    market.finalize();
    let books = books(
        [
            Statement::from(order(10, "A1", "AAPL", "Buy", "10.5", 100)),
            Statement::from(request),
            Statement::from(market),
        ],
        5,
    );
    assert_eq!(books.len(), 3);
    assert_eq!(
        books[1].get_bids(),
        [level("10.5", 100, 1)],
        "a cancel request moves nothing"
    );
    assert_eq!(books[1].get_updates(), 2, "but counts");
    assert_eq!(books[2].get_bids(), [], "a market order rests nothing");
}

#[test]
fn a_quote_rests_two_lanes_a_restatement_replaces_both_and_a_zero_side_withdraws() {
    let books = books(
        [
            Statement::from(quote(10, "Q1", "AAPL", ("10.4", 30), ("10.6", 40))),
            Statement::from(quote(20, "Q1", "AAPL", ("10.45", 35), ("10.55", 45))),
            Statement::from(quote(30, "Q1", "AAPL", ("10.45", 35), ("0", 0))),
        ],
        5,
    );
    assert_eq!(books[0].get_bids(), [level("10.4", 30, 1)]);
    assert_eq!(books[0].get_asks(), [level("10.6", 40, 1)]);
    assert_eq!(books[1].get_bids(), [level("10.45", 35, 1)]);
    assert_eq!(books[1].get_asks(), [level("10.55", 45, 1)]);
    assert_eq!(books[2].get_bids(), [level("10.45", 35, 1)]);
    assert_eq!(books[2].get_asks(), [], "a lane at zero is withdrawn");
    assert_eq!(books[2].get_bidpx(), Some(decimal("10.45")), "the one lane");
    assert_eq!(
        books[2].get_px(),
        Decimal18::ZERO,
        "no mid on a one-sided book"
    );
}

#[test]
fn prints_move_the_last_the_volume_and_the_average_and_a_bust_moves_nothing() {
    let mut trade = TradeData::at(40);
    trade.set_crosscode("M1".to_owned());
    trade.set_symbolticker(Some("AAPL".to_owned()));
    trade.set_px(decimal("11"));
    trade.set_qty(Decimal18::from_int(300));
    trade.set_srcuuids(vec![source("M1", 40)]);
    trade.finalize();
    let books = books(
        [
            Statement::from(order(10, "A1", "AAPL", "Buy", "10.5", 1_000)),
            Statement::from(print(20, "A1", "AAPL", "10", 100, "PartiallyFilled")),
            Statement::from(print(30, "A1", "AAPL", "11", 300, "PartiallyFilled")),
            // The trade of the same fill, counted again: the documented rule.
            Statement::from(trade),
            // A priceless print counts among the updates and moves nothing.
            Statement::from(print(50, "A1", "AAPL", "0", 5, "PartiallyFilled")),
            // A bust: the venue says the print did not happen.
            Statement::from(print(60, "A1", "AAPL", "9", 999, "TradeCancel")),
        ],
        5,
    );
    assert_eq!(books.len(), 6);
    assert_eq!(books[1].get_lastpx(), Some(decimal("10")));
    assert_eq!(books[2].get_lastqty(), Some(Decimal18::from_int(300)));
    assert_eq!(books[2].get_cumqty(), Some(Decimal18::from_int(400)));
    assert_eq!(books[2].get_avgpx(), Some(decimal("10.75")), "exactly");
    assert_eq!(
        books[3].get_cumqty(),
        Some(Decimal18::from_int(700)),
        "the trade counted too"
    );
    assert_eq!(books[4].get_cumqty(), Some(Decimal18::from_int(700)));
    assert_eq!(books[4].get_updates(), 5);
    assert_eq!(
        books[5].get_lastpx(),
        Some(decimal("11")),
        "a bust moved nothing"
    );
    assert_eq!(books[5].get_cumqty(), Some(Decimal18::from_int(700)));
    assert_eq!(
        books[5].get_bids(),
        [level("10.5", 1000, 1)],
        "a print rests nothing and touches no level"
    );
}

#[test]
fn the_chain_is_flat_and_carries_the_step_before() {
    let books = books(
        [
            Statement::from(order(10, "A1", "AAPL", "Buy", "10.5", 100)),
            Statement::from(order(20, "S1", "AAPL", "Sell", "10.7", 80)),
            Statement::from(order(30, "S2", "AAPL", "Sell", "10.6", 20)),
        ],
        5,
    );
    assert_eq!(
        books.iter().map(Event::get_seqnum).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(books[1].get_prevuuid(), Some(books[0].get_curruuid()));
    assert_eq!(books[2].get_prevuuid(), Some(books[1].get_curruuid()));
    assert_eq!(
        books[2].get_parentuuids(),
        [books[1].get_curruuid()],
        "the predecessor alone"
    );
    assert_eq!(
        books[2].get_prevpx(),
        Some(books[1].get_px()),
        "the mid before"
    );
    assert_eq!(
        books[2].get_prevqty(),
        Some(books[1].get_qty()),
        "the size before"
    );
    assert_eq!(
        books[1].get_prevpx(),
        None,
        "no mid on a one-sided book before"
    );
    assert!(
        books
            .iter()
            .all(|book| book.get_crossuuid() == books[0].get_crossuuid())
    );
}

#[test]
fn a_grid_reads_each_step_once_at_its_closing_state_and_the_end_flushes() {
    let statements = [
        Statement::from(order(1_100, "A1", "AAPL", "Buy", "10.5", 100)),
        Statement::from(order(1_200, "A2", "AAPL", "Buy", "10.4", 50)),
        Statement::from(order(1_300, "M1", "MSFT", "Buy", "400", 10)),
        // A step nothing of MSFT touches.
        Statement::from(order(2_500, "A3", "AAPL", "Buy", "10.3", 70)),
        Statement::from(order(4_100, "M1", "MSFT", "Buy", "401", 10)),
    ];
    let books: Vec<BookData> = BookIterator::new(statements.clone().map(Ok), depth(5), true)
        .with_snapshot_ns(1_000)
        .collect::<yggdryl::Result<_>>()
        .expect("every statement applies");
    assert_eq!(
        books
            .iter()
            .map(|book| (
                book.get_crosscode().to_owned(),
                book.get_snapunix(),
                book.get_currunix()
            ))
            .collect::<Vec<_>>(),
        [
            ("AAPL".to_owned(), Some(1_000), 1_200),
            ("MSFT".to_owned(), Some(1_000), 1_300),
            ("AAPL".to_owned(), Some(2_000), 2_500),
            ("MSFT".to_owned(), Some(4_000), 4_100),
        ]
    );
    assert_eq!(books[0].get_bids().len(), 2, "the step's closing state");
    assert_eq!(books[0].get_updates(), 2);
    assert_eq!(
        books[3].get_bids(),
        [level("401", 10, 1)],
        "restated, not doubled"
    );
    assert_eq!(books[3].get_seqnum(), 1, "chained per symbol across steps");
    // Without a grid, one per instant.
    let books: Vec<BookData> = BookIterator::new(statements.map(Ok), depth(5), true)
        .with_snapshot_ns(0)
        .collect::<yggdryl::Result<_>>()
        .expect("every statement applies");
    assert_eq!(books.len(), 5);
    assert!(books.iter().all(|book| book.get_snapunix().is_none()));
}

#[test]
fn an_unsorted_stream_is_sorted_first_and_a_sorted_one_refuses_an_earlier_instant() {
    let statements = [
        Statement::from(order(20, "A2", "AAPL", "Buy", "10.4", 50)),
        Statement::from(order(10, "A1", "AAPL", "Buy", "10.5", 100)),
    ];
    let books: Vec<BookData> = BookIterator::new(statements.clone().map(Ok), depth(5), false)
        .collect::<yggdryl::Result<_>>()
        .expect("every statement applies");
    assert_eq!(
        books.iter().map(Event::get_currunix).collect::<Vec<_>>(),
        [10, 20]
    );
    let results: Vec<yggdryl::Result<BookData>> =
        BookIterator::new(statements.map(Ok), depth(5), true).collect();
    assert_eq!(results.len(), 2);
    let refused = results[0].as_ref().unwrap_err().to_string();
    assert!(refused.contains("currunix"), "{refused}");
    let book = results[1].as_ref().expect("the stream goes on");
    assert_eq!(
        book.get_bids(),
        [level("10.4", 50, 1)],
        "the earlier statement moved nothing"
    );
}

#[test]
fn a_source_error_is_yielded_where_met_and_closes_nothing() {
    let items: Vec<yggdryl::Result<Statement>> = vec![
        Ok(Statement::from(order(10, "A1", "AAPL", "Buy", "10.5", 100))),
        Err(yggdryl::Error::InvalidRecord {
            path: "line".into(),
            reason: "a line that is no message".into(),
        }),
        Ok(Statement::from(order(10, "A2", "AAPL", "Buy", "10.5", 50))),
    ];
    let results: Vec<yggdryl::Result<BookData>> =
        BookIterator::new(items, depth(5), true).collect();
    assert_eq!(results.len(), 2);
    assert!(results[0].is_err(), "the error first, where it was met");
    let book = results[1].as_ref().expect("the instant closed whole");
    assert_eq!(
        book.get_bids(),
        [level("10.5", 150, 2)],
        "both statements of the instant applied"
    );
}

#[test]
fn an_instrument_named_by_its_ticker_then_its_isin_stays_one_book() {
    let mut named = order(20, "A2", "AAPL", "Buy", "10.4", 50);
    named.set_isincode(Some(Isin::new("US0378331005").expect("an ISIN")));
    named.finalize();
    // A print naming the ISIN alone, keyed under the maker it fills.
    let mut fill = print(30, "A1", "", "10.5", 10, "PartiallyFilled");
    fill.set_symbolticker(None);
    fill.set_isincode(Some(Isin::new("US0378331005").expect("an ISIN")));
    fill.finalize();
    let books = books(
        [
            Statement::from(order(10, "A1", "AAPL", "Buy", "10.5", 100)),
            Statement::from(named),
            Statement::from(fill),
        ],
        5,
    );
    assert_eq!(books.len(), 3);
    assert!(
        books.iter().all(|book| book.get_crosscode() == "AAPL"),
        "the first name met keys it"
    );
    assert_eq!(
        books[1].get_bids(),
        [level("10.5", 100, 1), level("10.4", 50, 1)]
    );
    assert_eq!(
        books[1].get_isincode().map(Isin::as_str),
        Some("US0378331005")
    );
    assert_eq!(books[2].get_lastqty(), Some(Decimal18::from_int(10)));
}

#[test]
fn a_maker_restated_under_another_symbol_leaves_the_ladder_it_rested_on() {
    // The same order, named by a ticker first and by a CUSIP alone later,
    // where nothing ties the two names: it rests once, under the new key.
    let mut renamed = order(20, "A1", "", "Buy", "10.5", 100);
    renamed.set_cusipcode(Some(yggdryl::Cusip::new("037833100").expect("a CUSIP")));
    renamed.finalize();
    let books = books(
        [
            Statement::from(order(10, "A1", "AAPL", "Buy", "10.5", 100)),
            Statement::from(renamed),
        ],
        5,
    );
    assert_eq!(books.len(), 3);
    assert_eq!(books[0].get_crosscode(), "AAPL");
    let at_twenty: Vec<(&str, usize)> = books[1..]
        .iter()
        .map(|book| (book.get_crosscode(), book.get_bids().len()))
        .collect();
    assert_eq!(
        at_twenty,
        [("037833100", 1), ("AAPL", 0)],
        "left one ladder, rests on the other"
    );
}

#[test]
fn the_declarations_are_read_back() {
    let iterator = BookIterator::new(
        std::iter::empty::<yggdryl::Result<Statement>>(),
        depth(3),
        true,
    )
    .with_snapshot_ns(7)
    .with_symbol(Symbol::new("AAPL"));
    assert_eq!(iterator.depth(), depth(3));
    assert_eq!(iterator.snapshot_ns(), Some(7));
    assert_eq!(iterator.symbol(), Some(&Symbol::new("AAPL")));
    assert_eq!(iterator.with_snapshot_ns(0).snapshot_ns(), None);
}
