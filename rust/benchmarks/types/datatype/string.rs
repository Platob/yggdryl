//! The string family: every one of its eighteen leaves through the doors a
//! caller uses, and the two directions across Arrow with the leaf as the
//! column.
//!
//! Cell widths straddle `smol_str`'s twenty-three-byte inline buffer on
//! purpose. That threshold is the whole allocation story of a string value -
//! below it a cell is free to build and free to clone, above it it is one
//! shared handle and a copy - so a benchmark that measured only one side of it
//! would report an average of two different regimes and hide a regression in
//! either. The counts themselves are asserted in `rust/tests/allocations.rs`;
//! these cases are the time those counts buy.

use std::hint::black_box;
use std::sync::Arc;

use arrow_array::{ArrayRef, StringArray};
use criterion::{BenchmarkId, Criterion, Throughput};
use yggdryl::StringType;
use yggdryl::{Charset, DataType, Scalar, Str};

use super::doors;

const ROWS: usize = crate::bench_profile::corpus(10_000, 1_024);

/// The widths a column case runs at: one inside the inline buffer, one past
/// it, and one where the copy a cell pays past the buffer is the whole cost.
const WIDTHS: [usize; 3] = [8, 64, 4096];

/// The widths a single-cell case runs at, straddling the inline buffer exactly.
const CELL_WIDTHS: [usize; 4] = [8, 23, 24, 64];

/// The number the six numbered leaves state through the doors: past the
/// inline buffer, so the scalar door adopts a handle rather than copying.
const BOUND: usize = 32;

/// The eighteen leaves, each numbered one at `bound`.
fn leaves(bound: usize) -> impl Iterator<Item = StringType> {
    let bound = u32::try_from(bound).expect("every benchmark width fits");
    StringType::ALL
        .into_iter()
        .map(move |leaf| match leaf.bound() {
            Some(_) => leaf
                .with_bound(bound)
                .expect("every benchmark width is a width"),
            None => leaf,
        })
}

/// One column of all-ASCII cells, which every charset here borrows.
fn ascii_column(rows: usize, width: usize) -> Scalar {
    Scalar::from_sequence((0..rows).map(|index| Scalar::from(format!("{index:0width$}"))))
}

/// The same column with one byte per cell above US-ASCII, which none borrows.
fn high_column(rows: usize, width: usize) -> Scalar {
    Scalar::from_sequence((0..rows).map(|index| {
        let mut text = format!("{index:0>pad$}", pad = width - 1);
        text.push('é');
        Scalar::from(text)
    }))
}

/// One cell's bytes as the charset stores them.
fn cell(charset: Charset, width: usize, high: bool) -> Vec<u8> {
    let mut text = "a".repeat(width.saturating_sub(usize::from(high)));
    if high {
        text.push('é');
    }
    charset
        .encode(&text)
        .expect("the benchmark text fits every charset here")
        .into_owned()
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
}

pub(crate) fn string_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("string");

    // Every leaf through every door: what a schema pays once per declared
    // column, and what one value pays entering it.
    let sample = Scalar::from("a".repeat(BOUND));
    let declared: Vec<DataType> = leaves(BOUND)
        .map(|leaf| DataType::string(leaf).expect("every leaf is a datatype"))
        .collect();
    for dtype in &declared {
        doors::leaf_doors(&mut group, dtype, &sample);
    }
    // The one reader, over every leaf that answers it.
    group.bench_function("string_parameters", |bencher| {
        bencher.iter(|| {
            for dtype in black_box(&declared) {
                black_box(dtype.string_parameters());
            }
        });
    });
    // The document a windows-1252 string crosses Arrow inside.
    let parameters = StringType::SizedCp1252String(32);
    group.bench_function("extension_round_trip", |bencher| {
        bencher.iter(|| {
            let rendered = black_box(parameters).extension_json();
            StringType::from_extension_json(black_box(&rendered))
                .expect("the document this crate renders is one it reads")
        });
    });

    // One cell through the storage door, with no Arrow around it: the only
    // case where a regression in the transcode cannot hide behind a builder.
    // UTF-8 and US-ASCII are validated, the legacy charset is transcribed, and
    // the fixed slot is filled to its width, so nothing is trimmed.
    for width in CELL_WIDTHS {
        for parameters in [
            StringType::Utf8String,
            StringType::AsciiString,
            StringType::FixedAsciiString(u32::try_from(width).expect("the widths fit")),
            StringType::Cp1252String,
        ] {
            let name = parameters.as_str();
            let ascii = cell(parameters.charset(), width, false);
            group.bench_function(
                BenchmarkId::new(format!("transcribe_cell_{name}"), width),
                |bencher| {
                    bencher.iter(|| Str::from_bytes(black_box(&ascii), black_box(parameters)));
                },
            );
            // US-ASCII has no byte above 0x7F to pay for.
            if parameters.charset() == Charset::Ascii {
                continue;
            }
            let high = cell(parameters.charset(), width, true);
            group.bench_function(
                BenchmarkId::new(format!("transcribe_cell_high_{name}"), width),
                |bencher| {
                    bencher.iter(|| Str::from_bytes(black_box(&high), black_box(parameters)));
                },
            );
        }
    }

    group.throughput(Throughput::Elements(ROWS as u64));
    // A text column read into every leaf: the reader's door, at the width
    // past the inline buffer, with the numbered leaves stated at that width
    // so that nothing is trimmed and nothing refused.
    let ingested: ArrayRef = Arc::new(StringArray::from_iter_values(
        (0..ROWS).map(|index| format!("{index:064}")),
    ));
    for leaf in leaves(64) {
        doors::ingest(
            &mut group,
            &DataType::string(leaf).expect("every leaf is a datatype"),
            &ingested,
        );
    }
    // Every leaf as the column, both ways, at every width. The numbered
    // leaves are stated at the column's own width so that nothing is
    // trimmed and nothing refused; the windows-1252 string also runs on the
    // payload that does not borrow, because only the pair says whether a
    // change helped or moved cost.
    for width in WIDTHS {
        let ascii = ascii_column(ROWS, width);
        for leaf in leaves(width) {
            column_round_trip(
                &mut group,
                &format!("{}_{width}", leaf.as_str()),
                &DataType::string(leaf).expect("every leaf is a datatype"),
                &ascii,
            );
        }
        let high = high_column(ROWS, width);
        column_round_trip(
            &mut group,
            &format!("cp1252_high_{width}"),
            &DataType::cp1252(),
            &high,
        );
    }

    group.finish();
}
