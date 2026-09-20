//! A book: the ladder for one instrument at one instant, one per symbol
//! per instant the makers and prints touched, or per grid step, to a
//! declared depth, an order logged twice counted once, and no message
//! that states it back.

use std::num::NonZeroU32;

use yggdryl::graph::{Element, Event, MarketElement};
use yggdryl::market::{Book, BookData, Level, Product};
use yggdryl::{Decimal18, FixMsg};

use super::{codec, message_reader, parsed, product_rows, rows_of};

/// One second per grid step.
const STEP: i64 = 1_000_000_000;

/// Two bids at one price and a third below, an offer, a two-sided quote,
/// then a fill that retires one bid, over three seconds; and one order of
/// another instrument.
const MAKERS: [&[u8]; 7] = [
    b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|15=USD|52=20260102-10:15:30.100|10=0|",
    b"8=FIX.4.4|35=D|11=A2|55=AAPL|54=1|38=50|44=10.5|15=USD|52=20260102-10:15:30.200|10=0|",
    b"8=FIX.4.4|35=D|11=A3|55=AAPL|54=1|38=70|44=10.4|15=USD|52=20260102-10:15:30.300|10=0|",
    b"8=FIX.4.4|35=D|11=S1|55=AAPL|54=2|38=80|44=10.7|15=USD|52=20260102-10:15:30.400|10=0|",
    b"8=FIX.4.4|35=S|117=Q1|55=AAPL|132=10.45|134=30|133=10.6|135=40|15=USD|52=20260102-10:15:31.100|10=0|",
    b"8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=2|38=100|14=100|151=0|31=10.5|32=100|55=AAPL|54=1|15=USD|52=20260102-10:15:32.100|10=0|",
    b"8=FIX.4.4|35=D|11=M1|55=MSFT|54=1|38=10|44=400|15=USD|52=20260102-10:15:32.500|10=0|",
];

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

#[test]
fn one_book_per_symbol_per_instant_touched_to_the_declared_depth() {
    let codec = codec();
    let books: Vec<BookData> = codec
        .books(parsed(&codec, &MAKERS), 2, 0)
        .expect("a depth")
        .collect::<yggdryl::Result<_>>()
        .expect("every message reads");
    // Six instants of AAPL, one of MSFT.
    assert_eq!(books.len(), 7);
    let [first, second, third, fourth, quoted, filled, other] = books.as_slice() else {
        panic!("seven books")
    };
    // The first instant: one bid, no offer, no mid, no grid.
    assert_eq!(first.get_crosscode(), "AAPL");
    assert_eq!(first.get_snapunix(), None);
    assert_eq!(first.get_currunix(), 1_767_348_930_100_000_000);
    assert_eq!(first.get_bids(), [level("10.5", 100, 1)]);
    assert_eq!(first.get_asks(), []);
    assert_eq!(
        first.get_bidpx(),
        Some(decimal("10.5")),
        "the top of the ladder is the lane"
    );
    assert_eq!(
        first.get_px(),
        Decimal18::ZERO,
        "no mid on a one-sided book"
    );
    assert_eq!(first.get_depth(), depth(2));
    assert_eq!(first.get_currency().as_str(), "USD");
    assert_eq!(first.get_symbolticker(), Some("AAPL"));
    assert!(
        first.get_identifiers().is_empty(),
        "a book goes by its symbol, which is its chain"
    );
    assert_eq!(first.get_updates(), 1);
    // Two bids at one price are one level, summed and counted.
    assert_eq!(second.get_bids(), [level("10.5", 150, 2)]);
    assert_eq!(
        third.get_bids(),
        [level("10.5", 150, 2), level("10.4", 70, 1)]
    );
    // The offer makes a mid, and the quote's lanes cut the ladder to the depth.
    assert_eq!(fourth.get_asks(), [level("10.7", 80, 1)]);
    assert_eq!(fourth.get_px(), decimal("10.6"), "the mid");
    assert_eq!(
        fourth.get_qty(),
        Decimal18::from_int(300),
        "resting on both ladders"
    );
    assert_eq!(
        quoted.get_bids(),
        [level("10.5", 150, 2), level("10.45", 30, 1)]
    );
    assert_eq!(
        quoted.get_asks(),
        [level("10.6", 40, 1), level("10.7", 80, 1)]
    );
    assert_eq!(quoted.spread(), Some(decimal("0.1")));
    assert_eq!(quoted.microprice(), Some(decimal("10.578947368421052631")));
    // The fill retires the first bid, and prints against the book.
    assert_eq!(
        filled.get_bids(),
        [level("10.5", 50, 1), level("10.45", 30, 1)]
    );
    assert_eq!(filled.get_lastpx(), Some(decimal("10.5")));
    assert_eq!(filled.get_lastqty(), Some(Decimal18::from_int(100)));
    assert_eq!(filled.get_cumqty(), Some(Decimal18::from_int(100)));
    assert_eq!(filled.get_avgpx(), Some(decimal("10.5")));
    assert_eq!(
        filled.get_updates(),
        7,
        "five makers, the order's fill and the print"
    );
    // The chain is the symbol's, flat: each book follows the one before
    // and descends from it alone.
    assert_eq!(second.get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(
        books[..6].iter().map(Event::get_seqnum).collect::<Vec<_>>(),
        [0, 1, 2, 3, 4, 5]
    );
    assert_eq!(filled.get_parentuuids(), [quoted.get_curruuid()]);
    assert_eq!(filled.get_prevpx(), Some(quoted.get_px()), "the mid before");
    assert_eq!(filled.get_prevqty(), Some(quoted.get_qty()));
    // A book's sources are the statements it was read at.
    let chained: Vec<FixMsg> = codec
        .lifecycle(parsed(&codec, &MAKERS))
        .collect::<yggdryl::Result<_>>()
        .expect("the lifecycle");
    assert_eq!(quoted.get_srcuuids(), [chained[4].get_curruuid()]);
    let mut retiring = vec![chained[0].get_curruuid(), chained[5].get_curruuid()];
    retiring.sort_unstable();
    assert_eq!(
        filled.get_srcuuids(),
        retiring,
        "the report, one message for its order and its print, and the placement it retired"
    );
    // Another instrument is another chain.
    assert_eq!(other.get_crosscode(), "MSFT");
    assert_eq!(other.get_seqnum(), 0);
    assert_eq!(other.get_bids(), [level("400", 10, 1)]);
    assert_eq!(other.get_updates(), 1);
    // Each identity is what the ladder states and when.
    for book in &books {
        assert_eq!(book.get_curruuid(), book.time_uuid().expect("an instant"));
        assert_eq!(
            book.get_state().as_str(),
            "00UNKNOWN",
            "a book has no lifecycle of its own"
        );
    }
}

#[test]
fn a_grid_reads_one_book_per_symbol_per_step_at_its_closing_state() {
    let codec = codec();
    let books: Vec<BookData> = codec
        .books(parsed(&codec, &MAKERS), 2, STEP)
        .expect("a grid and a depth")
        .collect::<yggdryl::Result<_>>()
        .expect("every message reads");
    // Three steps of AAPL, one of MSFT, in the order the steps closed.
    assert_eq!(books.len(), 4);
    let [first, second, third, other] = books.as_slice() else {
        panic!("four books")
    };
    assert_eq!(first.get_snapunix(), Some(1_767_348_930_000_000_000));
    assert_eq!(
        first.get_currunix(),
        1_767_348_930_400_000_000,
        "dated at the last instant that moved it"
    );
    assert_eq!(
        first.get_bids(),
        [level("10.5", 150, 2), level("10.4", 70, 1)]
    );
    assert_eq!(first.get_asks(), [level("10.7", 80, 1)]);
    assert_eq!(first.get_updates(), 4);
    assert_eq!(second.get_snapunix(), Some(1_767_348_931_000_000_000));
    assert_eq!(
        second.get_bids(),
        [level("10.5", 150, 2), level("10.45", 30, 1)]
    );
    assert_eq!(second.get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(third.get_snapunix(), Some(1_767_348_932_000_000_000));
    assert_eq!(
        third.get_bids(),
        [level("10.5", 50, 1), level("10.45", 30, 1)]
    );
    assert_eq!(third.get_seqnum(), 2);
    // The step's sources are every statement applied in it.
    assert_eq!(first.get_srcuuids().len(), 4);
    assert_eq!(other.get_crosscode(), "MSFT");
    assert_eq!(other.get_snapunix(), Some(1_767_348_932_000_000_000));
}

#[test]
fn an_order_logged_at_two_hops_rests_once_on_its_level() {
    let codec = codec();
    // The first bid logged twice, the second bid, then the offer a second
    // later.
    let lines: [&[u8]; 4] = [
        MAKERS[0],
        MAKERS[0],
        MAKERS[1],
        b"8=FIX.4.4|35=D|11=S1|55=AAPL|54=2|38=80|44=10.7|15=USD|52=20260102-10:15:31.400|10=0|",
    ];
    let books: Vec<BookData> = codec
        .books(parsed(&codec, &lines), 4, STEP)
        .expect("a grid and a depth")
        .collect::<yggdryl::Result<_>>()
        .expect("every message reads");
    assert_eq!(books.len(), 2);
    assert_eq!(
        books[0].get_bids(),
        [level("10.5", 150, 2)],
        "the twin counted once"
    );
    assert_eq!(books[0].get_updates(), 2, "the twin is one statement");
    assert_eq!(books[0].get_srcuuids().len(), 2, "and one message");
    assert_eq!(books[1].get_asks(), [level("10.7", 80, 1)]);
    assert_eq!(books[1].get_srcuuids().len(), 1);
}

#[test]
fn depth_and_grid_are_declarations() {
    let codec = codec();
    let refused = codec
        .books(parsed(&codec, &MAKERS), 0, STEP)
        .map(drop)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("depth"), "{refused}");
    let refused = codec
        .books(parsed(&codec, &MAKERS), 2, -1)
        .map(drop)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("snapshot_ns"), "{refused}");
    let mut book = BookData::at(7, NonZeroU32::MIN);
    assert!(
        book.set_bids(vec![level("1", 1, 1), level("2", 1, 1)])
            .is_err()
    );
    assert!(
        book.set_asks(vec![level("1", 1, 1), level("1", 2, 1)])
            .is_err(),
        "one level per price"
    );
}

#[test]
fn the_row_is_fixed_width_round_trips_and_the_arrow_door_agrees() {
    let codec = codec();
    let field = BookData::field(depth(2)).expect("the book row");
    let names: Vec<&str> = field.fields()[16..]
        .iter()
        .map(yggdryl::Field::name)
        .collect();
    assert_eq!(
        &names[..5],
        &["lastpx", "lastqty", "avgpx", "cumqty", "tradable"]
    );
    assert_eq!(&names[names.len() - 3..], &["bids", "asks", "updates"]);
    let bids = field.field("bids").expect("bids");
    assert!(!bids.is_nullable());
    assert!(matches!(
        bids.dtype(),
        yggdryl::DataType::Sequence(yggdryl::sequence::SequenceType::FixedSizeList(_, 2))
    ));
    let messages = parsed(&codec, &MAKERS);
    let books: Vec<BookData> = codec
        .books(messages.clone(), 2, 0)
        .expect("a depth")
        .collect::<yggdryl::Result<_>>()
        .expect("every message reads");
    for book in &books {
        let row = book.into_row().expect("a row");
        let cells = row.as_sequence().expect("cells");
        assert_eq!(
            cells[cells.len() - 3]
                .as_sequence()
                .map(<[yggdryl::Scalar]>::len),
            Some(2)
        );
        let again = BookData::from_row(&field, &row).expect("reads");
        assert_eq!(again.into_row().expect("a row"), row);
        assert_eq!(again.get_curruuid(), book.get_curruuid());
        assert_eq!(again.get_bids(), book.get_bids());
        assert_eq!(
            again.get_px(),
            book.get_px(),
            "the mid is re-derived from the ladders"
        );
        assert_eq!(again.get_updates(), book.get_updates());
    }
    let rows = rows_of(
        codec
            .books_arrow_reader(message_reader(&codec, messages.clone()), 2, 0)
            .expect("the book rows open"),
    );
    assert_eq!(
        rows,
        product_rows(codec.books(messages, 2, 0).expect("a depth"))
    );
    assert_eq!(rows.len(), 7);
}

#[test]
fn no_message_states_a_book_and_the_refusal_says_why() {
    let codec = codec();
    let book = codec
        .books(parsed(&codec, &MAKERS), 2, STEP)
        .expect("a grid")
        .next()
        .expect("a book")
        .expect("it reads");
    let refused = FixMsg::from_book(&codec, &book).unwrap_err().to_string();
    assert!(
        refused.contains("book") && refused.contains("does not guess"),
        "{refused}"
    );
}
