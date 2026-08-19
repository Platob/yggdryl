//! What binding costs, so "bind once per read" is a number rather than advice.

use std::hint::black_box;

use criterion::Criterion;
use yggdryl::Expr;

use super::schema;

pub fn benchmarks(criterion: &mut Criterion) {
    let schema = schema();
    let mut group = criterion.benchmark_group("expression_bind");

    for (name, text) in [
        ("small", "venue = 'XNAS'"),
        (
            "medium",
            "venue = 'XNAS' AND price BETWEEN 10.00 AND 20.00 AND id IS NOT NULL",
        ),
        // A long disjunction is what the coalescing rules exist for, so its
        // bind cost is what those rules cost.
        (
            "wide_disjunction",
            "id = 0 OR id = 1 OR id = 2 OR id = 3 OR id = 4 OR id = 5 OR id = 6 OR id = 7",
        ),
    ] {
        let expression: Expr = text.parse().expect("parses");
        group.bench_function(name, |bencher| {
            bencher.iter(|| black_box(&expression).bind(&schema).expect("binds"));
        });
    }

    // The optimizer's own cost, against the same plan with it effectively off:
    // a two-node predicate is below the threshold and pays nothing, which is
    // what sets the threshold.
    group.bench_function("simplify_small", |bencher| {
        let expression: Expr = "venue = 'XNAS'".parse().expect("parses");
        bencher.iter(|| black_box(&expression).simplify());
    });
    group.bench_function("simplify_wide_disjunction", |bencher| {
        let text = (0..200)
            .map(|value| format!("id = {value}"))
            .collect::<Vec<_>>()
            .join(" OR ");
        let expression: Expr = text.parse().expect("parses");
        bencher.iter(|| black_box(&expression).simplify());
    });

    group.finish();
}
