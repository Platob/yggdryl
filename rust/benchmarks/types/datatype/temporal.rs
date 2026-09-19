use std::hint::black_box;

use arrow_schema::{IntervalUnit as ArrowIntervalUnit, TimeUnit as ArrowTimeUnit};
use criterion::Criterion;
use yggdryl::{DataType, Scalar, TimeUnit, Timezone};

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

/// Reading one temporal out of the text a wire wrote it as.
///
/// The date-only spellings sit beside the clocked ones on purpose. A date
/// states no clock, and the reader decides that on the byte that ends the
/// date rather than by trying a clock and failing, so a clocked reading must
/// cost what it always did and a date must cost less than one, not more. Both
/// pay the same scalar wrapper, so what the numbers differ by is the reading.
pub(crate) fn temporal_text_benchmarks(criterion: &mut Criterion) {
    let naive = DataType::datetime64(TimeUnit::Nanosecond, Timezone::NAIVE)
        .expect("nanoseconds are a clock resolution");
    let utc = DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC)
        .expect("nanoseconds are a clock resolution");
    let mut group = criterion.benchmark_group("temporal_text");
    for (name, dtype, spelling) in [
        ("datetime_extended", &naive, "2026-08-18T10:15:30.123456789"),
        ("datetime_compact", &naive, "20260818101530.123456789"),
        ("datetime_fix", &naive, "20260818-10:15:30.123456789"),
        ("date_extended", &naive, "2026-08-18"),
        ("date_compact", &naive, "20260818"),
        ("timestamp_compact", &utc, "20260818101530.123456789Z"),
        ("date_compact_zoned", &utc, "20260818Z"),
    ] {
        group.bench_function(name, |bencher| {
            bencher.iter(|| {
                dtype
                    .scalar(Scalar::from(black_box(spelling)))
                    .expect("the spelling reads")
            });
        });
    }
    group.finish();
}
