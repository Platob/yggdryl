//! Cold parsing, and what the grammar's conveniences cost.

use std::hint::black_box;

use criterion::Criterion;
use yggdryl::{Expr, Selection};

pub fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("expression_parse");

    for (name, text) in [
        ("small", "venue = 'XNAS'"),
        (
            "medium",
            "venue = 'XNAS' AND price BETWEEN 10.00 AND 20.00 AND id IS NOT NULL",
        ),
        (
            "deep",
            "((((((venue = 'XNAS' AND id > 1) OR id < 9) AND price > 1.00) OR price < 99.00) \
              AND venue <> 'XNYS') OR id = 7)",
        ),
    ] {
        group.bench_function(name, |bencher| {
            bencher.iter(|| black_box(text).parse::<Expr>().expect("parses"));
        });
    }

    // The convenience legs: an encapsulated name and an accessor chain against
    // the plain spellings they cost more than, so the grammar's reach carries a
    // number rather than an assurance.
    for (name, text) in [
        ("bare_name", "total = 1"),
        ("encapsulated_name", "\"total amount\" = 1"),
        ("flat_column", "a = 1"),
        ("accessor_chain", "a.b[0]['k'] = 1"),
        ("accessor_range", "a[1:3] = 1"),
    ] {
        group.bench_function(name, |bencher| {
            bencher.iter(|| black_box(text).parse::<Expr>().expect("parses"));
        });
    }

    group.bench_function("display_parse_round_trip", |bencher| {
        let expression: Expr = "venue = 'XNAS' AND price > 10.00".parse().expect("parses");
        bencher.iter(|| {
            let canonical = black_box(&expression).to_string();
            canonical.parse::<Expr>().expect("round-trips")
        });
    });

    group.bench_function("selection", |bencher| {
        bencher.iter(|| {
            black_box("venue, price * 2 AS doubled, id")
                .parse::<Selection>()
                .expect("parses")
        });
    });

    group.finish();
}
