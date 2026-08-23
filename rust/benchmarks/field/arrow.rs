use std::hint::black_box;

use criterion::{BatchSize, Criterion};
use yggdryl::Field;

use super::nested_field;

pub fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("arrow");
    group.bench_function("field_projection_cold", |bencher| {
        bencher.iter_batched(
            nested_field,
            |field| {
                field
                    .into_arrow_ref()
                    .expect("the benchmark field is valid")
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("field_projection_cached", |bencher| {
        let field = nested_field();
        field
            .clone()
            .into_arrow_ref()
            .expect("the benchmark field is valid");
        bencher.iter(|| {
            black_box(&field)
                .clone()
                .into_arrow_ref()
                .expect("the cached benchmark field remains valid")
        });
    });
    group.bench_function("field_projection_consuming", |bencher| {
        bencher.iter_batched(
            nested_field,
            |field| field.into_arrow().expect("the benchmark field is valid"),
            BatchSize::SmallInput,
        );
    });
    group.bench_function("field_ffi_projection_native", |bencher| {
        let field = nested_field();
        bencher.iter(|| {
            black_box(&field)
                .clone()
                .into_arrow_ffi()
                .expect("the benchmark field is valid")
        });
    });
    group.bench_function("field_ffi_projection_cached_arrow", |bencher| {
        let field = nested_field();
        field
            .clone()
            .into_arrow_ref()
            .expect("the benchmark field is valid");
        bencher.iter(|| {
            black_box(&field)
                .clone()
                .into_arrow_ffi()
                .expect("the cached benchmark field remains valid")
        });
    });
    group.bench_function("struct_field_to_arrow_schema", |bencher| {
        let field = nested_field();
        bencher.iter(|| {
            black_box(&field)
                .clone()
                .into_arrow_schema()
                .expect("the benchmark struct field is valid")
        });
    });
    group.bench_function("struct_field_to_arrow_exchange_schema", |bencher| {
        let field = nested_field();
        bencher.iter(|| {
            black_box(&field)
                .clone()
                .into_arrow_exchange_schema()
                .expect("the benchmark struct field is exchangeable")
        });
    });
    group.bench_function("struct_field_from_arrow_schema", |bencher| {
        let schema = nested_field()
            .into_arrow_exchange_schema()
            .expect("the benchmark struct field is exchangeable");
        bencher.iter(|| {
            Field::from_arrow_schema("row", black_box(&schema))
                .expect("the benchmark Arrow schema remains importable")
        });
    });
    group.finish();
}
