//! The decimal family: the width a precision picks, each of the four
//! widths and the two fixed leaves through the doors a caller uses, a text
//! column and a float column read into each, and a column of each rendered
//! as text.

use std::hint::black_box;
use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, StringArray};
use criterion::measurement::WallTime;
use criterion::{BenchmarkGroup, BenchmarkId, Criterion, Throughput};
use yggdryl::{ArrowCastOptions, DataType, Field, Scalar, Serie};

use super::doors;

const ROWS: usize = crate::bench_profile::corpus(10_000, 1_024);

pub(crate) fn decimal_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("decimal");
    // The width a precision picks, which is the one rule behind `decimal`.
    group.bench_function("infer_decimal128", |bencher| {
        bencher.iter(|| DataType::decimal(black_box(38), black_box(18)))
    });
    group.bench_function("infer_decimal256", |bencher| {
        bencher.iter(|| DataType::decimal(black_box(39), black_box(18)))
    });
    // Each width at its own maximum precision, with the scale a price
    // carries, so a value entering it is rescaled and never refused; then
    // the two fixed leaves, whose scale is already eighteen.
    let leaves = [
        DataType::decimal32(9, 2).expect("nine digits fit thirty-two bits"),
        DataType::decimal64(18, 4).expect("eighteen digits fit sixty-four bits"),
        DataType::decimal128(38, 18)
            .expect("thirty-eight digits fit one hundred and twenty-eight bits"),
        DataType::decimal256(76, 18)
            .expect("seventy-six digits fit two hundred and fifty-six bits"),
        DataType::Decimal,
        DataType::BigDecimal,
    ];
    let sample = "1234.56";
    for dtype in &leaves {
        doors::leaf_doors(&mut group, dtype, &sample.into());
    }
    group.throughput(Throughput::Elements(ROWS as u64));
    let column: ArrayRef = Arc::new(StringArray::from_iter_values(std::iter::repeat_n(
        sample, ROWS,
    )));
    for dtype in &leaves {
        doors::ingest(&mut group, dtype, &column);
    }
    // A float reads as the number it names, rounded at the declared scale:
    // the crate's own reading, one shortest-text pass per cell.
    let floats: ArrayRef = Arc::new(Float64Array::from_iter_values(std::iter::repeat_n(
        1234.56, ROWS,
    )));
    for dtype in &leaves {
        ingest_float(&mut group, dtype, &floats);
    }
    // A fixed leaf renders its trimmed text; the `decimal128(38, 18)` it
    // rides renders through Arrow's kernel at full scale, the baseline.
    for dtype in [
        DataType::Decimal,
        DataType::BigDecimal,
        DataType::decimal128(38, 18)
            .expect("thirty-eight digits fit one hundred and twenty-eight bits"),
    ] {
        render_utf8(&mut group, &dtype, sample);
    }
    group.finish();
}

/// One float column cast into a field of the leaf, strictly.
fn ingest_float(group: &mut BenchmarkGroup<'_, WallTime>, dtype: &DataType, source: &ArrayRef) {
    let label = dtype.to_string();
    let target = Field::new("value", dtype.clone(), true);
    let strict = ArrowCastOptions::new().with_safe(false);
    let read = Serie::from_arrow_array(Some(&target), Arc::clone(source), strict)
        .expect("every float of the column is a value of the leaf");
    assert_eq!(
        read.scalar(0).expect("the first row"),
        dtype
            .scalar(Scalar::from("1234.56"))
            .expect("the sample is a value of the leaf"),
        "the float reads as the number it names"
    );
    group.bench_function(BenchmarkId::new("ingest_float", &label), |bencher| {
        bencher.iter(|| {
            Serie::from_arrow_array(
                Some(black_box(&target)),
                Arc::clone(black_box(source)),
                strict,
            )
            .expect("every float of the column is a value of the leaf")
        });
    });
}

/// One column of the leaf cast to `utf8`, the text a row spells too.
fn render_utf8(group: &mut BenchmarkGroup<'_, WallTime>, dtype: &DataType, sample: &str) {
    let label = dtype.to_string();
    let value = dtype
        .scalar(Scalar::from(sample))
        .expect("the sample is a value of the leaf");
    let column = Serie::from_scalars(
        Field::new("value", dtype.clone(), true),
        std::iter::repeat_n(value.clone(), ROWS),
    )
    .expect("a column of the sample");
    let text = Field::new("value", DataType::utf8(), true);
    let options = ArrowCastOptions::new();
    assert_eq!(
        column
            .cast(&text, options)
            .expect("the column renders")
            .scalar(0)
            .expect("the first row"),
        DataType::utf8()
            .scalar(value)
            .expect("the value has a text"),
        "a column renders the text a row spells"
    );
    group.bench_function(BenchmarkId::new("render_utf8", &label), |bencher| {
        bencher.iter(|| {
            black_box(&column)
                .cast(black_box(&text), options)
                .expect("the column renders")
        });
    });
}
