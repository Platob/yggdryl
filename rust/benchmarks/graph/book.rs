use std::hint::black_box;

use criterion::{BatchSize, Criterion, Throughput};
use smol_str::SmolStr;
use yggdryl::graph::{BookEvent, BookIterator, Element, Event, Market, MarketData, QuoteEvent};
use yggdryl::{Decimal, Side, State};

/// One resting quote of `side` at `level` prices from the touch, at `unix`.
fn entry(unix: i64, side: &str, level: usize, quantity: i64) -> MarketData {
    let offset = i64::try_from(level).expect("a bench corpus");
    let (name, price) = if side == "Buy" {
        ("B", 100_000 - offset)
    } else {
        ("A", 100_001 + offset)
    };
    let mut event = QuoteEvent::at(unix);
    event.set_crosscode(format!("{name}-{level}"));
    event.set_ticker(Some(SmolStr::new("BENCH")), true);
    event.set_side(Side::read(side).expect("a shipped side"), true);
    event.set_price(Some(Decimal::from_int(price)), true);
    event.set_quantity(Some(Decimal::from_int(quantity)), true);
    event.set_state(State::read("New").expect("the shipped new state"));
    event.finalize();
    MarketData::from(event)
}

/// The entries of a book of `levels` distinct prices on each side, one
/// entry at each, all at the instant `1`.
fn entries(levels: usize) -> Vec<MarketData> {
    ["Buy", "Sell"]
        .into_iter()
        .flat_map(|side| (0..levels).map(move |level| entry(1, side, level, 1)))
        .collect()
}

/// A walk's inputs: `inputs` quotes, sixteen to an instant, alternating
/// sides over 64 levels a side, so each side settles at 64 entries that
/// later inputs replace in turn.
fn stream(inputs: usize) -> Vec<MarketData> {
    (0..inputs)
        .map(|at| {
            let unix = i64::try_from(at / 16 + 1).expect("a bench corpus");
            let side = if at % 2 == 0 { "Buy" } else { "Sell" };
            let quantity = i64::try_from(at % 7 + 1).expect("a bench corpus");
            entry(unix, side, (at / 2) % 64, quantity)
        })
        .collect()
}

/// The quote resting at the touch of `side` in slot `slot`, at `unix`: one
/// of many at one price, the deep level a step finds its entry in.
fn at_touch(unix: i64, side: &str, slot: usize, quantity: i64) -> MarketData {
    let mut quote = entry(unix, side, 0, quantity);
    quote.set_crosscode(format!(
        "{}-0-{slot}",
        if side == "Buy" { "B" } else { "A" }
    ));
    quote.finalize();
    quote
}

/// A book of [`entries`].
fn book(levels: usize) -> BookEvent {
    let mut book = BookEvent::new(1, "BENCH");
    book.add_operations(entries(levels))
        .expect("the bench depth");
    book
}

pub fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("graph/book");
    for levels in [128, crate::bench_profile::corpus(1_024, 16)] {
        let book = book(levels);
        let bid = Side::read("Buy").expect("the shipped buy side");
        group.throughput(Throughput::Elements(u64::try_from(levels).unwrap()));
        // Every limit built and dropped: one vector of identities each.
        group.bench_function(format!("limits_{levels}"), |bencher| {
            bencher.iter(|| black_box(&book).limits(bid).map(black_box).count());
        });
        // One side's entries, borrowed from its store.
        group.bench_function(format!("alive_on_{levels}"), |bencher| {
            bencher.iter(|| black_box(&book).alive_on(bid).map(black_box).count());
        });
        group.bench_function(format!("depth_{levels}"), |bencher| {
            bencher.iter(|| black_box(&book).depth(bid, black_box(levels)));
        });
        group.bench_function(format!("imbalance_{levels}"), |bencher| {
            bencher.iter(|| black_box(&book).imbalance(black_box(levels)));
        });
        // The book's entries by kind, borrowed over both sides: the quotes
        // among its delta, every entry, and the resting orders, none - the filter's
        // own cost over every live entry.
        group.throughput(Throughput::Elements(u64::try_from(2 * levels).unwrap()));
        group.bench_function(format!("quotes_{levels}"), |bencher| {
            bencher.iter(|| black_box(&book).quotes().map(black_box).count());
        });
        group.bench_function(format!("ordlive_{levels}"), |bencher| {
            bencher.iter(|| black_box(&book).ordlive().map(black_box).count());
        });
        group.throughput(Throughput::Elements(1));
        group.bench_function(format!("spread_{levels}"), |bencher| {
            bencher.iter(|| black_box(&book).spread());
        });
        group.bench_function(format!("best_price_{levels}"), |bencher| {
            bencher.iter(|| black_box(&book).best_price(bid));
        });
        // One replacement of the entry at the touch; the book's clone is
        // outside the timer and handed back rather than dropped inside it.
        let update = entry(1, "Buy", 0, 2);
        group.bench_function(format!("add_operation_{levels}"), |bencher| {
            bencher.iter_batched(
                || (book.clone(), update.clone()),
                |(mut book, update)| {
                    book.add_operations([update]).expect("one replacement");
                    book
                },
                BatchSize::LargeInput,
            );
        });
        // One step of a walk: the replacement at the touch applied at the
        // next instant and its book emitted - its delta alone, the step
        // being no tick - the book before it dropped as a streaming
        // consumer drops it, so the side's store is changed in place. The
        // walk up to the step is outside the timer, and the walk and the
        // book are handed back rather than dropped inside it.
        let (depth, step) = (entries(levels), entry(2, "Buy", 0, 2));
        group.bench_function(format!("walk_step_{levels}"), |bencher| {
            bencher.iter_batched(
                || {
                    let source = depth.clone().into_iter().chain([step.clone()]);
                    let mut books = BookIterator::new(source, 0).expect("a walk");
                    drop(books.next().expect("the first book").expect("the depth"));
                    books
                },
                |mut books| {
                    let book = books
                        .next()
                        .expect("the step's book")
                        .expect("a replacement");
                    (books, book)
                },
                BatchSize::LargeInput,
            );
        });
    }
    for levels in [8, crate::bench_profile::corpus(1_024, 16)] {
        let depth = entries(levels);
        let steps = [entry(2, "Buy", 0, 2), entry(3, "Buy", 0, 3)];
        group.throughput(Throughput::Elements(1));
        // One step of a walk between ticks while the consumer holds every
        // book it emitted, as a collect does: the walk's first book and the
        // delta after it held, the next delta's step timed. A delta book
        // holds no side, so the walk changes the
        // side it alone holds in place at either depth.
        group.bench_function(format!("walk_step_delta_{levels}"), |bencher| {
            bencher.iter_batched(
                || {
                    let source = depth.clone().into_iter().chain(steps.clone());
                    let mut books = BookIterator::new(source, 0).expect("a walk");
                    let held = [
                        books.next().expect("the first book").expect("the depth"),
                        books
                            .next()
                            .expect("the first delta")
                            .expect("a replacement"),
                    ];
                    (books, held)
                },
                |(mut books, held)| {
                    let book = books
                        .next()
                        .expect("the step's book")
                        .expect("a replacement");
                    (books, held, book)
                },
                BatchSize::LargeInput,
            );
        });
        // A delta book rebuilt over the complete book before
        // it: the delta replayed over that book's sides, which the rebuild
        // copies once - the book before it is borrowed and kept. The walk's
        // first book follows no book, so it is whole over the empty one.
        let books = BookIterator::new(depth.into_iter().chain(steps), 0)
            .expect("a walk")
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("the walk's books");
        let origin = BookEvent::new(books[0].get_currunix(), books[0].get_crosscode());
        let first = books[0]
            .clone()
            .with_previous(&origin)
            .expect("the first book rebuilt");
        let previous = books[1]
            .clone()
            .with_previous(&first)
            .expect("the first delta rebuilt");
        group.bench_function(format!("rebuild_{levels}"), |bencher| {
            bencher.iter_batched(
                || books[2].clone(),
                |delta| delta.with_previous(&previous).expect("a rebuild"),
                BatchSize::LargeInput,
            );
        });
    }
    // A whole walk drained, then the same walk under a filter keeping the
    // bids: the filtered walk pulls 1,024 inputs ahead, lays them out as
    // one batch and folds what the expression engine keeps.
    let inputs = crate::bench_profile::corpus(4_096, 64);
    let source = stream(inputs);
    group.throughput(Throughput::Elements(u64::try_from(inputs).unwrap()));
    for (name, filter) in [("drain", None), ("drain_filtered", Some("side = 'BUYS'"))] {
        group.bench_function(format!("{name}_{inputs}"), |bencher| {
            bencher.iter_batched(
                || source.clone(),
                |source| {
                    let mut books = BookIterator::new(source.into_iter(), 0).expect("a walk");
                    if let Some(filter) = filter {
                        books = books.with_filter(filter).expect("a filter over the row");
                    }
                    books
                        .map(|book| {
                            let book = book.expect("a book");
                            book.delta().len() + book.events().len()
                        })
                        .sum::<usize>()
                },
                BatchSize::LargeInput,
            );
        });
    }
    // One step of a walk at a touch `entries` deep on each side: the level
    // the step changes is read off what it keeps, never summed.
    for entries in [8, crate::bench_profile::corpus(1_024, 16)] {
        let depth: Vec<MarketData> = ["Buy", "Sell"]
            .into_iter()
            .flat_map(|side| (0..entries).map(move |slot| at_touch(1, side, slot, 1)))
            .collect();
        let step = at_touch(2, "Buy", entries / 2, 2);
        group.throughput(Throughput::Elements(1));
        group.bench_function(format!("walk_step_deep_touch_{entries}"), |bencher| {
            bencher.iter_batched(
                || {
                    let mut books =
                        BookIterator::new(depth.clone().into_iter().chain([step.clone()]), 0)
                            .expect("a walk");
                    drop(books.next().expect("the first book").expect("the depth"));
                    books
                },
                |mut books| {
                    let book = books
                        .next()
                        .expect("the step's book")
                        .expect("a replacement");
                    (books, book)
                },
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}
