//! The string family: the grammar, the extension document, and the two
//! directions across Arrow for every layout at each charset class.
//!
//! Cell widths straddle `smol_str`'s twenty-three-byte inline buffer on
//! purpose. That threshold is the whole allocation story of a string value -
//! below it a cell is free to build and free to clone, above it it is one
//! shared handle and a copy - so a benchmark that measured only one side of it
//! would report an average of two different regimes and hide a regression in
//! either. The counts themselves are asserted in `rust/tests/allocations.rs`;
//! these cases are the time those counts buy.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};
use yggdryl::types::{StringLayout, StringParameters};
use yggdryl::{Charset, DataType, Scalar, Str};

const ROWS: usize = crate::bench_profile::corpus(10_000, 1_024);

/// The widths a column case runs at: one inside the inline buffer, one past it.
const WIDTHS: [usize; 2] = [8, 64];

/// The widths a single-cell case runs at, straddling the inline buffer exactly.
const CELL_WIDTHS: [usize; 4] = [8, 23, 24, 64];

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

    // The grammar: what a schema pays once per declared column.
    group.bench_function("parse_display_round_trip", |bencher| {
        bencher.iter(|| {
            let dtype = DataType::from_str(black_box("string(windows-1252,32)"))
                .expect("the static spelling must parse");
            DataType::from_str(black_box(&dtype.to_string()))
                .expect("canonical display output must round-trip")
        });
    });
    // The one reader, over every datatype that answers it.
    let declared: Vec<DataType> = vec![
        DataType::utf8(),
        DataType::large_utf8(),
        DataType::utf8_view(),
        DataType::ascii(),
        DataType::fixed_ascii(3).expect("three bytes is a width"),
        DataType::from_str("string(windows-1252,32)").expect("a charset string"),
    ];
    group.bench_function("string_parameters", |bencher| {
        bencher.iter(|| {
            for dtype in black_box(&declared) {
                black_box(dtype.string_parameters());
            }
        });
    });
    // The document a charset string crosses Arrow inside.
    let parameters = StringParameters::new(StringLayout::String, Charset::Cp1252)
        .try_with_bound(32)
        .expect("thirty-two bytes is a bound");
    group.bench_function("extension_round_trip", |bencher| {
        bencher.iter(|| {
            let rendered = black_box(parameters).extension_json();
            StringParameters::from_extension_json(black_box(&rendered))
                .expect("the document this crate renders is one it reads")
        });
    });
    // Cold projection against warm: the pair a cached projection has to
    // separate before either number means anything.
    group.bench_function("field_arrow_projection", |bencher| {
        let field = DataType::from_str("string(windows-1252)")
            .expect("a charset string")
            .required_field("value");
        bencher.iter(|| {
            black_box(&field)
                .clone()
                .into_arrow()
                .expect("the benchmark field is valid")
        });
    });

    // One cell through the storage door, with no Arrow around it: the only
    // case where a regression in the transcode cannot hide behind a builder.
    // UTF-8 and US-ASCII are validated, the legacy charset is transcribed, and
    // the fixed slot is filled to its width, so nothing is trimmed.
    for width in CELL_WIDTHS {
        let fixed = StringParameters::ascii(StringLayout::FixedString)
            .try_with_bound(u32::try_from(width).expect("the widths fit"))
            .expect("every cell width is a width");
        for (parameters, name) in [
            (StringParameters::utf8(StringLayout::String), "utf8"),
            (StringParameters::ascii(StringLayout::String), "ascii"),
            (fixed, "fixed_ascii"),
            (
                StringParameters::new(StringLayout::String, Charset::Cp1252),
                "cp1252",
            ),
        ] {
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
    for width in WIDTHS {
        let ascii = ascii_column(ROWS, width);
        let high = high_column(ROWS, width);
        // The three plain UTF-8 layouts Arrow names itself.
        column_round_trip(
            &mut group,
            &format!("utf8_{width}"),
            &DataType::utf8(),
            &ascii,
        );
        column_round_trip(
            &mut group,
            &format!("large_utf8_{width}"),
            &DataType::large_utf8(),
            &ascii,
        );
        column_round_trip(
            &mut group,
            &format!("utf8_view_{width}"),
            &DataType::utf8_view(),
            &ascii,
        );
        // The US-ASCII layouts: the same text storage under a document that
        // names the repertoire, and the fixed slot the codes ride.
        column_round_trip(
            &mut group,
            &format!("ascii_{width}"),
            &DataType::ascii(),
            &ascii,
        );
        column_round_trip(
            &mut group,
            &format!("fixed_ascii_{width}"),
            &DataType::fixed_ascii(u32::try_from(width).expect("the widths fit"))
                .expect("every column width is a width"),
            &ascii,
        );
        // The charset string, on the payload that borrows and the one that
        // does not. Only the pair says whether a change helped or moved cost.
        let latin = DataType::from_str("string(windows-1252)").expect("a charset string");
        column_round_trip(
            &mut group,
            &format!("charset_ascii_{width}"),
            &latin,
            &ascii,
        );
        column_round_trip(&mut group, &format!("charset_high_{width}"), &latin, &high);
        // And the fixed slot, which pads rather than offsets.
        let fixed = DataType::from_str("fixed_string(windows-1252,64)").expect("a fixed string");
        column_round_trip(&mut group, &format!("fixed_{width}"), &fixed, &ascii);
    }

    // Restating a layout: a storage handle adopted, never characters copied.
    let long = Scalar::from("a value well past the twenty-three byte inline buffer");
    let large = DataType::large_utf8();
    group.throughput(Throughput::Elements(1));
    group.bench_function("restate_layout", |bencher| {
        bencher.iter(|| {
            black_box(&large)
                .scalar(black_box(long.clone()))
                .expect("text restates into any layout")
        });
    });

    group.finish();
}
