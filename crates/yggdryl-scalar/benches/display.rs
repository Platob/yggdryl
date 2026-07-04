//! Benchmarks for the pretty `display()`: the atomic value form, a serie table
//! (only the first `max_rows` are formatted, so a big serie is not fully walked), a
//! recursive struct-serie table, and a data-type signature.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use yggdryl_scalar::yggdryl_dtype::{self as dtype, arrow_schema, DataType};
use yggdryl_scalar::{AnyScalar, Int64Scalar, Int64Serie, RecordScalar, Scalar, TypedStructSerie};

const N: usize = 4096;

fn display(c: &mut Criterion) {
    let mut group = c.benchmark_group("display");

    // The atomic value form: one small string.
    let scalar = Int64Scalar::new(-1234);
    group.bench_function("scalar_display", |b| b.iter(|| black_box(scalar.display())));

    // A serie table over a 4096-element serie — only the default 10 rows are
    // formatted, so this measures the header + fixed row budget, not the whole serie.
    let numbers = Int64Serie::from((0..N as i64).collect::<Vec<_>>());
    group.bench_function("serie_display", |b| b.iter(|| black_box(numbers.display())));

    // A struct serie: one column per field, still only the first rows formatted.
    let point_type = dtype::StructType::new(arrow_schema::Fields::from(vec![
        arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
        arrow_schema::Field::new("y", arrow_schema::DataType::Int64, false),
    ]));
    let rows: Vec<RecordScalar> = (0..N as i64)
        .map(|value| {
            RecordScalar::new(
                point_type.clone(),
                vec![
                    AnyScalar::from(Int64Scalar::new(value)),
                    AnyScalar::from(Int64Scalar::new(value + 1)),
                ],
            )
            .unwrap()
        })
        .collect();
    let points = TypedStructSerie::new(point_type.clone(), rows);
    group.bench_function("struct_serie_display", |b| {
        b.iter(|| black_box(points.display()))
    });

    // The recursive data-type signature.
    let nested = dtype::SerieType::new(point_type.to_arrow());
    group.bench_function("dtype_signature", |b| {
        b.iter(|| black_box(nested.display()))
    });

    group.finish();
}

criterion_group!(benches, display);
criterion_main!(benches);
