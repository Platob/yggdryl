use std::hint::black_box;

use criterion::Criterion;
use yggdryl::{DataType, Field, Scheme};

pub(crate) fn default_and_compatibility_benchmarks(criterion: &mut Criterion) {
    let nested = DataType::from_fields([
        Field::new("id", DataType::Int64, false),
        Field::new("label", DataType::Utf8, true),
        Field::new(
            "items",
            DataType::fixed_size_list(Field::new("item", DataType::Int32, true), 32).unwrap(),
            false,
        ),
    ])
    .unwrap();
    let fixed =
        DataType::fixed_size_list(Field::new("item", DataType::Int64, false), 1_024).unwrap();

    let mut defaults = criterion.benchmark_group("datatype_default");
    defaults.bench_function("scalar", |bencher| {
        bencher.iter(|| black_box(DataType::Int64.default_value().unwrap()));
    });
    defaults.bench_function("nested_struct", |bencher| {
        bencher.iter(|| black_box(&nested).default_value().unwrap());
    });
    defaults.bench_function("fixed_list_1024", |bencher| {
        bencher.iter(|| black_box(&fixed).default_value().unwrap());
    });
    defaults.finish();

    let arrow_wide = DataType::from_fields((0..1_024).map(|index| {
        Field::new(
            format!("column_{index:04}"),
            if index == 1_023 {
                DataType::UInt8
            } else {
                DataType::Int64
            },
            false,
        )
    }))
    .unwrap();
    let spark_wide = DataType::from_fields(
        (0..1_024).map(|index| Field::new(format!("column_{index:04}"), DataType::Int64, false)),
    )
    .unwrap();
    let mut compatibility = criterion.benchmark_group("datatype_compatibility");
    compatibility.bench_function("arrow_noop_wide_struct", |bencher| {
        bencher.iter(|| {
            black_box(&spark_wide)
                .clone()
                .into_scheme_compat(&Scheme::ARROW)
                .unwrap()
        });
    });
    compatibility.bench_function("spark_noop_wide_struct", |bencher| {
        bencher.iter(|| {
            black_box(&spark_wide)
                .clone()
                .into_scheme_compat(&Scheme::SPARK)
                .unwrap()
        });
    });
    compatibility.bench_function("spark_changed_last_wide_struct", |bencher| {
        bencher.iter(|| {
            black_box(&arrow_wide)
                .clone()
                .into_scheme_compat(&Scheme::SPARK)
                .unwrap()
        });
    });
    compatibility.finish();
}
