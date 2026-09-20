//! The three hops over the bridge's own capture: a line's `curruuid` is a
//! message's `srcuuids`, a message's `curruuid` is a product's `srcuuids`,
//! and a product built from the capture multiplies neither the messages
//! nor their identities.

use std::collections::BTreeSet;

use yggdryl::graph::{Element, Event};
use yggdryl::holder::Buffer;
use yggdryl::market::{Book, BookData, ExecutionData, OrderData, Product, QuoteData, TradeData};
use yggdryl::media::RecordOptions;
use yggdryl::text::{TextLine, TextOptions, read_text_lines};
use yggdryl::{FixCodec, FixMsg, IOMedia, Timezone, Url, Uuid};

use super::{codec as fixed_codec, product_rows, rows_of};

/// The capture, exactly as the bridge wrote it.
const LOG: &[u8] = include_bytes!("../fix/ulbridge.log");

/// How many lines the capture holds.
const LINES: usize = 144;

/// How many messages the capture reads as under the codec's own defaults.
const MESSAGES: usize = 79;

/// How many chains the lifecycle chains those messages into: the distinct
/// cross identities the walk answers, each a conversation the bridge
/// bracketed or an identifier a wire frame named.
const CHAINS: usize = 11;

/// How many statements each product door reads out of the capture, and
/// how many identities they are. An order is every message about an
/// order; an execution one report stating a fill, hop copies whose facts
/// agree folded into one; a trade one per report too, but a bridge adds
/// parties as it forwards a report, so copies that name different parties
/// are different statements of the match and stay apart; the capture
/// quotes nothing; a book is one per symbol per second a maker or a print
/// touched it in, read at the second's closing state. The fold leaves one
/// statement per identity, so the two counts agree, and a door that
/// yielded a twin would part them. The books moved from nine to ten when
/// the book became one per symbol per step touched rather than one per
/// instrument per step a maker was live in: a second in which only a
/// fill printed now reads a book.
const ORDERS: (usize, usize) = (18, 18);
const EXECUTIONS: (usize, usize) = (14, 14);
const TRADES: (usize, usize) = (19, 19);
const BOOKS: (usize, usize) = (10, 10);

/// One second per grid step, which is the whole capture.
const STEP: i64 = 1_000_000_000;

fn source() -> Buffer {
    Buffer::from_bytes(LOG.to_vec()).with_media_type(
        Url::from_str("file:///ulbridge.log")
            .expect("a URL")
            .media_type(),
    )
}

fn reading() -> RecordOptions {
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
        .expect("the bridge's row header compiles")
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    options.parse_mimetype = true;
    options.into()
}

fn codec() -> FixCodec {
    let RecordOptions::Text(options) = reading() else {
        panic!("a text read")
    };
    fixed_codec().with_capture_names(options.capture_names().map(ToOwned::to_owned))
}

fn lines() -> Vec<TextLine> {
    let RecordOptions::Text(options) = reading() else {
        panic!("a text read")
    };
    read_text_lines(&source(), &options)
        .expect("a line reader")
        .map(|line| line.expect("a line"))
        .collect()
}

fn messages(codec: &FixCodec) -> Vec<FixMsg> {
    codec
        .lifecycle(codec.parse_text_lines(lines()))
        .collect::<yggdryl::Result<_>>()
        .expect("every line reads")
}

fn identities<E: Element>(elements: &[E]) -> BTreeSet<Uuid> {
    elements.iter().map(Element::get_curruuid).collect()
}

/// Every source of every product is the identity of one message, every
/// product is one of at most as many as the messages it was read from,
/// and the count of statements and of identities is what the capture
/// pins.
fn joins<P: Product>(products: &[P], messages: &[FixMsg], pinned: (usize, usize)) {
    let known = identities(messages);
    for product in products {
        assert!(
            !product.get_srcuuids().is_empty(),
            "a product names what it was read from"
        );
        for source in product.get_srcuuids() {
            assert!(known.contains(source), "a source that is no message");
        }
        assert_eq!(
            product.get_curruuid(),
            product.time_uuid().expect("an instant")
        );
    }
    assert!(
        products.len() <= messages.len(),
        "products multiply no messages"
    );
    assert!(
        identities(products).len() <= known.len(),
        "products multiply no identities"
    );
    assert_eq!((products.len(), identities(products).len()), pinned);
}

#[test]
fn the_core_answers_the_capture_as_the_messages_and_chains_it_claims() {
    let codec = codec();
    let lines = lines();
    assert_eq!(lines.len(), LINES);
    let messages = messages(&codec);
    assert_eq!(messages.len(), MESSAGES);
    let chains: BTreeSet<Uuid> = messages.iter().map(Element::get_crossuuid).collect();
    assert_eq!(chains.len(), CHAINS);
    // The second hop: a message's sources are the lines it was read from.
    let line_identities = identities(&lines);
    for message in &messages {
        assert_eq!(message.get_srcuuids().len(), 1);
        assert!(line_identities.contains(&message.get_srcuuids()[0]));
    }
}

#[test]
fn every_product_joins_the_message_table_on_srcuuids_to_curruuid() {
    let codec = codec();
    let messages = messages(&codec);
    let parsed = || codec.parse_text_lines(lines());

    let orders: Vec<OrderData> = codec
        .orders(parsed())
        .collect::<yggdryl::Result<_>>()
        .expect("the orders read");
    joins(&orders, &messages, ORDERS);
    assert!(!orders.is_empty());
    // An order's chain joins its own table: every predecessor is an order.
    let order_identities = identities(&orders);
    for order in &orders {
        if let Some(previous) = order.get_prevuuid() {
            assert!(order_identities.contains(&previous));
        }
        for parent in order.get_parentuuids() {
            assert!(order_identities.contains(parent));
        }
    }

    let executions: Vec<ExecutionData> = codec
        .executions(parsed())
        .collect::<yggdryl::Result<_>>()
        .expect("the executions read");
    joins(&executions, &messages, EXECUTIONS);
    assert!(!executions.is_empty());
    assert!(
        executions.len() < orders.len(),
        "a fill is one report, an order every report"
    );

    let trades: Vec<TradeData> = codec
        .trades(parsed())
        .collect::<yggdryl::Result<_>>()
        .expect("the trades read");
    joins(&trades, &messages, TRADES);
    assert!(
        trades.len() >= executions.len(),
        "every fill reports a trade, and hops add parties"
    );

    let quotes: Vec<QuoteData> = codec
        .quotes(parsed())
        .collect::<yggdryl::Result<_>>()
        .expect("the quotes read");
    joins(&quotes, &messages, (0, 0));
    assert!(quotes.is_empty(), "the capture quotes nothing");

    let books: Vec<BookData> = codec
        .books(parsed(), 5, STEP)
        .expect("a grid and a depth")
        .collect::<yggdryl::Result<_>>()
        .expect("the books read");
    joins(&books, &messages, BOOKS);
    assert!(!books.is_empty());
    for book in &books {
        assert!(
            book.get_snapunix().is_some(),
            "every book is a snapshot of a step"
        );
        assert!(book.get_bids().len() <= 5 && book.get_asks().len() <= 5);
    }
}

/// Each Arrow door is its message door over the messages the rows hold -
/// [`FixCodec::messages`] - exactly as the lifecycle's Arrow twin is: the
/// rows the capture parsed into, read back as messages, walked and read
/// as products, row for row.
#[test]
fn every_arrow_door_agrees_with_its_message_door_over_the_capture() {
    let codec = codec();
    let reader = || {
        codec
            .parse_text_arrow_reader(source().read_arrow_reader(&reading()).expect("a reader"))
            .expect("the message rows open")
    };
    let parsed = || codec.messages(reader());
    assert_eq!(
        rows_of(codec.orders_arrow_reader(reader()).expect("orders")),
        product_rows(codec.orders(parsed()))
    );
    assert_eq!(
        rows_of(codec.executions_arrow_reader(reader()).expect("executions")),
        product_rows(codec.executions(parsed()))
    );
    assert_eq!(
        rows_of(codec.trades_arrow_reader(reader()).expect("trades")),
        product_rows(codec.trades(parsed()))
    );
    assert_eq!(
        rows_of(codec.quotes_arrow_reader(reader()).expect("quotes")),
        product_rows(codec.quotes(parsed()))
    );
    assert_eq!(
        rows_of(codec.books_arrow_reader(reader(), 5, STEP).expect("books")),
        product_rows(codec.books(parsed(), 5, STEP).expect("a grid"))
    );
}
