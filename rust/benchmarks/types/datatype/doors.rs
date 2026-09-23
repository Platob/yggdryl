//! The doors every leaf answers, timed one leaf at a time.
//!
//! A family is its leaves, so a case is one leaf through one door: the
//! grammar reading the canonical spelling, the spelling written back, the
//! Arrow datatype projected, a field built over the leaf, that field
//! projected to Arrow with whatever document the leaf rides in and imported
//! back to the same leaf, one value through the scalar door, and a column
//! cast into a field of the leaf. Each door is asserted once outside the
//! timer, so a case times a door that answers and never one that refuses.

use std::hint::black_box;
use std::sync::Arc;

use arrow_array::ArrayRef;
use criterion::measurement::WallTime;
use criterion::{BenchmarkGroup, BenchmarkId};
use yggdryl::{ArrowCastOptions, DataType, Field, Scalar, Serie};

/// The seven doors a leaf answers with nothing but itself and one value.
pub(crate) fn leaf_doors(
    group: &mut BenchmarkGroup<'_, WallTime>,
    dtype: &DataType,
    sample: &Scalar,
) {
    let label = dtype.to_string();
    assert_eq!(
        &DataType::from_str(&label).expect("the canonical spelling parses"),
        dtype,
        "the canonical spelling reads back to the leaf"
    );
    let field = Field::new("value", dtype.clone(), true);
    let projected = field
        .clone()
        .into_arrow_field()
        .expect("the benchmark field is valid");
    // A leaf Arrow has no storage for - `duration32` - reads back as the
    // storage it crossed in, so what is held is that the crossing is a fixed
    // point: the imported field projects to the very Arrow field it came
    // from, document and all.
    let imported = Field::from_arrow_field(&projected).expect("the projection imports");
    assert_eq!(
        imported
            .clone()
            .into_arrow_field()
            .expect("the imported field is valid"),
        projected,
        "the imported field projects to the Arrow field it came from"
    );
    dtype
        .scalar(sample.clone())
        .expect("the sample is a value of the leaf");

    group.bench_function(BenchmarkId::new("parse", &label), |bencher| {
        bencher
            .iter(|| DataType::from_str(black_box(&label)).expect("the canonical spelling parses"));
    });
    group.bench_function(BenchmarkId::new("display", &label), |bencher| {
        bencher.iter(|| black_box(dtype).to_string());
    });
    group.bench_function(BenchmarkId::new("arrow_projection", &label), |bencher| {
        bencher.iter(|| {
            black_box(dtype)
                .clone()
                .into_arrow_datatype()
                .expect("the leaf projects")
        });
    });
    group.bench_function(BenchmarkId::new("field", &label), |bencher| {
        bencher.iter(|| Field::new(black_box("value"), black_box(dtype).clone(), true));
    });
    // The projection a warmed field shares: one `Arc` clone once the cold
    // projection has been paid, which the assertion above did.
    group.bench_function(
        BenchmarkId::new("field_arrow_projection", &label),
        |bencher| {
            bencher.iter(|| {
                black_box(&field)
                    .clone()
                    .into_arrow_field()
                    .expect("the benchmark field is valid")
            });
        },
    );
    group.bench_function(BenchmarkId::new("field_arrow_import", &label), |bencher| {
        bencher.iter(|| {
            Field::from_arrow_field(black_box(&projected)).expect("the projection imports")
        });
    });
    group.bench_function(BenchmarkId::new("scalar", &label), |bencher| {
        bencher.iter(|| {
            black_box(dtype)
                .scalar(black_box(sample).clone())
                .expect("the sample is a value of the leaf")
        });
    });
}

/// One column cast into a field of the leaf, strictly: the reader's door.
///
/// The caller sets the group's throughput to the column's rows.
pub(crate) fn ingest(
    group: &mut BenchmarkGroup<'_, WallTime>,
    dtype: &DataType,
    source: &ArrayRef,
) {
    let label = dtype.to_string();
    let target = Field::new("value", dtype.clone(), true);
    let strict = ArrowCastOptions::new().with_safe(false);
    Serie::from_arrow_array(Some(&target), Arc::clone(source), strict)
        .expect("every cell of the column is a value of the leaf")
        .require_arrow_array()
        .expect("a cast column has a layout");
    group.bench_function(BenchmarkId::new("ingest", &label), |bencher| {
        bencher.iter(|| {
            Serie::from_arrow_array(
                Some(black_box(&target)),
                Arc::clone(black_box(source)),
                strict,
            )
            .expect("every cell of the column is a value of the leaf")
            .require_arrow_array()
            .expect("a cast column has a layout")
        });
    });
}
