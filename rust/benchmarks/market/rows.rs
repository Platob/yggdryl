//! What a product's row costs to write and to read back, against the
//! sixteen event columns alone.
//!
//! The reference is `baseline_event_cells`: the sixteen facts every event
//! row opens with, read through `EventColumn::fact` off the same product
//! and wrapped as a row. A product's `into_row` is those sixteen plus its
//! own columns, so its time less the baseline's is what its market facts
//! and its own columns cost - a book's two fixed-size ladders, a trade's
//! parties, a code cell per identifier - and `from_row` is the same
//! reading back through the value contract of the row's field. The
//! products are read out of the bridge's own capture, so each is what a
//! door answers rather than a value built by hand.

use std::hint::black_box;

use criterion::{BatchSize, Criterion};
use yggdryl::graph::EventColumn;
use yggdryl::market::{BookData, ExecutionData, OrderData, Product, QuoteData, TradeData};
use yggdryl::{Field, Scalar};

use super::{codec, messages};

/// One product's row timed both ways beside the sixteen alone.
fn timed<P: Product>(criterion: &mut Criterion, name: &str, field: &Field, product: &P) {
    let row = product.into_row().expect("a row");
    assert_eq!(
        P::from_row(field, &row)
            .expect("the row reads")
            .into_row()
            .expect("a row"),
        row,
        "the row round-trips"
    );
    let mut group = criterion.benchmark_group(format!("market/rows/{name}"));
    group.bench_function("baseline_event_cells", |bencher| {
        bencher.iter(|| {
            black_box(Scalar::from_sequence(EventColumn::ALL.into_iter().map(
                |column| column.fact(black_box(product)).unwrap_or(Scalar::Null),
            )))
        });
    });
    group.bench_function("into_row", |bencher| {
        bencher.iter(|| black_box(black_box(product).into_row().expect("a row")));
    });
    group.bench_function("from_row", |bencher| {
        bencher.iter_batched(
            || row.clone(),
            |row| black_box(P::from_row(black_box(field), &row).expect("the row reads")),
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

pub fn benchmarks(criterion: &mut Criterion) {
    let codec = codec();
    let messages = messages(&codec);
    let order = codec
        .orders(messages.clone())
        .next()
        .expect("an order")
        .expect("it reads");
    let execution = codec
        .executions(messages.clone())
        .next()
        .expect("an execution")
        .expect("it reads");
    let trade = codec
        .trades(messages.clone())
        .next()
        .expect("a trade")
        .expect("it reads");
    // The capture quotes nothing, so the quote is the one its own doctest
    // states: two lanes and an expiry.
    let mut quote = QuoteData::at(1_700_000_000_000_000_000);
    {
        use yggdryl::graph::{Element, Event, MarketElement};
        quote.set_crosscode("Q-1".to_owned());
        quote.set_bidpx(Some("82.4".parse().expect("a price")));
        quote.set_bidqty(Some(yggdryl::Decimal18::from_int(100)));
        quote.set_askpx(Some("82.6".parse().expect("a price")));
        quote.set_askqty(Some(yggdryl::Decimal18::from_int(100)));
        quote.set_expirunix(Some(1_700_000_060_000_000_000));
        quote.finalize();
    }
    let book = codec
        .books(messages, 5, 1_000_000_000)
        .expect("a grid and a depth")
        .next()
        .expect("a book")
        .expect("it reads");
    timed(
        criterion,
        "order",
        &OrderData::field().expect("the order row"),
        &order,
    );
    timed(
        criterion,
        "execution",
        &ExecutionData::field().expect("the execution row"),
        &execution,
    );
    timed(
        criterion,
        "trade",
        &TradeData::field().expect("the trade row"),
        &trade,
    );
    timed(
        criterion,
        "quote",
        &QuoteData::field().expect("the quote row"),
        &quote,
    );
    let depth = std::num::NonZeroU32::new(5).expect("five");
    timed(
        criterion,
        "book",
        &BookData::field(depth).expect("the book row"),
        &book,
    );
}
