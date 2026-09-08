use std::hint::black_box;
use std::sync::Arc;

use criterion::Criterion;
use yggdryl::FixCodec;

use super::seed;

/// A wide order: every facet a book asks for, plus a Parties group.
const ORDER: &str = "8=FIX.4.4|9=176|35=D|49=SENDER|56=TARGET|34=7|52=20240102-10:15:30.000|11=ORDER-1|55=AAPL|54=1|38=100|44=12.5|15=USD|60=20240102-10:15:30.000|10=203|";

/// The same shape a bridge writes, with a group to walk.
const PARTIED: &str = "MSGTYPE=D|CLORDID=ORDER-1|SYMBOL=AAPL|SIDE=1|ORDERQTY=100|#NOPARTYIDS=2|#NOPARTYIDS[0]=PARTYID=SYNTH-01\u{4}\u{3}PARTYIDSOURCE=D\u{4}\u{3}PARTYROLE=1|#NOPARTYIDS[1]=PARTYID=CLEARER-9\u{4}\u{3}PARTYIDSOURCE=D\u{4}\u{3}PARTYROLE=4";

pub fn benchmarks(criterion: &mut Criterion) {
    let reader = FixCodec::new(Arc::new(seed()));
    let order = reader
        .transform_line(ORDER.as_bytes(), false)
        .expect("a readable order");
    let partied = reader
        .transform_line(PARTIED.as_bytes(), false)
        .expect("a readable bridge row");
    let mut group = criterion.benchmark_group("fix/lift");

    // One facet, which is what a monitor asks for per row.
    group.bench_function("one", |bencher| {
        bencher.iter(|| black_box(&order).lifted(black_box("symbol")));
    });
    // The one that falls down a ladder before it answers.
    group.bench_function("ladder", |bencher| {
        bencher.iter(|| black_box(&order).lifted(black_box("transacttime")));
    });
    // A facet nothing answers, which walks every source and derives.
    group.bench_function("absent", |bencher| {
        bencher.iter(|| black_box(&order).lifted(black_box("execid")));
    });
    // Every facet at once, which is what a batch writer's row costs.
    group.bench_function("all", |bencher| {
        bencher.iter(|| black_box(&order).lift().count());
    });
    group.bench_function("source", |bencher| {
        bencher.iter(|| black_box(&order).lift_source(black_box("quantity")));
    });
    // The two role-addressed accessors, which walk a group.
    group.bench_function("party", |bencher| {
        bencher.iter(|| black_box(&partied).party(black_box("ClearingFirm")));
    });
    // Derived rather than stored, so it is measured on a message that is
    // otherwise clean: a caller who never asks pays nothing.
    group.bench_function("anomalies", |bencher| {
        bencher.iter(|| black_box(&order).anomalies().count());
    });
    group.finish();
}
