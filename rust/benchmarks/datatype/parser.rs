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
    // The three extension-era spellings: bare `variant` is its own datatype
    // (the parenthesis is what selects the dense-union sugar instead), and
    // the geospatial pair parses its CRS and edge-algorithm parameters.
    group.bench_function("geospatial_scalars", |bencher| {
        bencher.iter(|| {
            for spelling in ["variant", "geometry", "geography('OGC:CRS84','vincenty')"] {
                DataType::from_str(black_box(spelling))
                    .expect("the static geospatial spellings must parse");
            }
        });
    });
    group.bench_function("geospatial_display_parse_round_trip", |bencher| {
        let spellings = ["variant", "geometry", "geography('OGC:CRS84','vincenty')"]
            .map(|spelling| DataType::from_str(spelling).expect("static spellings must parse"));
        bencher.iter(|| {
            for dtype in &spellings {
                let canonical = black_box(dtype).to_string();
                DataType::from_str(black_box(&canonical))
                    .expect("canonical display output must round-trip");
            }
        });
    });
    group.finish();
}
