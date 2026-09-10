use std::hint::black_box;

use criterion::{BatchSize, Criterion};
use yggdryl::types::{Int64Field, StructField, integer};
use yggdryl::{DataType, Field, Scalar, TypedRecord};

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

    // The typed row view over a 16-column canonical row: building it is one
    // allocation, a name lookup and the cells are borrowed, and the sequence
    // it collapses back into is the one allocation a row build pays.
    let mut group = criterion.benchmark_group("typed/record");
    let columns = 16;
    let root = DataType::from_fields((0..columns).map(|index| {
        let name = format!("column_{index}");
        if index % 2 == 0 {
            DataType::Int64.required_field(name)
        } else {
            DataType::Utf8.nullable_field(name)
        }
    }))
    .expect("the benchmark row schema is valid")
    .required_field("row");
    let row = root
        .canonicalize_value(Scalar::from_sequence((0..columns).map(|index| {
            if index % 2 == 0 {
                Scalar::from(i64::from(index))
            } else {
                Scalar::from("XNAS")
            }
        })))
        .expect("the benchmark row satisfies its schema");
    let record = TypedRecord::new(&root, row.clone()).expect("the benchmark row is typed");
    group.bench_function("new", |bencher| {
        bencher.iter(|| TypedRecord::new(black_box(&root), black_box(&row).clone()).unwrap());
    });
    group.bench_function("get_by_name", |bencher| {
        bencher.iter(|| {
            black_box(&record)
                .get_by_name(black_box("column_15"))
                .unwrap()
        });
    });
    group.bench_function("into_scalar", |bencher| {
        bencher.iter_batched(
            || record.clone(),
            |record| black_box(record.into_scalar()),
            BatchSize::SmallInput,
        );
    });
    group.finish();
}
