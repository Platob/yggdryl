//! What the book iterator costs per statement, over hand-built streams
//! and no protocol: makers arriving and leaving, makers restated, and
//! prints, at two depths.
//!
//! The reference is `adds_and_deletes`: every statement rests a new order
//! or retires one, so each moves a level and the ladder is read once per
//! instant. `restatements` moves the same makers between two sizes, which
//! is a lift and a rest per statement on a ladder that never grows;
//! `prints` touches no level and moves the last, the volume and the
//! average. Throughput is in statements, and the depth is what the book
//! is cut to, never what the ladder holds, so the two depths should cost
//! alike but for the rows read.

use std::hint::black_box;
use std::num::NonZeroU32;

use criterion::{BatchSize, Criterion, Throughput};
use yggdryl::graph::{Element, Event, MarketElement};
use yggdryl::market::{BookIterator, ExecutionData, OrderData, Statement};
use yggdryl::{Currency, Decimal18, Side, State};

/// How many statements one stream holds.
const STATEMENTS: usize = crate::bench_profile::corpus(4_096, 64);

/// How many distinct makers the restatements move between.
const MAKERS: usize = 64;

fn order(unix: i64, code: usize, side: &str, px: i64, qty: i64, live: bool) -> Statement {
    let mut order = OrderData::at(unix);
    order.set_crosscode(format!("O-{code}"));
    order.set_symbolticker(Some("AAPL".to_owned()));
    order.set_side(Side::read(side).expect("a side"));
    order.set_px(Decimal18::from_int(px));
    order.set_qty(Decimal18::from_int(qty));
    order.set_state(State::read(if live { "New" } else { "Canceled" }).expect("a state"));
    order.set_currency(Currency::new("USD").expect("a currency"));
    order.finalize();
    order.into()
}

fn print(unix: i64, code: usize, px: i64, qty: i64) -> Statement {
    let mut fill = ExecutionData::at(unix);
    fill.set_crosscode(format!("O-{code}"));
    fill.set_symbolticker(Some("AAPL".to_owned()));
    fill.set_px(Decimal18::from_int(px));
    fill.set_qty(Decimal18::from_int(qty));
    fill.set_state(State::read("PartiallyFilled").expect("a state"));
    fill.finalize();
    fill.into()
}

/// Every statement a new order, every other one retiring the one before.
fn adds_and_deletes() -> Vec<Statement> {
    (0..STATEMENTS)
        .map(|at| {
            let unix = i64::try_from(at).expect("an instant") + 1;
            let side = if at % 2 == 0 { "Buy" } else { "Sell" };
            let px =
                1_000 + i64::try_from(at % 50).expect("a price") * if at % 2 == 0 { -1 } else { 1 };
            if at % 4 == 3 {
                order(unix, at - 2, side, px, 100, false)
            } else {
                order(unix, at, side, px, 100, true)
            }
        })
        .collect()
}

/// The same makers restated between two sizes.
fn restatements() -> Vec<Statement> {
    (0..STATEMENTS)
        .map(|at| {
            let unix = i64::try_from(at).expect("an instant") + 1;
            let code = at % MAKERS;
            let side = if code % 2 == 0 { "Buy" } else { "Sell" };
            let px = 1_000
                + i64::try_from(code % 8).expect("a price") * if code % 2 == 0 { -1 } else { 1 };
            let qty = if (at / MAKERS) % 2 == 0 { 100 } else { 150 };
            order(unix, code, side, px, qty, true)
        })
        .collect()
}

/// A handful of makers, then a print per instant.
fn prints() -> Vec<Statement> {
    let mut statements: Vec<Statement> = (0..MAKERS)
        .map(|code| {
            let side = if code % 2 == 0 { "Buy" } else { "Sell" };
            let px = 1_000
                + i64::try_from(code % 8).expect("a price") * if code % 2 == 0 { -1 } else { 1 };
            order(1, code, side, px, 100, true)
        })
        .collect();
    statements.extend((0..STATEMENTS.saturating_sub(MAKERS)).map(|at| {
        let unix = i64::try_from(at).expect("an instant") + 2;
        print(unix, at % MAKERS, 1_000, 10)
    }));
    statements
}

pub fn benchmarks(criterion: &mut Criterion) {
    for (name, statements) in [
        ("adds_and_deletes", adds_and_deletes()),
        ("restatements", restatements()),
        ("prints", prints()),
    ] {
        let mut group = criterion.benchmark_group(format!("market/iterator/{name}"));
        group.throughput(Throughput::Elements(
            u64::try_from(statements.len()).expect("a count"),
        ));
        for depth in [5_u32, 20] {
            let depth = NonZeroU32::new(depth).expect("a depth");
            group.bench_function(format!("depth_{depth}"), |bencher| {
                bencher.iter_batched(
                    || statements.clone(),
                    |statements| {
                        black_box(
                            BookIterator::new(
                                black_box(statements).into_iter().map(Ok),
                                depth,
                                true,
                            )
                            .map(|book| book.expect("a book").get_seqnum())
                            .sum::<u64>(),
                        )
                    },
                    BatchSize::LargeInput,
                );
            });
        }
        group.finish();
    }
}
