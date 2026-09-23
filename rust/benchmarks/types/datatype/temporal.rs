//! The five temporal families - date, time, datetime, duration, interval -
//! each of its leaves at each unit the width holds, through the doors a
//! caller uses; the width a resolution picks; the unit grammar; and one
//! temporal read out of the text a wire wrote it as.

use std::hint::black_box;
use std::sync::Arc;

use arrow_array::{ArrayRef, StringArray};
use arrow_schema::{IntervalUnit as ArrowIntervalUnit, TimeUnit as ArrowTimeUnit};
use criterion::measurement::WallTime;
use criterion::{BenchmarkGroup, Criterion, Throughput};
use yggdryl::{DataType, DateTimeType, Scalar, Serie, TemporalValue as _, TimeUnit, Timezone};
use yggdryl::{DateTime64, Duration32, Duration64, Interval, Time32, Time64};

use super::doors;

const ROWS: usize = crate::bench_profile::corpus(10_000, 1_024);

/// One family's leaves through every door, then a text column read into
/// each: the group is the family, the case the leaf at its unit.
fn family_doors(group: &mut BenchmarkGroup<'_, WallTime>, leaves: &[(DataType, &str)]) {
    for (dtype, spelling) in leaves {
        doors::leaf_doors(group, dtype, &Scalar::from(*spelling));
    }
    group.throughput(Throughput::Elements(ROWS as u64));
    for (dtype, spelling) in leaves {
        let column: ArrayRef = Arc::new(StringArray::from_iter_values(std::iter::repeat_n(
            *spelling, ROWS,
        )));
        doors::ingest(group, dtype, &column);
    }
}

pub(crate) fn date_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("date");
    family_doors(
        &mut group,
        &[
            (DataType::date32(), "2026-08-18"),
            (DataType::date64(), "2026-08-18"),
        ],
    );
    group.finish();
}

pub(crate) fn time_benchmarks(criterion: &mut Criterion) {
    // The width a resolution picks, which is the one rule behind `time`.
    let mut group = criterion.benchmark_group("time");
    group.bench_function("infer_time32", |bencher| {
        bencher.iter(|| DataType::time(black_box(TimeUnit::Millisecond)))
    });
    group.bench_function("infer_time64", |bencher| {
        bencher.iter(|| DataType::time(black_box(TimeUnit::Nanosecond)))
    });
    // The value restated at another resolution the same width holds.
    let millis = Time32::new(36_930_000, TimeUnit::Millisecond, Timezone::NAIVE)
        .expect("milliseconds fit the narrow width");
    group.bench_function("value_with_unit_time32", |bencher| {
        bencher.iter(|| {
            black_box(millis)
                .with_unit(black_box(TimeUnit::Second))
                .expect("whole seconds restate exactly")
        });
    });
    let micros = Time64::new(36_930_000_000, TimeUnit::Microsecond, Timezone::NAIVE)
        .expect("microseconds fit the wide width");
    group.bench_function("value_with_unit_time64", |bencher| {
        bencher.iter(|| {
            black_box(micros)
                .with_unit(black_box(TimeUnit::Nanosecond))
                .expect("a finer resolution restates exactly")
        });
    });
    family_doors(
        &mut group,
        &[
            (
                DataType::time32(TimeUnit::Second).expect("seconds fit time32"),
                "10:15:30",
            ),
            (
                DataType::time32(TimeUnit::Millisecond).expect("milliseconds fit time32"),
                "10:15:30.123",
            ),
            (
                DataType::time64(TimeUnit::Microsecond).expect("microseconds fit time64"),
                "10:15:30.123456",
            ),
            (
                DataType::time64(TimeUnit::Nanosecond).expect("nanoseconds fit time64"),
                "10:15:30.123456789",
            ),
        ],
    );
    group.finish();
}

pub(crate) fn datetime_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("datetime");
    // The leaf restated at another resolution, and in another zone.
    let leaf = DateTimeType::ALL[0];
    group.bench_function("leaf_with_unit", |bencher| {
        bencher.iter(|| {
            black_box(leaf)
                .with_unit(black_box(TimeUnit::Nanosecond))
                .expect("nanoseconds are a clock resolution")
        });
    });
    group.bench_function("leaf_with_timezone", |bencher| {
        bencher.iter(|| black_box(leaf).with_timezone(black_box(Timezone::UTC)));
    });
    // The value restated at another resolution.
    let micros = DateTime64::new(1_700_000_000_000_000, TimeUnit::Microsecond, Timezone::UTC)
        .expect("microseconds are a clock resolution");
    group.bench_function("value_with_unit", |bencher| {
        bencher.iter(|| {
            black_box(micros)
                .with_unit(black_box(TimeUnit::Nanosecond))
                .expect("a finer resolution restates exactly")
        });
    });
    let naive = |unit| DataType::datetime64(unit, Timezone::NAIVE).expect("a clock resolution");
    family_doors(
        &mut group,
        &[
            (naive(TimeUnit::Second), "2026-08-18T10:15:30"),
            (naive(TimeUnit::Millisecond), "2026-08-18T10:15:30.123"),
            (naive(TimeUnit::Microsecond), "2026-08-18T10:15:30.123456"),
            (naive(TimeUnit::Nanosecond), "2026-08-18T10:15:30.123456789"),
            (
                DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC)
                    .expect("a clock resolution"),
                "2026-08-18T10:15:30.123456Z",
            ),
        ],
    );
    group.finish();
}

pub(crate) fn duration_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("duration");
    // The value restated at another resolution the same width holds.
    let seconds =
        Duration32::new(90, TimeUnit::Second, Timezone::NAIVE).expect("seconds are a fixed length");
    group.bench_function("value_with_unit_duration32", |bencher| {
        bencher.iter(|| {
            black_box(seconds)
                .with_unit(black_box(TimeUnit::Millisecond))
                .expect("a finer resolution restates exactly")
        });
    });
    let millis = Duration64::new(90_000, TimeUnit::Millisecond, Timezone::NAIVE)
        .expect("milliseconds are a fixed length");
    group.bench_function("value_with_unit_duration64", |bencher| {
        bencher.iter(|| {
            black_box(millis)
                .with_unit(black_box(TimeUnit::Nanosecond))
                .expect("a finer resolution restates exactly")
        });
    });
    family_doors(
        &mut group,
        &[
            (
                DataType::duration32(TimeUnit::Second).expect("seconds are a fixed length"),
                "PT90S",
            ),
            (
                DataType::duration32(TimeUnit::Millisecond)
                    .expect("milliseconds are a fixed length"),
                "PT90.5S",
            ),
            (
                DataType::duration64(TimeUnit::Microsecond)
                    .expect("microseconds are a fixed length"),
                "PT90.5S",
            ),
            (
                DataType::duration64(TimeUnit::Nanosecond).expect("nanoseconds are a fixed length"),
                "PT90.5S",
            ),
        ],
    );
    group.finish();
}

/// An interval has no text spelling, so its doors take the value itself,
/// and the column cast is the same-layout cast a reader pays for a column
/// already in its declared layout.
pub(crate) fn interval_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("interval");
    // The value restated in another layout that holds the same parts.
    let nanos = Interval::new(0, 2, 3_000_000_000, TimeUnit::MonthDayNano)
        .expect("three parts fit the widest layout");
    group.bench_function("value_with_unit", |bencher| {
        bencher.iter(|| {
            black_box(nanos)
                .with_unit(black_box(TimeUnit::DayTime))
                .expect("whole milliseconds and no months fit the day-time layout")
        });
    });
    let leaves = [
        (
            DataType::interval(TimeUnit::YearMonth).expect("an interval layout"),
            Scalar::interval(14, 0, 0, TimeUnit::YearMonth).expect("months fit the layout"),
        ),
        (
            DataType::interval(TimeUnit::DayTime).expect("an interval layout"),
            Scalar::interval(0, 2, 3_000_000_000, TimeUnit::DayTime)
                .expect("days and whole milliseconds fit the layout"),
        ),
        (
            DataType::interval(TimeUnit::MonthDayNano).expect("an interval layout"),
            Scalar::interval(1, 2, 3, TimeUnit::MonthDayNano).expect("three parts fit the layout"),
        ),
    ];
    for (dtype, sample) in &leaves {
        doors::leaf_doors(&mut group, dtype, sample);
    }
    group.throughput(Throughput::Elements(ROWS as u64));
    for (dtype, sample) in &leaves {
        let field = dtype.clone().nullable_field("value");
        let column = Serie::from_scalars(field, std::iter::repeat_n(sample.clone(), ROWS))
            .and_then(|serie| serie.require_arrow_array())
            .expect("the benchmark column is valid");
        doors::ingest(&mut group, dtype, &column);
    }
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
