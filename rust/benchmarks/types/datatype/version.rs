//! The two hot operations owned by the generic version value.

use std::hint::black_box;

use criterion::Criterion;
use yggdryl::Version;

pub(crate) fn version_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("version");
    group.bench_function("parse", |bencher| {
        bencher.iter(|| {
            black_box("5.0SP10")
                .parse::<Version>()
                .expect("the static version is valid")
        });
    });

    let left = "5.0SP2"
        .parse::<Version>()
        .expect("the static version is valid");
    let right = "5.0SP10"
        .parse::<Version>()
        .expect("the static version is valid");
    group.bench_function("compare", |bencher| {
        bencher.iter(|| black_box(left).cmp(black_box(&right)));
    });
    group.finish();
}
