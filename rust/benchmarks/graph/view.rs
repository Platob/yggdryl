use std::hint::black_box;

use arrow_array::RecordBatch;
use criterion::{BatchSize, Criterion, Throughput};
use smol_str::SmolStr;
use yggdryl::arrow::batch_reader;
use yggdryl::expression::FieldPath;
use yggdryl::graph::{
    BookEvent, Element, Event, ExecutionEvent, Market, MarketData, MarketView, OrderEvent,
    TradeEvent,
};
use yggdryl::{Decimal, Side, State};

/// One dated order of `code` at `unix`, resting on the bid.
fn order(unix: i64, code: &str, price: i64) -> OrderEvent {
    let mut event = OrderEvent::at(unix);
    event.set_crosscode(code.to_owned());
    event.set_ticker(Some(SmolStr::new("BENCH")));
    event.set_side(Side::read("Buy").expect("a shipped side"));
    event.set_price(Some(Decimal::from_int(price)));
    event.set_quantity(Some(Decimal::from_int(10)));
    event.set_state(State::read("New").expect("the shipped new state"));
    event.finalize();
    event
}

/// One execution of `code` at `unix`.
fn execution(unix: i64, code: &str) -> ExecutionEvent {
    let mut event = ExecutionEvent::at(unix);
    event.set_crosscode(code.to_owned());
    event.set_ticker(Some(SmolStr::new("BENCH")));
    event.set_side(Side::read("Sell").expect("a shipped side"));
    event.set_price(Some(Decimal::from_int(100)));
    event.set_quantity(Some(Decimal::from_int(3)));
    event.set_lastpx(Some(Decimal::from_int(100)));
    event.set_lastqty(Some(Decimal::from_int(3)));
    event.set_state(State::read("Filled").expect("the shipped filled state"));
    event.finalize();
    event
}

/// The `marketdata` stream a view reads: an order, a two-execution trade and
/// a two-sided book per step, so every view has rows to keep.
fn leaves(steps: usize) -> Vec<MarketData> {
    let mut held = Vec::with_capacity(steps * 3);
    for step in 0..steps {
        let unix = 1 + i64::try_from(step).expect("a bench corpus");
        let code = format!("O-{step}");
        held.push(MarketData::from(order(unix, &code, 100)));
        let root = execution(unix, &format!("T-{step}"));
        let trade = TradeEvent::from_parts(
            &root,
            vec![
                execution(unix, &format!("T-{step}-a")),
                execution(unix, &format!("T-{step}-b")),
            ],
        )
        .expect("a bench trade");
        held.push(MarketData::from(trade));
        let mut book = BookEvent::new(unix, "BENCH");
        book.add_operations([
            MarketData::from(order(unix, &format!("B-{step}"), 99)),
            MarketData::from({
                let mut ask = order(unix, &format!("A-{step}"), 101);
                ask.set_side(Side::read("Sell").expect("a shipped side"));
                ask.finalize();
                ask
            }),
        ])
        .expect("a bench book");
        held.push(MarketData::from(book));
    }
    held
}

fn batches(steps: usize) -> Vec<RecordBatch> {
    MarketData::arrow_reader(leaves(steps), None, None)
        .expect("the bench stream")
        .collect::<Result<Vec<_>, _>>()
        .expect("every leaf lays out")
}

pub fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("graph/view");
    let steps = crate::bench_profile::corpus(2_048, 32);
    let batches = batches(steps);
    let schema = batches[0].schema();
    let rows = u64::try_from(steps * 3).expect("a bench corpus");
    group.throughput(Throughput::Elements(rows));
    let isin: FieldPath = "securityids['ISIN'] as isin".parse().expect("a lift");
    for (name, view, lifts) in [
        ("orders", MarketView::Orders, vec![]),
        ("orders_lifted", MarketView::Orders, vec![isin.clone()]),
        ("trades", MarketView::Trades, vec![]),
        ("book_sides", MarketView::BookSides, vec![]),
        ("books", MarketView::Books, vec![]),
        (
            "lifecycle",
            MarketView::Lifecycle {
                crosscode: SmolStr::new("O-1"),
            },
            vec![],
        ),
    ] {
        group.bench_function(name, |bencher| {
            bencher.iter_batched(
                || batch_reader(schema.clone(), batches.clone()),
                |reader| {
                    MarketData::apply_view(black_box(&view), black_box(&lifts), reader)
                        .expect("a view")
                        .map(|batch| batch.map(|batch| batch.num_rows()))
                        .sum::<Result<usize, _>>()
                        .expect("every batch applies")
                },
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}
