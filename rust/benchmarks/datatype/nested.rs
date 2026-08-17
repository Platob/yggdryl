use std::hint::black_box;

use criterion::Criterion;
use yggdryl::{DataType, Field};

use super::NESTED_SQL;

pub(crate) fn value_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("value");
    let data_type = DataType::from_str(NESTED_SQL).expect("static nested type must parse");

    group.bench_function("nested_datatype_clone", |bencher| {
        bencher.iter(|| black_box(&data_type).clone());
    });
    group.bench_function("datatype_stable_hash", |bencher| {
        bencher.iter(|| black_box(&data_type).stable_hash());
    });
    group.bench_function("nested_validate", |bencher| {
        bencher.iter(|| {
            black_box(&data_type)
                .validate()
                .expect("the benchmark datatype remains valid")
        });
    });
    let wide_fields = (0..1_024)
        .map(|index| Field::new(format!("column_{index:04}"), DataType::Int64, false))
        .collect::<Vec<_>>();
    group.bench_function("struct_from_fields_1024", |bencher| {
        bencher.iter(|| {
            DataType::from_fields(black_box(wide_fields.clone()))
                .expect("the generated field names are unique")
        });
    });
    let variant_fields = (0..128)
        .map(|index| Field::new(format!("member_{index:03}"), DataType::Int64, true))
        .collect::<Vec<_>>();
    group.bench_function("variant_from_fields_128", |bencher| {
        bencher.iter(|| {
            DataType::variant(black_box(variant_fields.clone()))
                .expect("128 members fit Arrow union type IDs")
        });
    });
    group.finish();
}
