use std::hint::black_box;

use criterion::{BatchSize, Criterion, Throughput};
use smol_str::SmolStr;
use yggdryl::graph::{
    BookEvent, Candle, CandleIterator, CandleOptions, Element, Event, ExecutionEvent, Market,
    MarketData, Ohlc, Operation, QuoteEvent,
};
use yggdryl::{Decimal, IdSource, IdType, Identifier, Side, State, Timezone};

/// Nanoseconds in one second.
const SECOND: i64 = 1_000_000_000;

/// `2026-10-25T00:00:00Z`, the day Europe/Zurich falls back at `01:00Z`.
const FALL_DAY: i64 = 1_792_886_400 * SECOND;

/// One resting quote of `side` at `price`, at `unix`.
fn entry(unix: i64, side: &str, price: i64) -> MarketData {
    let name = if side == "Buy" { "B" } else { "A" };
    let mut event = QuoteEvent::at(unix);
    event.set_crosscode(name.to_owned());
    event.set_ticker(Some(SmolStr::new("BENCH")), true);
    event.set_side(Side::read(side).expect("a shipped side"), true);
    event.set_price(Some(Decimal::from_int(price)), true);
    event.set_quantity(Some(Decimal::from_int(10)), true);
    event.set_state(State::read("New").expect("the shipped new state"));
    event.finalize();
    MarketData::from(event)
}

/// `count` two-sided books of one ticker, one every hundred milliseconds,
/// the touch moving one tick each way per book.
fn books(count: usize) -> Vec<BookEvent> {
    (0..count)
        .map(|index| {
            let unix = i64::try_from(index).expect("a bench corpus") * (SECOND / 10);
            let tick = i64::try_from(index % 7).expect("a bench corpus");
            let mut book = BookEvent::new(unix, "BENCH");
            book.add_operations([
                entry(unix, "Buy", 100_000 - tick),
                entry(unix, "Sell", 100_001 + tick),
            ])
            .expect("two entries");
            book
        })
        .collect()
}

/// One fill of 10 at `unix` whose `EXECID` is `index`'s, spelled past
/// `SmolStr`'s inline width as a venue's long identifiers are.
fn fill(unix: i64, index: usize) -> MarketData {
    let mut event = ExecutionEvent::at(unix);
    event.set_crosscode(format!("F-{index}"));
    event.set_ticker(Some(SmolStr::new("BENCH")), true);
    event.set_side(Side::read("Buy").expect("a shipped side"), true);
    event.set_lastqty(Some(Decimal::from_int(10)), true);
    event.set_state(State::read("Filled").expect("the shipped filled state"));
    event
        .insert_identifier(
            Identifier::new(
                IdSource::Fix,
                IdType::ExecId,
                &format!("BENCH-EXECUTION-IDENTIFIER-{index:08}"),
            )
            .expect("an identifier"),
        )
        .expect("an identifier the map holds");
    event.finalize();
    MarketData::from(event)
}

/// [`books`] each carrying two fills: one of its own and the one before it
/// delivered again, so every other statement is of a trade already counted.
fn traded_books(count: usize) -> Vec<BookEvent> {
    books(count)
        .into_iter()
        .enumerate()
        .map(|(index, mut book)| {
            let unix = book.get_currunix();
            book.add_operations([fill(unix, index), fill(unix, index.saturating_sub(1))])
                .expect("two executions");
            book
        })
        .collect()
}

/// One candle per book, as `arrow_reader` lays them out.
fn candles(count: usize) -> Vec<Candle> {
    (0..count)
        .map(|index| {
            let start = i64::try_from(index).expect("a bench corpus") * SECOND;
            Candle {
                crosscode: SmolStr::new_static("3:0:BENCH"),
                ticker: Some(SmolStr::new_static("BENCH")),
                start,
                end: start + SECOND,
                bid: Some(Ohlc::at(Decimal::from_int(100_000))),
                ask: Some(Ohlc::at(Decimal::from_int(100_001))),
                mid: Some(Ohlc::at("100000.5".parse().expect("a decimal"))),
                spread: Some(Ohlc::at(Decimal::ONE)),
                bidqty: Some(Decimal::from_int(10)),
                askqty: Some(Decimal::from_int(10)),
                books: 1,
                executions: 0,
                volume: Decimal::ZERO,
            }
        })
        .collect()
}

pub fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("graph/candle");
    let count = crate::bench_profile::corpus(10_000, 64);
    let books = books(count);
    let zones = [
        ("utc", Timezone::UTC),
        (
            "zurich",
            Timezone::from_str("Europe/Zurich").expect("a shipped zone"),
        ),
    ];
    group.throughput(Throughput::Elements(u64::try_from(count).unwrap()));
    for (name, zone) in zones {
        let options = CandleOptions::from_spelling("1m")
            .expect("a shipped spelling")
            .with_timezone(zone);
        // The books' clone is outside the timer; the walk consumes them and
        // hands the candles back rather than dropping them inside it.
        group.bench_function(format!("fold_{name}_{count}"), |bencher| {
            bencher.iter_batched(
                || books.clone(),
                |books| {
                    CandleIterator::new(books.into_iter().map(Ok), options.clone())
                        .map(|candle| candle.expect("a sorted stream"))
                        .collect::<Vec<_>>()
                },
                BatchSize::LargeInput,
            );
        });
    }
    // Two executions a book, one a trade of its own and one a statement
    // of the trade before: what counting each trade once costs the fold.
    let traded = traded_books(count);
    let minutes = CandleOptions::from_spelling("1m").expect("a shipped spelling");
    group.bench_function(format!("fold_executions_{count}"), |bencher| {
        bencher.iter_batched(
            || traded.clone(),
            |books| {
                CandleIterator::new(books.into_iter().map(Ok), minutes.clone())
                    .map(|candle| candle.expect("a sorted stream"))
                    .collect::<Vec<_>>()
            },
            BatchSize::LargeInput,
        );
    });
    // One candle per book: every bucket edge computed once per candle.
    let seconds = CandleOptions::from_spelling("1s").expect("a shipped spelling");
    group.bench_function(format!("fold_per_book_{count}"), |bencher| {
        bencher.iter_batched(
            || books.clone(),
            |books| {
                CandleIterator::new(books.into_iter().map(Ok), seconds.clone())
                    .map(|candle| candle.expect("a sorted stream"))
                    .collect::<Vec<_>>()
            },
            BatchSize::LargeInput,
        );
    });
    // Millisecond buckets through both passes of the hour a fall-back
    // repeats: the second pass's first book searches its bucket out of an
    // hour of repeated readings, which costs a logarithm of them.
    let fall_back: Vec<BookEvent> = (0..count)
        .map(|index| {
            let step = 2 * 3_600 * SECOND / i64::try_from(count).expect("a bench corpus");
            BookEvent::new(
                FALL_DAY + i64::try_from(index).expect("a bench corpus") * step,
                "BENCH",
            )
        })
        .collect();
    let milliseconds = CandleOptions::from_spelling("1ms")
        .expect("a shipped spelling")
        .with_timezone(Timezone::from_str("Europe/Zurich").expect("a shipped zone"));
    group.bench_function(format!("fold_fall_back_1ms_{count}"), |bencher| {
        bencher.iter_batched(
            || fall_back.clone(),
            |books| {
                CandleIterator::new(books.into_iter().map(Ok), milliseconds.clone())
                    .map(|candle| candle.expect("a sorted stream"))
                    .collect::<Vec<_>>()
            },
            BatchSize::LargeInput,
        );
    });
    let candles = candles(count);
    group.bench_function(format!("arrow_reader_{count}"), |bencher| {
        bencher.iter_batched(
            || candles.clone(),
            |candles| {
                Candle::arrow_reader(candles.into_iter().map(Ok), None)
                    .expect("the candle field")
                    .map(|batch| black_box(batch.expect("a laid-out batch")).num_rows())
                    .sum::<usize>()
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();
}
