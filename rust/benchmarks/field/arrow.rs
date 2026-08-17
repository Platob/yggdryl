use std::hint::black_box;

use criterion::{BatchSize, Criterion};

use super::nested_field;

pub fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("arrow");
    group.bench_function("field_projection_cold", |bencher| {
        bencher.iter_batched(
            nested_field,
            |field| field.to_arrow_ref().expect("the benchmark field is valid"),
            BatchSize::SmallInput,
        );
    });
    group.bench_function("field_projection_cached", |bencher| {
        let field = nested_field();
        field.to_arrow_ref().expect("the benchmark field is valid");
        bencher.iter(|| {
            black_box(&field)
                .to_arrow_ref()
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
                .to_arrow_ffi()
                .expect("the benchmark field is valid")
        });
    });
    group.bench_function("field_ffi_projection_cached_arrow", |bencher| {
        let field = nested_field();
        field.to_arrow_ref().expect("the benchmark field is valid");
        bencher.iter(|| {
            black_box(&field)
                .to_arrow_ffi()
                .expect("the cached benchmark field remains valid")
        });
    });
    group.finish();
}
