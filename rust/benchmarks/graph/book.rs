use std::hint::black_box;

use criterion::{BatchSize, Criterion, Throughput};
use smol_str::SmolStr;
use yggdryl::graph::{BookEvent, Element, Event, Market, MarketData, QuoteEvent};
use yggdryl::{Decimal, Side, State};

/// One resting quote of `side` at `level` prices from the touch.
fn entry(side: &str, level: usize, quantity: i64) -> MarketData {
    let offset = i64::try_from(level).expect("a bench corpus");
    let (name, price) = if side == "Buy" {
        ("B", 100_000 - offset)
    } else {
        ("A", 100_001 + offset)
    };
    let mut event = QuoteEvent::at(1);
    event.set_crosscode(format!("{name}-{level}"));
    event.set_ticker(Some(SmolStr::new("BENCH")));
    event.set_side(Side::read(side).expect("a shipped side"));
    event.set_price(Some(Decimal::from_int(price)));
    event.set_quantity(Some(Decimal::from_int(quantity)));
    event.set_state(State::read("New").expect("the shipped new state"));
    event.finalize();
    MarketData::from(event)
}

/// A book of `levels` distinct prices on each side, one entry at each.
fn book(levels: usize) -> BookEvent {
    let mut book = BookEvent::new(1, "BENCH");
    book.add_operations(
        ["Buy", "Sell"]
            .into_iter()
            .flat_map(|side| (0..levels).map(move |level| entry(side, level, 1))),
    )
    .expect("the bench depth");
    book
}

pub fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("graph/book");
    for levels in [128, crate::bench_profile::corpus(1_024, 16)] {
        let book = book(levels);
        let side = book.bid();
        group.throughput(Throughput::Elements(u64::try_from(levels).unwrap()));
        // Every limit built and dropped: one vector of identities each.
        group.bench_function(format!("limits_{levels}"), |bencher| {
            bencher.iter(|| black_box(side).limits().map(black_box).count());
        });
        group.bench_function(format!("depth_{levels}"), |bencher| {
            bencher.iter(|| black_box(side).depth(black_box(levels)));
        });
        group.bench_function(format!("imbalance_{levels}"), |bencher| {
            bencher.iter(|| black_box(&book).imbalance(black_box(levels)));
        });
        group.throughput(Throughput::Elements(1));
        group.bench_function(format!("spread_{levels}"), |bencher| {
            bencher.iter(|| black_box(&book).spread());
        });
        group.bench_function(format!("best_price_{levels}"), |bencher| {
            bencher.iter(|| black_box(side).best_price());
        });
        // One replacement of the entry at the touch; the side's clone is
        // outside the timer and handed back rather than dropped inside it.
        let update = entry("Buy", 0, 2);
        group.bench_function(format!("add_operation_{levels}"), |bencher| {
            bencher.iter_batched(
                || (side.clone(), update.clone()),
                |(mut side, update)| {
                    side.add_operation(update).expect("one replacement");
                    side
                },
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}
