use std::hint::black_box;

use criterion::{BatchSize, Criterion};
use yggdryl::DataType;

use super::NESTED_SQL;

pub(crate) fn arrow_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("arrow");
    group.bench_function("datatype_projection", |bencher| {
        let data_type = DataType::from_str(NESTED_SQL).expect("static nested type must parse");
        bencher.iter(|| {
            black_box(&data_type)
                .clone()
                .into_arrow()
                .expect("the benchmark datatype is valid")
        });
    });
    group.bench_function("datatype_ffi_projection", |bencher| {
        let data_type = DataType::from_str(NESTED_SQL).expect("static nested type must parse");
        bencher.iter(|| {
            black_box(&data_type)
                .clone()
                .into_arrow_ffi()
                .expect("the benchmark datatype is valid")
        });
    });
    group.bench_function("datatype_import_borrowed", |bencher| {
        let arrow = DataType::from_str(NESTED_SQL)
            .expect("static nested type must parse")
            .into_arrow()
            .expect("static nested type must project");
        bencher.iter(|| {
            DataType::from_arrow(black_box(&arrow))
                .expect("the benchmark Arrow datatype remains valid")
        });
    });
    group.bench_function("datatype_import_owned", |bencher| {
        let arrow = DataType::from_str(NESTED_SQL)
            .expect("static nested type must parse")
            .into_arrow()
            .expect("static nested type must project");
        bencher.iter_batched(
            || arrow.clone(),
            |arrow| DataType::try_from(arrow).expect("the owned Arrow datatype remains valid"),
            BatchSize::SmallInput,
        );
    });
    group.finish();
}
