//! The byte family: building a value, cloning it, and the two directions
//! across Arrow for the variable layouts and the fixed slot.
//!
//! Payload sizes straddle the value's thirty-byte inline buffer on purpose,
//! for the reason the string cases straddle theirs: below it a value is free
//! to build and free to clone, above it it is one shared handle and a copy,
//! and one number over both regimes would hide a regression in either. The
//! counts are asserted in `rust/tests/allocations.rs`; these cases are the
//! time those counts buy.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};
use yggdryl::{Bytes, DataType, Scalar};

const ROWS: usize = crate::bench_profile::corpus(10_000, 1_024);

/// The payload sizes every case runs at: two inside the inline buffer, two
/// past it, and one page.
const SIZES: [usize; 5] = [8, 30, 31, 64, 4096];

/// One payload of the size, with no byte repeated across a row.
fn payload(size: usize, seed: usize) -> Vec<u8> {
    (0..size)
        .map(|index| u8::try_from((index + seed) % 251).expect("a residue fits a byte"))
        .collect()
}

/// One column of payloads of the size.
fn column(rows: usize, size: usize) -> Scalar {
    Scalar::from_sequence((0..rows).map(|index| Scalar::from(payload(size, index))))
}

/// Time a column across Arrow in both directions.
fn column_round_trip(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    label: &str,
    dtype: &DataType,
    column: &Scalar,
) {
    let field = dtype.clone().nullable_field("value");
    group.bench_function(
        BenchmarkId::new(format!("{label}_write"), ROWS),
        |bencher| {
            bencher.iter(|| {
                yggdryl::arrow::array_from_value(black_box(&field), black_box(column))
                    .expect("the benchmark column is valid")
            });
        },
    );
    let array = yggdryl::arrow::array_from_value(&field, column).expect("the column builds");
    group.bench_function(BenchmarkId::new(format!("{label}_read"), ROWS), |bencher| {
        bencher.iter(|| {
            yggdryl::arrow::array_to_value(black_box(&field), black_box(array.as_ref()))
                .expect("the built column reads back")
        });
    });
    // Clone and drop of every row read out of the column, one value at a
    // time: what a value's handle costs once it is no longer a buffer Arrow
    // owns. The sequence itself is one shared handle, so it is walked.
    let rows = yggdryl::arrow::array_to_value(&field, array.as_ref())
        .expect("the built column reads back");
    let rows = rows
        .as_sequence()
        .expect("a column reads back as a sequence");
    group.bench_function(
        BenchmarkId::new(format!("{label}_row_clone"), ROWS),
        |bencher| {
            bencher.iter(|| black_box(rows).to_vec());
        },
    );
}

pub(crate) fn bytes_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("bytes");

    // The grammar: what a schema pays once per declared column.
    group.bench_function("parse_display_round_trip", |bencher| {
        bencher.iter(|| {
            let dtype = DataType::from_str(black_box("fixed_size_binary(16)"))
                .expect("the static spelling must parse");
            DataType::from_str(black_box(&dtype.to_string()))
                .expect("canonical display output must round-trip")
        });
    });

    // One value with no Arrow around it: the copy into the inline buffer or
    // the one allocation past it, then the clone each regime pays.
    for size in SIZES {
        let bytes = payload(size, 0);
        group.bench_function(BenchmarkId::new("construct", size), |bencher| {
            bencher.iter(|| Bytes::new(black_box(bytes.as_slice())));
        });
        let value = Bytes::new(&bytes);
        group.bench_function(BenchmarkId::new("clone", size), |bencher| {
            bencher.iter(|| black_box(&value).clone());
        });
        // Restating the layout: the width is checked, the payload shared.
        let fixed = DataType::fixed_size_binary(u32::try_from(size).expect("the sizes fit"))
            .expect("every payload size is a width");
        group.bench_function(BenchmarkId::new("restate_fixed", size), |bencher| {
            bencher.iter(|| {
                black_box(&fixed)
                    .scalar(Scalar::Bytes(black_box(&value).clone()))
                    .expect("the payload is exactly the width")
            });
        });
    }

    group.throughput(Throughput::Elements(ROWS as u64));
    for size in SIZES {
        let column = column(ROWS, size);
        column_round_trip(
            &mut group,
            &format!("binary_{size}"),
            &DataType::binary(),
            &column,
        );
        column_round_trip(
            &mut group,
            &format!("large_binary_{size}"),
            &DataType::large_binary(),
            &column,
        );
        column_round_trip(
            &mut group,
            &format!("binary_view_{size}"),
            &DataType::binary_view(),
            &column,
        );
        // The fixed slot, which every payload here fills exactly.
        let fixed = DataType::fixed_size_binary(u32::try_from(size).expect("the sizes fit"))
            .expect("every payload size is a width");
        column_round_trip(
            &mut group,
            &format!("fixed_size_binary_{size}"),
            &fixed,
            &column,
        );
    }

    group.finish();
}
