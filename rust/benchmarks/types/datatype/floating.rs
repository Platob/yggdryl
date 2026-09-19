//! The decimal family: the width a precision picks, and each of the four
//! widths through the doors a caller uses, with a text column read into
//! each.

use std::hint::black_box;
use std::sync::Arc;

use arrow_array::{ArrayRef, StringArray};
use criterion::{Criterion, Throughput};
use yggdryl::DataType;

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
    // carries, so a value entering it is rescaled and never refused.
    let leaves = [
        DataType::decimal32(9, 2).expect("nine digits fit thirty-two bits"),
        DataType::decimal64(18, 4).expect("eighteen digits fit sixty-four bits"),
        DataType::decimal128(38, 18)
            .expect("thirty-eight digits fit one hundred and twenty-eight bits"),
        DataType::decimal256(76, 18)
            .expect("seventy-six digits fit two hundred and fifty-six bits"),
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
    group.finish();
}
