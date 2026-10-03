//! `rust/src/graph/candle.rs`: the OHLC a bucket of books folds to, the
//! options that align buckets to a zone, the walk that emits candles and
//! the Arrow face a candle has.

use smol_str::SmolStr;
use yggdryl::graph::{
    BookEvent, BookIterator, Candle, CandleIterator, CandleOptions, Element, Event, Market,
    MarketData, Ohlc, QuoteEvent,
};
use yggdryl::{ArrowCastOptions, Decimal, Scalar, Serie, Timezone};

/// Nanoseconds in one second.
const SECOND: i64 = 1_000_000_000;

/// Nanoseconds in one minute.
const MINUTE: i64 = 60 * SECOND;

/// Nanoseconds in one hour.
const HOUR: i64 = 60 * MINUTE;

/// `2026-03-29T00:00:00Z`, the day Europe/Zurich springs forward at
/// `01:00Z`: its wall clock skips `02:00`-`03:00`.
const SPRING_DAY: i64 = 1_774_742_400 * SECOND;

/// `2026-10-25T00:00:00Z`, the day Europe/Zurich falls back at `01:00Z`:
/// its wall clock reads `02:00`-`03:00` twice.
const FALL_DAY: i64 = 1_792_886_400 * SECOND;

/// `2026-01-05T10:00:00Z`.
const OFFSET_DAY: i64 = 1_767_607_200 * SECOND;

/// One finalized quote of `ticker` going by `code`.
fn quote(
    unix: i64,
    ticker: &str,
    code: &str,
    side: &str,
    price: &str,
    quantity: i64,
) -> MarketData {
    let mut quote = QuoteEvent::at(unix);
    quote.set_crosscode(code.to_owned());
    quote.set_ticker(Some(SmolStr::new(ticker)), true);
    quote.set_side(yggdryl::Side::read(side).unwrap(), true);
    quote.set_price(Some(price.parse().unwrap()), true);
    quote.set_quantity(Some(Decimal::from_int(quantity)), true);
    quote.set_state(yggdryl::State::New);
    quote.finalize();
    MarketData::from(quote)
}

/// The books `operations` fold into, in stream order.
fn books(operations: Vec<MarketData>) -> Vec<BookEvent> {
    BookIterator::new(operations.into_iter(), 0)
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap()
}

/// The candles `operations` fold into under `options`.
fn candles(operations: Vec<MarketData>, options: CandleOptions) -> Vec<Candle> {
    CandleIterator::new(books(operations).into_iter().map(Ok), options)
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap()
}

/// The candles of empty books at `instants` under `options`.
fn empty_candles(instants: &[i64], options: CandleOptions) -> Vec<Candle> {
    let books = instants
        .iter()
        .map(|unix| Ok(BookEvent::new(*unix, "ACME")))
        .collect::<Vec<_>>();
    CandleIterator::new(books.into_iter(), options)
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap()
}

/// The Europe/Zurich zone.
fn zurich() -> Timezone {
    Timezone::from_str("Europe/Zurich").unwrap()
}

/// `text` as the decimal it spells.
fn decimal(text: &str) -> Decimal {
    text.parse().unwrap()
}

/// The reading `open, high, low, close` spell.
fn ohlc(open: &str, high: &str, low: &str, close: &str) -> Ohlc {
    Ohlc {
        open: decimal(open),
        high: decimal(high),
        low: decimal(low),
        close: decimal(close),
    }
}

/// The wall-clock hour of the day `zone` reads at `unix`.
fn local_hour(zone: Timezone, unix: i64) -> i64 {
    zone.into_local(unix / SECOND).unwrap().rem_euclid(86_400) / 3_600
}

#[test]
fn an_interval_must_be_positive() {
    for interval in [0, -1, i64::MIN] {
        let error = CandleOptions::new(interval).unwrap_err().to_string();
        assert!(
            error.contains("$.interval") && error.contains(&interval.to_string()),
            "{error}"
        );
    }
    assert_eq!(CandleOptions::new(1).unwrap().interval(), 1);
    assert!(CandleOptions::new(1).unwrap().timezone().is_utc());
}

#[test]
fn every_spelling_reads_and_writes_back() {
    let cases = [
        ("30s", 30 * SECOND),
        ("1m", MINUTE),
        ("5m", 5 * MINUTE),
        ("1h", HOUR),
        ("1d", 24 * HOUR),
        ("1w", 7 * 24 * HOUR),
        ("250ms", 250_000_000),
        ("7us", 7_000),
        ("3ns", 3),
    ];
    for (spelling, interval) in cases {
        let options = CandleOptions::from_spelling(spelling).unwrap();
        assert_eq!(options.interval(), interval, "{spelling}");
        assert_eq!(options.spelling(), spelling);
        assert!(options.timezone().is_utc());
    }
    // The widest unit that divides exactly is the one written.
    assert_eq!(CandleOptions::new(90 * SECOND).unwrap().spelling(), "90s");
    assert_eq!(CandleOptions::new(120 * SECOND).unwrap().spelling(), "2m");
    assert_eq!(
        CandleOptions::new(1_500_000_000).unwrap().spelling(),
        "1500ms"
    );
    assert_eq!(CandleOptions::new(14 * 24 * HOUR).unwrap().spelling(), "2w");
    for refused in [
        "",
        "m",
        "0s",
        "1x",
        "1.5m",
        "1 m",
        " 1m",
        "-1m",
        "1M",
        "1min",
        "99999999999999999999s",
        "100000000000w",
    ] {
        let error = CandleOptions::from_spelling(refused)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("$.interval") && error.contains(&format!("{refused:?}")),
            "{refused}: {error}"
        );
    }
}

#[test]
fn the_zone_is_stated_beside_the_interval() {
    let options = CandleOptions::from_spelling("1h")
        .unwrap()
        .with_timezone(zurich());
    assert_eq!(options.timezone(), &zurich());
    assert_eq!(options.spelling(), "1h");
    assert_eq!(
        options,
        CandleOptions::new(HOUR).unwrap().with_timezone(zurich())
    );
    assert_ne!(options, CandleOptions::new(HOUR).unwrap());
}

#[test]
fn an_unsorted_stream_is_refused_at_the_book_and_the_walk_fuses() {
    let books = vec![
        Ok(BookEvent::new(2_000, "ACME")),
        Ok(BookEvent::new(2_000, "ACME")),
        Ok(BookEvent::new(1_000, "ACME")),
        Ok(BookEvent::new(3_000, "ACME")),
    ];
    let mut candles = CandleIterator::new(books.into_iter(), CandleOptions::new(MINUTE).unwrap());
    let error = candles.next().unwrap().unwrap_err().to_string();
    assert_eq!(
        error,
        "invalid record value at $.book.currunix: expected an instant at or after 2000, got 1000"
    );
    assert!(candles.next().is_none(), "the walk fuses");
    assert!(candles.next().is_none());
}

#[test]
fn a_source_error_follows_the_completed_buckets_and_fuses() {
    let books = vec![
        Ok(BookEvent::new(10 * SECOND, "ACME")),
        Ok(BookEvent::new(70 * SECOND, "ACME")),
        Err(yggdryl::Error::InvalidRecord {
            path: "$.source".into(),
            reason: "the source failed".into(),
        }),
        Ok(BookEvent::new(80 * SECOND, "ACME")),
    ];
    let mut candles = CandleIterator::new(books.into_iter(), CandleOptions::new(MINUTE).unwrap());
    let first = candles.next().unwrap().unwrap();
    assert_eq!((first.start, first.end, first.books), (0, MINUTE, 1));
    let error = candles.next().unwrap().unwrap_err().to_string();
    assert_eq!(error, "invalid record value at $.source: the source failed");
    assert!(
        candles.next().is_none(),
        "the open bucket is dropped, not emitted"
    );
}

#[test]
fn an_empty_stream_yields_no_candle() {
    let mut candles = CandleIterator::new(std::iter::empty(), CandleOptions::new(MINUTE).unwrap());
    assert!(candles.next().is_none());
    assert!(candles.next().is_none());
    assert_eq!(candles.options().interval(), MINUTE);
}

#[test]
fn the_ohlc_of_every_reading_over_one_minute() {
    let operations = vec![
        quote(10 * SECOND, "ACME", "B", "Buy", "100", 5),
        quote(10 * SECOND, "ACME", "A", "Sell", "103", 7),
        quote(20 * SECOND, "ACME", "B", "Buy", "102", 5),
        quote(30 * SECOND, "ACME", "A", "Sell", "102.5", 7),
        quote(30 * SECOND, "ACME", "B", "Buy", "99", 5),
        quote(40 * SECOND, "ACME", "B", "Buy", "101", 8),
        quote(40 * SECOND, "ACME", "A", "Sell", "103.5", 9),
    ];
    let folded = books(operations.clone());
    assert_eq!(folded.len(), 4, "one book per instant");
    let candles = candles(operations, CandleOptions::from_spelling("1m").unwrap());
    assert_eq!(candles.len(), 1);
    let candle = &candles[0];
    assert_eq!(candle.crosscode, "3:0:ACME", "the book's stored code");
    assert_eq!(candle.ticker.as_deref(), Some("ACME"));
    assert_eq!((candle.start, candle.end), (0, MINUTE));
    assert_eq!(candle.bid, Some(ohlc("100", "102", "99", "101")));
    assert_eq!(candle.ask, Some(ohlc("103", "103.5", "102.5", "103.5")));
    assert_eq!(candle.mid, Some(ohlc("101.5", "102.5", "100.75", "102.25")));
    assert_eq!(candle.spread, Some(ohlc("3", "3.5", "1", "2.5")));
    assert_eq!(candle.bidqty, Some(Decimal::from_int(8)));
    assert_eq!(candle.askqty, Some(Decimal::from_int(9)));
    assert_eq!(candle.books, 4);
}

#[test]
fn a_one_sided_book_states_no_mid_or_spread() {
    let operations = vec![
        quote(10 * SECOND, "ACME", "B", "Buy", "100", 5),
        quote(20 * SECOND, "ACME", "B", "Buy", "101", 6),
    ];
    let candles = candles(operations, CandleOptions::from_spelling("1m").unwrap());
    assert_eq!(candles.len(), 1);
    let candle = &candles[0];
    assert_eq!(candle.bid, Some(ohlc("100", "101", "100", "101")));
    assert_eq!(candle.ask, None);
    assert_eq!(candle.mid, None);
    assert_eq!(candle.spread, None);
    assert_eq!(candle.bidqty, Some(Decimal::from_int(6)));
    assert_eq!(candle.askqty, None);
    assert_eq!(candle.books, 2);
}

#[test]
fn a_reading_a_later_book_lacks_keeps_the_earlier_ones() {
    // The ask side empties at the second book: the ask, the mid and the
    // spread keep what the first book read, the touch quantities are the
    // last book's.
    let mut cancel = quote(20 * SECOND, "ACME", "A", "Sell", "102", 0);
    if let MarketData::QuoteEvent(quote) = &mut cancel {
        quote.set_state(yggdryl::State::read("Canceled").unwrap());
    }
    cancel.finalize();
    let operations = vec![
        quote(10 * SECOND, "ACME", "B", "Buy", "100", 5),
        quote(10 * SECOND, "ACME", "A", "Sell", "102", 7),
        cancel,
    ];
    let folded = books(operations.clone());
    assert_eq!(folded.len(), 2);
    assert_eq!(folded[1].best_price(yggdryl::Side::Sell), None);
    let candles = candles(operations, CandleOptions::from_spelling("1m").unwrap());
    let candle = &candles[0];
    assert_eq!(candle.ask, Some(ohlc("102", "102", "102", "102")));
    assert_eq!(candle.mid, Some(ohlc("101", "101", "101", "101")));
    assert_eq!(candle.spread, Some(ohlc("2", "2", "2", "2")));
    assert_eq!(candle.bid, Some(ohlc("100", "100", "100", "100")));
    assert_eq!(candle.askqty, None);
    assert_eq!(candle.bidqty, Some(Decimal::from_int(5)));
    assert_eq!(candle.books, 2);
}

#[test]
fn buckets_close_when_the_stream_moves_past_them() {
    let operations = vec![
        quote(10 * SECOND, "ACME", "B", "Buy", "100", 5),
        quote(59 * SECOND, "ACME", "B", "Buy", "101", 5),
        quote(60 * SECOND, "ACME", "B", "Buy", "102", 5),
        quote(200 * SECOND, "ACME", "B", "Buy", "103", 5),
    ];
    let candles = candles(operations, CandleOptions::from_spelling("1m").unwrap());
    assert_eq!(
        candles
            .iter()
            .map(|candle| (candle.start, candle.end, candle.books))
            .collect::<Vec<_>>(),
        [
            (0, MINUTE, 2),
            (MINUTE, 2 * MINUTE, 1),
            (3 * MINUTE, 4 * MINUTE, 1)
        ],
        "an empty bucket yields no candle"
    );
    assert_eq!(
        candles[0].bid.map(|bid| bid.close),
        Some(Decimal::from_int(101))
    );
    assert_eq!(
        candles[1].bid.map(|bid| bid.open),
        Some(Decimal::from_int(102))
    );
}

#[test]
fn two_cross_codes_interleave_and_emit_in_cross_code_order() {
    let operations = vec![
        quote(10 * SECOND, "IBM", "IBM-B", "Buy", "100", 5),
        quote(20 * SECOND, "AAPL", "AAPL-B", "Buy", "200", 5),
        quote(30 * SECOND, "IBM", "IBM-B", "Buy", "101", 5),
        quote(70 * SECOND, "IBM", "IBM-B", "Buy", "102", 5),
        quote(80 * SECOND, "AAPL", "AAPL-B", "Buy", "201", 5),
    ];
    let candles = candles(operations, CandleOptions::from_spelling("1m").unwrap());
    assert_eq!(
        candles
            .iter()
            .map(|candle| (candle.crosscode.as_str(), candle.start, candle.books))
            .collect::<Vec<_>>(),
        [
            ("3:0:AAPL", 0, 1),
            ("3:0:IBM", 0, 2),
            ("3:0:AAPL", MINUTE, 1),
            ("3:0:IBM", MINUTE, 1)
        ]
    );
    assert_eq!(candles[1].bid, Some(ohlc("100", "101", "100", "101")));
    assert_eq!(candles[1].ticker.as_deref(), Some("IBM"));
}

#[test]
fn a_book_stating_no_ticker_states_none_on_its_candle() {
    let candles = empty_candles(&[10 * SECOND], CandleOptions::new(MINUTE).unwrap());
    assert_eq!(candles.len(), 1);
    assert_eq!(candles[0].ticker.as_deref(), Some("ACME"));
    let untickered = vec![Ok(BookEvent::new(10 * SECOND, ""))];
    let candles = CandleIterator::new(untickered.into_iter(), CandleOptions::new(MINUTE).unwrap())
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(candles[0].ticker, None);
    assert_eq!(
        candles[0].crosscode, "3:0:XX0000000000",
        "an empty symbol keys the book by the number that states none"
    );
}

#[test]
fn buckets_align_to_a_zone_with_a_half_hour_offset() {
    // Asia/Kolkata is +05:30: `10:45Z` reads `16:15`, whose hour opens at
    // `16:00` local, `10:30Z`.
    let kolkata = Timezone::from_str("Asia/Kolkata").unwrap();
    let options = CandleOptions::from_spelling("1h")
        .unwrap()
        .with_timezone(kolkata);
    let candles = empty_candles(
        &[OFFSET_DAY + 45 * MINUTE, OFFSET_DAY + 89 * MINUTE],
        options,
    );
    assert_eq!(
        candles
            .iter()
            .map(|candle| (candle.start, candle.end, candle.books))
            .collect::<Vec<_>>(),
        [(OFFSET_DAY + 30 * MINUTE, OFFSET_DAY + 90 * MINUTE, 2)]
    );
    // The same instants in UTC open on the UTC hour.
    let utc = empty_candles(
        &[OFFSET_DAY + 45 * MINUTE, OFFSET_DAY + 89 * MINUTE],
        CandleOptions::from_spelling("1h").unwrap(),
    );
    assert_eq!(
        utc.iter()
            .map(|candle| (candle.start, candle.end, candle.books))
            .collect::<Vec<_>>(),
        [
            (OFFSET_DAY, OFFSET_DAY + HOUR, 1),
            (OFFSET_DAY + HOUR, OFFSET_DAY + 2 * HOUR, 1)
        ]
    );
}

#[test]
fn hourly_candles_skip_the_hour_a_spring_forward_removes() {
    // Europe/Zurich, 2026-03-29: `00:30Z` reads `01:30 CET`, `01:30Z`
    // reads `03:30 CEST` - the wall clock never reads `02:xx`.
    let options = CandleOptions::from_spelling("1h")
        .unwrap()
        .with_timezone(zurich());
    let candles = empty_candles(
        &[
            SPRING_DAY + 30 * MINUTE,
            SPRING_DAY + 90 * MINUTE,
            SPRING_DAY + 150 * MINUTE,
        ],
        options,
    );
    assert_eq!(
        candles
            .iter()
            .map(|candle| (candle.start, candle.end))
            .collect::<Vec<_>>(),
        [
            (SPRING_DAY, SPRING_DAY + HOUR),
            (SPRING_DAY + HOUR, SPRING_DAY + 2 * HOUR),
            (SPRING_DAY + 2 * HOUR, SPRING_DAY + 3 * HOUR),
        ],
        "the buckets abut in UTC"
    );
    assert_eq!(
        candles
            .iter()
            .map(|candle| local_hour(zurich(), candle.start))
            .collect::<Vec<_>>(),
        [1, 3, 4],
        "no candle opens at the hour the zone skipped"
    );
}

#[test]
fn a_daily_candle_spans_twenty_three_hours_on_a_spring_forward_day() {
    let options = CandleOptions::from_spelling("1d")
        .unwrap()
        .with_timezone(zurich());
    let candles = empty_candles(&[SPRING_DAY + 30 * MINUTE, SPRING_DAY + 20 * HOUR], options);
    assert_eq!(candles.len(), 1);
    let candle = &candles[0];
    // Local midnight is `23:00Z` the day before; the next is `22:00Z`.
    assert_eq!(candle.start, SPRING_DAY - HOUR);
    assert_eq!(candle.end, SPRING_DAY + 22 * HOUR);
    assert_eq!(candle.end - candle.start, 23 * HOUR);
    assert_eq!(local_hour(zurich(), candle.start), 0);
    assert_eq!(local_hour(zurich(), candle.end), 0);
    assert_eq!(candle.books, 2);
}

#[test]
fn a_fall_back_folds_the_repeated_hour_into_one_rising_bucket() {
    // Europe/Zurich, 2026-10-25: `00:30Z` reads `02:30 CEST` and `01:30Z`
    // reads `02:30 CET`; both are the local hour `02`, one two-hour bucket.
    let hourly = CandleOptions::from_spelling("1h")
        .unwrap()
        .with_timezone(zurich());
    let candles = empty_candles(&[FALL_DAY + 30 * MINUTE, FALL_DAY + 90 * MINUTE], hourly);
    assert_eq!(
        candles
            .iter()
            .map(|candle| (candle.start, candle.end, candle.books))
            .collect::<Vec<_>>(),
        [(FALL_DAY, FALL_DAY + 2 * HOUR, 2)]
    );
    // Half-hour buckets: `02:00` opens once, at `00:00Z`, and `02:30` holds
    // every instant from `00:30Z` until the wall clock first reads `03:00`,
    // at `02:00Z`, so the edges rise and no bucket is re-entered.
    let halves = CandleOptions::from_spelling("30m")
        .unwrap()
        .with_timezone(zurich());
    let candles = empty_candles(
        &[
            FALL_DAY + 15 * MINUTE,
            FALL_DAY + 45 * MINUTE,
            FALL_DAY + 75 * MINUTE,
            FALL_DAY + 125 * MINUTE,
        ],
        halves,
    );
    assert_eq!(
        candles
            .iter()
            .map(|candle| (candle.start, candle.end, candle.books))
            .collect::<Vec<_>>(),
        [
            (FALL_DAY, FALL_DAY + 30 * MINUTE, 1),
            (FALL_DAY + 30 * MINUTE, FALL_DAY + 2 * HOUR, 2),
            (FALL_DAY + 2 * HOUR, FALL_DAY + 150 * MINUTE, 1),
        ]
    );
}

#[test]
fn a_sub_millisecond_interval_finds_its_bucket_in_a_fall_back_without_stepping_through_it() {
    // Europe/Zurich, 2026-10-25: `01:10Z` reads `02:10 CET`, the second
    // pass of the hour the fall-back repeats. Its bucket is the one holding
    // the latest reading the first pass made, the interval before `03:00`,
    // which it keeps until the wall clock first reads `03:00` at `02:00Z` -
    // found by searching the edges, never by stepping an interval at a time
    // through the fifty minutes of repeated readings, which at a nanosecond
    // is 3e12 steps.
    let (sent, received) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let answers = [1, 1_000, 1_000_000, SECOND].map(|interval| {
            let options = CandleOptions::new(interval)
                .unwrap()
                .with_timezone(zurich());
            let candles = empty_candles(&[FALL_DAY + 70 * MINUTE], options);
            (interval, candles[0].start, candles[0].end)
        });
        sent.send(answers).unwrap();
    });
    let answers = received
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the bucket of a repeated reading is found in a bounded search");
    for (interval, start, end) in answers {
        assert_eq!(
            (start, end),
            (FALL_DAY + HOUR - interval, FALL_DAY + 2 * HOUR),
            "{interval}ns"
        );
    }
}

#[test]
fn a_refused_book_follows_the_candles_of_the_bucket_it_completed() {
    // The third book's bucket ends past `i64` nanoseconds: the bucket the
    // stream moved past is emitted, then the refusal, then nothing.
    let folded = || {
        let stream = vec![
            Ok(BookEvent::new(10 * SECOND, "ACME")),
            Ok(BookEvent::new(20 * SECOND, "ACME")),
            Ok(BookEvent::new(i64::MAX, "ACME")),
        ];
        CandleIterator::new(stream.into_iter(), CandleOptions::new(MINUTE).unwrap())
    };
    let mut walk = folded();
    let first = walk.next().unwrap().unwrap();
    assert_eq!((first.start, first.end, first.books), (0, MINUTE, 2));
    let error = walk.next().unwrap().unwrap_err().to_string();
    assert!(
        error.starts_with("invalid record value at $.book.currunix:"),
        "{error}"
    );
    assert!(walk.next().is_none(), "the walk fuses");
    // A reader over the walk lays the completed bucket out before it fails.
    let mut batches = Candle::arrow_reader(folded(), None).unwrap();
    assert_eq!(batches.next().unwrap().unwrap().num_rows(), 1);
    assert!(batches.next().unwrap().is_err());
    assert!(batches.next().is_none());
}

#[test]
fn the_field_declares_every_cell() {
    let field = Candle::field().unwrap();
    assert_eq!(field.name(), "candle");
    assert!(!field.is_nullable());
    let declared = field
        .fields()
        .iter()
        .map(|child| {
            format!(
                "{}: {}{}",
                child.name(),
                child.dtype(),
                if child.is_nullable() { "" } else { " not null" }
            )
        })
        .collect::<Vec<_>>();
    let mut expected = vec![
        "crosscode: utf8 not null".to_owned(),
        "ticker: utf8".to_owned(),
        "start: datetime64(ns,\"UTC\") not null".to_owned(),
        "end: datetime64(ns,\"UTC\") not null".to_owned(),
    ];
    for reading in ["bid", "ask", "mid", "spread"] {
        for cell in ["open", "high", "low", "close"] {
            expected.push(format!("{reading}{cell}: decimal"));
        }
    }
    expected.extend([
        "bidqty: decimal".to_owned(),
        "askqty: decimal".to_owned(),
        "books: uint64 not null".to_owned(),
    ]);
    assert_eq!(declared, expected);
}

/// A candle stating every cell.
fn full_candle() -> Candle {
    Candle {
        crosscode: "3:0:ACME".into(),
        ticker: Some("ACME".into()),
        start: MINUTE,
        end: 2 * MINUTE,
        bid: Some(ohlc("100", "102", "99", "101")),
        ask: Some(ohlc("103", "103.5", "102.5", "103.5")),
        mid: Some(ohlc("101.5", "102.5", "100.75", "102.25")),
        spread: Some(ohlc("3", "3.5", "1", "2.5")),
        bidqty: Some(Decimal::from_int(8)),
        askqty: Some(Decimal::from_int(9)),
        books: 4,
    }
}

/// A candle of a book that stated nothing.
fn empty_candle() -> Candle {
    Candle {
        crosscode: "3:0:XX0000000000".into(),
        ticker: None,
        start: 0,
        end: MINUTE,
        bid: None,
        ask: None,
        mid: None,
        spread: None,
        bidqty: None,
        askqty: None,
        books: 1,
    }
}

#[test]
fn the_scalar_round_trips_as_the_named_struct_and_the_canonical_row() {
    for candle in [full_candle(), empty_candle()] {
        let named = candle.into_scalar();
        assert_eq!(named.as_struct().unwrap().len(), 23);
        assert_eq!(Candle::from_scalar(&named).unwrap(), candle);
        let row = Candle::field().unwrap().scalar(named).unwrap();
        assert_eq!(row.sequence_rows().unwrap().len(), 23);
        assert_eq!(Candle::from_scalar(&row).unwrap(), candle);
    }
    // The named struct restates what the door restates: a text price.
    let named = Scalar::from_struct([
        ("crosscode", Scalar::from("3:0:ACME")),
        ("start", Scalar::from(0i64)),
        ("end", Scalar::from(MINUTE)),
        ("bidopen", Scalar::from("100.5")),
        ("bidhigh", Scalar::from("100.5")),
        ("bidlow", Scalar::from("100.5")),
        ("bidclose", Scalar::from("100.5")),
        ("books", Scalar::from(1u8)),
    ])
    .unwrap();
    let read = Candle::from_scalar(&named).unwrap();
    assert_eq!(read.bid, Some(ohlc("100.5", "100.5", "100.5", "100.5")));
    assert_eq!(read.ticker, None);
    assert_eq!(read.ask, None);
    assert_eq!((read.start, read.end, read.books), (0, MINUTE, 1));
}

#[test]
fn a_scalar_of_another_shape_is_refused_under_the_candle() {
    let refused = |value: Scalar| Candle::from_scalar(&value).unwrap_err().to_string();
    assert!(refused(Scalar::from(1i64)).contains("$.candle"));
    assert!(refused(Scalar::Null).contains("$.candle"));
    // A required cell absent is a null the door refuses.
    let mut named = full_candle().into_scalar().as_struct().unwrap().clone();
    named.remove("crosscode");
    let error = refused(Scalar::from_struct(named.clone()).unwrap());
    assert!(error.contains("crosscode"), "{error}");
    // A reading stating some of its four cells is refused by this reading.
    let mut partial = full_candle().into_scalar().as_struct().unwrap().clone();
    partial.insert("bidhigh".into(), Scalar::Null);
    let error = refused(Scalar::from_struct(partial).unwrap());
    assert!(
        error.contains("$.candle") && error.contains("four decimals or four nulls"),
        "{error}"
    );
    // A name the struct should not hold.
    let mut extra = full_candle().into_scalar().as_struct().unwrap().clone();
    extra.insert("vwap".into(), Scalar::Null);
    let error = refused(Scalar::from_struct(extra).unwrap());
    assert!(error.contains("vwap"), "{error}");
}

#[test]
fn candles_lay_out_as_batches_and_read_back() {
    let candles = vec![full_candle(), empty_candle(), full_candle()];
    let mut batches = Candle::arrow_reader(candles.clone().into_iter().map(Ok), Some(2)).unwrap();
    let field = Candle::field().unwrap();
    let schema = field.clone().into_arrow_schema().unwrap();
    assert_eq!(batches.schema(), schema);
    let mut read = Vec::new();
    for batch in &mut batches {
        let batch = batch.unwrap();
        let rows =
            Serie::from_arrow_batch(Some(&field), &batch, ArrowCastOptions::default()).unwrap();
        for index in 0..rows.len() {
            read.push(Candle::from_scalar(&rows.scalar(index).unwrap()).unwrap());
        }
    }
    assert_eq!(read, candles);
    // An empty source is an empty reader under the same schema.
    let mut batches = Candle::arrow_reader(std::iter::empty(), None).unwrap();
    assert_eq!(batches.schema(), schema);
    assert!(batches.next().is_none());
    // A source error follows the completed prefix and fuses the reader.
    let source = vec![
        Ok(full_candle()),
        Err(yggdryl::Error::InvalidRecord {
            path: "$.source".into(),
            reason: "the source failed".into(),
        }),
        Ok(empty_candle()),
    ];
    let mut batches = Candle::arrow_reader(source, Some(1)).unwrap();
    assert_eq!(batches.next().unwrap().unwrap().num_rows(), 1);
    assert!(batches.next().unwrap().is_err());
    assert!(batches.next().is_none());
}

#[test]
fn candles_of_a_walk_round_trip_through_arrow() {
    let operations = vec![
        quote(10 * SECOND, "ACME", "B", "Buy", "100", 5),
        quote(10 * SECOND, "ACME", "A", "Sell", "103", 7),
        quote(70 * SECOND, "ACME", "B", "Buy", "101", 5),
    ];
    let options = CandleOptions::from_spelling("1m").unwrap();
    let walk = CandleIterator::new(
        books(operations.clone()).into_iter().map(Ok),
        options.clone(),
    );
    let mut batches = Candle::arrow_reader(walk, None).unwrap();
    let batch = batches.next().unwrap().unwrap();
    assert!(batches.next().is_none());
    assert_eq!(batch.num_rows(), 2);
    let rows = Serie::from_arrow_batch(
        Some(&Candle::field().unwrap()),
        &batch,
        ArrowCastOptions::default(),
    )
    .unwrap();
    let read = (0..rows.len())
        .map(|index| Candle::from_scalar(&rows.scalar(index).unwrap()).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(read, candles(operations, options));
    assert_eq!(read[0].ticker.as_deref(), Some("ACME"));
}

/// A candle reads a book's top of book, which every book states whether it
/// holds its sides or its deltas alone: the candles of a walk's books -
/// each its deltas alone, the first following no book - are the candles of
/// those books rebuilt whole, with no rebuild.
#[test]
fn candles_from_delta_books_equal_candles_from_complete_books() {
    let operations = vec![
        quote(OFFSET_DAY + SECOND, "ACME", "B-1", "Buy", "100", 10),
        quote(OFFSET_DAY + SECOND, "ACME", "A-1", "Sell", "102", 5),
        quote(OFFSET_DAY + 20 * SECOND, "ACME", "B-2", "Buy", "101", 4),
        quote(OFFSET_DAY + 70 * SECOND, "ACME", "A-1", "Sell", "103", 5),
        quote(OFFSET_DAY + 90 * SECOND, "ACME", "B-1", "Buy", "99", 1),
        quote(OFFSET_DAY + 130 * SECOND, "ACME", "A-2", "Sell", "101.5", 2),
    ];
    let books = books(operations);
    assert!(books.iter().all(|book| !book.is_complete()));
    let mut whole: Vec<BookEvent> = Vec::with_capacity(books.len());
    for book in &books {
        let origin = BookEvent::new(book.get_currunix(), book.get_crosscode());
        let previous = whole.last().unwrap_or(&origin);
        whole.push(book.clone().with_previous(previous).unwrap());
    }
    assert!(whole.iter().all(BookEvent::is_complete));
    let fold = |books: Vec<BookEvent>| {
        CandleIterator::new(
            books.into_iter().map(Ok),
            CandleOptions::new(MINUTE).unwrap(),
        )
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap()
    };
    let (deltas, rebuilt) = (fold(books), fold(whole));
    assert_eq!(deltas.len(), 3);
    assert_eq!(deltas, rebuilt);
}
