use std::hint::black_box;

use criterion::{BatchSize, Criterion};
use yggdryl::types::{Int64Field, StructField, integer};
use yggdryl::{DataType, Field};

pub fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("typed/integer");
    group.bench_function("static_construct", |bencher| {
        bencher.iter(|| Int64Field::new(black_box("id"), black_box(false)));
    });
    group.bench_function("checked_borrow", |bencher| {
        let field = Field::new("id", DataType::Int64, false);
        bencher.iter(|| {
            black_box(&field)
                .try_as_typed::<integer::Int64Type>()
                .expect("the benchmark field has the checked marker")
        });
    });
    group.bench_function("into_field", |bencher| {
        bencher.iter_batched(
            || Int64Field::new("id", false),
            |field| black_box(field.into_field()),
            BatchSize::SmallInput,
        );
    });
    group.finish();

    let mut group = criterion.benchmark_group("typed/struct");
    let root = StructField::try_new(
        "row",
        DataType::from_fields([DataType::Int64.required_field("id")])
            .expect("the benchmark Struct datatype is valid"),
        false,
    )
    .expect("the benchmark Struct field is valid");
    group.bench_function("into_struct_field", |bencher| {
        bencher.iter_batched(
            || root.clone(),
            |field| black_box(field.into_struct_field()),
            BatchSize::SmallInput,
        );
    });
    group.finish();
}
