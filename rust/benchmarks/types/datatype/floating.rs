use std::hint::black_box;

use criterion::Criterion;
use yggdryl::DataType;

pub(crate) fn decimal_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("decimal");
    group.bench_function("infer_decimal128", |bencher| {
        bencher.iter(|| DataType::decimal(black_box(38), black_box(18)))
    });
    group.bench_function("infer_decimal256", |bencher| {
        bencher.iter(|| DataType::decimal(black_box(39), black_box(18)))
    });
    group.finish();
}
