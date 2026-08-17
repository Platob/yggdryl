use std::hint::black_box;

use arrow_schema::{IntervalUnit as ArrowIntervalUnit, TimeUnit as ArrowTimeUnit};
use criterion::Criterion;
use yggdryl::{DataType, TimeUnit};

pub(crate) fn time_builder_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("time");
    group.bench_function("infer_time32", |bencher| {
        bencher.iter(|| DataType::time(black_box(TimeUnit::Millisecond)))
    });
    group.bench_function("infer_time64", |bencher| {
        bencher.iter(|| DataType::time(black_box(TimeUnit::Nanosecond)))
    });
    group.finish();
}

pub(crate) fn time_unit_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("time_unit");
    group.bench_function("parse_canonical_temporal", |bencher| {
        bencher.iter(|| {
            TimeUnit::from_str(black_box("us")).expect("the canonical static time unit must parse")
        });
    });
    group.bench_function("parse_arrow_interval_name", |bencher| {
        bencher.iter(|| {
            TimeUnit::from_str(black_box("MonthDayNano"))
                .expect("the static Arrow interval unit name must parse")
        });
    });
    group.bench_function("to_arrow_time_unit", |bencher| {
        bencher.iter(|| {
            ArrowTimeUnit::try_from(black_box(TimeUnit::Nanosecond))
                .expect("nanosecond is an Arrow time unit")
        });
    });
    group.bench_function("to_arrow_interval_unit", |bencher| {
        bencher.iter(|| {
            ArrowIntervalUnit::try_from(black_box(TimeUnit::MonthDayNano))
                .expect("month-day-nano is an Arrow interval unit")
        });
    });
    group.finish();
}
