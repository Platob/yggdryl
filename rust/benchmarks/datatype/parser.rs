use std::hint::black_box;

use criterion::Criterion;
use yggdryl::DataType;

use super::NESTED_SQL;

pub(crate) fn parser_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("parse");
    let near_limit = format!(
        "{}int64{}",
        "array<".repeat(DataType::PARSE_RECURSION_LIMIT - 8),
        ">".repeat(DataType::PARSE_RECURSION_LIMIT - 8)
    );
    group.bench_function("scalar_sql", |bencher| {
        bencher.iter(|| {
            DataType::from_str(black_box("BIGINT")).expect("the static scalar benchmark must parse")
        });
    });
    group.bench_function("nested_sql_hive", |bencher| {
        bencher.iter(|| {
            DataType::from_str(black_box(NESTED_SQL))
                .expect("the static nested benchmark must parse")
        });
    });
    group.bench_function("near_limit_nested", |bencher| {
        bencher.iter(|| {
            DataType::from_str(black_box(&near_limit))
                .expect("the near-limit nested benchmark must parse")
        });
    });
    group.finish();
}
